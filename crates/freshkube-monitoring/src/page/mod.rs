//! The Monitoring page: one dashboard at a time, from the built-ins or the
//! user's folder, answered by the cluster's own Prometheus API (found or
//! chosen in Settings) through the service proxy, by a URL the user
//! entered, or by example data with `--fixture`.
//!
//! The shell owns the page and tells it when it shows. Only a visible page
//! reads: it finds Prometheus, resolves the variables, and asks the panels
//! in or near the viewport. Every change that asks again (a source, a
//! dashboard, a variable, a range, a refresh) takes a new generation, and
//! an answer for an older one is dropped. Hiding drops the page's reads;
//! a client rebuild requested by a visible history keeps its own lifetime.
mod board;
mod catalog;
mod connection;
mod layout;
mod markers;
mod settings;
mod source;
#[cfg(test)]
mod tests;
mod view;

use std::cell::Cell;
use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use freshkube_core::cluster_source::{ClusterAccess, ClusterSource};
use freshkube_core::monitoring::scope::Resolved;
use freshkube_core::monitoring::{Endpoint, QueryError};
use freshkube_reads::Loader;
use gpui_kit::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, ScrollHandle, Subscription, Task,
    WeakEntity,
};
use tokio::runtime::Handle;

use super::store::{MonitoringStore, Saved};
use crate::alerts::{AlertObject, AlertsEvent, AlertsPage, Which};

use board::Board;
pub use catalog::{Catalog, Entry, EntryId, FolderState};
use connection::{Connection, ConnectionReads};
use markers::MarkerState;
pub use markers::TalosNodes;
pub use settings::{settings_section, source_section};

/// How long one request through the proxy may take, past the transport's
/// own timeout, before the page gives up on it.
const DEADLINE: Duration = Duration::from_secs(45);

/// Completion identity contains no credentials. The request generation is
/// the loader's; the page's generation schedules viewport work.
#[derive(Clone, PartialEq, Eq)]
struct ReadIdentity {
    source: Option<String>,
    client: u64,
    kind: ReadKind,
}

#[derive(Clone, PartialEq, Eq)]
enum ReadKind {
    Variables {
        board: u64,
    },
    Panel {
        board: u64,
        panel: usize,
    },
    Connect {
        endpoint: Option<Endpoint>,
    },
    Confirm {
        service: freshkube_core::monitoring::PrometheusService,
    },
    Rebuild {
        source: (
            String,
            Option<Endpoint>,
            bool,
            u64,
            Option<gpui_kit::EntityId>,
        ),
    },
    KubeMarkers {
        since: i64,
    },
    TalosMarkers {
        connection: u64,
        nodes: Vec<(String, String)>,
        since: i64,
    },
    Test {
        context: String,
    },
}

type Read = Loader<(), Option<ReadIdentity>>;
/// The token also tracks a pending control or panel; it owns no work.
type Request = freshkube_core::snapshot::Request<Option<ReadIdentity>>;

/// What the page tells the shell.
#[derive(Clone, Debug, PartialEq)]
pub enum MonitoringEvent {
    /// The column's dashboards changed, or which one is open.
    Catalog,
    /// The Prometheus that pod and node history read may have changed.
    History,
    /// The breadcrumb's Dashboards: show the column that lists them.
    Dashboards,
    /// An alert's label names an object, for Resources to open.
    OpenObject(AlertObject),
    /// Which of the column's pages shows changed.
    Section,
    /// How the open entry's metrics, and so its alerts, are found changed:
    /// the session's alert read follows.
    Scope,
}

/// What the page shows: a dashboard, or one of the alert pages (#884).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Dashboards,
    Alerts(Which),
}

/// The part of the dashboard in view, in dp from the grid's top: `top` is
/// negative while the grid starts below the top of what is in view.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Viewport {
    top: f32,
    height: f32,
    /// The panels stack in one column.
    narrow: bool,
    /// How far the scroller had scrolled when this was measured, in dp.
    scrolled: f32,
}

