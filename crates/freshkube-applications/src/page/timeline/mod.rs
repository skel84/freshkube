//! An application's Timeline tab (#844): what happened to it in a window,
//! from every source that can say, as one list newest first, under a chip
//! for each source (its count, or why it wasn't read) and the window's
//! control.
//!
//! The merge, the retention marking and the quiet hours are delivery's
//! `timeline::derive`, which reads nothing. This owns the read: example
//! data answers at once, a live read lists the application's workloads'
//! Warning events, pods, ReplicaSets and nodes in the open cluster through a
//! reader that can only read (delivery's `timeline::live`). It reads only
//! while the tab shows: hiding drops the read, showing again reads once what
//! it has is 30 s old, and Refresh and a new window read again.
use std::time::Duration;

use chrono::Utc;
use freshkube_core::coroot as api;
use freshkube_delivery::entry;
use freshkube_delivery::timeline::example::{self, Shape};
use freshkube_delivery::timeline::incidents::{self, Absent};
use freshkube_delivery::timeline::live::{self, Place};
use freshkube_delivery::timeline::{self, Source, SourceRead, WINDOWS};
use freshkube_observability::OutcomeReader;
use freshkube_reads::{Loader, OnShow};
use freshkube_ui::table::{self as kit, TableState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::backend;
use crate::resources::ClusterAccess;

mod table;
mod view;

pub use table::Column;

/// The tab's id prefix: `timeline-chips`, `-list`, `-window`.
pub(super) const PREFIX: &str = "timeline";
/// What the tab reads again once it is this old, on showing.
const FRESH_FOR: Duration = Duration::from_secs(30);
const ON_SHOW: OnShow = OnShow::ReadIfOlder(FRESH_FOR);
/// How long one read may take before every source says it timed out.
const DEADLINE: Duration = Duration::from_secs(30);

/// What the Incidents read goes over: Observability's connection and the
/// open entry's Coroot link, or why there is none.
#[derive(Clone)]
pub struct CorootFeed {
    pub reader: Option<OutcomeReader>,
    pub link: Result<api::Link, Absent>,
}

/// Which provider (the connection) an Incidents read went over.
type IncidentIdentity = (String, u32, Option<api::ProviderId>);

/// Where the tab's sources come from.
#[derive(Clone)]
pub enum Fetch {
    /// Example data's, `FRESHKUBE_TIMELINE`'s shape.
    Example {
        shape: Shape,
        /// Only tests set it, so a read stays in flight with a real task.
        delay: Duration,
    },
    /// The open cluster's.
    Live {
        runtime: tokio::runtime::Handle,
        access: ClusterAccess,
        /// The connection the list reads, as its own loader keys it.
        source_id: String,
        place: Box<Place>,
        coroot: Box<CorootFeed>,
    },
}

impl Fetch {
    /// Which connection, application and workloads a read is for, so an
    /// answer for another never lands.
    fn identity(&self, app: &str) -> String {
        match self {
            Self::Example { shape, .. } => format!("example/{shape:?}/{app}"),
            Self::Live {
                source_id,
                place,
                coroot,
                ..
            } => live_identity(source_id, app, place, &coroot.link),
        }
    }

    fn cluster(&self) -> String {
        match self {
            Self::Example { .. } => String::new(),
            Self::Live { place, .. } => place.cluster.clone(),
        }
    }
}

/// Which connection, application and workloads a live read is for.
fn live_identity(
    source_id: &str,
    app: &str,
    place: &Place,
    link: &Result<api::Link, Absent>,
) -> String {
    format!("{source_id}/{app}/{place:?}/{link:?}")
}

/// One line of the list as the table draws it.
#[derive(Clone, Debug)]
pub struct TimelineRow {
    pub key: SharedString,
    pub tone: freshkube_ui::ui::Tone,
    pub time: SharedString,
    pub source: SharedString,
    pub what: SharedString,
    pub detail: SharedString,
    pub read_from: SharedString,
    /// Not an entry: a retention or quiet row, drawn dimmer.
    pub note: bool,
    pub label: SharedString,
}

/// A chip as drawn.
#[derive(Clone, Debug)]
pub struct TimelineChip {
    pub text: SharedString,
    pub tone: freshkube_ui::ui::Tone,
    pub label: SharedString,
    /// The source was read (in whole or in part), not refused or missing.
    pub read: bool,
    /// The element id, `timeline-chip-⟨source⟩`.
    pub id: SharedString,
}

pub struct TimelineView {
    app: SharedString,
    fetch: Fetch,
    loader: Loader<Vec<SourceRead>, (String, u32)>,
    /// The Incidents read: its own owner, since it goes over Observability's
    /// connection and not the cluster's. It reads and drops with `loader`.
    incidents: Loader<SourceRead, IncidentIdentity>,
    /// Hears Observability's connection start, finish or drop.
    connection: Option<Subscription>,
    hours: u32,
    /// The time example data is dated from: taken at its first read and
    /// kept, so a window or a refresh doesn't date it again and pass old
    /// lines for new ones.
    anchor: Anchor,
    /// Every source not read, for a first read that failed whole.
    fallback: Vec<SourceRead>,
    /// The banner for a refresh that failed over an earlier read: its text
    /// and label, derived with the answer.
    stale: Option<(SharedString, SharedString)>,
    /// A failure the next example read answers with, for tests.
    #[cfg(any(test, feature = "testing"))]
    fail_next: Option<String>,
    visible: bool,
    /// Derived when an answer lands.
    pub(super) chips: Vec<TimelineChip>,
    pub(super) rows: Vec<TimelineRow>,
    columns: Vec<Column>,
    width: f32,
    table: TableState,
    loading: kit::LoadingRows,
    loading_motion: Entity<kit::LoadingMotion>,
}

impl TimelineView {
    pub(super) fn new(app: &str, fetch: Fetch, cx: &mut Context<Self>) -> Self {
        let loading = kit::LoadingRows::new(PREFIX);
        let loading_motion = cx.new(|_| loading.motion(kit::Look::Pulse));
        let mut loader = Loader::default();
        loader.set_visible(false, ON_SHOW, cx);
        let mut incidents = Loader::default();
        incidents.set_visible(false, ON_SHOW, cx);
        let (columns, width) = table::columns(&[]);
        let mut this = Self {
            app: app.to_owned().into(),
            fetch,
            loader,
            incidents,
            connection: None,
            hours: WINDOWS[0],
            anchor: Anchor::default(),
            fallback: Vec::new(),
            stale: None,
            #[cfg(any(test, feature = "testing"))]
            fail_next: None,
            visible: false,
            chips: Vec::new(),
            rows: Vec::new(),
            columns,
            width,
            table: TableState::new(PREFIX),
            loading,
            loading_motion,
        };
        this.follow_connection(cx);
        this
    }

    /// Hears Observability's connection change, which asks Coroot again.
    fn follow_connection(&mut self, cx: &mut Context<Self>) {
        let reader = match &self.fetch {
            Fetch::Live { coroot, .. } => coroot.reader.clone(),
            Fetch::Example { .. } => None,
        };
        self.connection = reader.and_then(|reader| {
            reader.on_connection(cx, |this, cx| {
                if this.visible {
                    this.read_incidents(cx);
                }
            })
        });
    }

    /// Tells Observability whether this tab, shown or hidden, wants Coroot
    /// connected, as the change page does.
    fn want_coroot(&self, wanted: bool, cx: &mut Context<Self>) {
        if let Fetch::Live { coroot, .. } = &self.fetch
            && let Some(reader) = &coroot.reader
        {
            reader.want(cx.entity_id(), wanted, cx);
        }
    }

    pub(super) fn loading_motion(&self) -> Option<Entity<kit::LoadingMotion>> {
        kit::TableSource::loading(self)
            .is_some()
            .then(|| self.loading_motion.clone())
    }

    /// Where the sources come from now: another cluster's or application's
    /// answer is forgotten and read again if shown.
    pub(super) fn set_fetch(&mut self, fetch: Fetch, cx: &mut Context<Self>) {
        if fetch.identity(&self.app) == self.fetch.identity(&self.app) {
            self.fetch = fetch;
            self.follow_connection(cx);
            return;
        }
        self.want_coroot(false, cx);
        self.fetch = fetch;
        self.follow_connection(cx);
        self.anchor = Anchor::default();
        self.fallback.clear();
        self.loader.reset();
        self.incidents.reset();
        self.derive(cx);
        if self.visible {
            self.want_coroot(true, cx);
            self.read(cx);
        }
    }

    /// Shown, the tab reads unless what it has is fresh; hidden, it drops
    /// the read in flight.
    pub(super) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        let reads_again = self.loader.set_visible(visible, ON_SHOW, cx);
        // Example data answers Incidents with the rest; only a live read asks.
        let incidents_again = self.incidents.set_visible(visible, ON_SHOW, cx)
            && matches!(self.fetch, Fetch::Live { .. });
        self.want_coroot(visible, cx);
        if visible && (reads_again || incidents_again) {
            self.read(cx);
        }
        cx.notify();
    }

    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.visible {
            self.read(cx);
        }
    }

    /// Reads for another window: 3 h by default, up to 24 h. Events and
    /// the rest are asked for what it holds.
    pub(super) fn set_window(&mut self, hours: u32, cx: &mut Context<Self>) {
        if hours == self.hours || !WINDOWS.contains(&hours) {
            return;
        }
        self.hours = hours;
        self.loader.reset();
        self.incidents.reset();
        self.derive(cx);
        if self.visible {
            self.read(cx);
        }
    }

    pub(super) fn hours(&self) -> u32 {
        self.hours
    }

    fn read(&mut self, cx: &mut Context<Self>) {
        let hours = self.hours;
        let identity = (self.fetch.identity(&self.app), hours);
        let request = self.loader.start(identity);
        let cluster = self.fetch.cluster();
        match self.fetch.clone() {
            Fetch::Example { shape, delay } => {
                let now = self.anchor.at(Utc::now().timestamp());
                #[cfg(any(test, feature = "testing"))]
                let result = match self.fail_next.take() {
                    Some(why) => Err(why),
                    None => Ok(example::reads(now, shape)),
                };
                #[cfg(not(any(test, feature = "testing")))]
                let result = Ok(example::reads(now, shape));
                if delay.is_zero() {
                    self.answer(&request, result, &cluster, now, cx);
                } else {
                    let task = cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(delay).await;
                        _ = this.update(cx, |this, cx| {
                            this.answer(&request, result, &cluster, now, cx)
                        });
                    });
                    self.loader.run(None, task);
                }
            }
            Fetch::Live {
                runtime,
                access,
                place,
                ..
            } => {
                self.read_incidents(cx);
                let now = Utc::now().timestamp();
                let (job, receiver) = backend::spawn_job(
                    &runtime,
                    DEADLINE,
                    "Reading the timeline timed out".to_owned(),
                    async move {
                        let client = access.client().await.map_err(|failure| failure.message)?;
                        let reader = entry::reader(client);
                        Ok(live::read(&reader, &place, now, hours).await)
                    },
                );
                let task = cx.spawn(async move |this, cx| {
                    let result = receiver
                        .await
                        .unwrap_or_else(|_| Err("Reading the timeline stopped".into()));
                    _ = this.update(cx, |this, cx| {
                        this.answer(&request, result, &cluster, now, cx)
                    });
                });
                self.loader.run(Some(job), task);
            }
        }
        cx.notify();
    }

    /// Asks Coroot for the incidents of the application's workloads, over
    /// Observability's connection and the entry's link, only while shown.
    /// What can't be asked is answered at once as not read, with why.
    fn read_incidents(&mut self, cx: &mut Context<Self>) {
        let Fetch::Live { place, coroot, .. } = self.fetch.clone() else {
            return;
        };
        if !self.visible {
            return;
        }
        let hours = self.hours;
        let now = Utc::now().timestamp();
        let from = place.cluster.clone();
        let base = self.fetch.identity(&self.app);
        let connection = coroot
            .reader
            .as_ref()
            .and_then(|reader| reader.connection(cx));
        let identity = (base, hours, connection.as_ref().map(|c| c.id()));
        let link = match (&coroot.link, &connection) {
            (Ok(link), Some(_)) => link.clone(),
            (Err(absent), _) => {
                return self.settle_incidents(
                    identity,
                    &from,
                    "",
                    &place,
                    Err(absent.clone()),
                    now,
                    hours,
                );
            }
            (Ok(link), None) => {
                let cluster = link.cluster().to_owned();
                return self.settle_incidents(
                    identity,
                    &from,
                    &cluster,
                    &place,
                    Err(Absent::NotConnected),
                    now,
                    hours,
                );
            }
        };
        let Some(connection) = connection else {
            return;
        };
        let request = self.incidents.start(identity);
        let cluster = link.cluster().to_owned();
        let (job, receiver) = connection.incidents(
            link,
            hours,
            chrono::DateTime::from_timestamp(now, 0).unwrap_or_default(),
        );
        let task = cx.spawn(async move |this, cx| {
            let sample = match receiver.await {
                Ok(Ok(Ok(list))) => Ok(list),
                Ok(Ok(Err(why))) => Err(Absent::Failed(why)),
                Ok(Err(why)) => Err(Absent::Other(why)),
                Err(_) => Err(Absent::Other("Reading Coroot's incidents stopped".into())),
            };
            _ = this.update(cx, |this, cx| {
                let read = incidents::read(&from, &cluster, &place.workloads, sample, now, hours);
                if this.incidents.land(&request, Ok(read), cx) {
                    this.derive(cx);
                }
                cx.notify();
            });
        });
        self.incidents.run(job, task);
        self.derive(cx);
        cx.notify();
    }

    /// Answers the Incidents read without asking Coroot: it can't be asked.
    #[allow(clippy::too_many_arguments)]
    fn settle_incidents(
        &mut self,
        identity: IncidentIdentity,
        from: &str,
        cluster: &str,
        place: &Place,
        sample: Result<Vec<api::Incident>, Absent>,
        now: i64,
        hours: u32,
    ) {
        let read = incidents::read(from, cluster, &place.workloads, sample, now, hours);
        self.incidents.resolve(identity, Ok(read));
    }

    /// An answer, published only while it is the one in flight. A read that
    /// failed whole keeps the last answer, marked stale; with none, every
    /// source is not read, with why, never quiet.
    fn answer(
        &mut self,
        request: &freshkube_core::snapshot::Request<(String, u32)>,
        result: Result<Vec<SourceRead>, String>,
        cluster: &str,
        now: i64,
        cx: &mut Context<Self>,
    ) {
        let failure = result.as_ref().err().cloned();
        if self.loader.land(request, result, cx) {
            self.fallback = match failure {
                Some(why) if self.loader.data().is_none() => unread(cluster, now, &why),
                _ => Vec::new(),
            };
            self.derive(cx);
        }
        cx.notify();
    }

    /// The banner for a refresh that failed over an earlier read: what shows
    /// is that read, from when it was made.
    fn stale_banner(&self) -> Option<(SharedString, SharedString)> {
        let data = self.loader.data()?;
        let why = self.loader.error()?;
        let at = data.iter().map(|read| read.read_at).max()?;
        let text = format!(
            "Showing the last read, from {} UTC. {why}",
            timeline::clock(at)
        );
        let label = format!("Couldn't read again: {text}");
        Some((text.into(), label.into()))
    }

    pub(super) fn stale(&self) -> Option<(SharedString, SharedString)> {
        self.stale.clone()
    }

    /// The chips and the list for the answer and window, derived when they
    /// change.
    fn derive(&mut self, _: &mut Context<Self>) {
        self.stale = self.stale_banner();
        let reads = self
            .loader
            .data()
            .or(Some(&self.fallback).filter(|reads| !reads.is_empty()));
        let Some(reads) = reads else {
            self.chips.clear();
            self.rows.clear();
            (self.columns, self.width) = table::columns(&[]);
            return;
        };
        let now = reads
            .iter()
            .map(|read| read.read_at)
            .max()
            .unwrap_or_default();
        let merged;
        let reads = match self.incident_read() {
            Some(incidents) => {
                merged = reads
                    .iter()
                    .filter(|read| read.source != Source::Incidents)
                    .cloned()
                    .chain([incidents])
                    .collect::<Vec<_>>();
                &merged
            }
            None => reads,
        };
        let timeline = timeline::derive(reads, now, self.hours);
        self.chips = view::chips(&timeline);
        self.rows = table::rows(&timeline);
        (self.columns, self.width) = table::columns(&self.rows);
    }
}

