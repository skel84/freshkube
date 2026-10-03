mod connection;
mod kubeconfig;
mod kubernetes_only;
mod kubernetes_summary;
pub(crate) mod nodes;
mod object_links;
mod overview;
mod pages;
mod search;
mod services;
mod shell;
#[cfg(any(debug_assertions, feature = "stress"))]
mod startup;
mod system_services;
mod target;
#[cfg(test)]
mod tests;
mod view;

use crate::{
    GpuiOptions,
    backend::{self, AppliedConfig, OwnedJob, Target},
    connection_preferences::ConnectionStore,
    fixture,
    forwards::ForwardsIndicator,
    logs::LogPanel,
    maintenance::MaintenanceView,
    mutation::{self, Operations},
    presentation::{self, Health, LoadHistory, NodeSummary},
    resources::{
        self, KubeAccess, KubeSource, NotServed, ResourcesScreen, custom::CustomResources,
        navigation, shell as pod_shell,
    },
    screens::{
        DiagnosticsScreen, EtcdScreen, LifecycleScreen, LiveSource, NetworkScreen,
        OperationsScreen, ProcessesScreen, ScreenEvent, ScreenHandle, ScreenPanel, ScreenSource,
        SecurityScreen, StorageScreen, WorkloadsScreen,
    },
    state::Snapshot,
    text_size, theme,
    ui::{clock, dp},
};
use freshkube_core::cluster_overview::{
    ClusterOverview, ClusterOverviewCollector, KubeconfigSelection,
};
use freshkube_core::resources::{ResourceKind, builtin};
use gpui_kit::component::{
    ActiveTheme, Theme, ThemeMode, TitleBar,
    input::{InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};
use talos_rs::{ServiceInfo, TalosClient};
use tokio::runtime::Handle;

/// Render counters for tests that prove a view was (not) redrawn.
pub(crate) mod probe {
    #[cfg(test)]
    thread_local! {
        static HITS: std::cell::RefCell<std::collections::BTreeMap<&'static str, usize>> =
            const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    }

    /// Records one render of the named view.
    #[cfg(test)]
    pub(crate) fn hit(name: &'static str) {
        HITS.with(|hits| *hits.borrow_mut().entry(name).or_default() += 1);
    }

    #[cfg(not(test))]
    #[inline(always)]
    pub(crate) fn hit(_: &'static str) {}

    /// Renders of the named view so far on this thread.
    #[cfg(test)]
    pub(crate) fn count(name: &'static str) -> usize {
        HITS.with(|hits| hits.borrow().get(name).copied().unwrap_or(0))
    }
}

pub(crate) use pages::Page;
use pages::{Area, ColumnReveal, ScreenKind};

pub(crate) const AUTO_REFRESH: Duration = Duration::from_secs(15);
/// The icon rail's width, and the navigation column's beside it, in dp.
pub(crate) const RAIL_WIDTH: f32 = 64.;
pub(crate) const COLUMN_WIDTH: f32 = 208.;
pub(crate) const PAGE_PADDING: f32 = 26.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeView {
    Cards,
    Table,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HealthFilter {
    All,
    Unhealthy,
    Unknown,
}

impl HealthFilter {
    fn accepts(self, health: Health) -> bool {
        match self {
            HealthFilter::All => true,
            HealthFilter::Unhealthy => health == Health::Unhealthy,
            HealthFilter::Unknown => health == Health::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Appearance {
    System,
    Light,
    Dark,
}

gpui_kit::actions!(
    pilot,
    [
        Quit,
        Refresh,
        ShowOverview,
        ShowNodes,
        ShowNamespaces,
        ShowEvents,
        ShowHealth,
        ShowEtcd,
        ShowSystemServices,
        ShowSecurity,
        ShowLifecycle,
        NextScreen,
        PreviousScreen,
        PreviousContext,
        NextContext,
        PreviousNode,
        NextNode,
        PreviousService,
        NextService,
        GoToKind
    ]
);

pub(crate) fn run(options: GpuiOptions, runtime: Handle) -> color_eyre::Result<()> {
    let error = std::sync::Arc::new(std::sync::Mutex::new(None));
    let launch_error = error.clone();
    gpui_kit::application()
        .with_assets(theme::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::install(cx);
            text_size::install(options.preferences.clone(), cx);
            cx.bind_keys([
                KeyBinding::new("secondary-q", Quit, None),
                KeyBinding::new("secondary-r", Refresh, Some("Freshkube")),
            ]);
            cx.on_action(|_: &Quit, cx| {
                // Quitting mid-operation would abandon a half-done change,
                // and quitting ends a shell, so it asks first.
                let Some(window) = cx.windows().into_iter().next() else {
                    return cx.quit();
                };
                let may_quit = window
                    .update(cx, |_, window, cx| {
                        mutation::may_close(window, cx)
                            && pod_shell::may_close(window, cx, |_, cx| cx.quit())
                    })
                    .unwrap_or(true);
                if may_quit {
                    cx.quit();
                }
            });
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            #[cfg(not(any(debug_assertions, feature = "stress")))]
            let window_size = size(px(1320.), px(860.));
            #[cfg(any(debug_assertions, feature = "stress"))]
            let window_size =
                startup::window_size(std::env::var("FRESHKUBE_WINDOW_SIZE").ok().as_deref());
            let window_options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(window_size, cx)),
                window_min_size: Some(size(px(760.), px(560.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(
                        if options.maintenance_endpoint().is_some() {
                            "Freshkube (maintenance)"
                        } else {
                            "Freshkube"
                        }
                        .into(),
                    ),
                    // Centred in the 52 px header.
                    traffic_light_position: Some(point(px(18.), px(20.))),
                    ..TitleBar::title_bar_options()
                }),
                ..TitleBar::window_options()
            };
            // Maintenance mode replaces the cluster shell: no talosconfig,
            // no cluster, only the insecure node workflow.
            let opened = match options.maintenance_endpoint().map(str::to_owned) {
                Some(endpoint) => gpui_kit::open_window(window_options, cx, |window, cx| {
                    cx.new(|cx| MaintenanceView::new(endpoint, runtime, window, cx))
                })
                .map(|_| ()),
                None => gpui_kit::open_window(window_options, cx, |window, cx| {
                    cx.new(|cx| Pilot::new(options, runtime, window, cx))
                })
                .map(|_| ()),
            };
            if let Err(failure) = opened {
                *launch_error.lock().expect("launch error mutex") = Some(failure.to_string());
                cx.quit();
            }
            cx.activate(true);
        });
    if let Some(error) = error.lock().expect("launch error mutex").take() {
        color_eyre::eyre::bail!("Could not open GPUI window: {error}");
    }
    Ok(())
}

/// The countdown ring in the title bar. It is its own view so the one-second
/// tick redraws only the ring, not the page behind it.
pub(crate) struct Countdown {
    remaining: f32,
    visible: bool,
}

impl Countdown {
    /// Sets what the next render draws; returns whether that differs.
    fn set(&mut self, remaining: f32, visible: bool) -> bool {
        let changed = self.visible != visible || (visible && self.remaining != remaining);
        self.remaining = remaining;
        self.visible = visible;
        changed
    }
}

impl Render for Countdown {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::ui::countdown_ring(self.remaining, self.visible, cx)
    }
}

/// Overview and Services are drawn by the shell itself; hosting each in a view
/// of its own lets GPUI reuse the last frame until the shell's state changes,
/// like the screens, which are views already.
struct PageHost {
    pilot: WeakEntity<Pilot>,
    page: Page,
}

impl Render for PageHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        probe::hit(self.page.slug());
        let page = self.page;
        self.pilot
            .update(cx, |pilot, cx| match page {
                Page::Overview => pilot.render_overview(window, cx),
                Page::Nodes => pilot.render_nodes(window, cx),
                _ => pilot.system_services.clone().into_any_element(),
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}

/// What a cached page view is laid out as: the whole space it is given.
fn cached_page_style() -> StyleRefinement {
    StyleRefinement::default().size_full()
}

pub(crate) struct Pilot {
    runtime: Handle,
    connection_store: Option<ConnectionStore>,
    /// The absolute path of the last successfully parsed Talos configuration.
    loaded_config_path: Option<PathBuf>,
    applied: AppliedConfig,
    path: Entity<InputState>,
    service_filter: Entity<InputState>,
    contexts: Vec<String>,
    /// Node counts for contexts that loaded at least once this session.
    context_nodes: BTreeMap<String, usize>,
    context_display: shell::ContextDisplay,
    config_error: Option<String>,
    config_loading: bool,
    config_generation: u64,
    epoch: u64,
    overview: Snapshot<ClusterOverview>,
    kubernetes_summary: Snapshot<Arc<freshkube_core::kubernetes_summary::KubernetesSummary>>,
    summary_health: Option<Result<Arc<crate::screens::WorkloadData>, String>>,
    summary_job: Option<OwnedJob>,
    summary_task: Option<Task<()>>,
    object_open_job: Option<OwnedJob>,
    object_open_task: Option<Task<()>>,
    object_open_sequence: u64,
    services: Snapshot<Vec<ServiceInfo>, Target>,
    service_display: services::ServiceDisplay,
    nodes: Vec<NodeSummary>,
    load_history: LoadHistory,
    selected_node: Option<String>,
    selected_service: Option<String>,
    logs: Entity<LogPanel>,
    countdown: Entity<Countdown>,
    /// The status bar's port forwards, which redraw on their own.
    forwards: Entity<ForwardsIndicator>,
    overview_page: Entity<PageHost>,
    services_page: Entity<PageHost>,
    nodes_page: Entity<PageHost>,
    node_workspace: nodes::Nodes,
    node_pods: Entity<ResourcesScreen>,
    screens: Vec<(ScreenKind, ScreenHandle)>,
    resources: Entity<ResourcesScreen>,
    /// The Kubernetes kind the Resources page shows.
    resource_kind: ResourceKind,
    last_kind: ResourceKind,
    system_services: Entity<system_services::SystemServices>,
    /// The sidebar's Custom Resources, discovered when opened.
    custom: Entity<CustomResources>,
    /// Command-K's palette of kinds.
    search: Entity<search::Search>,
    /// The panel gallery a debug fixture check shows in place of the page.
    gallery: Option<gpui_kit::AnyView>,
    /// The rail's area, whose pages or kinds the column lists.
    area: Area,
    /// The kind each built-in group showed last, by group slug.
    group_kinds: BTreeMap<&'static str, String>,
    /// The custom kind shown last, which Custom Resources opens again.
    last_custom: Option<ResourceKind>,
    /// The Control plane page shown last.
    last_control: Page,
    /// Problem dots on the rail, from the overview's cards.
    rail_marks: shell::RailMarks,
    column_scroll: ScrollHandle,
    /// A column row to scroll into view on the next frame.
    column_reveal: Option<ColumnReveal>,
    /// Kubernetes credentials source for the overview roster and screens.
    kubeconfig: KubeconfigSelection,
    /// Set without Talos: contexts come from a kubeconfig and only the
    /// Resources page reads.
    kubernetes_only: Option<kubernetes_only::KubernetesOnly>,
    /// Whether the settings popover is open; a page can open it too.
    settings_open: bool,
    kubeconfig_draft: kubeconfig::KubeconfigDraft,
    page: Page,
    overview_display: crate::presentation::overview::Overview,
    attention: crate::presentation::attention::Attention,
    attention_expanded: bool,
    health_filter: HealthFilter,
    appearance: Appearance,
    automatic: bool,
    elapsed: Duration,
    fixture: bool,
    fixture_tick: u64,
    focus: FocusHandle,
    node_focus: FocusHandle,
    service_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
    config_job: Option<OwnedJob>,
    overview_job: Option<OwnedJob>,
    service_job: Option<OwnedJob>,
    config_task: Option<Task<()>>,
    overview_task: Option<Task<()>>,
    service_task: Option<Task<()>>,
    _tick: Task<()>,
}

impl Pilot {
    fn new(
        options: GpuiOptions,
        runtime: Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The window opens on Overview, which has no navigation column.
        crate::screens::set_chrome_width(RAIL_WIDTH);
        cx.bind_keys([
            KeyBinding::new("secondary-1", ShowOverview, Some("Freshkube")),
            KeyBinding::new("secondary-2", ShowNodes, Some("Freshkube")),
            KeyBinding::new("secondary-3", ShowNamespaces, Some("Freshkube")),
            KeyBinding::new("secondary-4", ShowEvents, Some("Freshkube")),
            KeyBinding::new("secondary-5", ShowHealth, Some("Freshkube")),
            KeyBinding::new("secondary-6", ShowEtcd, Some("Freshkube")),
            KeyBinding::new("secondary-7", ShowSystemServices, Some("Freshkube")),
            KeyBinding::new("secondary-8", ShowSecurity, Some("Freshkube")),
            KeyBinding::new("secondary-9", ShowLifecycle, Some("Freshkube")),
            // Reaches the screens without a number of their own.
            KeyBinding::new("ctrl-tab", NextScreen, Some("Freshkube")),
            KeyBinding::new("ctrl-shift-tab", PreviousScreen, Some("Freshkube")),
            KeyBinding::new("alt-up", PreviousContext, Some("Freshkube")),
            KeyBinding::new("alt-down", NextContext, Some("Freshkube")),
            KeyBinding::new("secondary-k", GoToKind, Some("Freshkube")),
            KeyBinding::new("up", PreviousNode, Some("TalosNodes")),
            KeyBinding::new("left", PreviousNode, Some("TalosNodes")),
            KeyBinding::new("down", NextNode, Some("TalosNodes")),
            KeyBinding::new("right", NextNode, Some("TalosNodes")),
            KeyBinding::new("up", PreviousService, Some("TalosServices")),
            KeyBinding::new("down", NextService, Some("TalosServices")),
        ]);
        let path = cx.new(|cx| InputState::new(window, cx).placeholder("Default talosconfig"));
        path.update(cx, |input, cx| {
            input.set_value(
                options
                    .config_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                window,
                cx,
            )
        });
        let service_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter services"));
        let logs = cx.new(|cx| LogPanel::new(runtime.clone(), options.tail, window, cx));
        // Overview is the first page; logs collect in the background until
        // Logs is shown.
        logs.update(cx, |logs, cx| logs.set_visible(false, cx));
        let countdown = cx.new(|_| Countdown {
            remaining: 1.,
            visible: false,
        });
        let pilot = cx.weak_entity();
        let overview_page = cx.new(|_| PageHost {
            pilot: pilot.clone(),
            page: Page::Overview,
        });
        let nodes_page = cx.new(|_| PageHost {
            pilot: pilot.clone(),
            page: Page::Nodes,
        });
        let node_workspace = nodes::Nodes::new(runtime.clone(), window, cx);
        let node_pods = cx.new(|cx| ResourcesScreen::new(runtime.clone(), window, cx));
        let services_page = cx.new(|_| PageHost {
            pilot,
            page: Page::SystemServices,
        });
        let mut subscriptions = Vec::new();
        // The hosted pages read the shell's state, so whatever notifies the
        // shell redraws them; the countdown tick notifies only its ring.
        let hosted = [
            overview_page.clone(),
            services_page.clone(),
            nodes_page.clone(),
        ];
        subscriptions.push(cx.observe(&cx.entity(), move |_, _, cx| {
            for page in &hosted {
                page.update(cx, |_, cx| cx.notify());
            }
        }));
        // The status bar shows the running operation; the window refuses to
        // close under it. Cached screens may read the slot too.
        let operations = Operations::global(cx);
        subscriptions.push(cx.observe(&operations, |view, _, cx| {
            view.notify_cached(cx);
            cx.notify();
        }));
        window.on_window_should_close(cx, |window, cx| {
            mutation::may_close(window, cx)
                && pod_shell::may_close(window, cx, |window, _| window.remove_window())
        });
        subscriptions.push(cx.subscribe_in(
            &node_pods,
            window,
            |this, _, event: &resources::NodePodsEvent, window, cx| match event {
                resources::NodePodsEvent::Back => this.node_back(window, cx),
                resources::NodePodsEvent::Open(identity) => {
                    let identity = identity.clone();
                    this.open_object(
                        builtin("pods").expect("pod kind"),
                        identity.into(),
                        resources::Tab::Overview,
                        window,
                        cx,
                    );
                }
            },
        ));
        cx.bind_keys([
            KeyBinding::new("down", NextNode, Some("NodeWorkspace")),
            KeyBinding::new("up", PreviousNode, Some("NodeWorkspace")),
            KeyBinding::new("enter", nodes::OpenNode, Some("NodeWorkspace")),
            KeyBinding::new("escape", nodes::BackNode, Some("NodeWorkspace")),
            KeyBinding::new(
                "secondary-shift-enter",
                nodes::ExpandNode,
                Some("NodeWorkspace"),
            ),
            KeyBinding::new("secondary-}", nodes::NextNodeTab, Some("NodeWorkspace")),
            KeyBinding::new("secondary-{", nodes::PreviousNodeTab, Some("NodeWorkspace")),
            KeyBinding::new("right", nodes::NextNodeTab, Some("NodeWorkspaceTabs")),
            KeyBinding::new("left", nodes::PreviousNodeTab, Some("NodeWorkspaceTabs")),
        ]);
        let screens = ScreenKind::ALL
            .into_iter()
            .map(|page| {
                let runtime = runtime.clone();
                let handle = match page {
                    ScreenKind::Processes => {
                        Self::screen::<ProcessesScreen>(runtime, &mut subscriptions, window, cx)
                    }
                    ScreenKind::Storage => {
                        Self::screen::<StorageScreen>(runtime, &mut subscriptions, window, cx)
                    }
                    ScreenKind::Network => {
                        Self::screen::<NetworkScreen>(runtime, &mut subscriptions, window, cx)
                    }
                    ScreenKind::Diagnostics => {
                        Self::screen::<DiagnosticsScreen>(runtime, &mut subscriptions, window, cx)
                    }
                    ScreenKind::Etcd => {
                        Self::screen::<EtcdScreen>(runtime, &mut subscriptions, window, cx)
                    }
                    ScreenKind::Health => {
                        Self::screen::<WorkloadsScreen>(runtime, &mut subscriptions, window, cx)
                    }
                    ScreenKind::Security => {
                        Self::screen::<SecurityScreen>(runtime, &mut subscriptions, window, cx)
                    }
                    ScreenKind::Lifecycle => {
                        Self::screen::<LifecycleScreen>(runtime, &mut subscriptions, window, cx)
                    }
                    ScreenKind::Operations => {
                        Self::screen::<OperationsScreen>(runtime, &mut subscriptions, window, cx)
                    }
                };
                (page, handle)
            })
            .collect();
        let resources = cx.new(|cx| ResourcesScreen::new(runtime.clone(), window, cx));
        subscriptions.push(cx.subscribe_in(
            &resources,
            window,
            |this, _, event: &resources::ResourceLink, window, cx| {
                this.resource_link(event.clone(), window, cx)
            },
        ));
        let custom = cx.new(|_| CustomResources::new(runtime.clone()));
        subscriptions.extend([
            cx.observe(&custom, |_, _, cx| cx.notify()),
            // A kind that stopped being served may have taken its group's
            // other kinds with it; discover the group again.
            cx.subscribe(&resources, |view, _, NotServed(kind), cx| {
                let group = kind.group.clone();
                view.custom
                    .update(cx, |custom, cx| custom.not_served(&group, cx));
            }),
        ]);
        // Cached views keep their last frame; a font size or palette change
        // that doesn't refresh the window by itself must still redraw them.
        subscriptions.push(cx.observe_global_in::<Theme>(window, |view, window, cx| {
            view.prepare_context_display(window, cx);
            view.notify_cached(cx);
            cx.notify();
        }));
        subscriptions.extend([
            cx.subscribe_in(&service_filter, window, |view, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    view.rebuild_service_rows(cx);
                    cx.notify();
                }
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                if this.appearance == Appearance::System {
                    Theme::sync_system_appearance(Some(window), cx);
                }
            }),
            // Batches arriving while Logs is hidden never notify; the shell
            // shows only what start, stop and the visible page change.
            cx.observe(&logs, |_, _, cx| cx.notify()),
            cx.observe(&service_filter, |_, _, cx| cx.notify()),
        ]);
        let tick = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this
                    .update_in(cx, |view, window, cx| {
                        view.elapsed += Duration::from_secs(1);
                        if view.automatic
                            && view.elapsed >= AUTO_REFRESH
                            && !view.overview.is_loading()
                            && !view.config_loading
                        {
                            view.refresh(window, cx);
                        }
                        // The ring advances every second; a refresh that
                        // started above notified the shell by itself.
                        view.tick_countdown(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut view = Self {
            runtime: runtime.clone(),
            connection_store: options.preferences.as_deref().map(ConnectionStore::new),
            loaded_config_path: None,
            applied: AppliedConfig {
                path: options.config_path,
                context: options.context,
            },
            path,
            service_filter,
            contexts: Vec::new(),
            context_nodes: BTreeMap::new(),
            context_display: shell::ContextDisplay::default(),
            config_error: None,
            config_loading: false,
            config_generation: 0,
            epoch: 0,
            overview: Snapshot::default(),
            kubernetes_summary: Snapshot::default(),
            summary_health: None,
            summary_job: None,
            summary_task: None,
            object_open_job: None,
            object_open_task: None,
            object_open_sequence: 0,
            services: Snapshot::default(),
            service_display: services::ServiceDisplay::default(),
            nodes: Vec::new(),
            load_history: LoadHistory::default(),
            selected_node: None,
            selected_service: None,
            logs,
            countdown,
            forwards: cx.new(ForwardsIndicator::new),
            overview_page,
            services_page,
            nodes_page,
            node_workspace,
            node_pods,
            screens,
            resources,
            resource_kind: builtin(navigation::DEFAULT_KIND).expect("the default kind is built in"),
            last_kind: builtin(navigation::DEFAULT_KIND).expect("the default kind is built in"),
            system_services: cx.new(|cx| system_services::SystemServices::new(window, cx)),
            custom,
            search: cx.new(|cx| search::Search::new(runtime.clone(), window, cx)),
            gallery: None,
            area: Area::Overview,
            group_kinds: BTreeMap::new(),
            last_custom: None,
            last_control: Page::Etcd,
            rail_marks: Default::default(),
            column_scroll: ScrollHandle::new(),
            column_reveal: None,
            kubeconfig: KubeconfigSelection::Automatic,
            kubernetes_only: None,
            settings_open: false,
            kubeconfig_draft: Default::default(),
            page: Page::Overview,
            overview_display: Default::default(),
            attention: Default::default(),
            attention_expanded: false,
            health_filter: HealthFilter::All,
            appearance: Appearance::System,
            automatic: true,
            elapsed: Duration::ZERO,
            fixture: options.fixture,
            fixture_tick: 0,
            focus: cx.focus_handle(),
            node_focus: cx.focus_handle(),
            service_focus: cx.focus_handle(),
            _subscriptions: subscriptions,
            config_job: None,
            overview_job: None,
            service_job: None,
            config_task: None,
            overview_task: None,
            service_task: None,
            _tick: tick,
        };
        if view.fixture {
            view.contexts = fixture::CONTEXTS
                .iter()
                .map(|name| (*name).into())
                .collect();
            view.applied.context = Some(view.contexts[0].clone());
            view.seed_fixture_history();
            view.refresh(window, cx);
        } else if options.kubernetes_only {
            view.kubernetes_only = Some(kubernetes_only::KubernetesOnly::new(
                options.kubeconfig_path,
                options.kube_context,
            ));
            view.load_kube_contexts(window, cx);
        } else {
            view.load_configuration(window, cx);
            if let Some(path) = options.kubeconfig_path {
                view.inspect_kubeconfig_file(path, window, cx);
            }
        }
        view._subscriptions.push(cx.subscribe_in(
            &view.system_services,
            window,
            |this, _, event: &system_services::ServiceEvent, window, cx| {
                let (node, service, tab) = match event {
                    system_services::ServiceEvent::Logs(node, service) => {
                        (node, service, nodes::NodeTab::Logs)
                    }
                    system_services::ServiceEvent::Open(node, service) => {
                        (node, service, nodes::NodeTab::Services)
                    }
                };
                this.open_node_by_name(node, tab, window, cx);
                this.selected_service = Some(service.clone());
                if tab == nodes::NodeTab::Logs {
                    this.open_logs(window, cx);
                }
                cx.notify();
            },
        ));
        view.prepare_context_display(window, cx);
        // Initial shell focus makes contextual commands available without a click.
        window.focus(&view.focus, cx);
        // Without Talos, the one page that reads is the first one shown.
        if view.kubernetes_only.is_some() {
            view.navigate_from_keyboard(Page::Overview, window, cx);
        }
        #[cfg(any(debug_assertions, feature = "stress"))]
        view.open_startup_page(window, cx);
        #[cfg(feature = "stress")]
        crate::stress::start(window, cx);
        view
    }

    fn invalidate_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.epoch = self.epoch.wrapping_add(1);
        self.overview_task = None;
        self.overview_job = None;
        self.service_task = None;
        self.service_job = None;
        self.selected_node = None;
        self.selected_service = None;
        self.nodes.clear();
        self.system_services
            .update(cx, |services, cx| services.set_nodes(&[], cx));
        self.service_display = services::ServiceDisplay::default();
        self.load_history.clear();
        self.overview = Snapshot::default();
        self.kubernetes_summary = Snapshot::default();
        self.system_services
            .update(cx, |services, cx| services.set_nodes(&self.nodes, cx));
        self.rebuild_joined_nodes();
        self.push_node_rows(cx);
        self.prepare_context_display(window, cx);
        self.node_workspace
            .document
            .update(cx, |pane, cx| pane.close(cx));
        self.sync_node_visibility(window, cx);
        self.summary_health = None;
        self.summary_job = None;
        self.summary_task = None;
        self.attention_expanded = false;
        self.object_open_job = None;
        self.object_open_task = None;
        self.object_open_sequence = self.object_open_sequence.wrapping_add(1);
        self.services = Snapshot::default();
        self.rebuild_service_rows(cx);
        self.logs
            .update(cx, |logs, cx| logs.set_target(None, Vec::new(), window, cx));
        self.push_source(window, cx);
    }

    fn load_configuration(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.config_task = None;
        self.config_job = None;
        self.config_generation = self.config_generation.wrapping_add(1);
        let generation = self.config_generation;
        self.invalidate_target(window, cx);
        self.contexts.clear();
        self.context_nodes.clear();
        self.loaded_config_path = None;
        self.config_error = None;
        self.config_loading = true;
        let (job, receiver) = backend::load_contexts(self.runtime.clone(), self.applied.clone());
        self.config_job = Some(job);
        self.config_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Configuration worker stopped".into()));
            _ = this.update_in(cx, |view, window, cx| {
                if generation != view.config_generation {
                    return;
                }
                view.config_loading = false;
                view.config_job = None;
                match result {
                    Ok(catalog) => {
                        view.loaded_config_path = Some(catalog.path);
                        view.contexts = catalog.names;
                        if view.applied.context.is_none() {
                            view.applied.context = Some(catalog.current);
                        }
                        view.remember_connection(window, cx);
                        view.refresh(window, cx);
                    }
                    Err(error) => view.config_error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Applies the talosconfig path typed in Settings and reloads contexts.
    /// Without Talos, a path switches the window to Talos.
    fn apply_config_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fixture || self.config_loading {
            return;
        }
        self.unless_shell(window, cx, Self::use_config_path);
    }

    fn use_config_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.path.read(cx).value().to_string();
        if self.kubernetes_only.is_some() {
            // The default talosconfig is what's missing.
            if draft.trim().is_empty() {
                return;
            }
            self.leave_kubernetes_only(window, cx);
        }
        self.applied.path = if draft.trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(draft.trim()))
        };
        self.applied.context = None;
        self.load_configuration(window, cx);
    }

    /// Opens the native file picker; a chosen file fills the Talosconfig
    /// field and is applied at once. Cancelling leaves everything unchanged.
    fn browse_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fixture || self.config_loading {
            return;
        }
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use Talosconfig".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = selection.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            _ = this.update_in(cx, |view, window, cx| {
                let text = path.display().to_string();
                view.path
                    .update(cx, |input, cx| input.set_value(text, window, cx));
                view.apply_config_path(window, cx);
            });
        })
        .detach();
    }

    fn selected_summary(&self) -> Option<&NodeSummary> {
        self.nodes
            .iter()
            .find(|node| Some(&node.name) == self.selected_node.as_ref())
    }

    fn target(&self) -> Option<(Target, TalosClient)> {
        let cluster = self.overview.data()?;
        let node = self.selected_summary()?;
        Some((
            Target {
                epoch: self.epoch,
                context: cluster.name.clone(),
                node: node.name.clone(),
                address: node.address.clone(),
            },
            cluster.client.clone()?,
        ))
    }

    fn screen<T: ScreenPanel>(
        runtime: Handle,
        subscriptions: &mut Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ScreenHandle {
        let screen = cx.new(|cx| T::new(runtime, window, cx));
        subscriptions.push(cx.subscribe_in(&screen, window, Self::screen_event));
        ScreenHandle::new(screen)
    }

    fn screen_event<T>(
        &mut self,
        _: &Entity<T>,
        event: &ScreenEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ScreenEvent::Back => self.node_back(window, cx),
            ScreenEvent::RefreshSummary => self.refresh_summary(window, cx),
            ScreenEvent::OpenLogs(service) => {
                self.selected_service = Some(service.clone());
                self.open_logs(window, cx);
            }
            ScreenEvent::SelectNode(node) => {
                self.open_node_by_name(node, nodes::NodeTab::Overview, window, cx)
            }
            ScreenEvent::OpenLogsOn { node, service } => {
                if !self.nodes.iter().any(|known| &known.name == node) {
                    return;
                }
                self.open_node_by_name(node, nodes::NodeTab::Logs, window, cx);
                self.selected_service = Some(service.clone());
                self.open_logs(window, cx);
            }
        }
    }

    /// What screens inspect: the target node, its roster and, unless this is
    /// example data, live Talos access. `None` until a target exists.
    fn screen_source(&self) -> Option<ScreenSource> {
        let cluster = self.overview.data()?;
        let node = self.selected_summary()?;
        let live = if self.fixture {
            None
        } else {
            let mut collector = ClusterOverviewCollector::new(
                self.applied.path.clone(),
                self.applied.context.clone(),
            );
            collector.set_kubeconfig_selection(self.kubeconfig.clone());
            Some(LiveSource {
                client: cluster.client.clone()?,
                cluster: Arc::new(cluster.clone()),
                collector,
                config_path: self.applied.path.clone(),
            })
        };
        Some(ScreenSource {
            target: Target {
                epoch: self.epoch,
                context: cluster.name.clone(),
                node: node.name.clone(),
                address: node.address.clone(),
            },
            nodes: Arc::new(self.nodes.clone()),
            live,
        })
    }

    /// Where the Resources page reads: example objects, or the Kubernetes
    /// API of the Talos cluster. Its id covers what picks the cluster and
    /// its credentials, not the target node, so changing node keeps a watch.
    fn kube_source(&self) -> Option<KubeSource> {
        if self.fixture {
            let context = self.applied.context.clone()?;
            return Some(KubeSource {
                id: resources::example::connection(&context),
                context,
                access: KubeAccess::Example,
            });
        }
        if let Some(kube) = &self.kubernetes_only {
            let access = kube.access()?.clone();
            return Some(KubeSource {
                id: access.id(),
                context: access.context().to_owned(),
                access: KubeAccess::Direct(access),
            });
        }
        let context = self.applied.context.clone()?;
        let cluster = self.overview.data()?;
        let mut collector =
            ClusterOverviewCollector::new(self.applied.path.clone(), Some(context.clone()));
        collector.set_kubeconfig_selection(self.kubeconfig.clone());
        let path = self
            .applied
            .path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        Some(KubeSource {
            id: format!("talos:{path}:{context}:{:?}", self.kubeconfig),
            context: cluster.name.clone(),
            access: KubeAccess::Talos(Box::new(LiveSource {
                client: cluster.client.clone()?,
                cluster: Arc::new(cluster.clone()),
                collector,
                config_path: self.applied.path.clone(),
            })),
        })
    }

    fn active_screen(&self) -> Option<ScreenHandle> {
        let page = if self.page == Page::Nodes && self.node_workspace.open {
            self.node_workspace.tab.screen()?
        } else {
            self.page.screen()?
        };
        self.screens
            .iter()
            .find(|(candidate, _)| *candidate == page)
            .map(|(_, handle)| handle.clone())
    }

    /// Hands every screen the current source; the visible one loads if empty.
    fn push_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = self.screen_source();
        for (_, screen) in &self.screens {
            screen.set_source(source.clone(), window, cx);
        }
        if let Some(data) = self.summary_health.clone() {
            self.deliver_workloads(data, cx);
        }
        if let Some(screen) = self.active_screen() {
            screen.activate(window, cx);
        }
        let source = self.kube_source();
        self.custom
            .update(cx, |custom, cx| custom.set_source(source.clone(), cx));
        self.node_pods.update(cx, |resources, cx| {
            resources.set_source(source.clone(), window, cx)
        });
        self.resources
            .update(cx, |resources, cx| resources.set_source(source, window, cx));
    }

    /// A refresh the user asked for. Unlike the automatic one it also lists
    /// the Resources page again; its watch keeps it current otherwise.
    fn refresh_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh(window, cx);
        if self.page == Page::Resources {
            self.resources
                .update(cx, |resources, cx| resources.refresh(window, cx));
        }
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.kubernetes_only.is_some() {
            self.refresh_kubernetes(window, cx);
            self.refresh_summary(window, cx);
            return;
        }
        if self.config_loading || self.overview.is_loading() {
            return;
        }
        self.elapsed = Duration::ZERO;
        let request = self.overview.begin(self.applied.clone());
        if self.fixture {
            self.fixture_tick += 1;
            let context = self.applied.context.clone().unwrap_or_default();
            self.overview
                .apply(&request, Ok(fixture::cluster(&context, self.fixture_tick)));
            self.overview_succeeded();
            self.sync_nodes(window, cx);
            self.refresh_services(window, cx);
            self.refresh_summary(window, cx);
            self.refresh_screen(window, cx);
            cx.notify();
            return;
        }
        // The visible screen refreshes alongside the overview with the
        // handles it already has; a new roster reaches it when this lands.
        self.refresh_screen(window, cx);
        let epoch = self.epoch;
        let (job, receiver) = backend::collect(
            self.runtime.clone(),
            self.applied.clone(),
            self.kubeconfig.clone(),
        );
        self.overview_job = Some(job);
        self.overview_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Overview worker stopped".into()));
            _ = this.update_in(cx, |view, window, cx| {
                // Node switches freeze the request too; don't apply a snapshot
                // requested for a different foreground target.
                if epoch != view.epoch {
                    return;
                }
                view.overview_job = None;
                if view.overview.apply(&request, result) {
                    let fresh = !view.overview.is_stale() && view.overview.error().is_none();
                    if fresh {
                        view.overview_succeeded();
                    }
                    view.sync_nodes(window, cx);
                    if fresh {
                        view.refresh_services(window, cx);
                        view.refresh_summary(window, cx);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn refresh_screen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Health {
            return;
        }
        if let Some(screen) = self.active_screen() {
            screen.refresh(window, cx);
        }
    }

    /// Example data starts with five minutes of load history so the
    /// sparklines show a trend immediately.
    fn seed_fixture_history(&mut self) {
        let context = self.applied.context.clone().unwrap_or_default();
        for _ in 1..presentation::LOAD_HISTORY_LEN {
            self.fixture_tick += 1;
            let nodes =
                presentation::node_summaries(&fixture::cluster(&context, self.fixture_tick));
            self.load_history.record(&nodes);
        }
    }

    /// Records per-refresh history once a snapshot for the applied target lands.
    fn overview_succeeded(&mut self) {
        let Some(cluster) = self.overview.data() else {
            return;
        };
        let nodes = presentation::node_summaries(cluster);
        self.load_history.record(&nodes);
        if let Some(context) = self.applied.context.clone() {
            self.context_nodes.insert(context, nodes.len());
        }
    }

    fn set_appearance(
        &mut self,
        appearance: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.appearance = appearance;
        match appearance {
            Appearance::System => Theme::sync_system_appearance(Some(window), cx),
            Appearance::Light => Theme::change(ThemeMode::Light, Some(window), cx),
            Appearance::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
        }
        cx.notify();
    }

    fn simulate_failure(&mut self, cx: &mut Context<Self>) {
        let request = self.overview.begin(self.applied.clone());
        self.overview.apply(
            &request,
            Err("Example: the Talos API didn't answer within 10 s".into()),
        );
        cx.notify();
    }

    fn status<T, I: Clone + Eq>(snapshot: &Snapshot<T, I>) -> String {
        let refreshed = snapshot
            .last_successful()
            .map(|time| {
                let time: chrono::DateTime<chrono::Local> = time.into();
                format!("Last success {}", time.format("%H:%M:%S"))
            })
            .unwrap_or_else(|| "No successful refresh".into());
        let state = if snapshot.is_loading() {
            "Loading"
        } else if snapshot.is_stale() {
            "Stale · previous data retained"
        } else if snapshot.data().is_some() {
            "Current snapshot"
        } else {
            "Unavailable"
        };
        format!("{state} · {refreshed}")
    }

    /// Width available to page content in `dp`, for choosing grid column
    /// counts.
    fn content_width(window: &Window) -> f32 {
        crate::screens::content_width(window)
    }

    fn render_logs_page(&self) -> AnyElement {
        div()
            .size_full()
            .min_h_0()
            .pt(dp(22.))
            .px(dp(PAGE_PADDING))
            .child(self.logs.clone().cached(cached_page_style()))
            .into_any_element()
    }

    /// Marks every cached view dirty, for state they read from outside their
    /// own entity.
    fn notify_cached(&self, cx: &mut App) {
        for (_, screen) in &self.screens {
            App::notify(cx, screen.view().entity_id());
        }
        App::notify(cx, self.logs.entity_id());
        App::notify(cx, self.resources.entity_id());
        let detail = self.resources.read(cx).detail_view();
        App::notify(cx, detail);
        App::notify(cx, self.overview_page.entity_id());
        App::notify(cx, self.services_page.entity_id());
        App::notify(cx, self.nodes_page.entity_id());
        App::notify(cx, self.node_pods.entity_id());
        App::notify(cx, self.node_workspace.document.entity_id());
    }

    /// Whether contexts or a connection are loading, for the refresh button.
    fn loading(&self) -> bool {
        let connecting = self
            .kubernetes_only
            .as_ref()
            .is_some_and(|kube| kube.connection == kubernetes_only::KubeConnection::Connecting);
        self.config_loading || self.overview.is_loading() || connecting
    }

    /// How full the countdown ring is, and whether it shows at all.
    fn countdown_state(&self) -> (f32, bool) {
        let loading = self.loading();
        (
            1. - self.elapsed.as_secs_f32() / AUTO_REFRESH.as_secs_f32(),
            self.automatic && self.overview.data().is_some() && !loading,
        )
    }

    /// Moves the ring on without redrawing anything else.
    fn tick_countdown(&self, cx: &mut Context<Self>) {
        let (remaining, visible) = self.countdown_state();
        self.countdown.update(cx, |countdown, cx| {
            if countdown.set(remaining, visible) {
                cx.notify();
            }
        });
    }
}
