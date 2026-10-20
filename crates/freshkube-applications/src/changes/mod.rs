//! The Changes list (#564): every Freight of the open cluster's Kargo
//! projects as a change, with how sure it is that each Stage's pods run
//! it, as a table page in the Applications area. Core's
//! `delivery::changes` derives the rows; this page shows them, grouped
//! Not fully confirmed, Confirmed to the running pods, then Not running in
//! any Stage, the last two folded while anything needs a look. The
//! selection's trail shows in the Inspector, and Enter or its button opens
//! the change page in the list's place; its breadcrumb comes back.
//!
//! The Applications page owns it and tells it when it shows. It reads only
//! while shown, by the change page's rules: on showing once what it has is
//! older than [`FRESH_FOR`], on Refresh, and never while hidden, which
//! drops the read in flight. Another connection drops what was read.
pub(crate) mod display;
pub(crate) mod outcome;
mod table;
mod view;

use std::collections::BTreeSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use freshkube_core::snapshot::Request;
use freshkube_delivery::change::example::Shape;
use freshkube_delivery::change::live::Place;
use freshkube_delivery::changes::{self as core_changes, Changes as Derived, Fold, live::Where};
use freshkube_delivery::entry::{self, pool};
use freshkube_reads::{Loader, OnShow};
use freshkube_ui::inspector::InspectorSplit;
use freshkube_ui::status::{Part, Segment};
use freshkube_ui::table::{self as kit, TableState};
use freshkube_ui::ui::Tone;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::across::{Across, Alone};
use super::change::{ChangeEvent, ChangePage, Coroot, Fetch};
use super::links::Connections;
use crate::backend;
use crate::resources::{ClusterAccess, ClusterSource, ResourceLink};
use display::{Body, ChangeRow, Display};

/// The page's id prefix: `changes-title`, `changes-row-<key>`, …
const PREFIX: &str = "changes";
/// The list's key context, drawn in every state.
pub(crate) const CONTEXT: &str = "Changes";
/// A read this recent is shown again without reading.
const FRESH_FOR: Duration = Duration::from_secs(30);
/// Showing the list reads again once what it has is [`FRESH_FOR`] old.
const ON_SHOW: OnShow = OnShow::ReadIfOlder(FRESH_FOR);
/// How long one read may take before it fails whole: past core's budget,
/// at which it answers with what was read in time.
const DEADLINE: Duration = Duration::from_secs(core_changes::live::BUDGET.as_secs() + 30);

gpui_kit::actions!(
    changes,
    [
        /// Selects the next change.
        NextChange,
        /// Selects the previous change.
        PreviousChange,
        /// Clears the selection, which closes the Inspector.
        ClearChange,
        /// Opens the selected change's page.
        OpenChange
    ]
);

pub(crate) fn key_bindings() -> [KeyBinding; 4] {
    [
        KeyBinding::new("down", NextChange, Some(CONTEXT)),
        KeyBinding::new("up", PreviousChange, Some(CONTEXT)),
        KeyBinding::new("escape", ClearChange, Some(CONTEXT)),
        KeyBinding::new("enter", OpenChange, Some(CONTEXT)),
    ]
}

/// What the page asks of the page that owns it: the change page's links.
pub(crate) enum ChangesEvent {
    Open(Box<ResourceLink>),
    Switch(SharedString, Box<ResourceLink>),
    Revision(crate::observability::RevisionLink),
}

/// One read: the list, and when example data's times count from.
#[derive(Clone)]
struct Read {
    changes: Derived,
    /// Example data's reference time, which its change pages share.
    example: Option<DateTime<Utc>>,
}

/// One line of the table: a group header, or a row by index.
#[derive(Clone, Copy)]
enum Entry {
    Group(Fold),
    Row(usize),
}