impl TimelineView {
    /// The Incidents source as it stands: Coroot's answer, or not read while
    /// it is asked. `None` leaves the one the cluster read carries (example
    /// data's, or a placeholder).
    fn incident_read(&self) -> Option<SourceRead> {
        let Fetch::Live { place, .. } = &self.fetch else {
            return None;
        };
        match self.incidents.data() {
            Some(read) => Some(read.clone()),
            None if self.incidents.is_loading() => Some(SourceRead::not_read(
                Source::Incidents,
                &place.cluster,
                self.loader.data()?.iter().map(|r| r.read_at).max()?,
                "reading Coroot's incidents",
            )),
            None => None,
        }
    }
}

/// The time example data is dated from: the first read's, kept.
#[derive(Default)]
struct Anchor(Option<i64>);

impl Anchor {
    fn at(&mut self, wall: i64) -> i64 {
        *self.0.get_or_insert(wall)
    }
}

/// Every source not read, for a read that failed whole.
fn unread(cluster: &str, now: i64, why: &str) -> Vec<SourceRead> {
    Source::ALL
        .into_iter()
        .map(|source| SourceRead::not_read(source, cluster, now, why))
        .collect()
}

#[cfg(any(test, feature = "testing"))]
impl TimelineView {
    /// Each line: its time, source and words, or a note's.
    pub fn lines_text(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| format!("{} {} {}", row.time, row.source, row.what))
            .collect()
    }

    /// Each line's detail column.
    pub fn detail_text(&self) -> Vec<String> {
        self.rows.iter().map(|row| row.detail.to_string()).collect()
    }

    /// Each chip's words.
    pub fn chips_text(&self) -> Vec<String> {
        self.chips
            .iter()
            .map(|chip| chip.text.to_string())
            .collect()
    }

    pub fn is_reading(&self) -> bool {
        self.loader.is_loading()
    }

    /// Rises with every read begun or forgotten.
    pub fn read_revision(&self) -> u64 {
        self.loader.revision()
    }

    /// Reads from example data of another shape, slowly, as a source change
    /// does: the answer still in flight must not land.
    pub fn use_example(&mut self, shape: Shape, cx: &mut Context<Self>) {
        self.set_fetch(
            Fetch::Example {
                shape,
                delay: Duration::from_secs(5),
            },
            cx,
        );
    }

    /// Reads from `fetch`, as the shell's hand-over does.
    pub fn use_fetch(&mut self, fetch: Fetch, cx: &mut Context<Self>) {
        self.set_fetch(fetch, cx);
    }

    /// Each chip's label, with what was cut or why it wasn't read.
    pub fn chip_labels(&self) -> Vec<String> {
        self.chips
            .iter()
            .map(|chip| chip.label.to_string())
            .collect()
    }

    /// Reads again now.
    pub fn refresh_now(&mut self, cx: &mut Context<Self>) {
        self.refresh(cx);
    }

    /// The next example read fails whole with `why`.
    pub fn fail_next_read(&mut self, why: &str) {
        self.fail_next = Some(why.to_owned());
    }

    /// The stale banner's text, when there is one.
    pub fn stale_text(&self) -> Option<String> {
        self.stale.as_ref().map(|(text, _)| text.to_string())
    }

    pub fn answered(&self) -> bool {
        self.loader.data().is_some()
    }

    pub fn window_hours(&self) -> u32 {
        self.hours
    }
}

