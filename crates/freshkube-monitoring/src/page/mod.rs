//! The Monitoring page: one dashboard at a time, from the built-ins or the
//! user's folder, answered by the cluster's own Prometheus API (found or
//! chosen in Settings) through the service proxy, by a URL the user
//! entered, or by example data with `--fixture`.
//!
//! The shell owns the page and tells it when it shows. Only a visible page
//! reads: it finds Prometheus, resolves the variables, and asks the panels
//! in or near the viewport. Every change that asks again (a source, a
//! dashboard, a variable, a range, a refresh) takes a new generation, and
//! an answer for an older one is dropped. Hiding drops every request.
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

use freshkube_core::cluster_source::ClusterSource;
use freshkube_core::monitoring::QueryError;
use gpui_kit::{AppContext, Context, EventEmitter, FocusHandle, ScrollHandle, Task};
use tokio::runtime::Handle;

use super::request::{self, Request};
use super::store::{MonitoringStore, Saved};

use board::Board;
pub use catalog::{Catalog, Entry, EntryId, FolderState};
use connection::Connection;
use markers::MarkerState;
pub use markers::TalosNodes;
pub use settings::settings_section;
pub use source::source_section;

/// How long one request through the proxy may take, past the transport's
/// own timeout, before the page gives up on it.
const DEADLINE: Duration = Duration::from_secs(45);

/// What the page tells the shell.
#[derive(Clone, Debug, PartialEq)]
pub enum MonitoringEvent {
    /// The column's dashboards changed, or which one is open.
    Catalog,
    /// The Prometheus that pod and node history read may have changed.
    History,
    /// The breadcrumb's Dashboards: show the column that lists them.
    Dashboards,
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

pub struct MonitoringPage {
    runtime: Handle,
    store: Option<MonitoringStore>,
    saved: Saved,
    visible: bool,
    source: Option<ClusterSource>,
    connection: Connection,
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
    /// Example data never answers: [`hold_examples`](Self::hold_examples).
    hold: bool,
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
        let mut page = Self {
            runtime,
            store,
            saved,
            visible: false,
            source: None,
            connection: Connection::None,
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
            hold: false,
        };
        page.read_folder(cx);
        page
    }

    /// Example data never answers, so every panel stays on its loading
    /// state, for captures (`FRESHKUBE_FIXTURE_HOLD=monitoring`). A real
    /// source still answers.
    pub fn hold_examples(&mut self) {
        self.hold = true;
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

    /// Shows or hides the page. Showing connects and asks what the viewport
    /// needs; hiding drops every request and the refresh timer.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
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
            self.connection.hide();
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
        self.source = source;
        if same {
            return;
        }
        self.reset_form();
        self.connection = Connection::None;
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
        if !self.visible {
            return;
        }
        if self.connection.usable() {
            self.resolve_variables(cx);
        } else {
            self.connection = Connection::None;
            cx.emit(MonitoringEvent::History);
            self.connect(cx);
        }
        cx.notify();
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

    /// Runs `work` where its source answers and hands the result to
    /// `done`, unless the request is dropped first.
    fn run<T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T, QueryError>> + Send + 'static,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, Result<T, QueryError>, &mut Context<Self>) + 'static,
    ) -> Request {
        request::run(self.example(), &self.runtime, DEADLINE, work, cx, done)
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
