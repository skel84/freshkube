//! The Applications page: what core's `applications` derives from Kargo,
//! Argo CD and the `part-of` label, as a table page grouped by the rule that
//! found each application, with the selection in the Inspector.
//!
//! The shell owns the page and tells it when it shows. It reads only while
//! shown: showing it reads when it has nothing or what it has is older than
//! [`FRESH_FOR`], Refresh reads again, and hiding drops the read in flight.
//! A read is keyed by the connection's id, so another connection's answer
//! is never shown as this one's.
mod display;
mod example;
mod table;
#[cfg(test)]
mod tests;
mod view;

use std::time::{Duration, Instant};

use freshkube_core::applications::{
    Derived, Inputs, Override, SessionInputs, SessionKey, derive, read::read_session,
};
use freshkube_core::delivery::read::ReadOnlyClient;
use freshkube_core::snapshot::{Request, Snapshot};
use freshkube_ui::inspector::InspectorSplit;
use freshkube_ui::status::{Part, Segment};
use freshkube_ui::table::{self as kit, TableState};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::backend::{self, OwnedJob};
use crate::resources::{KubeAccess, KubeSource};
#[cfg(test)]
use display::Body;
use display::{ARGOCD_NAMESPACE, Display, Labels, MARKS, Mark};
pub(crate) use example::Variant;

/// The page's id prefix: `applications-title`, `-list`, `-tally-…`.
const PREFIX: &str = "applications";
/// The list's key context, around the table, drawn in every state.
pub(crate) const CONTEXT: &str = "Applications";
/// A read this recent is shown again without reading.
const FRESH_FOR: Duration = Duration::from_secs(30);
/// How long one cluster's read may take: up to a few hundred lists.
const DEADLINE: Duration = Duration::from_secs(60);

gpui_kit::actions!(
    applications,
    [
        /// Selects the next application.
        NextApplication,
        /// Selects the previous application.
        PreviousApplication,
        /// Clears the selection, which closes the Inspector.
        ClearApplication
    ]
);

pub(crate) fn key_bindings() -> [KeyBinding; 3] {
    [
        KeyBinding::new("down", NextApplication, Some(CONTEXT)),
        KeyBinding::new("up", PreviousApplication, Some(CONTEXT)),
        KeyBinding::new("escape", ClearApplication, Some(CONTEXT)),
    ]
}

/// One read: what was derived and what its clusters are called.
#[derive(Clone)]
struct Read {
    derived: Derived,
    labels: Labels,
}

impl Read {
    fn of(inputs: &Inputs, labels: Labels) -> Self {
        Self {
            derived: derive(inputs, &Override::default()),
            labels,
        }
    }
}

/// One line of the table: a rule's group header, or a row by index.
#[derive(Clone, Copy)]
enum Entry {
    Group(usize),
    Row(usize),
}

pub(crate) struct ApplicationsPage {
    runtime: tokio::runtime::Handle,
    source: Option<KubeSource>,
    visible: bool,
    /// The example's shape, in example data.
    variant: Variant,
    /// Example data never answers, for captures (`FRESHKUBE_FIXTURE_HOLD`).
    hold: bool,
    snapshot: Snapshot<Read, String>,
    /// Whether a read is in flight; dropping its job and task stops it.
    pending: bool,
    job: Option<OwnedJob>,
    task: Option<Task<()>>,
    /// When the shown read was asked for, on the executor's clock.
    read_at: Option<Instant>,
    display: Display,
    /// The table's lines, derived when the rows or a filter change.
    lines: Vec<Entry>,
    /// Rows per rule under the filters, for the group headers.
    groups: [usize; 4],
    /// Rows per mark under the text filter, for the chips.
    counts: [usize; 3],
    mark: Option<Mark>,
    columns: Vec<table::Column>,
    width: f32,
    table: TableState,
    loading: kit::LoadingRows,
    loading_motion: Entity<kit::LoadingMotion>,
    filter: Entity<InputState>,
    focus: FocusHandle,
    split: InspectorSplit,
    /// The frame's scroll, used while the window is short.
    page_scroll: ScrollHandle,
    /// The selected application's id, kept while a read or filter shows it.
    selected: Option<SharedString>,
    /// `5 applications in 6 clusters`, in the status bar.
    pub(crate) status: Segment,
    _subscription: Subscription,
}