#[cfg(test)]
mod tests {
    use super::{Source, timeline, unread, view};

    #[test]
    fn a_live_read_is_keyed_by_the_connection_the_application_and_its_place() {
        use freshkube_core::workloads::WorkloadKind;
        use freshkube_delivery::timeline::live::{Place, Workload};
        let place = |name: &str, count: usize| Place {
            cluster: "core-fra".into(),
            workloads: vec![(
                "shop".into(),
                (0..count)
                    .map(|ix| {
                        let name = if ix == 0 { name } else { "worker" };
                        Workload::new(name, WorkloadKind::Deployment)
                    })
                    .collect(),
            )],
            ..Default::default()
        };
        let key = |source: &str, place: &Place| {
            super::live_identity(source, "checkout", place, &Err(super::Absent::NotLinked))
        };
        let base = key("conn-a", &place("api", 1));
        assert_ne!(base, key("conn-b", &place("api", 1)), "another connection");
        assert_ne!(
            base,
            key("conn-a", &place("web", 1)),
            "another workload, same count"
        );
        assert_ne!(base, key("conn-a", &place("api", 2)), "another count");
        assert_eq!(base, key("conn-a", &place("api", 1)));
        let mut argo = place("api", 1);
        argo.argo = vec![("argocd".into(), vec!["shop-prod".into()])];
        assert_ne!(base, key("conn-a", &argo), "an Argo CD Application");
        let mut elsewhere = place("api", 1);
        elsewhere.argo_elsewhere = 1;
        assert_ne!(base, key("conn-a", &elsewhere), "one read elsewhere");
        let mut kargo = place("api", 1);
        kargo.kargo = vec!["checkout".into()];
        assert_ne!(base, key("conn-a", &kargo), "a Kargo Project");
        let moved = super::live_identity(
            "conn-a",
            "checkout",
            &place("api", 1),
            &Err(super::Absent::NoProject),
        );
        assert_ne!(base, moved, "another Coroot link");
    }

