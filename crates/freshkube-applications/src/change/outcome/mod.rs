//! What Coroot found after a deploy, on the change page
//! (`docs/COROOT.md#what-its-metrics-did-after-it`): an **After the deploy**
//! section in the Inspector of a Deployment hop, and a small mark on the
//! hop once its outcome is read.
//!
//! The page owns one read. It starts only for the selected hop and only
//! while the page shows, carries the identity it was asked for (the hop,
//! its entry's link, the Deployment, its hash, the window, the connection)
//! and a request generation, and is dropped when the page hides or another
//! hop is selected. A late answer, or an answer that is not for the
//! question, never lands. An answer is kept per hop while its question
//! stays the same; editing a link, or a refreshed change that names
//! another revision, clears it, and a failed read over an answer for the
//! same question keeps that answer, marked stale.
//!
//! Nothing here judges: Coroot's status and findings are its own words and
//! no state reads as fine.
mod example;
mod section;
mod session;

use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use freshkube_core::coroot as api;
use freshkube_core::workspace::Entry;
use freshkube_delivery::change::{Change, Object, Shows, Target};
use freshkube_observability::{After, OutcomeReader};
use freshkube_reads::Loader;
use freshkube_ui::ui::Tone;
use gpui_kit::*;

use super::{ChangePage, Fetch};

pub use example::Kind;
#[cfg(any(test, feature = "testing"))]
pub(super) use section::Act;
pub(super) use section::Section;
pub(crate) use session::{SessionMemo, session as session_memo};

/// An outcome read this recent is shown again without reading.
const FRESH_FOR: Duration = Duration::from_secs(30);

/// Why an entry has no Coroot link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unlinked {
    /// The entry names no Coroot cluster.
    NotLinked,
    /// It names one, and neither it nor the connection names a project.
    NoProject,
}

/// Each workspace entry's link to a Coroot project and cluster, or why it
/// has none (`api::Link::of`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Links(BTreeMap<String, Result<api::Link, Unlinked>>);

impl Links {
    /// The links of `entries`, with the connection's project as theirs
    /// when they name none.
    pub fn of(entries: &[Entry], connection_project: Option<&str>) -> Self {
        Self(
            entries
                .iter()
                .map(|entry| {
                    let link = api::Link::of(entry, connection_project).map_err(|why| match why {
                        api::Outcome::NoProjectForEntry => Unlinked::NoProject,
                        _ => Unlinked::NotLinked,
                    });
                    (entry.id.clone(), link)
                })
                .collect(),
        )
    }

    pub(crate) fn of_entry(&self, entry: &str) -> Result<&api::Link, Unlinked> {
        match self.0.get(entry) {
            Some(Ok(link)) => Ok(link),
            Some(Err(why)) => Err(*why),
            None => Err(Unlinked::NotLinked),
        }
    }
}

/// What the shell hands the page of Coroot: Observability's connection and
/// each entry's link.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Live {
    pub reader: Option<OutcomeReader>,
    pub links: Links,
}

/// Where a page's outcomes come from.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Coroot {
    /// Nothing was handed over: the page has no section.
    #[default]
    None,
    /// Example data's answers, or the one outcome every hop shows.
    Example(Option<Kind>),
    Live(Live),
}

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
        kind: Option<Kind>,
    },
}

impl Identity {
    fn provider(&self) -> Option<api::ProviderId> {
        match self {
            Self::Live(key) => key.provider,
            Self::Example { .. } => None,
        }
    }

