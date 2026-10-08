//! Inspection screens beyond Overview, Services and Logs.
//!
//! Each screen is its own entity, like the log panel. The shell hands every
//! screen a [`ScreenSource`] whenever the target node or its cluster snapshot
//! changes, activates a screen when it becomes visible, and forwards refreshes
//! to the visible screen only. A screen owns its requests, its data and its
//! example data; hidden screens never contact the cluster.
mod diagnostics;
mod etcd;
mod lifecycle;
mod network;
mod operations;
mod processes;
mod security;
mod storage;
mod workloads;

pub(crate) use diagnostics::DiagnosticsScreen;
pub(crate) use etcd::EtcdScreen;
pub(crate) use lifecycle::LifecycleScreen;
pub(crate) use network::NetworkScreen;
pub(crate) use operations::OperationsScreen;
pub(crate) use processes::ProcessesScreen;
pub(crate) use security::SecurityScreen;
pub(crate) use storage::StorageScreen;
pub(crate) use workloads::{Summary as HealthSummary, WorkloadData, WorkloadsScreen};

use std::{future::Future, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

use freshkube_core::{
    cluster_overview::{ClusterOverview, ClusterOverviewCollector},
    inspection::InspectionTarget,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use talos_rs::TalosClient;
use tokio::runtime::Handle;

use crate::backend::{self, OwnedJob, Target};
use crate::palette::palette;
use crate::presentation::NodeSummary;
use crate::state::Snapshot;
use crate::ui::{self, MONO_FONT, clock, dp};
// The page widths live in `freshkube_ui::page`; screens reach them here.
pub(crate) use freshkube_ui::page::{content_width, inset_width, page_width, set_chrome_width};

/// Upper bound for one screen request, including Kubernetes client setup.
pub(crate) const SCREEN_DEADLINE: Duration = Duration::from_secs(60);
const KUBERNETES_SETUP: Duration = Duration::from_secs(45);

/// Whether a screen inspects the target node or its whole cluster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    Node,
    Cluster,
}

/// Everything a screen needs to load data for the current target.
#[derive(Clone)]
pub(crate) struct ScreenSource {
    /// Request identity. A different value discards the previous target's data.
    pub(crate) target: Target,
    /// The cluster roster as of the latest overview, target included.
    pub(crate) nodes: Arc<Vec<NodeSummary>>,
    /// Talos access, or `None` for offline example data.
    pub(crate) live: Option<LiveSource>,
}

impl ScreenSource {
    pub(crate) fn node(&self) -> Option<&NodeSummary> {
        self.nodes.iter().find(|node| node.name == self.target.node)
    }

    /// Whether the overview last heard from the target node.
    pub(crate) fn responding(&self) -> bool {
        self.node().is_some_and(|node| node.responding)
    }

    pub(crate) fn is_example(&self) -> bool {
        self.live.is_none()
    }

    pub(crate) fn inspection_target(&self) -> InspectionTarget {
        InspectionTarget::new(self.target.node.clone(), self.target.address.clone())
    }
}

/// Talos access for a connected context. Clone it into a screen's Tokio work.
#[derive(Clone)]
pub(crate) struct LiveSource {
    /// Context client; pin it with `with_node` (core collectors do this themselves).
    pub(crate) client: TalosClient,
    pub(crate) cluster: Arc<ClusterOverview>,
    pub(crate) collector: ClusterOverviewCollector,
    /// The talosconfig in use; `None` means TALOSCONFIG or ~/.talos/config.
    pub(crate) config_path: Option<PathBuf>,
}

impl LiveSource {
    /// An identity-validated Kubernetes client for this cluster, using the
    /// kubeconfig source chosen in Settings. Never an ambient kubeconfig.
    pub(crate) async fn kubernetes(&self) -> Result<kube::Client, String> {
        self.kubernetes_with_source()
            .await
            .map(|(client, _, _)| client)
    }

    /// Drops the reused Kubernetes client and prepared kubeconfig, so the next
    /// call revalidates and rebuilds. Call it when a Kubernetes request made
    /// with a client from [`Self::kubernetes`] fails.
    pub(crate) fn forget_kubernetes(&self) {
        self.collector.forget_kubernetes_client(&self.cluster);
    }

    /// As [`Self::kubernetes`], with the kubeconfig actually used and, when a
    /// selected file was rejected, why.
    ///
    /// The identity-validated client is reused for the same context, pinned node
    /// and kubeconfig selection for a couple of minutes.
    pub(crate) async fn kubernetes_with_source(
        &self,
    ) -> Result<(kube::Client, String, Option<String>), String> {
        tokio::time::timeout(
            KUBERNETES_SETUP,
            self.collector.kubernetes_client_with_source(&self.cluster),
        )
        .await
        .map_err(|_| "Setting up the Kubernetes client timed out".to_owned())?
        .map_err(|error| error.to_string())
    }
}

/// Requests a screen makes of the shell.
#[derive(Clone, Debug)]
pub(crate) enum ScreenEvent {
    /// Refresh the shell's shared Kubernetes facts.
    RefreshSummary,
    Back,
    /// Collect this service's logs on the target node and show the Logs page.
    OpenLogs(String),
    /// Make this node the target.
    SelectNode(String),
    /// Ask the shell to read the Talos overview again, after it failed.
    RetryCluster,
    /// Make `node` the target, then show `service`'s logs there. Ignored when
    /// the node isn't in the roster.
    OpenLogsOn {
        node: String,
        service: String,
    },
}

/// Common lifecycle behavior between the shell and a screen. Feature-specific
/// data is delivered through the screen's typed entity handle.
pub(crate) trait ScreenPanel: Render + EventEmitter<ScreenEvent> + Sized {
    fn set_embedded(&mut self, _embedded: bool, _cx: &mut Context<Self>) {}

    fn new(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self;

    /// Called whenever the target or its cluster snapshot changes. A different
    /// `source.target` must drop in-flight work and data for the old target;
    /// the same target only refreshes the stored handles.
    fn set_source(
        &mut self,
        source: Option<ScreenSource>,
        window: &mut Window,
        cx: &mut Context<Self>,
    );

    /// The screen became visible, or its target changed while visible. Load
    /// when there is nothing for the current target yet.
    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>);

    /// A manual or automatic refresh while the screen is visible.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>);

    /// An explicit request from the screen's Refresh or Retry button.
    fn manual_refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh(window, cx);
    }

    /// The user navigated here: focus the main list so keys work at once.
    /// Not called on source changes, so it never steals focus from a popover.
    fn focus(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    /// The page's line in the status bar, which the shell draws while the
    /// screen is the visible page. The shell reads it before the screen
    /// renders, so derive it here when its inputs changed, as Resources
    /// does, and notify when they change; `None` draws no segment.
    fn status(&mut self) -> Option<&freshkube_ui::status::Segment> {
        None
    }

    /// The motion over the table's loading rows while its first answer is
    /// to come ([`TableLoading::motion`]); the shell draws it beside the
    /// page, so its frames redraw neither the page nor the cached chrome.
    fn loading_motion(&self, _cx: &App) -> Option<Entity<freshkube_ui::table::LoadingMotion>> {
        None
    }
}

type SourceFn = Rc<dyn Fn(Option<ScreenSource>, &mut Window, &mut App)>;
type WindowFn = Rc<dyn Fn(&mut Window, &mut App)>;
type EmbeddedFn = Rc<dyn Fn(bool, &mut App)>;
type StatusFn = Rc<dyn Fn(&mut App) -> Option<freshkube_ui::status::Segment>>;
type MotionFn = Rc<dyn Fn(&App) -> Option<Entity<freshkube_ui::table::LoadingMotion>>>;

/// A type-erased screen, so the shell can keep every screen in one list.
#[derive(Clone)]
pub(crate) struct ScreenHandle {
    view: AnyView,
    set_source: SourceFn,
    activate: WindowFn,
    refresh: WindowFn,
    focus: WindowFn,
    embedded: EmbeddedFn,
    status: StatusFn,
    motion: MotionFn,
}

impl ScreenHandle {
    pub(crate) fn new<T: ScreenPanel>(entity: Entity<T>) -> Self {
        let (sourced, activated, refreshed, focused) = (
            entity.clone(),
            entity.clone(),
            entity.clone(),
            entity.clone(),
        );
        let (embedded, status, motion) = (entity.clone(), entity.clone(), entity.clone());
        Self {
            motion: Rc::new(move |cx| motion.read(cx).loading_motion(cx)),
            status: Rc::new(move |cx| status.update(cx, |screen, _| screen.status().cloned())),
            embedded: Rc::new(move |value, cx| {
                embedded.update(cx, |screen, cx| screen.set_embedded(value, cx))
            }),
            view: entity.into(),
            set_source: Rc::new(
                move |source: Option<ScreenSource>, window: &mut Window, cx: &mut App| {
                    sourced.update(cx, |screen, cx| screen.set_source(source, window, cx))
                },
            ),
            activate: Rc::new(move |window: &mut Window, cx: &mut App| {
                activated.update(cx, |screen, cx| screen.activate(window, cx))
            }),
            refresh: Rc::new(move |window: &mut Window, cx: &mut App| {
                refreshed.update(cx, |screen, cx| screen.refresh(window, cx))
            }),
            focus: Rc::new(move |window: &mut Window, cx: &mut App| {
                focused.update(cx, |screen, cx| screen.focus(window, cx))
            }),
        }
    }

    /// The motion over the screen's loading rows, while they show.
    pub(crate) fn loading_motion(
        &self,
        cx: &App,
    ) -> Option<Entity<freshkube_ui::table::LoadingMotion>> {
        (self.motion)(cx)
    }

    /// The screen's status bar segment, if it has one.
    pub(crate) fn status(&self, cx: &mut App) -> Option<freshkube_ui::status::Segment> {
        (self.status)(cx)
    }

    pub(crate) fn set_embedded(&self, embedded: bool, cx: &mut App) {
        (self.embedded)(embedded, cx)
    }

    pub(crate) fn view(&self) -> AnyView {
        self.view.clone()
    }

    pub(crate) fn set_source(
        &self,
        source: Option<ScreenSource>,
        window: &mut Window,
        cx: &mut App,
    ) {
        (self.set_source)(source, window, cx)
    }

    pub(crate) fn activate(&self, window: &mut Window, cx: &mut App) {
        (self.activate)(window, cx)
    }

    pub(crate) fn refresh(&self, window: &mut Window, cx: &mut App) {
        (self.refresh)(window, cx)
    }

    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        (self.focus)(window, cx)
    }
}

