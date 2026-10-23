//! A new version waits behind a review in two steps: each trust change
//! first, on its own, then the rest. It never applies without both, and only
//! for the version it was made for.

use std::collections::BTreeSet;

use serde_json::Value;

use super::file::{KEY_FILE, Version};
use crate::write::Protection;

/// What kind of trust a change moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TrustKind {
    /// A signing key's fingerprint.
    SigningKey,
    /// A protection level looser than before.
    LoosensProtection,
    /// Any other setting that decides where a sign-in, a token or a read goes,
    /// or how it is checked: every setting not on the safe list.
    Reach,
}

impl TrustKind {
    pub fn words(self) -> &'static str {
        match self {
            TrustKind::SigningKey => "Signing key",
            TrustKind::LoosensProtection => "Loosens protection",
            TrustKind::Reach => "Changes where a sign-in or read goes",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Changed,
    Removed,
}

/// One setting a version would set, change or stop setting.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub at: String,
    pub kind: ChangeKind,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub trust: Option<TrustKind>,
}

/// The two steps, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Trust,
    Rest,
}

/// Why a review does not apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotReviewed {
    /// A step is not confirmed.
    Step(Step),
    /// The review was made for another applied or waiting version.
    Stale,
}

/// The settings that move no trust: anything else that changes is a trust
/// change. Protection is judged by direction, not by name.
fn is_safe(key: &str) -> bool {
    let tail = key.split_once("].").map_or(key, |(_, rest)| rest);
    let cluster_marker = key.starts_with("clusters[") && key.ends_with(']');
    cluster_marker
        || matches!(
            tail,
            "role"
                | "coroot.project"
                | "coroot.cluster"
                | "metrics.value"
                | "metrics.label"
                | "delivery.build_namespace"
                | "delivery.stage_naming.name_template"
                | "delivery.stage_naming.project_key"
        )
}

fn trust_of(key: &str, before: Option<&Value>, after: Option<&Value>) -> Option<TrustKind> {
    if key.starts_with("fingerprint:") {
        return Some(TrustKind::SigningKey);
    }
    if key.starts_with("clusters[") && key.ends_with(".protection") {
        let level = |v: Option<&Value>| -> Protection {
            v.and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default()
        };
        return (level(after) > level(before)).then_some(TrustKind::LoosensProtection);
    }
    (!is_safe(key)).then_some(TrustKind::Reach)
}

/// All keys a version sets, with signing key fingerprints as `fingerprint:path`.
fn flat(version: Option<&Version>) -> std::collections::BTreeMap<String, Value> {
    let mut all = std::collections::BTreeMap::new();
    if let Some(version) = version {
        all.extend(version.values.clone());
        for (path, print) in &version.key_fingerprints {
            all.insert(format!("fingerprint:{path}"), Value::String(print.clone()));
        }
    }
    all.remove(KEY_FILE);
    all
}

#[derive(Clone, Debug, PartialEq)]
pub struct Review {
    changes: Vec<Change>,
    /// The applied version this was made against, and the one it is for.
    base: Option<String>,
    target: Version,
    /// The trust changes the person has confirmed, each on its own.
    confirmed: BTreeSet<String>,
    rest_done: bool,
}

impl Review {
    /// Only a [`super::Membership`] makes a review, of its waiting version
    /// against its applied one.
    pub(super) fn new(applied: Option<&Version>, pending: Version) -> Self {
        let before = flat(applied);
        let after = flat(Some(&pending));
        let mut changes = Vec::new();
        for key in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
            let (was, now) = (before.get(key), after.get(key));
            if was == now {
                continue;
            }
            let kind = match (was, now) {
                (None, _) => ChangeKind::Added,
                (_, None) => ChangeKind::Removed,
                _ => ChangeKind::Changed,
            };
            changes.push(Change {
                at: key.clone(),
                kind,
                before: was.cloned(),
                after: now.cloned(),
                trust: trust_of(key, was, now),
            });
        }
        // Trust changes first, then by key.
        changes.sort_by(|a, b| {
            b.trust
                .is_some()
                .cmp(&a.trust.is_some())
                .then(a.trust.cmp(&b.trust))
                .then(a.at.cmp(&b.at))
        });
        Self {
            changes,
            base: applied.map(|v| v.id.clone()),
            target: pending,
            confirmed: BTreeSet::new(),
            rest_done: false,
        }
    }

    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    /// The version this review is for.
    pub fn target_id(&self) -> &str {
        &self.target.id
    }

    /// The step the person is on: every trust change is confirmed on its own
    /// before the rest can be.
    pub fn step(&self) -> Step {
        if self.trust_pending().next().is_some() {
            Step::Trust
        } else {
            Step::Rest
        }
    }

    pub fn trust_changes(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.trust.is_some())
    }

    pub fn other_changes(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.trust.is_none())
    }

    /// The trust changes not yet confirmed.
    pub fn trust_pending(&self) -> impl Iterator<Item = &Change> {
        self.trust_changes()
            .filter(|c| !self.confirmed.contains(&c.at))
    }

    pub fn is_confirmed(&self, at: &str) -> bool {
        self.confirmed.contains(at)
    }

    /// Confirms one trust change. False when `at` is not a trust change.
    pub fn confirm_trust(&mut self, at: &str) -> bool {
        let known = self.trust_changes().any(|c| c.at == at);
        if known {
            self.confirmed.insert(at.to_string());
        }
        known
    }

    /// Confirms the rest of the diff. Refused while a trust change waits.
    pub fn confirm_rest(&mut self) -> bool {
        let ready = self.step() == Step::Rest;
        if ready {
            self.rest_done = true;
        }
        ready
    }

    pub fn is_finished(&self) -> bool {
        self.step() == Step::Rest && self.rest_done
    }

    /// The version, if every step is confirmed and this review is of the
    /// given applied and waiting versions.
    pub(super) fn apply(
        self,
        applied: Option<&Version>,
        pending: Option<&Version>,
    ) -> Result<Version, NotReviewed> {
        if self.base.as_deref() != applied.map(|v| v.id.as_str()) || pending != Some(&self.target) {
            return Err(NotReviewed::Stale);
        }
        if self.step() == Step::Trust {
            return Err(NotReviewed::Step(Step::Trust));
        }
        if !self.rest_done {
            return Err(NotReviewed::Step(Step::Rest));
        }
        Ok(self.target)
    }
}