pub struct ChangesPage {
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
    /// connection. It is hidden while the list is, and its answer's age
    /// decides whether showing reads.
    loader: Loader<Read, (String, u64)>,
    /// Which connection each cluster is, for the change page's links.
    connections: Connections,
    /// Where a live read reads each part, and how the workspace maps Argo
    /// CD's destinations.
    across: Across,
    /// Where an opened change gets what Coroot found after a deploy.
    coroot: Coroot,
    outcome_memo: Entity<crate::change::SessionMemo>,
    _outcomes: Subscription,
    _connection: Option<Subscription>,
    /// Where Argo CD's Applications are read, as the Applications read
    /// found it.
    #[cfg(any(test, feature = "testing"))]
    pub argocd_namespace: String,
    #[cfg(not(any(test, feature = "testing")))]
    argocd_namespace: String,
    /// The namespace the shown live read used, or the one in flight.
    #[cfg(any(test, feature = "testing"))]
    pub read_namespace: Option<String>,
    #[cfg(not(any(test, feature = "testing")))]
    read_namespace: Option<String>,
    #[cfg(any(test, feature = "testing"))]
    pub display: Display,
    #[cfg(not(any(test, feature = "testing")))]
    display: Display,
    /// A refresh that failed over an earlier read, and its label.
    stale: Option<(SharedString, SharedString)>,
    /// The setup owner's sanitized problem, independent of cluster reads.
    setup_problem: Option<SharedString>,
    lines: Vec<Entry>,
    /// Each group's rows, in [`Fold::ALL`]'s order, and the rows the lines
    /// show; derived with the lines.
    group_rows: [usize; 3],
    shown_rows: usize,
    /// The groups folded.
    folded: BTreeSet<Fold>,
    columns: Vec<table::Column>,
    width: f32,
    table: TableState,
    loading: kit::LoadingRows,
    loading_motion: Entity<kit::LoadingMotion>,
    focus: FocusHandle,
    split: InspectorSplit,
    /// The frame's scroll, used while the window is short.
    page_scroll: ScrollHandle,
    selected: Option<SharedString>,
    /// Whether the Inspector shows the selection. A right-click selects
    /// without opening it.
    inspect: bool,
    /// The change shown in the list's place.
    change: Option<(Entity<ChangePage>, Subscription)>,
    /// `6 changes · 2 not fully confirmed`, in the status bar.
    pub(crate) status: Segment,
}

impl EventEmitter<ChangesEvent> for ChangesPage {}