/// One screen's data for one target: a [`Snapshot`] plus the work that fills it.
pub(crate) struct Loader<T> {
    state: Snapshot<T, Target>,
    job: Option<OwnedJob>,
    task: Option<Task<()>>,
    /// Rises whenever what [`data`](Self::data) or [`error`](Self::error)
    /// return may have changed, so a screen derives its display data again
    /// only then.
    revision: u64,
}

impl<T> Default for Loader<T> {
    fn default() -> Self {
        Self {
            state: Snapshot::default(),
            job: None,
            task: None,
            revision: 0,
        }
    }
}

impl<T: Send + 'static> Loader<T> {
    /// Forgets data and cancels in-flight work, e.g. when the target changes.
    pub(crate) fn reset(&mut self) {
        let revision = self.revision + 1;
        *self = Self::default();
        self.revision = revision;
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Runs `work` on Tokio for `identity`. The result lands only if this is
    /// still the latest request for that identity; `slot` finds the loader
    /// again inside the screen when it does. `what` names the data in
    /// lowercase with its article, e.g. "the process list".
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn load<V: 'static>(
        &mut self,
        identity: Target,
        runtime: &Handle,
        what: &str,
        work: impl Future<Output = Result<T, String>> + Send + 'static,
        slot: fn(&mut V) -> &mut Loader<T>,
        cx: &mut Context<V>,
    ) {
        let request = self.state.begin(identity);
        self.revision += 1;
        let (job, receiver) = backend::spawn_job(
            runtime,
            SCREEN_DEADLINE,
            format!("Loading {what} timed out"),
            work,
        );
        self.job = Some(job);
        let stopped = format!("Loading {what} stopped unexpectedly");
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = receiver.await.unwrap_or_else(|_| Err(stopped));
            _ = this.update(cx, |view, cx| {
                let loader = slot(view);
                if loader.state.apply(&request, result) {
                    loader.job = None;
                    loader.revision += 1;
                }
                cx.notify();
            });
        }));
    }

    /// Applies a result at once; used for example data.
    pub(crate) fn resolve(&mut self, identity: Target, result: Result<T, String>) {
        self.job = None;
        self.task = None;
        let request = self.state.begin(identity);
        self.state.apply(&request, result);
        self.revision += 1;
    }

    pub(crate) fn data(&self) -> Option<&T> {
        self.state.data()
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.state.error()
    }

    pub(crate) fn is_loading(&self) -> bool {
        self.state.is_loading()
    }

    pub(crate) fn last_successful(&self) -> Option<std::time::SystemTime> {
        self.state.last_successful()
    }

    pub(crate) fn last_failure(&self) -> Option<std::time::SystemTime> {
        self.state.last_failure()
    }
}

