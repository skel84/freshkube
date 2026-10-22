//! Waiting to ship (#842): what is built or deployed somewhere but not yet
//! in production, across the Kargo projects of the open cluster, as a table
//! page in the Applications area. `delivery::waiting` derives it; this
//! page shows it: one group for each application with its gap to
//! production, the one waiting longest first, then the applications that
//! weren't read, then, folded in one group, those that are caught up.
//!
//! The find box answers "did the fix ship?": a pull request number or a
//! commit narrows the list, and the Inspector traces the selection to
//! production. Selecting a row also says what promoting now would ship.
//!
//! The Applications page owns it and tells it when it shows. It reads only
//! while shown, by the Changes list's rules: on showing once what it has is
//! older than [`FRESH_FOR`], on Refresh, and never while hidden, which
//! drops the read in flight. Another connection drops what was read. The
//! read is the Changes list's own (a project's Freight and Stages, four
//! projects at a time, within its 90 s budget) plus one GitHub call for
//! each Freight in a gap; it reads no Argo CD, builds or pods.
pub(crate) mod display;
mod table;
mod view;

use std::time::Duration;

use chrono::Utc;
use freshkube_core::snapshot::Request;
use freshkube_delivery::change::example::Shape;
use freshkube_delivery::changes::live::Where;
use freshkube_delivery::entry::{self, pool};
use freshkube_delivery::waiting::{self as core_waiting, Waiting};
use freshkube_reads::{Loader, OnShow};
use freshkube_ui::inspector::InspectorSplit;
use freshkube_ui::status::{Part, Segment};
use freshkube_ui::table::{self as kit, TableState};
use freshkube_ui::ui::Tone;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::across::{Across, Alone};
use crate::backend;
use crate::resources::{ClusterAccess, ClusterSource};
use display::{Body, Display, WaitRow};

/// The page's id prefix: `waiting-title`, `waiting-row-<key>`, …
const PREFIX: &str = "waiting";
/// The page's key context, drawn in every state.
pub(crate) const CONTEXT: &str = "Waiting";
/// A read this recent is shown again without reading.
const FRESH_FOR: Duration = Duration::from_secs(30);
/// Showing the page reads again once what it has is [`FRESH_FOR`] old.
const ON_SHOW: OnShow = OnShow::ReadIfOlder(FRESH_FOR);
/// How long one read may take before it fails whole: past the read's
/// budget, at which it answers with what was read in time.
const DEADLINE: Duration = Duration::from_secs(core_waiting::live::BUDGET.as_secs() + 30);

gpui_kit::actions!(
    waiting,
    [
        /// Selects the next row.
        NextWaiting,
        /// Selects the previous row.
        PreviousWaiting,
        /// Clears the selection, which closes the Inspector.
        ClearWaiting
    ]
);

pub(crate) fn key_bindings() -> [KeyBinding; 3] {
    [
        KeyBinding::new("down", NextWaiting, Some(CONTEXT)),
        KeyBinding::new("up", PreviousWaiting, Some(CONTEXT)),
        KeyBinding::new("escape", ClearWaiting, Some(CONTEXT)),
    ]
}

/// One read: the page.
#[derive(Clone)]
struct Read {
    waiting: Waiting,
}

/// One line of the table: a group header, or a row by index.
#[derive(Clone, Copy)]
enum Entry {
    Group(usize),
    Row(usize),
}

pub struct WaitingPage {
    runtime: tokio::runtime::Handle,
    secrets: Option<freshkube_core::secrets::Secrets>,
    setup_revision: u64,
    source: Option<ClusterSource>,
    visible: bool,
    /// What example data couldn't read (`FRESHKUBE_APPLICATIONS`).
    shape: Shape,
    /// Example data answers after this, in tests, so a read stays in
    /// flight with a real task; zero answers at once.
    example_delay: Duration,
    /// The read, its answer and the work in flight, keyed by the
    /// connection. It is hidden while the page is, and its answer's age
    /// decides whether showing reads.
    loader: Loader<Read, (String, u64)>,
    /// Where a live read reads each part.
    across: Across,
    #[cfg(any(test, feature = "testing"))]
    pub display: Display,
    #[cfg(not(any(test, feature = "testing")))]
    display: Display,
    /// A refresh that failed over an earlier read, and its label.
    stale: Option<(SharedString, SharedString)>,
    /// The setup owner's sanitized problem, independent of cluster reads.
    setup_problem: Option<SharedString>,
    lines: Vec<Entry>,
    /// The rows the lines show.
    shown_rows: usize,
    /// Whether the caught-up group is folded.
    folded: bool,
    columns: Vec<table::Column>,
    width: f32,
    table: TableState,
    loading: kit::LoadingRows,
    loading_motion: Entity<kit::LoadingMotion>,
    find: Entity<InputState>,
    _find: Subscription,
    focus: FocusHandle,
    split: InspectorSplit,
    /// The frame's scroll, used while the window is short.
    page_scroll: ScrollHandle,
    selected: Option<SharedString>,
    /// Whether the Inspector shows the selection. A right-click selects
    /// without opening it.
    inspect: bool,
    /// `5 Freight waiting in 2 applications`, in the status bar.
    pub(crate) status: Segment,
}

