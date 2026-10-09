//! The Applications page: what core's `applications` derives from Kargo,
//! Argo CD and the `part-of` label, as a table page grouped by the rule that
//! found each application, with the selection in the Inspector.
//!
//! The shell owns the page and tells it when it shows. It reads only while
//! shown: showing it reads when it has nothing or what it has is older than
//! [`FRESH_FOR`], Refresh reads again, and hiding drops the read in flight.
//! A read is keyed by the connection's id, so another connection's answer
//! is never shown as this one's.
//!
//! Enter, a double-click or the row menu's Open shows the selected
//! application's own page (`page/`) in the list's place; its breadcrumb
//! comes back to the list with the selection kept. Each new read reaches the
//! open page, and one without its application closes it. A part opens in
//! Resources as any object link does (`links.rs`).
//!
//! On a Kargo application's page, a Stage leads to the change it carries
//! (`change/`), which shows in the application page's place; its
//! breadcrumb comes back. Only example data has a change to follow yet.
mod change;
mod column;
mod display;
mod example;
mod links;
mod page;
mod table;
#[cfg(test)]
mod tests;
mod view;

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use freshkube_core::applications::{
    Derived, Inputs, Override, SessionInputs, SessionKey, derive, read::read_session,
};
use freshkube_core::delivery::change::{self as delivery_change, Change};
use freshkube_core::delivery::read::ReadOnlyClient;
use freshkube_core::snapshot::{Request, Snapshot};
use freshkube_ui::inspector::InspectorSplit;
use freshkube_ui::status::{Part, Segment};
use freshkube_ui::table::{self as kit, TableState};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::backend::{self, OwnedJob};
use crate::resources::{KubeAccess, KubeSource, ResourceLink};
use change::{ChangeEvent, ChangePage};
pub(crate) use column::Column;
#[cfg(test)]
pub(crate) use column::{ColumnApp, ColumnSection};
use display::{Body, Display, Labels, MARKS, Mark};
pub(crate) use example::Variant;
use links::Connections;
use page::{ApplicationEvent, ApplicationPage};

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
        ClearApplication,
        /// Opens the selected application's page.
        OpenApplication
    ]
);

pub(crate) fn key_bindings() -> Vec<KeyBinding> {
    let mut bindings = vec![
        KeyBinding::new("down", NextApplication, Some(CONTEXT)),
        KeyBinding::new("up", PreviousApplication, Some(CONTEXT)),
        KeyBinding::new("escape", ClearApplication, Some(CONTEXT)),
        KeyBinding::new("enter", OpenApplication, Some(CONTEXT)),
    ];
    bindings.extend(page::key_bindings());
    bindings.extend(change::key_bindings());
    bindings
}

/// One read: what was derived, what its clusters are called, and which
/// connection each is.
#[derive(Clone)]
struct Read {
    derived: Derived,
    labels: Labels,
    connections: Connections,
    changes: Changes,
}

/// Where a Stage's change comes from.
#[derive(Clone, Copy, Debug)]
enum Changes {
    /// Nothing reads a change yet.
    None,
    /// Example data's, its times counted from when it was read.
    Example(DateTime<Utc>),
}

impl Changes {
    /// The Freight a Kargo project's Stage carries, when a change was read.
    fn freight_of(self, project: &str, stage: &str) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Example(_) => delivery_change::example::freight_of(project, stage),
        }
    }

    /// The change a Kargo project's Stage carries, when one was read.
    fn of_stage(self, project: &str, stage: &str) -> Option<Change> {
        match self {
            Self::None => None,
            Self::Example(now) => delivery_change::example::for_stage(project, stage, now),
        }
    }
}

impl Read {
    fn of(inputs: &Inputs, labels: Labels, connections: Connections) -> Self {
        Self {
            derived: derive(inputs, &Override::default()),
            labels,
            connections,
            changes: Changes::None,
        }
    }