const SPLIT_WIDTH: f32 = 900.;
const PANE_WIDTH: f32 = 460.;
/// A detail with a few short fields, such as a disk's or a volume's.
pub(crate) const NARROW_PANE_WIDTH: f32 = 340.;
const PANE_MIN_WIDTH: f32 = 320.;
pub(crate) const SPLIT_GAP: f32 = 14.;

/// Whether a list's detail fits beside it: on the page, or in the node
/// inspector when the screen is `embedded` there.
pub(crate) fn beside(window: &Window, embedded: bool) -> bool {
    let width = if embedded {
        embedded_width(window)
    } else {
        content_width(window)
    };
    width >= SPLIT_WIDTH
}

/// The width inside the node inspector's tab body in `dp`, where embedded
/// screens choose between side-by-side and stacked layouts.
pub(crate) fn embedded_width(window: &Window) -> f32 {
    let pane = NODE_PANE_WIDTH.get().min(page_width(window));
    (pane - freshkube_ui::page::PANE_PADDING * 2.).max(240.)
}

/// A list with its detail beside it on a wide page and below it on a narrow
/// one, at DESIGN.md's detail pane sizes. The pane's size is fixed until
/// change 9's Inspector.
pub(crate) fn split(
    id: &'static str,
    beside: bool,
    table: AnyElement,
    pane: Option<AnyElement>,
) -> AnyElement {
    split_at(id, beside, PANE_WIDTH, table, pane)
}