impl Default for Viewport {
    /// Before the first frame lays the page out: a tall window at the top.
    fn default() -> Self {
        Self {
            top: 0.,
            height: 900.,
            narrow: false,
            scrolled: 0.,
        }
    }
}

impl Viewport {
    /// The part of the grid whose panels are asked and drawn: what is in
    /// view, half a screen above and a screen below.
    fn reach(&self) -> (f32, f32) {
        (self.top - self.height / 2., self.top + self.height * 2.)
    }

    /// The viewport once the scroller has moved on to `scrolled` dp: it is
    /// measured while a frame is laid out, one scroll behind the next one.
    fn at(self, scrolled: f32) -> Self {
        Self {
            top: self.top + scrolled - self.scrolled,
            scrolled,
            ..self
        }
    }
}

/// How the open entry's metrics are found, handed in by the shell from the
/// setup file: `resolved`, and the cluster that hosts a central store's
/// Service (the open one, or another entry's). Discovery, the default, is
/// how the page found Prometheus before the file.
#[derive(Clone, Debug)]
pub struct Scope {
    pub resolved: Resolved,
    pub host: Option<ClusterAccess>,
}

impl Default for Scope {
    fn default() -> Self {
        Self {
            resolved: Resolved::Discovery,
            host: None,
        }
    }
}

/// What decides the page's scope besides its connection, kept on the page so
/// each window has its own: the last good setup, the workspace entry the
/// window is on, and the clusters opened to reach a store another entry
/// hosts (kept while their context and kubeconfig are unchanged).
#[derive(Default)]
pub struct ScopeInputs {
    pub setup: Option<freshkube_core::setup::Setup>,
    pub entry: Option<String>,
    pub hosts: std::collections::BTreeMap<String, freshkube_core::cluster_source::DirectAccess>,
}

pub struct MonitoringPage {
    runtime: Handle,
    store: Option<MonitoringStore>,
    saved: Saved,
    /// Whether the dashboards show: the page shows and no alert page
    /// takes its place. Every dashboard read follows it.
    visible: bool,
    /// Whether the page shows at all, as the shell says.
    shown: bool,
    section: Section,
    /// The Alerts, Rules and Silences pages, in [`Which::ALL`]'s order.
    alerts: [Entity<AlertsPage>; 3],
    _alerts: Vec<Subscription>,
    source: Option<ClusterSource>,
    connection: Connection,
    connection_reads: ConnectionReads,
    /// How the setup file says the open entry's metrics are found.
    scope: Scope,
    inputs: ScopeInputs,
    /// Counts the Ready clients built again, so history takes the new one.
    clients: u64,
    /// History reaches this connection owner without retaining the page.
    owner: WeakEntity<Self>,
    /// Only the histories currently shown through this connection.
    histories: Vec<WeakEntity<crate::history::HistoryView>>,
    catalog: Catalog,
    /// The dashboard chosen, kept across contexts.
    chosen: EntryId,
    board: Option<Board>,
    /// Counts every change that asks the panels again; answers carry it.
    generation: u64,
    refresh_every: Option<u64>,
    refresh_task: Option<Task<()>>,
    scroll: ScrollHandle,
    viewport: Rc<Cell<Viewport>>,
    focus: FocusHandle,
    /// Deploys and node events across the charts.
    markers: MarkerState,
    /// The shell's Talos nodes, for reboots.
    talos: Option<TalosNodes>,
    /// Why the last save failed, for Settings.
    save_error: Option<gpui_kit::SharedString>,
    /// Where URL tokens are kept; none in example mode and tests.
    secrets: Option<freshkube_core::secrets::Secrets>,
    /// Tokens typed this session, by URL, so a connection never waits
    /// for the store to finish writing one.
    tokens: std::collections::BTreeMap<String, String>,
    /// The last credential store step; the next one waits for it.
    secret_queue: Option<futures::channel::oneshot::Receiver<()>>,
    /// Settings' Metrics source form, made when Settings first draws it.
    form: Option<source::SourceForm>,
    /// Unix seconds now. Tests fix it, so example data is the same each run.
    now: fn() -> i64,
}