impl WaitingPage {
    pub(crate) fn new(
        runtime: tokio::runtime::Handle,
        shape: Shape,
        secrets: Option<freshkube_core::secrets::Secrets>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let loading = kit::LoadingRows::new(PREFIX);
        let loading_motion = cx.new(|_| loading.motion(kit::Look::Pulse));
        let (columns, width) = table::columns();
        // The page starts hidden, and so does its read.
        let mut loader = Loader::default();
        loader.set_visible(false, ON_SHOW, cx);
        let find = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Pull request or commit: did it ship?")
        });
        let subscription = cx.subscribe(&find, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.show_read(cx);
            }
        });
        Self {
            runtime,
            secrets,
            setup_revision: 0,
            source: None,
            visible: false,
            shape,
            example_delay: Duration::ZERO,
            loader,
            across: Across::default(),
            display: Display::default(),
            stale: None,
            setup_problem: None,
            lines: Vec::new(),
            shown_rows: 0,
            folded: false,
            columns,
            width,
            table: TableState::new(PREFIX),
            loading,
            loading_motion,
            find,
            _find: subscription,
            focus: cx.focus_handle(),
            split: InspectorSplit::new(PREFIX, cx),
            page_scroll: ScrollHandle::new(),
            selected: None,
            inspect: false,
            status: Segment::default(),
        }
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn set_example_delay(&mut self, delay: Duration) {
        self.example_delay = delay;
    }

    /// Holds the example source the page has, or lets it answer again
    /// (`Hold::applications`), as `FRESHKUBE_FIXTURE_HOLD` does at launch.
    #[cfg(any(test, feature = "testing"))]
    pub fn set_hold(&mut self, hold: bool) {
        if let Some(ClusterSource {
            access: ClusterAccess::Example(held),
            ..
        }) = &mut self.source
        {
            held.applications = hold;
        }
    }

    /// What example data can't read, as the Applications example's
    /// variant says.
    pub(crate) fn set_shape(&mut self, shape: Shape) {
        self.shape = shape;
    }

    /// The connection the page reads. Another connection drops what was
    /// read and what is in flight; the page reads it again if shown.
    pub fn set_source(&mut self, source: Option<ClusterSource>, cx: &mut Context<Self>) {
        let same = self.source.as_ref().map(|s| &s.id) == source.as_ref().map(|s| &s.id);
        self.source = source;
        if same {
            return;
        }
        self.loader.reset();
        self.selected = None;
        self.inspect = false;
        self.folded = false;
        self.show_read(cx);
        if self.visible {
            self.read(cx);
        }
    }

    /// Where a live read reads each part. A live page reads again, now if
    /// shown, else when next shown.
    pub fn set_clusters(&mut self, across: Across, cx: &mut Context<Self>) {
        let remapped = across != self.across;
        self.across = across;
        if remapped && self.is_live() {
            self.loader.expire();
            if self.visible && (self.loader.is_loading() || self.loader.data().is_some()) {
                self.read(cx);
            }
        }
    }

    pub(crate) fn set_setup_problem(
        &mut self,
        problem: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        if self.setup_problem != problem {
            self.setup_problem = problem;
            cx.notify();
        }
    }

    /// A setup reload replaces trust and cancels every delivery read made
    /// with the old setup.
    pub(crate) fn reload_setup(&mut self, across: &Across, cx: &mut Context<Self>) {
        self.setup_revision += 1;
        self.across.github = across.github.clone();
        self.across.stage_naming = across.stage_naming.clone();
        if self.is_live() {
            self.loader.reset();
            self.show_read(cx);
            if self.visible {
                self.read(cx);
            }
        }
    }

    /// Whether the page reads a cluster, not example data.
    fn is_live(&self) -> bool {
        self.source
            .as_ref()
            .is_some_and(|source| !matches!(source.access, ClusterAccess::Example(_)))
    }

    /// Shown, the page reads unless what it has is fresh; hidden, it drops
    /// the read in flight.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        let reads_again = self.loader.set_visible(visible, ON_SHOW, cx);
        if !visible {
            cx.notify();
            return;
        }
        if reads_again {
            self.read(cx);
        }
    }

    /// Refresh: reads again while shown.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.visible {
            self.read(cx);
        }
    }

    /// The settings handed to the page's delivery read.
    pub(crate) fn where_to_read(&self, source: &ClusterSource) -> Where {
        Where {
            cluster: source.id.clone(),
            label: source.context.clone(),
            argocd_namespace: "argocd".into(),
            mapping: self.across.mapping.clone(),
            sources: self.across.sources.clone(),
            public_key_file: self.across.public_key_file.clone(),
            transparency_log: self.across.transparency_log,
            transparency_url: self.across.transparency_url.clone(),
            evidence_result: self.across.evidence_result.clone(),
            build_namespace: self.across.build_namespace.clone(),
            stage_naming: self.across.stage_naming.clone(),
            github: self.across.github.clone(),
        }
    }

    fn read(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let request = self.loader.start((source.id.clone(), self.setup_revision));
        match &source.access {
            ClusterAccess::Example(hold) => {
                // Held, it stays in flight, so the page keeps its loading
                // rows.
                if hold.applications {
                    cx.notify();
                    return;
                }
                let read = Read {
                    waiting: core_waiting::derive(&core_waiting::example::read_shaped(
                        Utc::now(),
                        self.shape,
                    )),
                };
                if self.example_delay.is_zero() {
                    self.answer(&request, Ok(read), cx);
                } else {
                    let delay = self.example_delay;
                    let task = cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(delay).await;
                        _ = this.update(cx, |page, cx| page.answer(&request, Ok(read), cx));
                    });
                    self.loader.run(None, task);
                }
            }
            access => {
                let access = access.clone();
                let at = self.where_to_read(&source);
                let across = self.across.clone();
                let secrets = self.secrets.clone();
                let (job, receiver) = backend::spawn_job(
                    &self.runtime,
                    DEADLINE,
                    "Reading what waits to ship timed out".into(),
                    async move {
                        let open = across.open(&access, Alone::AsEntry).await?;
                        let kubeconfigs = across.kubeconfigs();
                        let pool = pool::Pool::new(
                            &at.sources,
                            Some(pool::Open {
                                entry: at.mapping.open.as_deref(),
                                reader: open.as_ref().map_err(Clone::clone),
                            }),
                            |chosen| entry::connect_entry(kubeconfigs.clone(), chosen),
                            core_waiting::live::BUDGET,
                        )
                        .with_secrets(secrets);
                        let read = core_waiting::live::read_in(
                            &pool,
                            &at,
                            Utc::now(),
                            core_waiting::live::BUDGET,
                        )
                        .await;
                        Ok(Read {
                            waiting: core_waiting::derive(&read),
                        })
                    },
                );
                let task = cx.spawn(async move |this, cx| {
                    let result = receiver
                        .await
                        .unwrap_or_else(|_| Err("Reading what waits to ship stopped".into()));
                    _ = this.update(cx, |page, cx| page.answer(&request, result, cx));
                });
                self.loader.run(Some(job), task);
            }
        }
        cx.notify();
    }

    /// A read's answer, published only while it is the one in flight.
    fn answer(
        &mut self,
        request: &Request<(String, u64)>,
        result: Result<Read, String>,
        cx: &mut Context<Self>,
    ) {
        let first = self.loader.data().is_none();
        if self.loader.land(request, result, cx) {
            if first {
                // The caught-up group starts folded while anything waits.
                self.folded = true;
            }
            self.show_read(cx);
        }
    }

    /// What the find box holds.
    fn query_text(&self, cx: &App) -> String {
        self.find.read(cx).value().to_string()
    }

    /// Derives the display from the loader and the find box, and the lines.
    fn show_read(&mut self, cx: &mut Context<Self>) {
        let cluster = self.source.as_ref().map(|s| s.context.clone());
        let text = self.query_text(cx);
        self.display = Display::new(
            self.loader.data().map(|read| &read.waiting),
            self.loader.error(),
            cluster.as_deref().unwrap_or_default(),
            &text,
        );
        self.stale = self.loader.data().and(self.loader.error()).map(|why| {
            let text = format!("Showing the last read. {why}");
            let label = format!("Couldn't read again: {text}");
            (text.into(), label.into())
        });
        if self.display.waiting == 0 {
            self.folded = false;
        }
        self.status = self.segment();
        self.rebuild(cx);
    }

    /// The lines, when the rows or the fold change; drawing only reads
    /// them. A row that went is no longer selected. A search shows every
    /// group it found something in, folded or not.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let searching = self.display.query.is_some();
        self.lines.clear();
        self.shown_rows = 0;
        for (ix, group) in self.display.groups.iter().enumerate() {
            self.lines.push(Entry::Group(ix));
            if group.folds && self.folded && !searching {
                continue;
            }
            for (row_ix, row) in self.display.rows.iter().enumerate() {
                if row.group == ix {
                    self.shown_rows += 1;
                    self.lines.push(Entry::Row(row_ix));
                }
            }
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|key| !self.display.rows.iter().any(|row| &row.key == key))
        {
            self.selected = None;
            self.inspect = false;
        }
        cx.notify();
    }

    fn segment(&self) -> Segment {
        let context = self.source.as_ref().map(|s| s.context.clone());
        match &self.display.body {
            Body::Refused { .. } => {
                let part = Part::new("Kargo not permitted").tone(Tone::Unknown);
                return Segment::new(context, vec![part]);
            }
            Body::Failed { .. } => {
                let part = Part::new("couldn't read what waits").tone(Tone::Warn);
                return Segment::new(context, vec![part]);
            }
            Body::Table | Body::Empty { .. } => {}
        }
        if self.loader.data().is_none() {
            return Segment::default();
        }
        let (items, apps) = (self.display.items, self.display.waiting);
        let mut parts = vec![Part::new(format!(
            "{items} Freight waiting in {apps} {}",
            if apps == 1 {
                "application"
            } else {
                "applications"
            }
        ))];
        let unread = self
            .display
            .groups
            .iter()
            .find(|group| group.fold == core_waiting::Fold::NotRead)
            .map_or(0, |group| group.rows);
        if unread > 0 {
            parts.push(Part::new(format!("{unread} not read")).tone(Tone::Unknown));
        }
        if self.loader.is_stale() {
            parts.push(Part::new("last read").tone(Tone::Warn));
        }
        Segment::new(context, parts)
    }

    /// The motion over the loading rows, which the shell mounts beside the
    /// cached page.
    pub(crate) fn loading_motion(&self) -> Option<Entity<kit::LoadingMotion>> {
        kit::TableSource::loading(self)
            .is_some()
            .then(|| self.loading_motion.clone())
    }

    /// Puts the keyboard on the page.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    /// The keyboard handle of the page.
    pub(crate) fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn selected_row(&self) -> Option<&WaitRow> {
        let key = self.selected.as_ref()?;
        self.display.rows.iter().find(|row| &row.key == key)
    }

    /// Selects a row, unfolding its group so it shows.
    pub fn select(&mut self, key: &str, window: &Window, cx: &mut Context<Self>) {
        let Some(row) = self.display.rows.iter().find(|row| row.key.as_ref() == key) else {
            return;
        };
        let (key, group) = (row.key.clone(), row.group);
        self.selected = Some(key);
        self.inspect = true;
        if self.display.groups[group].folds && self.folded {
            self.folded = false;
            self.rebuild(cx);
        }
        kit::reveal(self, ScrollStrategy::Nearest);
        if freshkube_ui::page::page_width(window) < freshkube_ui::inspector::SPLIT_WIDTH {
            kit::reveal_when_settled(
                cx.entity().downgrade(),
                |page| page.selected.is_some(),
                window,
            );
        }
        cx.notify();
    }

    fn step(&mut self, delta: isize, window: &Window, cx: &mut Context<Self>) {
        if let Some(key) = kit::step(self, delta, cx) {
            self.select(&key, window, cx);
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

    fn toggle_fold(&mut self, cx: &mut Context<Self>) {
        self.folded = !self.folded;
        self.rebuild(cx);
    }

    fn unfold_all(&mut self, cx: &mut Context<Self>) {
        self.folded = false;
        self.rebuild(cx);
    }
}

#[cfg(any(test, feature = "testing"))]
impl WaitingPage {
    /// The lines, as the table shows them: groups as `# title`, rows by
    /// their key.
    pub fn lines_text(&self) -> Vec<String> {
        self.lines
            .iter()
            .map(|entry| match entry {
                Entry::Group(ix) => format!("# {}", self.display.groups[*ix].title),
                Entry::Row(ix) => self.display.rows[*ix].key.to_string(),
            })
            .collect()
    }

    pub fn selected(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }

    pub fn is_reading(&self) -> bool {
        self.loader.is_loading()
    }

    pub fn body(&self) -> &Body {
        &self.display.body
    }

    /// Types into the find box, as a person does.
    pub fn find_text(&self, cx: &App) -> String {
        self.query_text(cx)
    }

    /// An answer, as a read brings it.
    pub fn read_answer(&mut self, waiting: Waiting, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let request = self.loader.start((source.id, self.setup_revision));
        self.answer(&request, Ok(Read { waiting }), cx);
    }

    /// A read that fails, as a cluster that stops answering does.
    pub fn fail_read(&mut self, why: &str, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let request = self.loader.start((source.id, self.setup_revision));
        self.answer(&request, Err(why.into()), cx);
    }

    /// The find box, for a test that types into it.
    pub fn find_input(&self) -> &Entity<InputState> {
        &self.find
    }
}