impl ChangesPage {
    pub(crate) fn new(
        runtime: tokio::runtime::Handle,
        shape: Shape,
        secrets: Option<freshkube_core::secrets::Secrets>,
        cx: &mut Context<Self>,
    ) -> Self {
        let loading = kit::LoadingRows::new(PREFIX);
        let loading_motion = cx.new(|_| loading.motion(kit::Look::Pulse));
        let display = Display::default();
        let (columns, width) = table::columns(table::STAGES_MAX);
        // The list starts hidden, and so does its read.
        let mut loader = Loader::default();
        loader.set_visible(false, ON_SHOW, cx);
        let outcome_memo = crate::change::session_memo(cx);
        let outcomes = cx.observe(&outcome_memo, |page, _, cx| page.sync_outcomes(cx));
        Self {
            runtime,
            secrets,
            setup_revision: 0,
            source: None,
            visible: false,
            shape,
            example_delay: Duration::ZERO,
            loader,
            connections: Connections::default(),
            across: Across::default(),
            coroot: Coroot::None,
            outcome_memo,
            _outcomes: outcomes,
            _connection: None,
            argocd_namespace: "argocd".into(),
            read_namespace: None,
            display,
            stale: None,
            setup_problem: None,
            lines: Vec::new(),
            group_rows: [0; 3],
            shown_rows: 0,
            folded: BTreeSet::new(),
            columns,
            width,
            table: TableState::new(PREFIX),
            loading,
            loading_motion,
            focus: cx.focus_handle(),
            split: InspectorSplit::new(PREFIX, cx),
            page_scroll: ScrollHandle::new(),
            selected: None,
            inspect: false,
            change: None,
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
    /// read, what is in flight and the change shown; the page reads it
    /// again if shown.
    pub fn set_source(&mut self, source: Option<ClusterSource>, cx: &mut Context<Self>) {
        let same = self.source.as_ref().map(|s| &s.id) == source.as_ref().map(|s| &s.id);
        self.source = source;
        if same {
            return;
        }
        self.loader.reset();
        self.read_namespace = None;
        self.argocd_namespace = "argocd".into();
        self.selected = None;
        self.inspect = false;
        self.change = None;
        self.folded.clear();
        self.show_read(cx);
        if self.visible {
            self.read(cx);
        }
    }

    /// Which connection each cluster is, and where a live read reads each
    /// part; the change shown follows them.
    pub fn set_clusters(
        &mut self,
        connections: Connections,
        across: Across,
        cx: &mut Context<Self>,
    ) {
        let remapped = across != self.across;
        self.connections = connections.clone();
        self.across = across.clone();
        self.sync_outcomes(cx);
        if let Some((change, _)) = &self.change {
            change.update(cx, |page, cx| page.set_clusters(across, connections, cx));
        }
        // A live list reads each part where the workspace says and
        // resolves its Stages by the mapping: read it again, now if shown,
        // else when next shown. The change shown read itself again above.
        if remapped && self.is_live() {
            self.read_again(cx);
        }
    }

    pub(crate) fn set_setup_problem(
        &mut self,
        problem: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        if self.setup_problem == problem {
            return;
        }
        self.setup_problem = problem.clone();
        if let Some((change, _)) = &self.change {
            change.update(cx, |page, cx| page.set_setup_problem(problem, cx));
        }
        cx.notify();
    }

    /// A setup reload replaces trust and cancels every delivery read made
    /// with the old setup, including a change opened from this list.
    pub(crate) fn reload_setup(
        &mut self,
        public_key_file: Option<std::path::PathBuf>,
        evidence_result: Option<freshkube_core::setup::Evidence>,
        build_namespace: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.setup_revision += 1;
        self.across.public_key_file = public_key_file.clone();
        self.across.evidence_result = evidence_result.clone();
        self.across.build_namespace = build_namespace.clone();
        if let Some((change, _)) = &self.change {
            change.update(cx, |page, cx| {
                page.reload_setup(public_key_file, evidence_result, build_namespace, cx)
            });
        }
        if self.is_live() {
            self.loader.reset();
            self.show_read(cx);
            if self.visible {
                self.read(cx);
            }
        }
    }

    /// Where the change opened from the list gets what Coroot found after
    /// a deploy; the change shown follows.
    pub fn set_coroot(&mut self, coroot: Coroot, cx: &mut Context<Self>) {
        if coroot == self.coroot {
            return;
        }
        self._connection = match &coroot {
            Coroot::Live(live) => live
                .reader
                .as_ref()
                .and_then(|reader| reader.on_connection(cx, |page, cx| page.sync_outcomes(cx))),
            _ => None,
        };
        self.coroot = coroot.clone();
        self.sync_outcomes(cx);
        if let Some((change, _)) = &self.change {
            change.update(cx, |page, cx| page.set_coroot(coroot, cx));
        }
        cx.notify();
    }

    /// Where the Applications read found Argo CD's Applications. A live
    /// list read in another namespace reads again, now if shown, else when
    /// next shown.
    pub fn set_argocd_namespace(&mut self, namespace: String, cx: &mut Context<Self>) {
        if namespace == self.argocd_namespace {
            return;
        }
        self.argocd_namespace = namespace;
        let read_elsewhere = self
            .read_namespace
            .as_ref()
            .is_some_and(|read| *read != self.argocd_namespace);
        if read_elsewhere && self.is_live() {
            self.read_again(cx);
        }
    }

    /// Whether the page reads a cluster, not example data.
    fn is_live(&self) -> bool {
        self.source
            .as_ref()
            .is_some_and(|source| !matches!(source.access, ClusterAccess::Example(_)))
    }

    /// What was read is no longer fresh: read the list now if shown.
    fn read_again(&mut self, cx: &mut Context<Self>) {
        self.loader.expire();
        if self.visible && (self.loader.is_loading() || self.loader.data().is_some()) {
            self.read(cx);
        }
    }

    /// Shown, the page reads unless what it has is fresh; hidden, it drops
    /// the read in flight. The change shown follows.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        if let Some((change, _)) = &self.change {
            change.update(cx, |change, cx| change.set_visible(visible, cx));
        }
        let reads_again = self.loader.set_visible(visible, ON_SHOW, cx);
        if !visible {
            cx.notify();
            return;
        }
        if reads_again {
            self.read(cx);
        }
    }