impl ApplicationsPage {
    pub(crate) fn new(
        runtime: tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter applications…"));
        let subscription = cx.subscribe(&filter, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.refilter(cx);
            }
        });
        let (columns, width) = table::columns(&[]);
        let loading = kit::LoadingRows::new(PREFIX);
        let loading_motion = cx.new(|_| loading.motion(kit::Look::Pulse));
        let split = {
            let file = crate::navigation_file::NavigationFile::global(cx);
            InspectorSplit::new(
                file.inspector_width(PREFIX),
                move |width, cx| file.set_inspector_width(PREFIX, width, cx),
                cx,
            )
        };
        Self {
            runtime,
            source: None,
            visible: false,
            variant: Variant::from_env(),
            hold: crate::fixture::hold().applications,
            snapshot: Snapshot::default(),
            pending: false,
            job: None,
            task: None,
            read_at: None,
            display: Display::default(),
            lines: Vec::new(),
            groups: [0; 4],
            counts: [0; 3],
            mark: None,
            columns,
            width,
            table: TableState::new(PREFIX),
            loading,
            loading_motion,
            filter,
            focus: cx.focus_handle(),
            split,
            page_scroll: ScrollHandle::new(),
            selected: None,
            status: Segment::default(),
            _subscription: subscription,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_variant(&mut self, variant: Variant) {
        self.variant = variant;
    }

    #[cfg(test)]
    pub(crate) fn set_hold(&mut self, hold: bool) {
        self.hold = hold;
    }

    /// The connection the page reads. Another connection drops what was
    /// read and what is in flight; the page reads it again if shown.
    pub(crate) fn set_source(&mut self, source: Option<KubeSource>, cx: &mut Context<Self>) {
        let same = self.source.as_ref().map(|s| &s.id) == source.as_ref().map(|s| &s.id);
        self.source = source;
        if same {
            return;
        }
        self.stop();
        self.snapshot = Snapshot::default();
        self.read_at = None;
        self.selected = None;
        self.show_read(cx);
        if self.visible {
            self.read(cx);
        }
    }

    /// Shown, the page reads unless what it has is fresh; hidden, it drops
    /// the read in flight.
    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        if !visible {
            self.stop();
            cx.notify();
            return;
        }
        let now = cx.background_executor().now();
        let fresh = self.snapshot.data().is_some()
            && self
                .read_at
                .is_some_and(|at| now.saturating_duration_since(at) < FRESH_FOR);
        if !fresh {
            self.read(cx);
        }
    }

    /// Refresh: reads again, whatever is in flight, while shown.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.visible {
            self.read(cx);
        }
    }

    /// Drops the read in flight; what was read stays.
    fn stop(&mut self) {
        self.pending = false;
        self.job = None;
        self.task = None;
    }

    #[cfg(test)]
    pub(crate) fn is_reading(&self) -> bool {
        self.pending
    }

    fn read(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        self.stop();
        let request = self.snapshot.begin(source.id.clone());
        self.read_at = Some(cx.background_executor().now());
        match &source.access {
            KubeAccess::Example => {
                if self.hold {
                    // Stays in flight, so the page keeps its loading rows.
                    self.pending = true;
                } else {
                    let inputs = example::inputs(self.variant);
                    let read = Read::of(&inputs, example::labels(&inputs));
                    self.snapshot.apply(&request, Ok(read));
                    self.show_read(cx);
                }
            }
            access => {
                let access = access.clone();
                let key = SessionKey::new(source.id.clone());
                let labels = Labels::new([(key.clone(), source.context.clone())]);
                let (job, receiver) = backend::spawn_job(
                    &self.runtime,
                    DEADLINE,
                    "Reading the applications timed out".into(),
                    async move {
                        let client = access.client().await?;
                        let reader = ReadOnlyClient::new(client);
                        let session = read_session(&reader, key, ARGOCD_NAMESPACE).await;
                        let inputs = Inputs {
                            sessions: vec![session],
                            stage_naming: None,
                        };
                        Ok(Read::of(&inputs, labels))
                    },
                );
                self.job = Some(job);
                self.pending = true;
                self.task = Some(cx.spawn(async move |this, cx| {
                    let result = receiver
                        .await
                        .unwrap_or_else(|_| Err("Reading the applications stopped".into()));
                    _ = this.update(cx, |page, cx| page.answer(&request, result, cx));
                }));
            }
        }
        cx.notify();
    }

    /// A read's answer, published only while it is the one in flight.
    fn answer(
        &mut self,
        request: &Request<String>,
        result: Result<Read, String>,
        cx: &mut Context<Self>,
    ) {
        if !self.pending || !self.snapshot.apply(request, result) {
            return;
        }
        self.pending = false;
        self.job = None;
        self.show_read(cx);
    }

    /// Derives the display from the snapshot: what was read, or, when the
    /// cluster couldn't be reached before anything was read, every source
    /// as unread, which is never "no applications".
    fn show_read(&mut self, cx: &mut Context<Self>) {
        let unread = || -> Option<Read> {
            let source = self.source.as_ref()?;
            let why = self.snapshot.error()?;
            let key = SessionKey::new(source.id.clone());
            let inputs = Inputs {
                sessions: vec![SessionInputs::unread(key.clone(), why)],
                stage_naming: None,
            };
            Some(Read::of(
                &inputs,
                Labels::new([(key, source.context.clone())]),
            ))
        };
        self.display = match self.snapshot.data().cloned().or_else(unread) {
            Some(read) => Display::new(&read.derived, &read.labels),
            None => Display::default(),
        };
        (self.columns, self.width) = table::columns(&self.display.rows);
        self.status = self.segment();
        self.rebuild(cx);
    }

    fn segment(&self) -> Segment {
        let Some(read) = self.snapshot.data() else {
            return Segment::default();
        };
        let plural = |count: usize, one: &str, many: &str| {
            format!("{count} {}", if count == 1 { one } else { many })
        };
        let clusters = read
            .derived
            .coverage
            .iter()
            .map(|c| &c.session)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        let mut parts = vec![Part::new(format!(
            "{} in {}",
            plural(self.display.rows.len(), "application", "applications"),
            plural(clusters, "cluster", "clusters")
        ))];
        let incomplete = self.display.marks[Mark::Incomplete.index()];
        if incomplete > 0 {
            parts.push(
                Part::new(format!("{incomplete} may be incomplete"))
                    .tone(freshkube_ui::ui::Tone::Unknown),
            );
        }
        if self.snapshot.is_stale() {
            parts.push(Part::new("last read").tone(freshkube_ui::ui::Tone::Warn));
        }
        Segment::new(self.source.as_ref().map(|s| s.context.clone()), parts)
    }

    /// The motion over the table's loading rows, which the shell mounts
    /// beside the cached page.
    pub(crate) fn loading_motion(&self) -> Option<Entity<kit::LoadingMotion>> {
        kit::TableSource::loading(self)
            .is_some()
            .then(|| self.loading_motion.clone())
    }

    /// Puts the keyboard on the list, where its keys are bound.
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    fn selected_row(&self) -> Option<&display::AppRow> {
        let key = self.selected.as_ref()?;
        self.display.rows.iter().find(|row| &row.key == key)
    }

    fn select(&mut self, key: SharedString, cx: &mut Context<Self>) {
        self.selected = Some(key);
        kit::reveal(self, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(key) = kit::step(self, delta, cx) {
            self.select(key, cx);
        }
    }

    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.selected.take().is_some() {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn toggle_mark(&mut self, mark: Mark, cx: &mut Context<Self>) {
        self.mark = (self.mark != Some(mark)).then_some(mark);
        self.refilter(cx);
    }

    /// A filter changed: the lines again, from the top.
    fn refilter(&mut self, cx: &mut Context<Self>) {
        self.rebuild(cx);
        self.table.reveal(0, ScrollStrategy::Top);
    }

    /// Derives the lines, keeping the filters and the scroll: rows matching
    /// the text, counted per mark, then those of the chosen mark, under a
    /// header per rule.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value().to_lowercase();
        self.counts = [0; 3];
        self.groups = [0; 4];
        let mut shown: Vec<usize> = Vec::new();
        for (ix, row) in self.display.rows.iter().enumerate() {
            if !row.query.contains(&query) {
                continue;
            }
            self.counts[row.mark.index()] += 1;
            if self.mark.is_none_or(|mark| mark == row.mark) {
                self.groups[display::rule_index(row.rule)] += 1;
                shown.push(ix);
            }
        }
        // Rows are sorted by rule, so a rule's rows are consecutive.
        self.lines.clear();
        let mut current = None;
        for ix in shown {
            let rule = display::rule_index(self.display.rows[ix].rule);
            if current != Some(rule) {
                self.lines.push(Entry::Group(rule));
                current = Some(rule);
            }
            self.lines.push(Entry::Row(ix));
        }
        // A row the filters hide, or that went, is no longer selected.
        if self.selected.as_ref().is_some_and(|key| {
            !self
                .lines
                .iter()
                .any(|entry| matches!(entry, Entry::Row(ix) if &self.display.rows[*ix].key == key))
        }) {
            self.selected = None;
        }
        cx.notify();
    }
}

#[cfg(test)]
impl ApplicationsPage {
    pub(crate) fn names(&self) -> Vec<String> {
        self.lines
            .iter()
            .filter_map(|entry| match entry {
                Entry::Row(ix) => Some(self.display.rows[*ix].name.to_string()),
                Entry::Group(_) => None,
            })
            .collect()
    }

    pub(crate) fn body(&self) -> &Body {
        &self.display.body
    }

    pub(crate) fn has_read(&self) -> bool {
        self.snapshot.data().is_some()
    }
}

/// Every mark, for the header's chips in order.
fn marks() -> impl Iterator<Item = Mark> {
    MARKS.into_iter()
}
