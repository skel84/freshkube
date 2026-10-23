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
use freshkube_delivery::entry;
use freshkube_delivery::timeline::example::{self, Shape};
use freshkube_delivery::timeline::live::{self, Place};
use freshkube_delivery::timeline::{self, Source, SourceRead, WINDOWS};
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

/// Where the tab's sources come from.
#[derive(Clone)]
pub(crate) enum Fetch {
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
        place: Place,
    },
}

impl Fetch {
    /// Which connection, application and workloads a read is for, so an
    /// answer for another never lands.
    fn identity(&self, app: &str) -> String {
        match self {
            Self::Example { shape, .. } => format!("example/{shape:?}/{app}"),
            Self::Live {
                source_id, place, ..
            } => live_identity(source_id, app, place),
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
fn live_identity(source_id: &str, app: &str, place: &Place) -> String {
    format!("{source_id}/{app}/{place:?}")
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
    pub source: Source,
    pub text: SharedString,
    pub tone: freshkube_ui::ui::Tone,
    pub label: SharedString,
    /// The source was read (in whole or in part), not refused or missing.
    pub read: bool,
}

pub struct TimelineView {
    app: SharedString,
    fetch: Fetch,
    loader: Loader<Vec<SourceRead>, (String, u32)>,
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
        let (columns, width) = table::columns(&[]);
        Self {
            app: app.to_owned().into(),
            fetch,
            loader,
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
            return;
        }
        self.fetch = fetch;
        self.anchor = Anchor::default();
        self.fallback.clear();
        self.loader.reset();
        self.derive(cx);
        if self.visible {
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
        if visible && reads_again {
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
        let timeline = timeline::derive(reads, now, self.hours);
        self.chips = view::chips(&timeline);
        self.rows = table::rows(&timeline);
        (self.columns, self.width) = table::columns(&self.rows);
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
        };
        let key = |source: &str, place: &Place| super::live_identity(source, "checkout", place);
        let base = key("conn-a", &place("api", 1));
        assert_ne!(base, key("conn-b", &place("api", 1)), "another connection");
        assert_ne!(
            base,
            key("conn-a", &place("web", 1)),
            "another workload, same count"
        );
        assert_ne!(base, key("conn-a", &place("api", 2)), "another count");
        assert_eq!(base, key("conn-a", &place("api", 1)));
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
}
