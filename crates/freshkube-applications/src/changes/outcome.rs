//! Session-only summaries of outcomes read by change pages. The list observes
//! this memo; looking up a summary never wants a connection or starts a read.
use freshkube_core::coroot as api;
use freshkube_delivery::change::{Change, Hop, Object, Phase, Shows, Target};
use gpui_kit::*;

use super::ChangesPage;
use crate::change::{Coroot, Mark, OutcomeKind};

/// Slice 4's exact read identity, shared by the read owner and the memo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    pub(crate) hop: String,
    pub(crate) query: api::Query,
    pub(crate) window: api::RevisionWindow,
    pub(crate) provider: Option<api::ProviderId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Identity {
    Live(Key),
    Example {
        hop: String,
        deployment: Object,
        hash: Option<String>,
        window: api::RevisionWindow,
        kind: Option<OutcomeKind>,
    },
}

impl Identity {
    fn of(
        change: &Change,
        hop: &Hop,
        coroot: &Coroot,
        open: (Option<&str>, Option<&str>),
        cx: &App,
    ) -> Option<Self> {
        match coroot {
            Coroot::None => None,
            Coroot::Example(kind) => Self::example(change, &hop.key, *kind, Default::default()),
            Coroot::Live(live) => {
                let (object, hash) = deployment(change, &hop.key)?;
                // A changed trail that can no longer ask this question must
                // not reuse an earlier verdict, even with the same hash.
                let Shows::Hop(detail) = &hop.shows else {
                    return None;
                };
                let readable = detail.actions.iter().any(|action| {
                    matches!(&action.target, Target::Revision { .. }) && action.disabled.is_none()
                }) && hash.as_deref().is_some_and(|hash| !hash.is_empty());
                if !readable {
                    return None;
                }
                let entry = if open.0 == Some(object.cluster.as_str()) {
                    open.1?
                } else {
                    object.cluster.as_str()
                };
                Some(Self::Live(Key {
                    hop: hop.key.clone(),
                    query: api::Query {
                        link: live.links.of_entry(entry).ok()?.clone(),
                        workload: api::Workload::Deployment,
                        namespace: object.namespace,
                        name: object.name,
                        hash,
                    },
                    window: Default::default(),
                    provider: live
                        .reader
                        .as_ref()
                        .and_then(|reader| reader.connection(cx))
                        .map(|connection| connection.id()),
                }))
            }
        }
    }

    pub(crate) fn example(
        change: &Change,
        hop: &str,
        kind: Option<OutcomeKind>,
        window: api::RevisionWindow,
    ) -> Option<Self> {
        let (deployment, hash) = deployment(change, hop)?;
        Some(Self::Example {
            hop: hop.to_owned(),
            deployment,
            hash,
            window,
            kind,
        })
    }
}

/// A status uses the change page's mark and word; other outcomes use its
/// complete sentence. Staleness describes the read, never a new verdict.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Summary {
    pub(crate) mark: Option<Mark>,
    pub(crate) words: SharedString,
    pub(crate) stale: Option<SharedString>,
}

#[derive(Default)]
pub(crate) struct Memo {
    answers: Vec<(Identity, Summary)>,
}

struct Session(Entity<Memo>);
impl Global for Session {}

pub(crate) fn session(cx: &mut App) -> Entity<Memo> {
    if let Some(session) = cx.try_global::<Session>() {
        return session.0.clone();
    }
    let memo = cx.new(|_| Memo::default());
    cx.set_global(Session(memo.clone()));
    memo
}

impl Memo {
    pub(crate) fn keep(&mut self, identity: Identity, summary: Summary, cx: &mut Context<Self>) {
        if let Some((_, kept)) = self.answers.iter_mut().find(|(key, _)| key == &identity) {
            *kept = summary;
        } else {
            self.answers.push((identity, summary));
        }
        cx.notify();
    }

    fn get(&self, identity: &Identity) -> Option<&Summary> {
        self.answers
            .iter()
            .find(|(key, _)| key == identity)
            .map(|(_, summary)| summary)
    }
}

fn deployment(change: &Change, hop: &str) -> Option<(Object, Option<String>)> {
    let Shows::Hop(detail) = &change.hop(hop)?.shows else {
        return None;
    };
    detail
        .actions
        .iter()
        .find_map(|action| match &action.target {
            Target::Revision { deployment, hash } if deployment.kind == "Deployment" => {
                Some((deployment.clone(), hash.clone()))
            }
            _ => None,
        })
}

/// One line for a Stage that has a Deployment. Derived when data or the
/// session memo changes, and kept apart from the row's Stages cell.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Line {
    pub(crate) stage: SharedString,
    pub(crate) summary: Option<Summary>,
}

impl ChangesPage {
    /// Re-evaluate against the current link and provider, also after a
    /// connection event while the list shows. No connection is requested.
    pub(super) fn sync_outcomes(&mut self, cx: &mut Context<Self>) {
        let memo = self.outcome_memo.read(cx);
        let Some(row) = self
            .display
            .rows
            .iter_mut()
            .find(|row| Some(&row.key) == self.selected.as_ref())
        else {
            return;
        };
        for cell in &mut row.stages {
            cell.after = None;
        }
        let Some(change) = self
            .loader
            .data()
            .and_then(|read| {
                read.changes
                    .rows
                    .iter()
                    .find(|raw| raw.key == row.key.as_ref())
            })
            .and_then(|row| row.trail.as_ref().ok())
        else {
            return;
        };
        if matches!(self.coroot, Coroot::None) {
            return;
        }
        for (stage_ix, stage) in change.stages.iter().enumerate() {
            let Some(hop) = change.hops.iter().find(|hop| {
                change.groups[hop.group].phase == Phase::Stage(stage_ix)
                    && deployment(change, &hop.key).is_some()
            }) else {
                continue;
            };
            let identity = Identity::of(
                change,
                hop,
                &self.coroot,
                (
                    self.source.as_ref().map(|source| source.id.as_str()),
                    self.across.mapping.open.as_deref(),
                ),
                cx,
            );
            if let Some(cell) = row
                .stages
                .iter_mut()
                .find(|cell| cell.stage.as_ref() == stage.name)
            {
                cell.after = Some(Line {
                    stage: stage.name.clone().into(),
                    summary: identity.as_ref().and_then(|key| memo.get(key)).cloned(),
                });
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