impl EventEmitter<MonitoringEvent> for MonitoringPage {}

impl MonitoringPage {
    pub fn new(
        runtime: Handle,
        preferences: Option<&std::path::Path>,
        secrets: Option<freshkube_core::secrets::Secrets>,
        cx: &mut Context<Self>,
    ) -> Self {
        let saved = preferences.map(super::store::load).unwrap_or_default();
        let store = preferences.map(|path| MonitoringStore::new(path, saved.clone()));
        let alerts = Which::ALL.map(|which| {
            let runtime = runtime.clone();
            cx.new(|cx| AlertsPage::new(which, runtime, cx))
        });
        let alert_subscriptions = alerts
            .iter()
            .map(|page| {
                cx.subscribe(page, |_, _, event: &AlertsEvent, cx| match event {
                    AlertsEvent::Open(object) => {
                        cx.emit(MonitoringEvent::OpenObject(object.clone()))
                    }
                })
            })
            .collect();
        let mut page = Self {
            runtime,
            store,
            saved,
            visible: false,
            shown: false,
            section: Section::Dashboards,
            alerts,
            _alerts: alert_subscriptions,
            source: None,
            connection: Connection::None,
            connection_reads: ConnectionReads::default(),
            scope: Scope::default(),
            inputs: ScopeInputs::default(),
            clients: 0,
            owner: cx.weak_entity(),
            histories: Vec::new(),
            catalog: Catalog::new(),
            chosen: EntryId::Builtin(freshkube_core::monitoring::builtin::BUILTINS[0].uid),
            board: None,
            generation: 0,
            refresh_every: None,
            refresh_task: None,
            scroll: ScrollHandle::new(),
            viewport: Rc::default(),
            focus: cx.focus_handle(),
            markers: MarkerState::default(),
            talos: None,
            save_error: None,
            secrets,
            tokens: Default::default(),
            secret_queue: None,
            form: None,
            now: || chrono::Utc::now().timestamp(),
        };
        page.read_folder(cx);
        page
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn chosen(&self) -> &EntryId {
        &self.chosen
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// Shows or hides the page. What shows is the chosen dashboard or alert
    /// page: showing connects and asks what the viewport needs, hiding
    /// drops the page's reads and refresh timer, keeping a rebuild
    /// requested by a history that still shows.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.shown = visible;
        self.sync_visible(cx);
    }

    /// Tells the dashboards and the alert pages whether they show: only
    /// the one the section names, and only while the page does.
    fn sync_visible(&mut self, cx: &mut Context<Self>) {
        let dashboards = self.shown && self.section == Section::Dashboards;
        for (which, page) in Which::ALL.into_iter().zip(&self.alerts) {
            let shows = self.shown && self.section == Section::Alerts(which);
            page.update(cx, |page, cx| page.set_visible(shows, cx));
        }
        self.set_dashboards_visible(dashboards, cx);
    }

    /// The Alerts, Rules or Silences page shows in place of the dashboard.
    pub fn show_alerts(&mut self, which: Which, cx: &mut Context<Self>) {
        if self.section == Section::Alerts(which) {
            return;
        }
        self.section = Section::Alerts(which);
        self.sync_visible(cx);
        cx.emit(MonitoringEvent::Section);
        cx.notify();
    }

    /// The dashboard shows in place of an alert page.
    pub(super) fn show_dashboards(&mut self, cx: &mut Context<Self>) {
        if self.section == Section::Dashboards {
            return;
        }
        self.section = Section::Dashboards;
        self.sync_visible(cx);
        cx.emit(MonitoringEvent::Section);
        cx.notify();
    }

    /// What example data's alert pages show (`FRESHKUBE_ALERTS`).
    pub fn set_alerts_shape(
        &mut self,
        shape: freshkube_core::monitoring::alerts::example::Shape,
        cx: &mut Context<Self>,
    ) {
        for page in &self.alerts {
            page.update(cx, |page, cx| page.set_shape(shape, cx));
        }
    }

    /// The status bar's line while an alert page shows.
    pub fn alerts_status(&self, cx: &App) -> Option<freshkube_ui::status::Segment> {
        let Section::Alerts(which) = self.section else {
            return None;
        };
        Some(self.alerts_page(which).read(cx).status.clone())
    }

    /// The motion over an alert page's loading rows.
    pub fn alerts_loading_motion(
        &self,
        cx: &App,
    ) -> Option<Entity<freshkube_ui::table::LoadingMotion>> {
        let Section::Alerts(which) = self.section else {
            return None;
        };
        self.alerts_page(which).read(cx).loading_motion()
    }

    pub fn section(&self) -> Section {
        self.section
    }

    /// The page for `which`.
    pub fn alerts_page(&self, which: Which) -> &Entity<AlertsPage> {
        &self.alerts[which as usize]
    }

    /// The keyboard handle of what shows: the alert page, or the dashboard.
    pub fn shown_focus(&self, cx: &App) -> FocusHandle {
        match self.section {
            Section::Alerts(which) => self.alerts_page(which).read(cx).focus_handle().clone(),
            Section::Dashboards => self.focus.clone(),
        }
    }

    fn set_dashboards_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if visible {
            if self.board.is_none() {
                self.load_board(cx);
            }
            self.connect(cx);
            self.start_refresh(cx);
        } else {
            self.histories.retain(|history| history.upgrade().is_some());
            self.connection
                .hide(!self.histories.is_empty(), &mut self.connection_reads);
            if let Some(board) = &mut self.board {
                board.hide();
            }
            self.markers.hide();
            self.refresh_task = None;
        }
        cx.notify();
    }