    /// A live read of one connection: its cluster is that connection.
    fn live(inputs: &Inputs, source: &KubeSource) -> Self {
        let key = SessionKey::new(source.id.clone());
        Self::of(
            inputs,
            Labels::new([(key.clone(), source.context.clone())]),
            Connections::new(source.id.clone(), [(key, source.id.clone())]),
        )
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
    /// Example data answers after this, in tests, so a read stays in
    /// flight with a real task; zero answers at once.
    example_delay: Duration,
    /// When the shown read's answer arrived, on the executor's clock.
    read_at: Option<Instant>,
    display: Display,
    /// The banner for a refresh that failed over an earlier read, and its
    /// label, derived with the display.
    stale: Option<(SharedString, SharedString)>,
    /// The table's lines, derived when the rows or a filter change.
    lines: Vec<Entry>,
    /// Rows per rule under the filters, for the group headers.
    groups: [usize; 4],
    /// Rows per mark under the text filter, for the chips.
    counts: [usize; 4],
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
    /// Whether the Inspector shows the selection. A right-click selects
    /// without opening it, so the table keeps its place under the menu.
    inspect: bool,
    /// The application whose page shows in the list's place.
    open: Option<(Entity<ApplicationPage>, Subscription)>,
    /// The change shown in the application page's place.
    change: Option<(Entity<ChangePage>, Subscription)>,
    /// The keyboard handle of a page that closed without a window at hand,
    /// kept until the next frame looks whether it had the keyboard.
    refocus: Option<FocusHandle>,
    /// `5 applications in 6 clusters`, in the status bar.
    pub(crate) status: Segment,
    /// What the shell's column lists, derived with the display.
    column: Column,
    /// Changes with the column's lines and the application shown, so the
    /// column draws again only then (`column.rs`).
    column_revision: usize,
    _subscription: Subscription,
}

/// A part to open in Resources, from the open application's page.
impl EventEmitter<ResourceLink> for ApplicationsPage {}

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
        let split = InspectorSplit::new(PREFIX, cx);
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
            example_delay: Duration::ZERO,
            read_at: None,
            display: Display::default(),
            stale: None,
            lines: Vec::new(),
            groups: [0; 4],
            counts: [0; 4],
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
            inspect: false,
            open: None,
            change: None,
            refocus: None,
            status: Segment::default(),
            column: Column::new(&Display::default(), false, false, false),
            column_revision: 0,
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

