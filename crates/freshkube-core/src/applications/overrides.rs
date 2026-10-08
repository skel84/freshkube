//! The user's override: rename, merge, split and hide, applied last. It is an
//! input here; keeping it in the workspace file is #44's. Ids are stable, so
//! what an override names survives a rename and a re-read, and a wish that
//! no longer matches anything is reported ([`Unmatched`]), not dropped.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{Application, ApplicationId, Basis, Evidence, MemberRef, Note, Rule, Unmatched};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Merge {
    pub from: Vec<ApplicationId>,
    pub into: ApplicationId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Split {
    pub member: MemberRef,
    /// The new application's name.
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Override {
    pub renames: BTreeMap<ApplicationId, String>,
    pub merges: Vec<Merge>,
    pub splits: Vec<Split>,
    pub hidden: BTreeSet<ApplicationId>,
}

type Apps = BTreeMap<ApplicationId, Application>;

pub(super) struct Applied {
    pub shown: Vec<Application>,
    pub hidden: Vec<Application>,
    pub unmatched: Vec<Unmatched>,
}

/// Splits, then merges, then renames, then hides.
pub(super) fn apply(mut apps: Apps, over: &Override) -> Applied {
    let mut unmatched = Vec::new();
    for split in &over.splits {
        if !split_off(&mut apps, split) {
            unmatched.push(Unmatched::Split(split.member.clone()));
        }
    }
    for merge in &over.merges {
        merge_into(&mut apps, merge, &mut unmatched);
    }
    for (id, name) in &over.renames {
        match apps.get_mut(id) {
            Some(app) if !name.trim().is_empty() => {
                app.name = name.trim().to_owned();
                app.evidence.push(Evidence::Override("rename"));
            }
            _ => unmatched.push(Unmatched::Rename(id.clone())),
        }
    }
    let mut hidden = Vec::new();
    for id in &over.hidden {
        match apps.remove(id) {
            Some(app) => hidden.push(app),
            None => unmatched.push(Unmatched::Hide(id.clone())),
        }
    }
    for app in apps.values_mut().chain(hidden.iter_mut()) {
        app.members.sort_by(|a, b| a.at.order().cmp(&b.at.order()));
    }
    Applied {
        shown: apps.into_values().collect(),
        hidden,
        unmatched,
    }
}

fn split_off(apps: &mut Apps, split: &Split) -> bool {
    let name = split.name.trim();
    if name.is_empty() {
        return false;
    }
    let Some((from, at)) = apps.iter().find_map(|(id, app)| {
        app.members
            .iter()
            .position(|m| m.at == split.member)
            .map(|at| (id.clone(), at))
    }) else {
        return false;
    };
    let source = apps.get_mut(&from).expect("found above");
    let mut member = source.members.remove(at);
    // What was said about the member goes with it.
    let (moved, kept) = std::mem::take(&mut source.notes)
        .into_iter()
        .partition::<Vec<_>, _>(|note| note.is_about(&split.member));
    source.notes = kept;
    member.basis = Basis::Override;
    let id = ApplicationId::new(Rule::Manual, name);
    let target = apps.entry(id.clone()).or_insert_with(|| Application {
        id,
        name: name.to_owned(),
        rule: Rule::Manual,
        evidence: vec![Evidence::Override("split")],
        members: Vec::new(),
        notes: Vec::new(),
    });
    target.members.push(member);
    target.notes.extend(moved);
    true
}

fn merge_into(apps: &mut Apps, merge: &Merge, unmatched: &mut Vec<Unmatched>) {
    if !apps.contains_key(&merge.into) {
        unmatched.push(Unmatched::MergeInto(merge.into.clone()));
        return;
    }
    let mut merged = Vec::new();
    let mut seen = BTreeSet::new();
    for id in merge
        .from
        .iter()
        .filter(|id| **id != merge.into && seen.insert((*id).clone()))
    {
        let Some(from) = apps.remove(id) else {
            unmatched.push(Unmatched::MergeFrom(id.clone()));
            continue;
        };
        let into = apps.get_mut(&merge.into).expect("checked above");
        for member in from.members {
            if !into.members.iter().any(|m| m.at == member.at) {
                into.members.push(member);
            }
        }
        for evidence in from.evidence {
            if !into.evidence.contains(&evidence) {
                into.evidence.push(evidence);
            }
        }
        into.notes.extend(from.notes);
        merged.push(id.clone());
    }
    if !merged.is_empty() {
        let into = apps.get_mut(&merge.into).expect("checked above");
        into.evidence.push(Evidence::Override("merge"));
        into.notes.push(Note::Merged { from: merged });
    }
}

impl Note {
    /// Whether the note is about this one member.
    fn is_about(&self, at: &MemberRef) -> bool {
        match self {
            Self::LowerClaim { member, .. }
            | Self::UnmappedDestination { member }
            | Self::ManagerUnknown { member } => member == at,
            _ => false,
        }
    }
}