    /// Refresh: reads again while shown; a change shown reads itself.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        if let Some((change, _)) = &self.change {
            change.update(cx, |change, cx| change.refresh(cx));
        }
        self.read(cx);
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
                let now = Utc::now();
                let read = Read {
                    changes: core_changes::derive(&core_changes::example::read_shaped(
                        now, self.shape,
                    )),
                    example: Some(now),
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
                self.read_namespace = Some(self.argocd_namespace.clone());
                let at = Where {
                    cluster: source.id.clone(),
                    label: source.context.clone(),
                    argocd_namespace: self.argocd_namespace.clone(),
                    mapping: self.across.mapping.clone(),
                    sources: self.across.sources.clone(),
                    public_key_file: self.across.public_key_file.clone(),
                    evidence_result: self.across.evidence_result.clone(),
                    build_namespace: self.across.build_namespace.clone(),
                };
                let across = self.across.clone();
                let secrets = self.secrets.clone();
                let (job, receiver) = backend::spawn_job(
                    &self.runtime,
                    DEADLINE,
                    "Reading the changes timed out".into(),
                    async move {
                        // The list has read with entries' deadlines since before there
                        // was a workspace.
                        let open = across.open(&access, Alone::AsEntry).await?;
                        let kubeconfigs = across.kubeconfigs();
                        let pool = pool::Pool::new(
                            &at.sources,
                            Some(pool::Open {
                                entry: at.mapping.open.as_deref(),
                                reader: open.as_ref().map_err(Clone::clone),
                            }),
                            |chosen| entry::connect_entry(kubeconfigs.clone(), chosen),
                            core_changes::live::BUDGET,
                        )
                        .with_secrets(secrets);
                        let read = core_changes::live::read_in(
                            &pool,
                            &at,
                            Utc::now(),
                            core_changes::live::BUDGET,
                        )
                        .await;
                        Ok(Read {
                            changes: core_changes::derive(&read),
                            example: None,
                        })
                    },
                );
                let task = cx.spawn(async move |this, cx| {
                    let result = receiver
                        .await
                        .unwrap_or_else(|_| Err("Reading the changes stopped".into()));
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
        if self.loader.land(request, result, cx) {
            self.show_read(cx);
        }
    }

    /// Derives the display from the loader. The first answer folds the
    /// groups that may fold while anything needs a look; a later one keeps
    /// the folds while something still does.
    fn show_read(&mut self, cx: &mut Context<Self>) {
        let cluster = self.source.as_ref().map(|s| s.context.clone());
        let first = self.display.rows.is_empty();
        self.display = Display::new(
            self.loader.data().map(|read| &read.changes),
            self.loader.error(),
            cluster.as_deref().unwrap_or_default(),
            Utc::now(),
        );
        self.sync_outcomes(cx);
        self.stale = self.loader.data().and(self.loader.error()).map(|why| {
            let text = format!("Showing the last read. {why}");
            let label = format!("Couldn't read again: {text}");
            (text.into(), label.into())
        });
        if !self.display.needs_a_look {
            self.folded.clear();
        } else if first {
            self.folded = Fold::ALL.into_iter().filter(|fold| fold.folds()).collect();
        }
        let stages = table::fit_stages(&mut self.display.rows, cx);
        (self.columns, self.width) = table::columns(stages);
        self.status = self.segment();
        self.rebuild(cx);
    }

    /// The lines, when the rows or the folds change; drawing only reads
    /// them. A row that went is no longer selected.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.lines.clear();
        self.shown_rows = 0;
        for (group, fold) in Fold::ALL.into_iter().enumerate() {
            let rows: Vec<usize> = (0..self.display.rows.len())
                .filter(|&ix| self.display.rows[ix].fold == fold)
                .collect();
            self.group_rows[group] = rows.len();
            if rows.is_empty() {
                continue;
            }
            self.lines.push(Entry::Group(fold));
            if !self.folded.contains(&fold) {
                self.shown_rows += rows.len();
                self.lines.extend(rows.into_iter().map(Entry::Row));
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
                let part = Part::new("couldn't read changes").tone(Tone::Warn);
                return Segment::new(context, vec![part]);
            }
            Body::Table | Body::Empty { .. } => {}
        }
        if self.loader.data().is_none() {
            return Segment::default();
        }
        let total = self.display.rows.len();
        let mut parts = vec![Part::new(format!(
            "{total} {}",
            if total == 1 { "change" } else { "changes" }
        ))];
        let looks = self
            .display
            .rows
            .iter()
            .filter(|row| row.fold == Fold::NotConfirmed)
            .count();
        if looks > 0 {
            parts.push(Part::new(format!("{looks} not fully confirmed")).tone(Tone::Unknown));
        }
        if self.loader.is_stale() {
            parts.push(Part::new("last read").tone(Tone::Warn));
        }
        Segment::new(context, parts)
    }

    /// The motion over the loading rows, which the shell mounts beside the
    /// cached page.
    pub(crate) fn loading_motion(&self, cx: &App) -> Option<Entity<kit::LoadingMotion>> {
        if let Some((change, _)) = &self.change {
            return change.read(cx).loading_motion();
        }
        kit::TableSource::loading(self)
            .is_some()
            .then(|| self.loading_motion.clone())
    }

    /// Puts the keyboard on the list.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    /// Puts the keyboard on what shows: the change, or the list.
    pub(crate) fn focus_shown(&self, window: &mut Window, cx: &mut App) {
        let focus = self.shown_focus(cx);
        window.focus(&focus, cx);
    }

    /// The keyboard handle of what shows: the change, or the list.
    pub(crate) fn shown_focus(&self, cx: &App) -> FocusHandle {
        match &self.change {
            Some((change, _)) => change.read(cx).focus_handle(),
            None => self.focus.clone(),
        }
    }

    /// The change shown in the list's place, if one is.
    #[cfg(any(test, feature = "testing"))]
    pub fn change_page(&self) -> Option<&Entity<ChangePage>> {
        self.change.as_ref().map(|(page, _)| page)
    }

    fn selected_row(&self) -> Option<&ChangeRow> {
        let key = self.selected.as_ref()?;
        self.display.rows.iter().find(|row| &row.key == key)
    }

    /// Selects a change, unfolding its group so its row shows.
    pub fn select(&mut self, key: &str, window: &Window, cx: &mut Context<Self>) {
        let Some(row) = self.display.rows.iter().find(|row| row.key.as_ref() == key) else {
            return;
        };
        let (key, fold) = (row.key.clone(), row.fold);
        self.selected = Some(key);
        self.inspect = true;
        if self.folded.remove(&fold) {
            self.rebuild(cx);
        }
        self.sync_outcomes(cx);
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

    fn toggle_fold(&mut self, fold: Fold, cx: &mut Context<Self>) {
        if !self.folded.remove(&fold) {
            self.folded.insert(fold);
        }
        self.rebuild(cx);
    }

    fn unfold_all(&mut self, cx: &mut Context<Self>) {
        self.folded.clear();
        self.rebuild(cx);
    }

    /// Enter, a double-click or the Inspector's button: the selected
    /// change's page in the list's place, with the keyboard on it.
    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let Some(fetch) = self.fetch(&row.project, &row.freight) else {
            return;
        };
        let connections = self.connections.clone();
        let visible = self.visible;
        let coroot = self.coroot.clone();
        let page = cx.new(|cx| {
            let mut page = ChangePage::new(fetch, None, connections, visible, window, cx);
            page.set_parent("Changes");
            page.set_setup_problem(self.setup_problem.clone(), cx);
            page.set_coroot(coroot, cx);
            page
        });
        let subscription =
            cx.subscribe_in(&page, window, |this, _, event, window, cx| match event {
                ChangeEvent::Back => this.close_change(window, cx),
                ChangeEvent::Open(link) => cx.emit(ChangesEvent::Open(link.clone())),
                ChangeEvent::Switch(entry, link) => {
                    cx.emit(ChangesEvent::Switch(entry.clone(), link.clone()))
                }
                ChangeEvent::Revision(link) => cx.emit(ChangesEvent::Revision(link.clone())),
            });
        window.focus(&page.read(cx).focus_handle(), cx);
        self.change = Some((page, subscription));
        cx.notify();
    }

    /// What reads a Freight's change: example data's, at the list's
    /// reference time, or the open connection's.
    fn fetch(&self, project: &str, freight: &str) -> Option<Fetch> {
        let source = self.source.as_ref()?;
        let read = self.loader.data()?;
        Some(match (&source.access, read.example) {
            (ClusterAccess::Example(_), Some(now)) => Fetch::Example {
                project: project.to_owned(),
                freight: freight.to_owned(),
                now,
                shape: self.shape,
                delay: self.example_delay,
            },
            (ClusterAccess::Example(_), None) => return None,
            (access, _) => Fetch::Live {
                runtime: self.runtime.clone(),
                access: access.clone(),
                secrets: self.secrets.clone(),
                setup_revision: self.setup_revision,
                place: Box::new(Place {
                    cluster: source.id.clone(),
                    label: source.context.clone(),
                    project: project.to_owned(),
                    freight: freight.to_owned(),
                    argocd_namespace: self.argocd_namespace.clone(),
                    mapping: self.across.mapping.clone(),
                    sources: self.across.sources.clone(),
                    public_key_file: self.across.public_key_file.clone(),
                    evidence_result: self.across.evidence_result.clone(),
                    build_namespace: self.across.build_namespace.clone(),
                }),
                kubeconfig: self.across.kubeconfig.clone(),
            },
        })
    }

    /// Back to the list, with the selection it had.
    pub(crate) fn close_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.change.take().is_some() {
            self.focus(window, cx);
            cx.notify();
        }
    }
}

#[cfg(any(test, feature = "testing"))]
impl ChangesPage {
    /// The rows' Freight, in the order the lines show them.
    pub fn lines_text(&self) -> Vec<String> {
        self.lines
            .iter()
            .map(|entry| match entry {
                Entry::Group(fold) => format!("# {}", fold.label()),
                Entry::Row(ix) => self.display.rows[*ix].freight.clone(),
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

    /// An answer, as a read brings it.
    pub fn read_answer(&mut self, changes: Derived, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let request = self.loader.start((source.id, self.setup_revision));
        let read = Read {
            changes,
            example: None,
        };
        self.answer(&request, Ok(read), cx);
    }

    /// A read that fails, as a cluster that stops answering does.
    pub fn fail_read(&mut self, why: &str, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let request = self.loader.start((source.id, self.setup_revision));
        self.answer(&request, Err(why.into()), cx);
    }
}