    #[cfg(test)]
    pub(crate) fn set_example_delay(&mut self, delay: Duration) {
        self.example_delay = delay;
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
        self.inspect = false;
        self.close_open(cx);
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
            self.derive_column();
            cx.notify();
            return;
        }
        // Fresh only by when an answer arrived: a read dropped on hide, or
        // a refresh that failed, doesn't make old data new.
        let now = cx.background_executor().now();
        let fresh = self.snapshot.data().is_some()
            && !self.snapshot.is_stale()
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
        match &source.access {
            KubeAccess::Example => {
                // Held, it stays in flight, so the page keeps its loading
                // rows.
                self.pending = true;
                if self.hold {
                    self.derive_column();
                    cx.notify();
                    return;
                }
                let inputs = example::inputs(self.variant);
                let read = Read {
                    changes: Changes::Example(Utc::now()),
                    ..Read::of(
                        &inputs,
                        example::labels(&inputs),
                        example::connections(&source.id),
                    )
                };
                if self.example_delay.is_zero() {
                    self.answer(&request, Ok(read), cx);
                } else {
                    let delay = self.example_delay;
                    self.task = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(delay).await;
                        _ = this.update(cx, |page, cx| page.answer(&request, Ok(read), cx));
                    }));
                }
            }
            access => {
                let access = access.clone();
                let key = SessionKey::new(source.id.clone());
                let (job, receiver) = backend::spawn_job(
                    &self.runtime,
                    DEADLINE,
                    "Reading the applications timed out".into(),
                    async move {
                        let client = access.client().await?;
                        let reader = ReadOnlyClient::new(client);
                        let session = read_session(&reader, key).await;
                        let inputs = Inputs {
                            sessions: vec![session],
                            stage_naming: None,
                        };
                        Ok(Read::live(&inputs, &source))
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
        self.derive_column();
        cx.notify();
    }

    /// A read's answer, published only while it is the one in flight.
    fn answer(
        &mut self,
        request: &Request<String>,
        result: Result<Read, String>,
        cx: &mut Context<Self>,
    ) {
        let answered = result.is_ok();
        if !self.pending || !self.snapshot.apply(request, result) {
            return;
        }
        self.pending = false;
        self.job = None;
        self.task = None;
        if answered {
            self.read_at = Some(cx.background_executor().now());
        }
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
            Some(Read::live(&inputs, source))
        };
        self.display = match self.snapshot.data().cloned().or_else(unread) {
            Some(read) => Display::new(&read.derived, &read.labels),
            None => Display::default(),
        };
        self.stale = self.snapshot.data().and(self.snapshot.error()).map(|why| {
            let text = format!("Showing the last read. {why}");
            let label = format!("Couldn't read again: {text}");
            (text.into(), label.into())
        });
        (self.columns, self.width) = table::columns(&self.display.rows);
        self.status = self.segment();
        self.derive_column();
        self.update_open(cx);
        self.rebuild(cx);
    }

    /// Hands the open page its application's new read; without it there,
    /// the page closes.
    fn update_open(&mut self, cx: &mut Context<Self>) {
        let Some((open, _)) = &self.open else {
            return;
        };
        let id = open.read(cx).id().clone();
        let read = self.snapshot.data();
        match read.and_then(|read| Some((read.derived.find(&id)?, read))) {
            Some((app, read)) => {
                let stale = self.stale.clone();
                open.update(cx, |page, cx| page.update(app, read, stale, cx));
            }
            None => self.close_open(cx),
        }
    }

    /// Drops the open page without a window at hand, as a read or another
    /// connection does: the next frame gives the keyboard back to the list
    /// if the page had it.
    fn close_open(&mut self, cx: &App) {
        let change = self
            .change
            .take()
            .map(|(page, _)| page.read(cx).focus_handle());
        if let Some((page, _)) = self.open.take() {
            self.column_revision += 1;
            self.refocus = change.or(Some(page.read(cx).focus_handle()));
        }
    }

    /// Hands the keyboard to the list when the page closed under it.
    fn refocus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(closed) = self.refocus.take() else {
            return;
        };
        if closed.is_focused(window) {
            let list = self.focus.clone();
            window.defer(cx, move |window, cx| window.focus(&list, cx));
        }
    }

    /// Shows the application's own page in the list's place, with the
    /// keyboard on it.
    fn open_application(
        &mut self,
        key: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(read) = self.snapshot.data() else {
            return;
        };
        let Some(app) = read
            .derived
            .applications
            .iter()
            .find(|app| app.id.as_str() == key.as_ref())
        else {
            return;
        };
        // Its page shows, not a change followed from another.
        self.change = None;
        let stale = self.stale.clone();
        let page = cx.new(|cx| ApplicationPage::new(app, read, stale, cx));
        let subscription =
            cx.subscribe_in(&page, window, |this, _, event, window, cx| match event {
                ApplicationEvent::Back => this.close_application(window, cx),
                ApplicationEvent::Open(link) => cx.emit(link.as_ref().clone()),
                ApplicationEvent::Follow(stage) => this.follow(stage, window, cx),
            });
        window.focus(&page.read(cx).focus_handle(), cx);
        self.open = Some((page, subscription));
        self.column_revision += 1;
        cx.notify();
    }

    /// Shows the change a Stage of the open Kargo application carries, in
    /// the application page's place, on the Stage's gates.
    fn follow(&mut self, stage: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (Some((open, _)), Some(read)) = (&self.open, self.snapshot.data()) else {
            return;
        };
        let project = open.read(cx).name().to_string();
        let Some(change) = read.changes.of_stage(&project, stage) else {
            return;
        };
        let connections = read.connections.clone();
        let page = cx.new(|cx| {
            let mut page = ChangePage::new(change, connections, cx);
            page.select_stage(stage, window, cx);
            page
        });
        let subscription =
            cx.subscribe_in(&page, window, |this, _, event, window, cx| match event {
                ChangeEvent::Back => this.close_change(window, cx),
                ChangeEvent::Open(link) => cx.emit(link.as_ref().clone()),
            });
        window.focus(&page.read(cx).focus_handle(), cx);
        self.change = Some((page, subscription));
        cx.notify();
    }

    /// Back to the application's page, with the selection it had.
    pub(super) fn close_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.change.take().is_some() {
            self.focus_shown(window, cx);
            cx.notify();
        }
    }

    /// The change shown, if one is.
    pub(crate) fn change_page(&self) -> Option<&Entity<ChangePage>> {
        self.change.as_ref().map(|(page, _)| page)
    }

    /// Follows a Stage of the open application, for `FRESHKUBE_PAGE=change`.
    pub(crate) fn follow_named(
        &mut self,
        stage: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.follow(stage, window, cx);
    }

    /// Back to the list, with the selection it had.
    fn close_application(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.change = None;
        if self.open.take().is_some() {
            self.column_revision += 1;
            self.focus(window, cx);
            cx.notify();
        }
    }

    /// Enter on the list: the selection's page.
    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.selected.clone() {
            self.open_application(&key, window, cx);
        }
    }

    /// Opens an application by its name, selecting it on the list first,
    /// for `FRESHKUBE_PAGE=application`.
    pub(crate) fn open_named(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self
            .display
            .rows
            .iter()
            .find(|row| row.name.as_ref() == name)
            .map(|row| row.key.clone())
        else {
            return;
        };
        self.select(key.clone(), cx);
        self.open_application(&key, window, cx);
    }

    /// The open application's page, if one shows.
    pub(crate) fn open_page(&self) -> Option<&Entity<ApplicationPage>> {
        self.open.as_ref().map(|(page, _)| page)
    }

    /// Puts the keyboard on what shows: the open application's page, or
    /// the list.
    pub(crate) fn focus_shown(&self, window: &mut Window, cx: &mut App) {
        if let Some(change) = self.change_page() {
            let focus = change.read(cx).focus_handle();
            window.focus(&focus, cx);
            return;
        }
        match self.open_page() {
            Some(page) => {
                let focus = page.read(cx).focus_handle();
                window.focus(&focus, cx);
            }
            None => self.focus(window, cx),
        }
    }

    fn segment(&self) -> Segment {
        use freshkube_ui::ui::Tone;
        let context = self.source.as_ref().map(|s| s.context.clone());
        // A refused or failed read says so, never a count of nothing.
        match &self.display.body {
            Body::Refused { .. } => {
                let part = Part::new("applications not permitted").tone(Tone::Unknown);
                return Segment::new(context, vec![part]);
            }
            Body::Failed { .. } => {
                let part = Part::new("couldn't read applications").tone(Tone::Warn);
                return Segment::new(context, vec![part]);
            }
            Body::Table | Body::Empty { .. } => {}
        }
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
            parts.push(Part::new(format!("{incomplete} may be incomplete")).tone(Tone::Unknown));
        }
        if self.snapshot.is_stale() {
            parts.push(Part::new("last read").tone(Tone::Warn));
        }
        Segment::new(context, parts)
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
        self.inspect = true;
        kit::reveal(self, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(key) = kit::step(self, delta, cx) {
            self.select(key, cx);
        }
    }

    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.inspect = false;
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
        self.counts = [0; 4];
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
            self.inspect = false;
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

    /// A read again that fails, as a cluster that stops answering does.
    pub(crate) fn fail_read(&mut self, why: &str, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        self.stop();
        let request = self.snapshot.begin(source.id);
        self.pending = true;
        self.answer(&request, Err(why.into()), cx);
    }
}

/// Every mark, for the header's chips in order.
fn marks() -> impl Iterator<Item = Mark> {
    MARKS.into_iter()
}