    #[test]
    fn example_data_is_dated_from_its_first_read_and_not_again() {
        let mut anchor = super::Anchor::default();
        assert_eq!(anchor.at(100), 100);
        assert_eq!(anchor.at(200), 100);
        anchor = super::Anchor::default();
        assert_eq!(anchor.at(300), 300);
    }

    #[test]
    fn a_read_that_failed_whole_leaves_every_source_not_read_and_nothing_quiet() {
        let reads = unread("core-fra", 1_800_000_000, "Unreachable: no answer");
        let timeline = timeline::derive(&reads, 1_800_000_000, 3);
        assert!(timeline.rows.is_empty());
        let chips = view::chips(&timeline);
        assert_eq!(chips.len(), Source::ALL.len());
        assert!(
            chips
                .iter()
                .all(|chip| chip.text.ends_with("not read: Unreachable: no answer"))
        );
    }

    #[test]
    fn live_syncs_say_latest_state_only_what_was_cut_and_why_not_read() {
        use freshkube_delivery::timeline::SourceRead;
        let now = 1_800_000_000;
        let syncs = |read: SourceRead| {
            let timeline = timeline::derive(&[read], now, 3);
            let chips = view::chips(&timeline);
            let chip = chips
                .iter()
                .find(|chip| chip.id.ends_with("syncs"))
                .unwrap();
            (chip.text.to_string(), chip.label.to_string(), timeline)
        };
        let read = SourceRead::read(Source::Syncs, "core-fra", now, Vec::new()).state_only();
        let (text, _, timeline) = syncs(read.clone());
        assert!(text.contains("latest state only"), "{text}");
        // It vouches for no hour: nothing is drawn quiet from it alone.
        assert!(timeline.rows.is_empty());
        let (text, label, _) =
            syncs(read.cut("2 Argo CD Applications are read through another connection"));
        assert!(text.contains("cut"), "{text}");
        assert!(label.contains("another connection"), "{label}");
        let (text, _, _) = syncs(SourceRead::not_read(
            Source::Syncs,
            "core-fra",
            now,
            "Forbidden: can't list Applications",
        ));
        assert!(
            text.ends_with("not read: Forbidden: can't list Applications"),
            "{text}"
        );
    }
}