/// [`split`] with a narrower pane, for a detail of a few short fields, so
/// the list keeps its columns beside it.
pub(crate) fn split_narrow(
    id: &'static str,
    beside: bool,
    table: AnyElement,
    pane: Option<AnyElement>,
) -> AnyElement {
    split_at(id, beside, NARROW_PANE_WIDTH, table, pane)
}

/// [`split_narrow`] for a list that fills the page's height and scrolls its
/// own rows, such as Processes': the pane runs beside it at its height, or
/// `below` dp high under it. The split keeps the list at least
/// [`page::SHORT_LIST_HEIGHT`] high, so a short page scrolls its frame.
pub(crate) fn split_fill(
    id: &'static str,
    beside: bool,
    below: f32,
    table: AnyElement,
    pane: AnyElement,
) -> AnyElement {
    let list = freshkube_ui::page::SHORT_LIST_HEIGHT;
    div()
        .id(id)
        .test_support()
        .flex()
        .flex_1()
        .gap(dp(SPLIT_GAP))
        .when_else(
            beside,
            |this| this.flex_row().items_stretch().min_h(dp(list)),
            |this| this.flex_col().min_h(dp(list + SPLIT_GAP + below)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .child(table),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .when_else(
                    beside,
                    |this| this.w(dp(NARROW_PANE_WIDTH)).min_w(dp(PANE_MIN_WIDTH)),
                    |this| this.w_full().h(dp(below)),
                )
                .child(pane),
        )
        .into_any_element()
}

pub(crate) fn split_at(
    id: &'static str,
    beside: bool,
    pane_width: f32,
    table: AnyElement,
    pane: Option<AnyElement>,
) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .flex()
        .items_start()
        .gap(dp(SPLIT_GAP))
        .when_else(beside, |this| this.flex_row(), |this| this.flex_col())
        .child(
            div()
                .min_w_0()
                .when_else(beside, |this| this.flex_1(), |this| this.w_full())
                .child(table),
        )
        .children(pane.map(|pane| {
            div()
                .when_else(
                    beside,
                    |this| this.w(dp(pane_width)).min_w(dp(PANE_MIN_WIDTH)).flex_none(),
                    |this| this.w_full(),
                )
                .child(pane)
        }))
        .into_any_element()
}

thread_local! {
    /// The node inspector's width in dp, which Nodes sets as it draws the
    /// inspector: the page's when it stacks or fills the page.
    static NODE_PANE_WIDTH: std::cell::Cell<f32> = const { std::cell::Cell::new(f32::MAX) };
}

pub(crate) fn set_node_pane_width(width: f32) {
    NODE_PANE_WIDTH.set(width);
}