    pub(crate) fn example(
        change: &Change,
        hop: &str,
        kind: Option<Kind>,
        window: api::RevisionWindow,
    ) -> Option<Self> {
        let ask = ask_of(change, hop)?;
        if ask.object.kind != "Deployment" {
            return None;
        }
        let (deployment, hash) = (ask.object, ask.hash);
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

/// An outcome kept for a hop while its question stays the same.
struct Memo {
    /// What was asked; none for example data's, which asks nothing.
    query: Option<api::Query>,
    window: api::RevisionWindow,
    /// The connection it was read over.
    provider: Option<api::ProviderId>,
    outcome: api::Outcome,
    after: Option<Rc<After>>,
    read: DateTime<Utc>,
    at: Instant,
    /// A later read of the same question failed; the answer is the last.
    stale: Option<String>,
}

/// The page's outcome state.
pub(super) struct Outcomes {
    pub(super) coroot: Coroot,
    window: api::RevisionWindow,
    /// The read in flight, which owns its job, its delivery and its
    /// generation; what it read is kept in `memo`.
    read: Loader<(), Key>,
    /// The question in flight, with the hop it is for.
    in_flight: Option<Key>,
    memo: BTreeMap<String, Memo>,
    /// The project's applications, for the next look of the same link while
    /// the page shows and for [`FRESH_FOR`].
    listed: Option<(api::Listed, Instant)>,
    /// The connection the last read or look for one found (`Some(None)`:
    /// none), so a `Connection` event that changes nothing reads nothing.
    seen: Option<Option<api::ProviderId>>,
    /// A look found no connection; read again when it changes.
    no_connection: bool,
    _connection: Option<Subscription>,
    /// Says the page no longer wants Coroot when it is dropped.
    _release: Option<Subscription>,
}

impl Default for Outcomes {
    fn default() -> Self {
        Self {
            coroot: Coroot::None,
            window: api::RevisionWindow::default(),
            read: Loader::default(),
            in_flight: None,
            memo: BTreeMap::new(),
            listed: None,
            seen: None,
            no_connection: false,
            _connection: None,
            _release: None,
        }
    }
}

/// A hop's Coroot question, from its Revision action.
struct Ask {
    object: Object,
    hash: Option<String>,
    /// Why delivery doesn't offer the revision, when it doesn't.
    why: Option<String>,
}

fn ask_of(change: &Change, hop: &str) -> Option<Ask> {
    let Shows::Hop(detail) = &change.hop(hop)?.shows else {
        return None;
    };
    detail
        .actions
        .iter()
        .find_map(|action| match &action.target {
            Target::Revision { deployment, hash } => Some(Ask {
                object: deployment.clone(),
                hash: hash.clone(),
                why: action.disabled.clone(),
            }),
            _ => None,
        })
}

impl ChangePage {
    /// The page's source of outcomes, as the shell hands it. Answers of
    /// another source, link or kind are not this one's: they are cleared.
    pub fn set_coroot(&mut self, coroot: Coroot, cx: &mut Context<Self>) {
        if self.outcomes.coroot == coroot {
            return;
        }
        let reader_of = |coroot: &Coroot| match coroot {
            Coroot::Live(live) => live.reader.clone(),
            _ => None,
        };
        let (old, new) = (reader_of(&self.outcomes.coroot), reader_of(&coroot));
        self.outcomes.coroot = coroot;
        self.outcomes.forget();
        if old != new {
            // The old reader's connection isn't wanted by this page any more.
            if let Some(old) = old {
                old.want(cx.entity_id(), false, cx);
            }
            self.outcomes._connection = new.as_ref().and_then(|reader| {
                reader.on_connection(cx, |page, cx| page.connection_changed(cx))
            });
            let id = cx.entity_id();
            self.outcomes._release = new
                .clone()
                .map(|reader| cx.on_release(move |_, cx| reader.want(id, false, cx)));
        }
        // Shown, the page wants Coroot connected; hidden, it doesn't.
        if let Some(reader) = &new {
            reader.want(cx.entity_id(), self.visible, cx);
        }
        self.detail = self.derive_detail();
        self.sync_marks();
        self.read_outcome(false, cx);
        cx.notify();
    }

    /// The connection started, finished, failed or dropped: what was read
    /// over the old one is foreign, and a look that found none is asked again.
    fn connection_changed(&mut self, cx: &mut Context<Self>) {
        // Only a different connection (or none) makes what was read foreign.
        if let Coroot::Live(Live {
            reader: Some(reader),
            ..
        }) = &self.outcomes.coroot
            && self.outcomes.seen == Some(reader.connection(cx).map(|c| c.id()))
        {
            return;
        }
        self.outcomes.forget();
        self.detail = self.derive_detail();
        self.sync_marks();
        self.read_outcome(false, cx);
        cx.notify();
    }

    /// The entry a hop's object is read on: the workspace entry the open
    /// connection is, else the object's cluster, which in a workspace is
    /// its entry's id.
    fn entry_of(&self, object: &Object) -> Option<String> {
        match &self.fetch {
            Fetch::Live { place, .. } if place.cluster == object.cluster => {
                place.mapping.open.clone()
            }
            _ => Some(object.cluster.clone()),
        }
    }

    /// What a hop would ask of Coroot, or the outcome that says why it
    /// asks nothing.
    fn query_of(&self, hop: &str) -> Option<Result<(api::Query, Ask), (api::Outcome, Ask)>> {
        let ask = ask_of(&self.change, hop)?;
        let Coroot::Live(live) = &self.outcomes.coroot else {
            return None;
        };
        if ask.object.kind == "Rollout" {
            return Some(Err((api::Outcome::NotDeployment, ask)));
        }
        if ask.why.is_some() || ask.hash.as_deref().is_none_or(str::is_empty) {
            return Some(Err((api::Outcome::NotLooked, ask)));
        }
        let link = match self
            .entry_of(&ask.object)
            .map(|entry| live.links.of_entry(&entry))
        {
            Some(Ok(link)) => link.clone(),
            Some(Err(Unlinked::NoProject)) => {
                return Some(Err((api::Outcome::NoProjectForEntry, ask)));
            }
            Some(Err(Unlinked::NotLinked)) | None => {
                return Some(Err((api::Outcome::NotLinked, ask)));
            }
        };
        let query = api::Query {
            link,
            workload: api::Workload::Deployment,
            namespace: ask.object.namespace.clone(),
            name: ask.object.name.clone(),
            hash: ask.hash.clone(),
        };
        Some(Ok((query, ask)))
    }

    /// Reads the selected hop's outcome, when it is a Deployment's and the
    /// page shows. Not again while what is kept is fresh, unless `force`.
    pub(super) fn read_outcome(&mut self, force: bool, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        let Some(hop) = self.selected.clone().map(|key| key.to_string()) else {
            return;
        };
        // One read at a time, for the selected hop: another hop's is dropped.
        if self
            .outcomes
            .in_flight
            .as_ref()
            .is_some_and(|key| key.hop != hop)
        {
            self.outcomes.read.reset();
            self.outcomes.in_flight = None;
        }
        match self.outcomes.coroot.clone() {
            Coroot::None => {}
            Coroot::Example(kind) => self.read_example(&hop, kind, force, cx),
            Coroot::Live(live) => self.read_live(&hop, &live, force, cx),
        }
    }

    fn read_live(&mut self, hop: &str, live: &Live, force: bool, cx: &mut Context<Self>) {
        let query = match self.query_of(hop) {
            Some(Ok((query, _))) => query,
            // Nothing is looked for: no read, and what the hop asked
            // before is no longer its answer.
            Some(Err(_)) => {
                self.outcomes.memo.remove(hop);
                self.outcomes.read.reset();
                self.outcomes.in_flight = None;
                self.detail = self.derive_detail();
                cx.notify();
                return;
            }
            None => return,
        };
        let connection = live
            .reader
            .as_ref()
            .and_then(|reader| reader.connection(cx));
        self.outcomes.seen = Some(connection.as_ref().map(|c| c.id()));
        let Some(connection) = connection else {
            self.outcomes.no_connection = true;
            self.outcomes.read.reset();
            self.outcomes.in_flight = None;
            self.detail = self.derive_detail();
            cx.notify();
            return;
        };
        self.outcomes.no_connection = false;
        let key = Key {
            hop: hop.to_owned(),
            query: query.clone(),
            window: self.outcomes.window,
            provider: Some(connection.id()),
        };
        let kept = self.outcomes.memo.get(hop);
        let fresh = kept.is_some_and(|memo| {
            memo.query.as_ref() == Some(&key.query)
                && memo.window == key.window
                && memo.provider == key.provider
                && memo.stale.is_none()
                && cx.background_executor().now().duration_since(memo.at) < FRESH_FOR
        });
        if !force && (fresh || self.outcomes.in_flight.as_ref() == Some(&key)) {
            return;
        }
        let request = self.outcomes.read.start(key.clone());
        self.outcomes.in_flight = Some(key.clone());
        let now = cx.background_executor().now();
        let listed = self
            .outcomes
            .listed
            .as_ref()
            .filter(|(_, at)| now.duration_since(*at) < FRESH_FOR)
            .map(|(listed, _)| listed.clone());
        let (job, receiver) = connection.look(query, listed, key.window, Utc::now());
        let task = cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err(api::ReadError::Failed.to_string()));
            _ = this.update(cx, |page, cx| page.landed(&request, key, result, cx));
        });
        self.outcomes.read.run(job, task);
        self.detail = self.derive_detail();
        cx.notify();
    }

    /// An answer for `key`, published only while it is the latest for it
    /// and is for the question asked.
    fn landed(
        &mut self,
        request: &freshkube_core::snapshot::Request<Key>,
        key: Key,
        result: Result<api::Answer, String>,
        cx: &mut Context<Self>,
    ) {
        if !self.outcomes.read.land(request, Ok(()), cx) {
            return;
        }
        self.outcomes.in_flight = None;
        // An answer over a connection that is not this reader's now is
        // foreign, whatever it says: read again over the current one.
        let current = match &self.outcomes.coroot {
            Coroot::Live(Live {
                reader: Some(reader),
                ..
            }) => reader.connection(cx).map(|c| c.id()),
            _ => None,
        };
        if current != key.provider {
            self.connection_changed(cx);
            return;
        }
        let outcome = match result {
            Ok(answer)
                if key
                    .provider
                    .is_some_and(|provider| answer.is_for(&key.query, key.window, provider)) =>
            {
                if let Some(listed) = answer.listed.clone() {
                    self.outcomes.listed = Some((listed, cx.background_executor().now()));
                }
                answer.outcome
            }
            // A look for another entry, link, window or connection is
            // never drawn as this one's: it is a failed read.
            Ok(_) => api::Outcome::Failed(api::ReadError::InvalidResponse),
            Err(_) => api::Outcome::Failed(api::ReadError::Failed),
        };
        self.keep(&key, outcome, cx);
        self.publish_outcome(Identity::Live(key.clone()), &key.hop, cx);
        self.detail = self.derive_detail();
        self.sync_marks();
        cx.notify();
    }

    /// Keeps `outcome` for the hop, or, for a failure over an answer to the
    /// same question, marks that answer stale.
    fn keep(&mut self, key: &Key, outcome: api::Outcome, cx: &mut Context<Self>) {
        let at = cx.background_executor().now();
        if let api::Outcome::Failed(error) = &outcome
            && let Some(memo) = self.outcomes.memo.get_mut(&key.hop)
            && memo.query.as_ref() == Some(&key.query)
            && memo.window == key.window
            && memo.provider == key.provider
            && !matches!(memo.outcome, api::Outcome::Failed(_))
        {
            memo.stale = Some(error.to_string());
            return;
        }
        let after = match &outcome {
            api::Outcome::Found(view) => Some((view.as_ref(), None)),
            api::Outcome::EndsEarly(view, at) => Some((view.as_ref(), Some(*at))),
            _ => None,
        }
        .map(|(view, ends)| Rc::new(After::new(view, ends, Utc::now(), cx)));
        self.outcomes.memo.insert(
            key.hop.clone(),
            Memo {
                query: Some(key.query.clone()),
                window: key.window,
                provider: key.provider,
                outcome,
                after,
                read: Utc::now(),
                at,
                stale: None,
            },
        );
    }

    /// Hand off only a completed read, already checked against the request
    /// and current provider. No charts or live work enter the session memo.
    fn publish_outcome(&self, identity: Identity, hop: &str, cx: &mut Context<Self>) {
        let Some(memo) = self.outcomes.memo.get(hop) else {
            return;
        };
        let words = match memo.after.as_deref() {
            Some(after) => after.status_word().into(),
            None => match self.section(hop) {
                Some(section) => section.words().into(),
                None => return,
            },
        };
        let summary = Summary {
            mark: memo.after.as_deref().map(Mark::of),
            words,
            stale: memo.stale.as_ref().map(|why| {
                format!(
                    "Showing the last read at {}. {why}",
                    super::clock(memo.read)
                )
                .into()
            }),
        };
        session_memo(cx).update(cx, |memo, cx| memo.keep(identity, summary, cx));
    }

    /// Drops what was read for a question the change no longer asks: a
    /// refreshed change that names another revision, or a link edited.
    pub(super) fn prune_outcomes(&mut self) {
        let asked: Vec<(String, Option<api::Query>)> = self
            .outcomes
            .memo
            .keys()
            .map(|hop| {
                let query = match self.query_of(hop) {
                    Some(Ok((query, _))) => Some(query),
                    _ => None,
                };
                (hop.clone(), query)
            })
            .collect();
        for (hop, query) in asked {
            let kept = &self.outcomes.memo[&hop];
            // Example data asks nothing, so only a live memo can be foreign.
            let foreign = matches!(self.outcomes.coroot, Coroot::Live(_))
                && (kept.query.is_none() || kept.query != query);
            if foreign {
                self.outcomes.memo.remove(&hop);
            }
        }
    }

    /// Sets each row's Coroot mark from what was read.
    pub(super) fn sync_marks(&mut self) {
        for row in &mut self.rows {
            row.coroot = self
                .outcomes
                .memo
                .get(row.key.as_ref())
                .and_then(|memo| memo.after.as_deref())
                .map(Mark::of);
        }
    }

    /// Retry: reads the selected hop again.
    pub(super) fn retry_outcome(&mut self, cx: &mut Context<Self>) {
        self.read_outcome(true, cx);
    }

    /// Tells Observability whether this page, shown or hidden, wants Coroot
    /// connected.
    pub(super) fn want_coroot(&self, wanted: bool, cx: &mut Context<Self>) {
        if let Coroot::Live(Live {
            reader: Some(reader),
            ..
        }) = &self.outcomes.coroot
        {
            reader.want(cx.entity_id(), wanted, cx);
        }
    }

    /// The outcome section hides the read: no work is kept for a hidden
    /// page.
    pub(super) fn hide_outcomes(&mut self) {
        self.outcomes.read.reset();
        self.outcomes.in_flight = None;
        // The applications list lasts for the page's shown life.
        self.outcomes.listed = None;
    }
}

