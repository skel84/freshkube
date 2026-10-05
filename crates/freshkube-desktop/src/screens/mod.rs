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
pub(crate) use workloads::{WorkloadData, WorkloadsScreen};

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
use crate::desktop::{COLUMN_WIDTH, PAGE_PADDING, RAIL_WIDTH};
use crate::palette::palette;
use crate::presentation::NodeSummary;
use crate::state::Snapshot;
use crate::ui::{self, MONO_FONT, clock, dp, dp_px};

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
}

type SourceFn = Rc<dyn Fn(Option<ScreenSource>, &mut Window, &mut App)>;
type WindowFn = Rc<dyn Fn(&mut Window, &mut App)>;
type EmbeddedFn = Rc<dyn Fn(bool, &mut App)>;

/// A type-erased screen, so the shell can keep every screen in one list.
#[derive(Clone)]
pub(crate) struct ScreenHandle {
    view: AnyView,
    set_source: SourceFn,
    activate: WindowFn,
    refresh: WindowFn,
    focus: WindowFn,
    embedded: EmbeddedFn,
}

impl ScreenHandle {
    pub(crate) fn new<T: ScreenPanel>(entity: Entity<T>) -> Self {
        let (sourced, activated, refreshed, focused) = (
            entity.clone(),
            entity.clone(),
            entity.clone(),
            entity.clone(),
        );
        let embedded = entity.clone();
        Self {
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
}

impl<T> Default for Loader<T> {
    fn default() -> Self {
        Self {
            state: Snapshot::default(),
            job: None,
            task: None,
        }
    }
}

impl<T: Send + 'static> Loader<T> {
    /// Forgets data and cancels in-flight work, e.g. when the target changes.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
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

/// The width of a page in `dp` (pixels at the default text size): the
/// window less the rail and column. A page without margins has all of it,
/// so its breakpoints compare this; a larger text size leaves less room, as
/// a narrower window would.
pub(crate) fn page_width(window: &Window) -> f32 {
    let viewport = window.viewport_size().width / dp_px(1., window);
    (viewport - CHROME_WIDTH.get()).max(240.)
}

/// The width inside a page's [`inset`](freshkube_ui::page::inset), such as
/// its header's, in `dp`.
pub(crate) fn inset_width(window: &Window) -> f32 {
    (page_width(window) - freshkube_ui::page::PANE_PADDING * 2.).max(240.)
}

/// The width inside a [padded](freshkube_ui::page::padded) page's margins in
/// `dp`, for choosing between side-by-side and stacked layouts there.
pub(crate) fn content_width(window: &Window) -> f32 {
    (page_width(window) - PAGE_PADDING * 2.).max(240.)
}

thread_local! {
    /// The width of the navigation beside the page in dp: the rail, and the
    /// column when it shows. Windows draw on one thread, and the shell sets
    /// it whenever the column shows or hides.
    static CHROME_WIDTH: std::cell::Cell<f32> =
        const { std::cell::Cell::new(RAIL_WIDTH + COLUMN_WIDTH) };
}

pub(crate) fn set_chrome_width(width: f32) {
    CHROME_WIDTH.set(width);
}

pub(crate) fn page_scroll(id: &'static str) -> Stateful<Div> {
    div().id(id).size_full().overflow_y_scroll()
}

pub(crate) fn page_body() -> Div {
    v_flex()
        .px(dp(PAGE_PADDING))
        .pt(dp(22.))
        .pb(dp(30.))
        .gap(dp(20.))
}

pub(crate) fn mono(text: impl Into<SharedString>) -> Div {
    div()
        .font_family(MONO_FONT)
        .text_size(dp(12.))
        .child(text.into())
}

/// Title, what it covers, when it was last updated, and the screen's refresh.
pub(crate) fn header<V: ScreenPanel, T: Send + 'static>(
    title: &'static str,
    source: &ScreenSource,
    scope: Scope,
    loader: &Loader<T>,
    cx: &mut Context<V>,
) -> Div {
    header_mode(title, source, scope, loader, false, cx)
}

pub(crate) fn header_mode<V: ScreenPanel, T: Send + 'static>(
    title: &'static str,
    source: &ScreenSource,
    scope: Scope,
    loader: &Loader<T>,
    embedded: bool,
    cx: &mut Context<V>,
) -> Div {
    let p = palette(cx);
    let target = &source.target;
    let scope_line = match scope {
        Scope::Node => h_flex()
            .child("on")
            .child(mono(target.node.clone()))
            .child("·")
            .child(mono(target.address.clone())),
        Scope::Cluster => h_flex().child("in").child(mono(target.context.clone())),
    }
    .gap_1p5()
    .text_size(dp(12.5))
    .text_color(p.muted)
    .when_some(loader.last_successful(), |this, time| {
        this.child("·").child(format!("updated {}", clock(time)))
    })
    .when(source.is_example(), |this| this.child("· example data"));
    let loading = loader.is_loading();
    h_flex()
        .items_end()
        .gap_3()
        .flex_wrap()
        .child(
            v_flex()
                .gap(dp(7.))
                .child(ui::page_title(title))
                .when(!embedded, |this| this.child(scope_line)),
        )
        .child(div().flex_1())
        .child(
            Button::new("screen-refresh")
                .outline()
                .small()
                .icon(IconName::RefreshCw)
                .label("Refresh")
                .loading(loading)
                .disabled(loading)
                .on_click(cx.listener(|view, _, window, cx| view.manual_refresh(window, cx))),
        )
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

/// What to show instead of data: no target yet, a silent node, the first
/// load, or a first load that failed. `None` means render the data.
pub(crate) fn gate<V: ScreenPanel, T: Send + 'static>(
    source: Option<&ScreenSource>,
    loader: &Loader<T>,
    scope: Scope,
    what: &str,
    cx: &mut Context<V>,
) -> Option<AnyElement> {
    if loader.data().is_some() {
        return None;
    }
    let Some(source) = source else {
        return Some(
            ui::empty_state(
                IconName::Server,
                "No node selected",
                "Pick a target node in the title bar.",
                None,
                Vec::new(),
                cx,
            )
            .into_any_element(),
        );
    };
    if loader.is_loading() {
        return Some(skeleton(cx));
    }
    if scope == Scope::Node && !source.responding() {
        let target = &source.target;
        return Some(
            ui::empty_state(
                IconName::Unplug,
                format!("{} isn't responding", target.node),
                format!(
                    "{} needs the Talos API at {}:50000. Pick another target node in the title bar, or retry.",
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
    // Not requested yet; the shell activates visible screens right away.
    Some(skeleton(cx))
}

/// The whole page while [`gate`] applies: the header (when there is a
/// target) above the gate's state. `None` means render the data.
pub(crate) fn gated_page<V: ScreenPanel, T: Send + 'static>(
    id: &'static str,
    title: &'static str,
    scope: Scope,
    source: Option<&ScreenSource>,
    loader: &Loader<T>,
    what: &str,
    cx: &mut Context<V>,
) -> Option<AnyElement> {
    gated_page_mode(id, title, scope, source, loader, what, false, cx)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn gated_page_mode<V: ScreenPanel, T: Send + 'static>(
    id: &'static str,
    title: &'static str,
    scope: Scope,
    source: Option<&ScreenSource>,
    loader: &Loader<T>,
    what: &str,
    embedded: bool,
    cx: &mut Context<V>,
) -> Option<AnyElement> {
    let content = gate(source, loader, scope, what, cx)?;
    let header = source.map(|source| header_mode(title, source, scope, loader, embedded, cx));
    Some(
        page_scroll(id)
            .child(page_body().children(header).child(content))
            .into_any_element(),
    )
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

/// A table column: a fixed width, or `None` to take the remaining space.
#[derive(Clone, Copy)]
pub(crate) struct Column {
    pub(crate) label: &'static str,
    pub(crate) width: Option<f32>,
}

pub(crate) fn cell(column: Column) -> Div {
    let cell = div().px_3().min_w_0().whitespace_nowrap().truncate();
    match column.width {
        Some(width) => cell.flex_none().w(dp(width)),
        None => cell.flex_1().min_w(dp(160.)),
    }
}

pub(crate) fn table_head(columns: &[Column], cx: &App) -> Div {
    let p = palette(cx);
    h_flex()
        .py(dp(9.))
        .border_b_1()
        .border_color(p.line)
        .children(
            columns
                .iter()
                .map(|column| cell(*column).child(ui::caption(column.label, cx))),
        )
}

/// Width a table needs before it scrolls sideways, in `dp`.
pub(crate) fn table_width(columns: &[Column]) -> f32 {
    columns
        .iter()
        .map(|column| column.width.unwrap_or(160.) + 24.)
        .sum()
}

fn skeleton(cx: &App) -> AnyElement {
    panel(cx)
        .p_3()
        .gap_3()
        .children((0..9).map(|_| ui::skeleton(relative(0.7), dp(12.))))
        .into_any_element()
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