pub(crate) fn mono(text: impl Into<SharedString>) -> Div {
    div()
        .font_family(MONO_FONT)
        .text_size(dp(12.))
        .child(text.into())
}

/// A screen's meta line under its `PageHeader`: where it reads, unless the
/// node pane around it already names the node, then the screen's own
/// `parts`, when it last updated and whether the data is an example, with a
/// " · " between them.
pub(crate) fn meta<T: Send + 'static>(
    source: Option<&ScreenSource>,
    scope: Scope,
    loader: &Loader<T>,
    embedded: bool,
    parts: impl IntoIterator<Item = SharedString>,
) -> Vec<AnyElement> {
    let place = source.filter(|_| !embedded).map(|source| {
        let target = &source.target;
        SharedString::from(match scope {
            Scope::Node => format!("on {} · {}", target.node, target.address),
            Scope::Cluster => target.context.clone(),
        })
    });
    let updated = loader
        .last_successful()
        .map(|time| SharedString::from(format!("updated {}", clock(time))));
    let example = source
        .filter(|source| source.is_example())
        .map(|_| SharedString::from("example data"));
    let mut meta = Vec::new();
    for part in place.into_iter().chain(parts).chain(updated).chain(example) {
        if !meta.is_empty() {
            meta.push(" · ".into_any_element());
        }
        meta.push(part.into_any_element());
    }
    meta
}

/// A cluster page's line in the status bar: its context, the screen's own
/// `parts`, when it last updated and whether the data is an example. Derive
/// it when the loader's answer changes, not in render.
pub(crate) fn segment<T: Send + 'static>(
    source: Option<&ScreenSource>,
    loader: &Loader<T>,
    parts: impl IntoIterator<Item = freshkube_ui::status::Part>,
) -> freshkube_ui::status::Segment {
    use freshkube_ui::status::{Part, Segment};
    let updated = loader
        .last_successful()
        .map(|time| Part::new(format!("updated {}", clock(time))).minor());
    let example = source
        .filter(|source| source.is_example())
        .map(|_| Part::new("example data").minor());
    Segment::new(
        source.map(|source| source.target.context.clone()),
        parts.into_iter().chain(updated).chain(example),
    )
}

/// A screen's Refresh in its `PageHeader`: a ghost icon with a tooltip,
/// busy while a read runs.
pub(crate) fn refresh_control<V: ScreenPanel, T: Send + 'static>(
    id: SharedString,
    label: &'static str,
    source: Option<&ScreenSource>,
    loader: &Loader<T>,
    cx: &mut Context<V>,
) -> Button {
    let loading = loader.is_loading();
    Button::new(id)
        .ghost()
        .small()
        .size(dp(ui::CONTROL_HEIGHT))
        .icon(ui::refresh_icon(loading, cx))
        .accessibility_label(label)
        .tooltip(if loading { "Refreshing…" } else { label })
        .disabled(loading || source.is_none())
        .on_click(cx.listener(|view, _, window, cx| view.manual_refresh(window, cx)))
}

pub(crate) fn retry_button<V: ScreenPanel>(id: &'static str, cx: &mut Context<V>) -> AnyElement {
    Button::new(id)
        .primary()
        .icon(IconName::RefreshCw)
        .label("Retry")
        .on_click(cx.listener(|view, _, window, cx| view.manual_refresh(window, cx)))
        .into_any_element()
}

/// "the process list" → "The process list", for the start of a sentence.
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Where the Talos overview, which every screen's source comes from, stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reading {
    /// Neither answered nor failed: pages show their loading state.
    Waiting,
    /// Nothing answered, for this reason: nothing is shown as missing.
    Failed(SharedString),
    /// Answered. Without a source the cluster has no node to target.
    Answered,
}

/// The shell's latest [`Reading`], derived when the overview changes.
struct ClusterReading(Reading);

impl Global for ClusterReading {}

/// The overview's state; `Waiting` until a shell has published one.
pub(crate) fn reading(cx: &App) -> Reading {
    cx.try_global::<ClusterReading>()
        .map_or(Reading::Waiting, |global| global.0.clone())
}

/// Publishes the overview's state. Returns whether it changed, so the
/// caller redraws the screens that show it.
pub(crate) fn set_reading(reading: Reading, cx: &mut App) -> bool {
    if cx.try_global::<ClusterReading>().map(|global| &global.0) == Some(&reading) {
        return false;
    }
    cx.set_global(ClusterReading(reading));
    true
}