impl Outcomes {
    /// Forgets what was read and any look in flight: the source, a link or
    /// the connection changed, so none of it is this question's.
    fn forget(&mut self) {
        self.read.reset();
        self.in_flight = None;
        self.memo.clear();
        self.listed = None;
        self.seen = None;
        self.no_connection = false;
    }
}

/// A hop's Coroot mark: Coroot's status glyph and what its tooltip says.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub tone: Tone,
    pub tooltip: SharedString,
}

impl Mark {
    fn of(after: &After) -> Self {
        let status = after.status_word();
        Self {
            tone: after.tone(),
            tooltip: match after.first_finding() {
                Some(finding) => format!("Coroot: {status} · {finding}"),
                None => format!("Coroot: {status}"),
            }
            .into(),
        }
    }
}

/// A look the test holds, for the page's tests.
#[cfg(any(test, feature = "testing"))]
impl ChangePage {
    /// The outcome section's state, in words, for tests.
    pub fn outcome_words(&self) -> Option<String> {
        self.detail.as_ref()?.after_words()
    }

    /// Reads the selected hop's outcome again, as Retry does.
    pub fn read_outcome_again(&mut self, cx: &mut Context<Self>) {
        self.retry_outcome(cx);
    }

    /// The outcome banner's tone and which button it carries (`retry`,
    /// `coroot` or `clusters`), when the section is a banner.
    pub fn outcome_banner(&self) -> Option<(Tone, Option<&'static str>)> {
        self.detail.as_ref()?.after_banner()
    }

    /// Whether an outcome read is in flight.
    pub fn outcome_reading(&self) -> bool {
        self.outcomes.read.is_loading()
    }

    /// The Coroot mark on a hop's row, if it has one.
    pub fn coroot_mark(&self, hop: &str) -> Option<(Tone, String)> {
        let row = self.rows.iter().find(|row| row.key.as_ref() == hop)?;
        row.coroot
            .as_ref()
            .map(|mark| (mark.tone, mark.tooltip.to_string()))
    }
}
