//! Session-only summaries of outcomes read by change pages. The list observes
//! this memo; looking up a summary never wants a connection or starts a read.
use std::collections::VecDeque;

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
    fn provider(&self) -> Option<api::ProviderId> {
        match self {
            Self::Live(key) => key.provider,
            Self::Example { .. } => None,
        }
    }

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
    answers: VecDeque<(Identity, Summary)>,
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

/// At most this many completed summaries survive in one app session.
const MAX_ANSWERS: usize = 256;

impl Memo {
    pub(crate) fn keep(&mut self, identity: Identity, summary: Summary, cx: &mut Context<Self>) {
        let provider = identity.provider();
        self.answers.retain(|(key, _)| key.provider() == provider);
        // A completed refresh is the newest summary, too.
        if let Some(index) = self.answers.iter().position(|(key, _)| key == &identity) {
            self.answers.remove(index);
        }
        self.answers.push_back((identity, summary));
        if self.answers.len() > MAX_ANSWERS {
            self.answers.pop_front();
        }
        cx.notify();
    }

    pub(crate) fn get(&self, identity: &Identity) -> Option<&Summary> {
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

/// The line's fixed copy; the view and derived labels use the same source.
pub(super) const AFTER_DEPLOY: &str = "After the deploy:";
pub(super) const UNREAD: &str = "After the deploy: open the change to read it";

/// One line for a Stage that has a Deployment. Derived when data or the
/// session memo changes, and kept apart from the row's Stages cell.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Line {
    pub(crate) id: ElementId,
    pub(crate) mark_id: ElementId,
    pub(crate) stale_id: ElementId,
    pub(crate) label: SharedString,
    pub(crate) summary: Option<Summary>,
}

impl Line {
    fn new(stage: &str, summary: Option<Summary>) -> Self {
        let id: ElementId = SharedString::from(format!("changes-after-{stage}")).into();
        let label = match &summary {
            Some(summary) => format!(
                "{AFTER_DEPLOY} {}{}",
                summary.words,
                if summary.stale.is_some() {
                    " · last read"
                } else {
                    ""
                }
            )
            .into(),
            None => UNREAD.into(),
        };
        Self {
            mark_id: (id.clone(), "mark").into(),
            stale_id: (id.clone(), "stale").into(),
            id,
            label,
            summary,
        }
    }
}

/// Derive just the selected Inspector's lines. A Stage with no Deployment
/// has no line; a changed or unavailable identity has the unread link.
fn lines(
    change: Option<&Change>,
    stages: &[super::display::Cell],
    coroot: &Coroot,
    open: (Option<&str>, Option<&str>),
    memo: &Memo,
    cx: &App,
) -> Vec<Option<Line>> {
    stages
        .iter()
        .map(|cell| {
            if matches!(coroot, Coroot::None) {
                return None;
            }
            let change = change?;
            let stage = change
                .stages
                .iter()
                .position(|stage| stage.name == cell.stage.as_ref())?;
            let hop = change.hops.iter().find(|hop| {
                change.groups[hop.group].phase == Phase::Stage(stage)
                    && deployment(change, &hop.key).is_some()
            })?;
            let identity = Identity::of(change, hop, coroot, open, cx);
            let summary = identity.as_ref().and_then(|key| memo.get(key)).cloned();
            Some(Line::new(cell.stage.as_ref(), summary))
        })
        .collect()
}

impl ChangesPage {
    /// Re-evaluate against the current link and provider, also after a
    /// connection event while the list shows. No connection is requested.
    pub(super) fn sync_outcomes(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self
            .display
            .rows
            .iter_mut()
            .find(|row| Some(&row.key) == self.selected.as_ref())
        else {
            return;
        };
        let change = self
            .loader
            .data()
            .and_then(|read| {
                read.changes
                    .rows
                    .iter()
                    .find(|raw| raw.key == row.key.as_ref())
            })
            .and_then(|row| row.trail.as_ref().ok())
            .map(AsRef::as_ref);
        let next = lines(
            change,
            &row.stages,
            &self.coroot,
            (
                self.source.as_ref().map(|source| source.id.as_str()),
                self.across.mapping.open.as_deref(),
            ),
            self.outcome_memo.read(cx),
            cx,
        );
        if row
            .stages
            .iter()
            .map(|cell| cell.after.as_ref())
            .eq(next.iter().map(Option::as_ref))
        {
            return;
        }
        for (cell, line) in row.stages.iter_mut().zip(next) {
            cell.after = line;
        }
        cx.notify();
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