/// What a page shows while it has no source: its failure with a retry, or,
/// once the overview answered with no node to target, that there is none.
/// While the overview is read, its table shows its loading rows instead
/// ([`first_read`]), so this draws nothing.
pub(crate) fn unsourced<V: ScreenPanel>(what: &str, cx: &mut Context<V>) -> AnyElement {
    let (id, state) = match reading(cx) {
        Reading::Waiting => return div().into_any_element(),
        Reading::Failed(reason) => (
            "screen-unreachable",
            ui::empty_state(
                IconName::CircleDashed,
                format!("Couldn't load {what}"),
                "The cluster hasn't answered, so nothing is shown as failed. Retry, or check the Talos API connection.",
                Some(reason.to_string()),
                vec![
                    Button::new("screen-retry")
                        .primary()
                        .icon(IconName::RefreshCw)
                        .label("Retry")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(ScreenEvent::RetryCluster)))
                        .into_any_element(),
                ],
                cx,
            )
            .into_any_element(),
        ),
        Reading::Answered => (
            "screen-no-node",
            ui::empty_state(
                IconName::Server,
                "No node selected",
                "Open a node in Nodes to inspect it.",
                None,
                Vec::new(),
                cx,
            )
            .into_any_element(),
        ),
    };
    div()
        .id(id)
        .test_support()
        .size_full()
        .child(state)
        .into_any_element()
}

/// Whether a screen's first answer is still to come: the overview is read
/// before there is a source, or the screen's first read is in flight or not
/// asked yet. Its table shows loading rows meanwhile ([`TableLoading`]); a
/// silent node or a failed first read shows its state instead.
pub(crate) fn first_read<T: Send + 'static>(
    source: Option<&ScreenSource>,
    loader: &Loader<T>,
    scope: Scope,
    cx: &App,
) -> bool {
    if loader.data().is_some() {
        return false;
    }
    let Some(source) = source else {
        return reading(cx) == Reading::Waiting;
    };
    if loader.is_loading() {
        return true;
    }
    if scope == Scope::Node && !source.responding() {
        return false;
    }
    loader.error().is_none()
}

/// What a table screen shows until its first answer: the shared loading
/// rows under its real header, which its `TableSource::loading` returns
/// while [`showing`](Self::showing), and their motion, which the shell
/// draws beside the page ([`ScreenPanel::loading_motion`]). The screen sets
/// it from [`first_read`] as it renders; it holds no state of its own.
pub(crate) struct TableLoading {
    rows: freshkube_ui::table::LoadingRows,
    motion: Entity<freshkube_ui::table::LoadingMotion>,
    showing: std::cell::Cell<bool>,
}

impl TableLoading {
    pub(crate) fn new(prefix: &str, cx: &mut App) -> Self {
        let rows = freshkube_ui::table::LoadingRows::new(prefix);
        let motion = cx.new(|_| rows.motion(freshkube_ui::table::Look::Pulse));
        Self {
            rows,
            motion,
            showing: std::cell::Cell::new(false),
        }
    }

    /// Says whether the table shows the rows this frame: [`first_read`].
    pub(crate) fn show(&self, showing: bool) {
        self.showing.set(showing);
    }

    /// For `TableSource::loading`.
    pub(crate) fn rows(&self) -> Option<&freshkube_ui::table::LoadingRows> {
        self.showing.get().then_some(&self.rows)
    }

    /// For [`ScreenPanel::loading_motion`], from the screen's [`first_read`].
    pub(crate) fn motion(
        &self,
        showing: bool,
    ) -> Option<Entity<freshkube_ui::table::LoadingMotion>> {
        showing.then(|| self.motion.clone())
    }
}