    /// A new source from the shell. The same connection keeps everything;
    /// another one forgets the Prometheus found and asks again.
    pub fn set_source(&mut self, source: Option<ClusterSource>, cx: &mut Context<Self>) {
        let id = |source: &Option<ClusterSource>| source.as_ref().map(|source| source.id.clone());
        let same = id(&self.source) == id(&source);
        for page in &self.alerts {
            let source = source.clone();
            page.update(cx, |page, cx| page.set_source(source, cx));
        }
        self.source = source;
        if same {
            return;
        }
        self.forget_answers(cx);
    }

    pub fn inputs_mut(&mut self) -> &mut ScopeInputs {
        &mut self.inputs
    }

    /// How the page finds its metrics now.
    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    /// The cluster the page is opened on, which hosts a store that is in the
    /// open entry's own cluster.
    pub fn open_access(&self) -> Option<ClusterAccess> {
        self.source.as_ref().map(|source| source.access.clone())
    }

    /// How the setup file says this entry's metrics are found. The same
    /// resolution keeps everything; another one forgets what was read and
    /// asks again, so an answer for the previous scope never draws.
    pub fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        let same = self.scope.resolved == scope.resolved;
        let alerts = freshkube_core::monitoring::alerts::Scope::of(&scope.resolved);
        for page in &self.alerts {
            let (alerts, host) = (alerts.clone(), scope.host.clone());
            page.update(cx, |page, cx| page.set_scope(alerts, host, cx));
        }
        self.scope = scope;
        if same {
            return;
        }
        cx.emit(MonitoringEvent::Scope);
        // History keys its source by this count, so it reads again too.
        self.clients += 1;
        self.forget_answers(cx);
    }

    /// Drops the connection and every answer read through it, and reads
    /// again when shown.
    fn forget_answers(&mut self, cx: &mut Context<Self>) {
        self.reset_form();
        self.connection_reads.forget();
        self.connection = Connection::None;
        self.histories.clear();
        cx.emit(MonitoringEvent::History);
        // Another cluster's answers never show as this one's.
        self.board = None;
        self.markers.forget();
        if self.visible {
            self.load_board(cx);
        }
        cx.notify();
    }

    /// The shell's Refresh: look again for Prometheus when it wasn't found,
    /// else read the variables and every panel again.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Section::Alerts(which) = self.section {
            self.alerts_page(which)
                .update(cx, |page, cx| page.refresh(cx));
            return;
        }
        if !self.visible {
            return;
        }
        if self.connection.usable() {
            self.allow_rebuild();
            self.resolve_variables(cx);
        } else {
            self.connection_reads.forget();
            self.connection = Connection::None;
            cx.emit(MonitoringEvent::History);
            self.connect(cx);
        }
        cx.notify();
    }

    /// Example data whose source holds it, so every panel stays on its
    /// loading state, for captures (`FRESHKUBE_FIXTURE_HOLD=monitoring`). A
    /// real source still answers.
    fn held(&self) -> bool {
        matches!(
            self.source.as_ref().map(|source| &source.access),
            Some(ClusterAccess::Example(hold)) if hold.monitoring
        )
    }

    fn example(&self) -> bool {
        self.source
            .as_ref()
            .is_some_and(|source| source.access.is_example())
    }

    fn context(&self) -> Option<String> {
        self.source.as_ref().map(|source| source.context.clone())
    }

    /// The source chosen in Settings for this context; none is automatic.
    fn chosen_source(&self) -> Option<&super::store::Choice> {
        self.saved.choices.get(&self.source.as_ref()?.context)
    }

    fn save(&self, change: impl FnOnce(&mut Saved), cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        store.update(change);
        let saving = cx.background_spawn(async move { store.save_latest() });
        cx.spawn(async move |this, cx| {
            let error = saving.await.err().map(|error| error.to_string().into());
            _ = this.update(cx, |this, cx| {
                this.save_error = error;
                cx.notify();
            });
        })
        .detach();
    }

    fn read_identity(&self, kind: ReadKind) -> ReadIdentity {
        ReadIdentity {
            source: self.source.as_ref().map(|source| source.id.clone()),
            client: self.clients,
            kind,
        }
    }

    /// Starts the actual owner's loader and dispatches its typed answer.
    /// Tokens kept by controls describe pending work, never cancel it.
    fn run<T: Send + 'static>(
        &mut self,
        identity: ReadIdentity,
        slot: impl Fn(&mut Self) -> Option<&mut Read> + Clone + 'static,
        work: impl Future<Output = Result<T, QueryError>> + Send + 'static,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, Result<T, QueryError>, &mut Context<Self>) + 'static,
    ) -> Request {
        let request = slot(self)
            .expect("the read has an owner")
            .start(Some(identity));
        let captured = request.clone();
        let delivery = slot.clone();
        let (job, task) = crate::dispatch(
            self.example(),
            &self.runtime,
            DEADLINE,
            work,
            cx,
            move |this, result, cx| this.land(&captured, delivery, result, cx, done),
        );
        slot(self).expect("the read keeps its owner").run(job, task);
        request
    }

    /// Keep the typed result intact for the accepted owner's callback.
    fn land<T>(
        &mut self,
        request: &Request,
        slot: impl FnOnce(&mut Self) -> Option<&mut Read>,
        result: Result<T, QueryError>,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, Result<T, QueryError>, &mut Context<Self>),
    ) {
        let completion = result
            .as_ref()
            .map(|_| ())
            .map_err(|error| error.message.clone());
        if slot(self).is_some_and(|read| read.land(request, completion, cx)) {
            done(self, result, cx);
        }
    }

    /// Asks again on the chosen interval while the page shows.
    fn start_refresh(&mut self, cx: &mut Context<Self>) {
        self.refresh_task = None;
        let (true, Some(every)) = (self.visible, self.refresh_every) else {
            return;
        };
        self.refresh_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(every))
                    .await;
                if this.update(cx, |this, cx| this.ask_again(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    pub(super) fn set_refresh(&mut self, every: Option<u64>, cx: &mut Context<Self>) {
        self.refresh_every = every;
        self.start_refresh(cx);
        cx.notify();
    }
}