/// What to show instead of data: no target yet, a silent node, or a first
/// load that failed. `None` means render the table: its data, or its
/// loading rows while the first answer is to come ([`first_read`]).
pub(crate) fn gate<V: ScreenPanel, T: Send + 'static>(
    source: Option<&ScreenSource>,
    loader: &Loader<T>,
    scope: Scope,
    what: &str,
    cx: &mut Context<V>,
) -> Option<AnyElement> {
    if loader.data().is_some() || first_read(source, loader, scope, cx) {
        return None;
    }
    let Some(source) = source else {
        return Some(unsourced(what, cx));
    };
    if scope == Scope::Node && !source.responding() {
        let target = &source.target;
        return Some(
            ui::empty_state(
                IconName::Unplug,
                format!("{} isn't responding", target.node),
                format!(
                    "{} needs the Talos API at {}:50000. Open another node in Nodes, or retry.",
                    capitalized(what),
                    target.address
                ),
                loader.error().map(str::to_owned),
                vec![retry_button("screen-retry", cx)],
                cx,
            )
            .into_any_element(),
        );
    }
    if let Some(error) = loader.error() {
        return Some(
            ui::empty_state(
                IconName::CircleDashed,
                format!("Couldn't load {what}"),
                "Nothing is known yet, so nothing is shown as failed. Retry, or check the Talos API connection.",
                Some(error.to_owned()),
                vec![retry_button("screen-retry", cx)],
                cx,
            )
            .into_any_element(),
        );
    }
    // Not requested yet: `first_read` answered above.
    None
}

/// Data is still shown, but the latest refresh failed.
pub(crate) fn failure_banner<V: ScreenPanel, T: Send + 'static>(
    loader: &Loader<T>,
    cx: &mut Context<V>,
) -> Option<Div> {
    let error = loader.error()?.to_owned();
    loader.data()?;
    let failed = loader
        .last_failure()
        .map(clock)
        .unwrap_or_else(|| "the last attempt".into());
    let kept = loader
        .last_successful()
        .map(clock)
        .unwrap_or_else(|| "earlier".into());
    Some(ui::warning_banner(
        Some(format!("Refresh failed at {failed}.").into()),
        format!("Showing data from {kept}. {error}"),
        Some(
            Button::new("screen-retry")
                .small()
                .icon(IconName::RefreshCw)
                .label("Retry")
                .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx)))
                .into_any_element(),
        ),
        cx,
    ))
}

/// Optional sources that didn't answer. Their values show as unknown, never as failed.
pub(crate) fn partial_notice(missing: Vec<String>, cx: &App) -> Option<AnyElement> {
    if missing.is_empty() {
        return None;
    }
    let p = palette(cx);
    Some(
        h_flex()
            .id("partial-notice")
            .test_support()
            .role(Role::Status)
            .aria_label(format!("Unavailable: {}", missing.join("; ")))
            .items_start()
            .gap_2p5()
            .px_3()
            .py_2()
            .rounded(px(8.))
            .border_1()
            .border_color(p.line)
            .bg(p.surface)
            .text_size(dp(12.5))
            .child(
                Icon::new(IconName::CircleDashed)
                    .size(dp(15.))
                    .text_color(p.unk_ink)
                    .mt(dp(1.)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Some sources didn't answer; their values show as unknown."),
                    )
                    .children(
                        missing
                            .into_iter()
                            .map(|reason| div().text_color(p.muted).child(reason)),
                    ),
            )
            .into_any_element(),
    )
}

/// A bordered surface for lists, tables and detail panes.
pub(crate) fn panel(cx: &App) -> Div {
    freshkube_ui::page::card(cx)
}

/// A labelled value, as in the Services detail pane.
pub(crate) fn field(label: &'static str, value: impl IntoElement, cx: &App) -> Div {
    let p = palette(cx);
    h_flex()
        .items_start()
        .gap_4()
        .child(
            div()
                .w(dp(132.))
                .flex_none()
                .pt(dp(1.))
                .text_size(dp(12.))
                .text_color(p.muted)
                .child(label),
        )
        .child(div().flex_1().min_w_0().text_size(dp(13.)).child(value))
}

/// A small figure with a caption, for summary rows above a list.
pub(crate) fn stat(label: &str, value: impl Into<SharedString>, cx: &App) -> Div {
    let p = palette(cx);
    v_flex()
        .gap_1()
        .px_3p5()
        .py_2p5()
        .rounded(px(8.))
        .border_1()
        .border_color(p.line)
        .bg(p.surface)
        .min_w(dp(120.))
        .child(ui::caption(label, cx))
        .child(
            div()
                .font_weight(ui::TITLE_WEIGHT)
                .text_size(dp(18.))
                .child(value.into()),
        )
}

#[cfg(test)]
mod tests {
    // Not `super::*`: it brings gpui_kit's `test` macro over the built-in one.
    use super::capitalized;

    #[test]
    fn capitalized_starts_a_sentence() {
        assert_eq!(capitalized("the process list"), "The process list");
        assert_eq!(capitalized("etcd status"), "Etcd status");
        assert_eq!(capitalized(""), "");
    }
}
