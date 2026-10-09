mod access;
mod app_menu;
#[cfg(test)]
mod applications_column_tests;
mod connection;
pub(crate) mod dock;
#[cfg(test)]
mod gate_tests;
mod kubeconfig;
mod kubernetes_only;
mod kubernetes_summary;
#[cfg(test)]
mod link_tests;
#[cfg(test)]
mod menu_tests;
pub(crate) mod nodes;
mod object_links;
mod overview;
mod pages;
#[cfg(test)]
mod proof_tests;
mod search;
mod services;
mod session;
mod settings;
mod shell;
#[cfg(any(debug_assertions, feature = "stress"))]
mod startup;
mod switch;
#[cfg(test)]
mod switch_tests;
mod system_services;
mod target;
#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
mod use_kubernetes_tests;
mod view;

use crate::{
    GpuiOptions,
    backend::{self, AppliedConfig, OwnedJob, Target},
    connection_preferences::ConnectionStore,
    fixture,
    forwards::ForwardsIndicator,
    logs::{LogPanel, TalosPanel},
    maintenance::MaintenanceView,
    monitoring::history::HistoryView,
    monitoring::page::{MonitoringEvent, MonitoringPage, TalosNodes},
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
    ui::clock,
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

pub(crate) use freshkube_probe::probe;

pub(crate) use pages::Page;
use pages::{Area, ColumnReveal, ScreenKind};

pub(crate) const AUTO_REFRESH: Duration = Duration::from_secs(15);
/// A key context no element draws: a binding in it only gives macOS's bar
/// the key to show.
const MENU_BAR_ONLY: &str = "MenuBarOnly";
pub(crate) use freshkube_ui::page::{COLUMN_WIDTH, PAGE_PADDING, RAIL_WIDTH};
// Desktop's screen tests measure pages at this old path.
#[cfg(test)]
pub(crate) use freshkube_ui::layout_check;

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

pub(crate) use freshkube_ui::page::Refresh;

gpui_kit::actions!(
    pilot,
    [
        Quit,
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
        GoToKind,
        ToggleColumn
    ]
);

/// Quit answers: quitting mid-operation would abandon a half-done change,
/// and quitting ends a shell, so it asks first. `quit` ends the app, once
/// they agree; tests pass their own, since a test app can't quit.
///
/// Its key and the menus send Quit from inside the window's own update,
/// where the window can't be updated again, so it asks once that update
/// ends; asking inside it found no window and quit without a word.
pub(crate) fn on_quit(cx: &mut App, quit: fn(&mut App)) {
    cx.on_action(move |_: &Quit, cx| {
        cx.defer(move |cx| {
            let Some(window) = cx.windows().into_iter().next() else {
                return quit(cx);
            };
            let may_quit = window
                .update(cx, |_, window, cx| {
                    mutation::may_close(window, cx)
                        && pod_shell::may_close(window, cx, move |_, cx| quit(cx))
                })
                .unwrap_or(true);
            if may_quit {
                quit(cx);
            }
        });
    });
}

pub(crate) fn run(options: GpuiOptions, runtime: Handle) -> color_eyre::Result<()> {
    let error = std::sync::Arc::new(std::sync::Mutex::new(None));
    let launch_error = error.clone();
    gpui_kit::application()
        .with_assets(theme::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::install(cx);
            text_size::install(options.preferences.clone(), cx);
            // Kit's animations draw still when the OS asks for reduced motion.
            // Tests never run this, so they keep the reduced motion they set.
            freshkube_ui::motion::follow_system(cx);
            cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
            freshkube_ui::platform::bind_keys(cx);
            // Maintenance mode has the same menus; the shell sets them again
            // as what they state changes.
            cx.set_menus(app_menu::menus(
                freshkube_ui::platform::Platform::current(),
                &app_menu::MenuState::default(),
            ));
            on_quit(cx, |cx| cx.quit());
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
            // Debug fixture checks open maintenance mode on an invented node
            // that stops at the review (`maintenance/example.rs`).
            #[cfg(debug_assertions)]
            let example_maintenance = options.is_fixture()
                && std::env::var("FRESHKUBE_PAGE").as_deref() == Ok("maintenance");
            #[cfg(not(debug_assertions))]
            let example_maintenance = false;
            let window_options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(window_size, cx)),
                window_min_size: Some(size(px(760.), px(560.))),
                // The window's controls sit on the header at the saved text
                // size; the shell moves them when it changes.
                titlebar: Some(freshkube_ui::platform::titlebar_options(
                    if options.maintenance_endpoint().is_some() || example_maintenance {
                        "Freshkube (maintenance)"
                    } else {
                        "Freshkube"
                    },
                    crate::text_size::current(cx),
                )),
                ..TitleBar::window_options()
            };
            // Maintenance mode replaces the cluster shell: no talosconfig,
            // no cluster, only the insecure node workflow.
            let opened = match options.maintenance_endpoint().map(str::to_owned) {
                #[cfg(debug_assertions)]
                None if example_maintenance => {
                    gpui_kit::open_window(window_options, cx, |window, cx| {
                        cx.new(|cx| MaintenanceView::example(runtime, window, cx))
                    })
                    .map(|_| ())
                }
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

/// Puts the window's controls on the header at the current text size: the
/// header is in dp and grows with it, macOS's traffic lights don't.
pub(crate) fn place_window_controls(window: &mut Window, cx: &App) {
    freshkube_ui::platform::place_window_controls(window, crate::text_size::current(cx));
}

/// What a cached page view is laid out as: the whole space it is given.
fn cached_page_style() -> StyleRefinement {
    StyleRefinement::default().size_full()
}

/// The entities whose events the shell routes.
struct PageEntities<'a> {
    node_pods: &'a Entity<ResourcesScreen>,
    resources: &'a Entity<ResourcesScreen>,
    dock: &'a Entity<dock::Dock>,
    observability: &'a Entity<crate::observability::ObservabilityPage>,
    monitoring: &'a Entity<MonitoringPage>,
    custom: &'a Entity<CustomResources>,
    applications: &'a Entity<crate::applications::ApplicationsPage>,
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
    /// The cluster sessions: a workspace of one.
    registry: session::Registry,
    /// How the active workspace entry was defined when it was opened.
    active_definition: session::Definition,
    /// The Talos entry waiting for its talosconfig's contexts.
    entry_open: Option<switch::EntryOpen>,
    /// A link into another entry, waiting for it to open.
    pending_link: Option<switch::PendingLink>,
    /// What the notices about links and switches said, for tests: the
    /// window offers no way to read a notice back.
    #[cfg(test)]
    pub(crate) told: Vec<String>,
    /// The read that finds the default kubeconfig file an entry's context is in.
    entry_locate: Option<(OwnedJob, Task<()>)>,
    entry_generation: u64,
    /// The workspace revision the header's list was built from.
    switcher_revision: u64,
    /// When the restored summary on show was read; none once a read replaces it.
    restored_taken: Option<std::time::SystemTime>,
    /// "Last known · 3 min ago", beside the Overview's connection state.
    age_label: Entity<switch::AgeLabel>,
    /// The header's list of the workspace file's clusters.
    switcher: Vec<switch::ClusterItem>,
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
    fps: Entity<shell::fps::Fps>,
    /// The header, rail and column, each cached beside the page.
    chrome: shell::ChromeParts,
    /// The status bar's port forwards, which redraw on their own.
    forwards: Entity<ForwardsIndicator>,
    /// Logs under the page, in tabs; app-wide, like the forwards.
    pub(crate) dock: Entity<dock::Dock>,
    overview_page: Entity<PageHost>,
    services_page: Entity<PageHost>,
    nodes_page: Entity<PageHost>,
    node_workspace: nodes::Nodes,
    node_pods: Entity<ResourcesScreen>,
    screens: Vec<(ScreenKind, ScreenHandle)>,
    /// Typed observation recipients; `screens` wraps these same entities.
    health: Entity<WorkloadsScreen>,
    lifecycle: Entity<LifecycleScreen>,
    resources: Entity<ResourcesScreen>,
    /// The Kubernetes kind the Resources page shows.
    resource_kind: ResourceKind,
    last_kind: ResourceKind,
    system_services: Entity<system_services::SystemServices>,
    settings_page: Entity<settings::SettingsPage>,
    applications: Entity<crate::applications::ApplicationsPage>,
    /// `workspace.json` beside the preferences; none without preferences.
    workspace_file: Option<PathBuf>,
    /// The sidebar's Custom Resources, discovered when opened.
    custom: Entity<CustomResources>,
    /// Command-K's palette of kinds.
    search: Entity<search::Search>,
    /// The Monitoring page, which reads only while it shows.
    monitoring: Entity<MonitoringPage>,
    observability: Entity<crate::observability::ObservabilityPage>,
    column_state: shell::ColumnState,
    /// The node pane's CPU and memory, when the context has a Prometheus.
    node_history: Entity<HistoryView>,
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
    /// The expanded column's lines, keyboard and scroll.
    column_list: freshkube_ui::source_list::SourceList<shell::ColumnKey>,
    /// The column's line that `column_reveal` lands on, derived with it.
    column_reveal_line: Option<usize>,
    /// Whether the column's rows to reveal have all arrived.
    column_settled: bool,
    /// Whether what the column shows changed since its lines were derived.
    column_stale: bool,
    /// The icon rail, which scrolls when the window is short or the text
    /// large.
    rail_scroll: ScrollHandle,
    /// The collapsed column's icons, which scroll in the same way.
    compact_column_scroll: ScrollHandle,
    /// A column row to scroll into view on the next frame.
    column_reveal: Option<ColumnReveal>,
    /// The column row scrolled into view and waiting to clear the fades.
    column_reveal_pass: Option<ColumnReveal>,
    /// The active area, and the room it had, last scrolled into view.
    rail_revealed: Option<(Area, shell::Room)>,
    /// The rail button scrolled into view and waiting to clear the fades.
    rail_reveal_pass: Option<(Area, shell::Room)>,
    /// The Observability column's list, which scrolls when the window is
    /// short or the text large.
    obs_column_scroll: ScrollHandle,
    /// The destination, whether the column was collapsed and the room it
    /// had, last scrolled into view.
    obs_column_revealed: Option<(crate::observability::Destination, bool, shell::Room)>,
    /// The destination scrolled into view and waiting to clear the fades.
    obs_column_reveal_pass: Option<(crate::observability::Destination, bool, shell::Room)>,
    /// Kubernetes credentials source for the overview roster and screens.
    kubeconfig: KubeconfigSelection,
    /// Set without Talos: contexts come from a kubeconfig and only the
    /// Resources page reads.
    kubernetes_only: Option<kubernetes_only::KubernetesOnly>,
    /// Whether the settings popover is open; a page can open it too.
    settings_open: bool,
    /// The platform whose menu the window has: the one it runs on, but a
    /// debug build shows the menu button with `FRESHKUBE_APP_MENU=button`,
    /// and tests choose.
    menu_platform: freshkube_ui::platform::Platform,
    /// The menu button's menu while it's open, and its dismissal's
    /// subscription.
    app_menu_popup: Option<(Entity<gpui_kit::component::menu::PopupMenu>, Subscription)>,
    /// What had the keyboard when the menu button was pressed, before its
    /// popover took it: the view the menu's entries act on.
    app_menu_focus: Option<FocusHandle>,
    /// What the installed menus state, to set them again only on a change.
    menu_state: Option<app_menu::MenuState>,
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
    /// Example data holds the Talos overview and the Kubernetes summary
    /// (`fixture::hold`), so the pages that wait for them stay loading.
    fixture_hold: bool,
    fixture_tick: u64,
    focus: FocusHandle,
    node_focus: FocusHandle,
    service_focus: FocusHandle,
    /// Scrolls the Services page, or the node's Services tab.
    service_scroll: ScrollHandle,
    /// The selected service changed by pointer or keyboard, so a stacked
    /// layout brings its details into view once they are laid out.
    reveal_service: bool,
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
    /// The Workloads screen, for its tests to read its rows.
    #[cfg(test)]
    pub(crate) fn workloads(&self) -> Entity<WorkloadsScreen> {
        self.health.clone()
    }

    /// The Applications page and the page shown, for its tests.
    #[cfg(test)]
    pub(crate) fn applications(&self) -> (Entity<crate::applications::ApplicationsPage>, Page) {
        (self.applications.clone(), self.page)
    }

    /// The object Resources shows in its drawer, for tests of the links
    /// that open one.
    #[cfg(test)]
    pub(crate) fn opened_object(&self, cx: &App) -> Option<resources::model::ResourceIdentity> {
        self.resources.read(cx).detail_identity(cx).cloned()
    }

    /// Chooses another context, as the header's switcher does.
    #[cfg(test)]
    pub(crate) fn choose_context(
        &mut self,
        context: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_context(context.to_owned(), window, cx);
    }

    /// The shell's global key bindings, by key context.
    fn bind_shell_keys(cx: &mut App) {
        freshkube_ui::source_list::bind_keys(cx);
        cx.bind_keys([
            KeyBinding::new(
                "secondary-r",
                Refresh,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-b",
                ToggleColumn,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-1",
                ShowOverview,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-2",
                ShowNodes,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-3",
                ShowNamespaces,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-4",
                ShowEvents,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-5",
                ShowHealth,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-6",
                ShowEtcd,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-7",
                ShowSystemServices,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-8",
                ShowSecurity,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-9",
                ShowLifecycle,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            // macOS's bar shows and binds an action's first binding, and
            // GPUI hands AppKit the name `tab`, of which AppKit keeps the
            // T: ⌃T would switch screens. The Tab character itself, in a
            // context nothing draws, binds nothing here, and AppKit shows
            // no key for it, since Control-Tab moves its focus.
            KeyBinding::new("ctrl-\t", NextScreen, Some(MENU_BAR_ONLY)),
            KeyBinding::new("ctrl-shift-\t", PreviousScreen, Some(MENU_BAR_ONLY)),
            // Reaches the screens without a number of their own.
            KeyBinding::new(
                "ctrl-tab",
                NextScreen,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "ctrl-shift-tab",
                PreviousScreen,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "alt-up",
                PreviousContext,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "alt-down",
                NextContext,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-k",
                GoToKind,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new("up", PreviousNode, Some("TalosNodes")),
            KeyBinding::new("left", PreviousNode, Some("TalosNodes")),
            KeyBinding::new("down", NextNode, Some("TalosNodes")),
            KeyBinding::new("right", NextNode, Some("TalosNodes")),
            KeyBinding::new("up", PreviousService, Some("TalosServices")),
            KeyBinding::new("down", NextService, Some("TalosServices")),
        ]);
        cx.bind_keys([
            KeyBinding::new("down", NextNode, Some("NodeWorkspace")),
            KeyBinding::new("up", PreviousNode, Some("NodeWorkspace")),
            KeyBinding::new("enter", nodes::OpenNode, Some("NodeWorkspace")),
            KeyBinding::new("h", nodes::ToggleHealthyNodes, Some("NodeWorkspace")),
            KeyBinding::new(
                "down",
                system_services::NextService,
                Some(system_services::CONTEXT),
            ),
            KeyBinding::new(
                "up",
                system_services::PreviousService,
                Some(system_services::CONTEXT),
            ),
            KeyBinding::new(
                "enter",
                system_services::OpenServiceNode,
                Some(system_services::CONTEXT),
            ),
            KeyBinding::new(
                "o",
                system_services::OpenServiceNode,
                Some(system_services::CONTEXT),
            ),
            KeyBinding::new(
                "l",
                system_services::ServiceLogs,
                Some(system_services::CONTEXT),
            ),
            KeyBinding::new(
                "escape",
                system_services::ClearService,
                Some(system_services::CONTEXT),
            ),
            KeyBinding::new("down", settings::NextCluster, Some(settings::CONTEXT)),
            KeyBinding::new("up", settings::PreviousCluster, Some(settings::CONTEXT)),
            KeyBinding::new("escape", settings::ClearCluster, Some(settings::CONTEXT)),
            KeyBinding::new("a", settings::AddCluster, Some(settings::CONTEXT)),
            // A key shows its last binding: E, as Enter on a list opens.
            KeyBinding::new("enter", settings::EditCluster, Some(settings::CONTEXT)),
            KeyBinding::new("e", settings::EditCluster, Some(settings::CONTEXT)),
            KeyBinding::new(
                "backspace",
                settings::RemoveCluster,
                Some(settings::CONTEXT),
            ),
            KeyBinding::new(
                "secondary-alt-up",
                settings::MoveClusterUp,
                Some(settings::CONTEXT),
            ),
            KeyBinding::new(
                "secondary-alt-down",
                settings::MoveClusterDown,
                Some(settings::CONTEXT),
            ),
            KeyBinding::new("r", settings::ReloadWorkspace, Some(settings::CONTEXT)),
        ]);
        cx.bind_keys(crate::applications::key_bindings());
        cx.bind_keys([
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
    }

    /// Routes the pages' and the dock's events to the shell.
    fn subscribe_page_events(
        subscriptions: &mut Vec<Subscription>,
        pages: PageEntities<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let PageEntities {
            node_pods,
            resources,
            dock,
            observability,
            monitoring,
            custom,
            applications,
        } = pages;
        // A part of an open application, opened as every object link is.
        subscriptions.push(cx.subscribe_in(
            applications,
            window,
            |this, _, link: &resources::ResourceLink, window, cx| {
                this.resource_link(link.clone(), window, cx)
            },
        ));
        subscriptions.push(cx.subscribe_in(
            node_pods,
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
        subscriptions.push(cx.subscribe_in(
            resources,
            window,
            |this, _, event: &resources::ResourceLink, window, cx| {
                this.resource_link(event.clone(), window, cx)
            },
        ));
        subscriptions.push(cx.observe(dock, |_, _, cx| cx.notify()));
        subscriptions.push(cx.subscribe_in(
            dock,
            window,
            |this, _, event: &dock::DockEvent, window, cx| match event {
                dock::DockEvent::Leave(back) => {
                    // What had the keyboard may no longer be drawn, as after
                    // going to another page; the shell's root contains it
                    // only if the last frame drew it.
                    if let Some(back) = back {
                        window.focus(back, cx);
                        if this.focus.contains_focused(window, cx) {
                            return;
                        }
                    }
                    this.focus_page(window, cx)
                }
            },
        ));
        subscriptions.push(
            cx.subscribe_in(observability, window, |this, _, event, window, cx| {
                use crate::observability::ObservabilityEvent;
                match event {
                    ObservabilityEvent::Navigation => cx.notify(),
                    ObservabilityEvent::Dashboards => {
                        this.navigate_from_keyboard(Page::Monitoring, window, cx)
                    }
                    ObservabilityEvent::OpenExamplePod {
                        namespace,
                        name,
                        logs,
                    } if this.fixture => this.open_object(
                        builtin("pods").unwrap(),
                        resources::model::ObjectRef {
                            connection: None,
                            namespace: namespace.clone(),
                            name: name.clone(),
                            uid: String::new(),
                        },
                        if *logs {
                            resources::Tab::Logs
                        } else {
                            resources::Tab::Overview
                        },
                        window,
                        cx,
                    ),
                    ObservabilityEvent::OpenExamplePod { .. } => {}
                    ObservabilityEvent::OpenObject {
                        source,
                        app,
                        subject,
                    } => {
                        if this
                            .observability
                            .read(cx)
                            .link_is_current(source, app, subject)
                            && this
                                .kube_source()
                                .is_some_and(|source| source.id == subject.access())
                        {
                            this.open_object(
                                subject.kind().clone(),
                                resources::model::ObjectRef {
                                    connection: Some(subject.access().into()),
                                    namespace: subject.namespace().into(),
                                    name: subject.name().into(),
                                    uid: String::new(),
                                },
                                resources::Tab::Overview,
                                window,
                                cx,
                            );
                        }
                    }
                }
            }),
        );
        subscriptions.push(
            cx.subscribe_in(
                monitoring,
                window,
                |this, _, event, window, cx| match event {
                    MonitoringEvent::Catalog => cx.notify(),
                    MonitoringEvent::History => this.push_history(cx),
                    // The column lists the dashboards; an open one stays open.
                    MonitoringEvent::Dashboards => {
                        if this.column_collapsed(window) {
                            this.toggle_column(window, cx);
                        }
                    }
                },
            ),
        );
        subscriptions.extend([
            cx.observe(custom, |_, _, cx| cx.notify()),
            // A kind that stopped being served may have taken its group's
            // other kinds with it; discover the group again.
            cx.subscribe(resources, |view, _, NotServed(kind), cx| {
                let group = kind.group.clone();
                view.custom
                    .update(cx, |custom, cx| custom.not_served(&group, cx));
            }),
        ]);
    }

    /// The one-second tick: the countdown, and the automatic refresh.
    fn start_tick(window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this
                    .update_in(cx, |view, window, cx| {
                        view.elapsed += Duration::from_secs(1);
                        if view.automatic
                            && view.kubernetes_only.is_none()
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
        })
    }

    /// Loads the first source: the example clusters, a kubeconfig without
    /// Talos, or the talosconfig.
    fn open_initial_source(
        &mut self,
        kubernetes_only: bool,
        kubeconfig_path: Option<PathBuf>,
        kube_context: Option<String>,
        remembered_kubernetes: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fixture {
            self.contexts = fixture::CONTEXTS
                .iter()
                .map(|name| (*name).into())
                .collect();
            self.applied.context = Some(self.contexts[0].clone());
            self.seed_fixture_history();
            self.refresh(window, cx);
        } else if kubernetes_only {
            let kube = kubernetes_only::KubernetesOnly::new(kubeconfig_path, kube_context);
            self.kubernetes_only = Some(if remembered_kubernetes {
                kube.remembering()
            } else {
                kube
            });
            self.load_kube_contexts(window, cx);
        } else {
            self.load_configuration(window, cx);
            if let Some(path) = kubeconfig_path {
                self.inspect_kubeconfig_file(path, None, window, cx);
            }
        }
    }

    fn new(
        options: GpuiOptions,
        runtime: Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let names_source =
            options.fixture || options.named_source || options.maintenance_endpoint.is_some();
        let launched = switch::Launched {
            kubernetes_only: options.kubernetes_only,
            kubeconfig: options.kubeconfig_path.clone(),
            kube_context: options.kube_context.clone(),
            talosconfig: options.config_path.clone(),
            talos_context: options.context.clone(),
        };
        // The window opens on Overview, which has no navigation column.
        crate::screens::set_chrome_width(RAIL_WIDTH);
        // Opened before the pages, which read their splits' sizes from it.
        let navigation =
            crate::navigation_file::NavigationFile::open(options.preferences.as_deref());
        navigation.clone().install(cx);
        Self::bind_shell_keys(cx);
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
        let mut node_workspace = nodes::Nodes::new(runtime.clone(), window, cx);
        // The header the loading rows sit under before anything answers.
        node_workspace.rebuild_columns(!options.kubernetes_only);
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
        let health =
            Self::screen_entity::<WorkloadsScreen>(runtime.clone(), &mut subscriptions, window, cx);
        let lifecycle =
            Self::screen_entity::<LifecycleScreen>(runtime.clone(), &mut subscriptions, window, cx);
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
                    ScreenKind::Health => ScreenHandle::new(health.clone()),
                    ScreenKind::Security => {
                        Self::screen::<SecurityScreen>(runtime, &mut subscriptions, window, cx)
                    }
                    ScreenKind::Lifecycle => ScreenHandle::new(lifecycle.clone()),
                    ScreenKind::Operations => {
                        Self::screen::<OperationsScreen>(runtime, &mut subscriptions, window, cx)
                    }
                };
                (page, handle)
            })
            .collect();
        let resources = cx.new(|cx| ResourcesScreen::new(runtime.clone(), window, cx));
        let dock = cx.new(|cx| dock::Dock::new(runtime.clone(), cx));
        // Its height and whether it shows are laid out by the shell.
        let custom = cx.new(|_| CustomResources::new(runtime.clone()));
        let secrets = options.keyring.then(|| -> crate::secrets::Secrets {
            std::sync::Arc::new(crate::secrets::SystemStore)
        });
        let monitoring = cx.new(|cx| {
            let mut page = MonitoringPage::new(
                runtime.clone(),
                options.preferences.as_deref(),
                secrets.clone(),
                cx,
            );
            if crate::fixture::hold().monitoring {
                page.hold_examples();
            }
            page
        });
        let observability = cx.new(|cx| {
            let mut page = crate::observability::ObservabilityPage::new(
                options.fixture,
                runtime.clone(),
                options.preferences.as_deref(),
                secrets,
                window,
                cx,
            );
            if crate::fixture::hold().coroot {
                page.hold_examples();
            }
            page
        });
        // Cached views keep their last frame; a font size or palette change
        // that doesn't refresh the window by itself must still redraw them.
        subscriptions.push(cx.observe_global_in::<Theme>(window, |view, window, cx| {
            place_window_controls(window, cx);
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
        let applications =
            cx.new(|cx| crate::applications::ApplicationsPage::new(runtime.clone(), window, cx));
        Self::subscribe_page_events(
            &mut subscriptions,
            PageEntities {
                node_pods: &node_pods,
                resources: &resources,
                dock: &dock,
                observability: &observability,
                monitoring: &monitoring,
                custom: &custom,
                applications: &applications,
            },
            window,
            cx,
        );
        let tick = Self::start_tick(window, cx);
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
            registry: session::Registry::new(),
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
            fps: cx.new(shell::fps::Fps::new),
            chrome: shell::ChromeParts::new(cx),
            forwards: cx.new(ForwardsIndicator::new),
            dock,
            overview_page,
            services_page,
            nodes_page,
            node_workspace,
            node_pods,
            screens,
            health,
            lifecycle,
            resources,
            resource_kind: builtin(navigation::DEFAULT_KIND).expect("the default kind is built in"),
            last_kind: builtin(navigation::DEFAULT_KIND).expect("the default kind is built in"),
            system_services: cx.new(|cx| system_services::SystemServices::new(window, cx)),
            settings_page: cx.new(settings::SettingsPage::new),
            applications,
            workspace_file: options
                .preferences
                .as_deref()
                .map(|preferences| preferences.with_file_name("workspace.json")),
            custom,
            search: cx.new(|cx| search::Search::new(runtime.clone(), window, cx)),
            monitoring,
            observability,
            column_state: shell::ColumnState::new(navigation),
            node_history: cx.new(|_| HistoryView::new(runtime.clone(), "node")),
            area: Area::Overview,
            group_kinds: BTreeMap::new(),
            last_custom: None,
            last_control: Page::Etcd,
            rail_marks: Default::default(),
            column_list: freshkube_ui::source_list::SourceList::new("nav-column-list", cx),
            column_reveal_line: None,
            column_settled: true,
            column_stale: true,
            rail_scroll: ScrollHandle::new(),
            compact_column_scroll: ScrollHandle::new(),
            column_reveal: None,
            column_reveal_pass: None,
            rail_revealed: None,
            rail_reveal_pass: None,
            obs_column_scroll: ScrollHandle::new(),
            obs_column_revealed: None,
            obs_column_reveal_pass: None,
            kubeconfig: KubeconfigSelection::Automatic,
            kubernetes_only: None,
            active_definition: Default::default(),
            entry_open: None,
            pending_link: None,
            #[cfg(test)]
            told: Vec::new(),
            entry_locate: None,
            entry_generation: 0,
            switcher_revision: 0,
            restored_taken: None,
            age_label: cx.new(|_| switch::AgeLabel::new()),
            switcher: Vec::new(),
            settings_open: false,
            menu_platform: app_menu::menu_platform(),
            app_menu_popup: None,
            app_menu_focus: None,
            menu_state: None,
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
            fixture_hold: options.fixture && options.hold_talos,
            fixture_tick: 0,
            focus: cx.focus_handle(),
            node_focus: cx.focus_handle(),
            service_focus: cx.focus_handle(),
            service_scroll: ScrollHandle::new(),
            reveal_service: false,
            _subscriptions: subscriptions,
            config_job: None,
            overview_job: None,
            service_job: None,
            config_task: None,
            overview_task: None,
            service_task: None,
            _tick: tick,
        };
        view.load_workspace(cx);
        view.rebuild_switcher(cx);
        let workspace_entry = view.launch_entry(names_source, cx);
        let started_entry = workspace_entry.is_some();
        if let Some((id, note)) = workspace_entry {
            view.activate_entry(&id, window, cx);
            if let Some(note) = note {
                gpui_kit::component::WindowExt::push_notification(window, note, cx);
            }
        } else {
            view.open_initial_source(
                options.kubernetes_only,
                options.kubeconfig_path,
                options.kube_context,
                options.remembered_kubernetes,
                window,
                cx,
            );
        }
        if !started_entry {
            view.adopt_launched_entry(&launched, cx);
        }
        view._subscriptions
            .push(cx.observe(&view.settings_page, |this, _, cx| this.rebuild_switcher(cx)));
        view.watch_menus(window, cx);
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
        view.publish_reading(cx);
        view.chrome.watch(&view, cx);
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
        // The whole session goes, with its summary tasks, before anything
        // below rebuilds from it.
        self.registry.reset_active();
        self.search.update(cx, |search, cx| search.invalidate(cx));
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
        self.system_services
            .update(cx, |services, cx| services.set_nodes(&self.nodes, cx));
        self.rebuild_joined_nodes(cx);
        self.push_node_rows(cx);
        self.prepare_context_display(window, cx);
        self.node_workspace
            .document
            .update(cx, |pane, cx| pane.close(cx));
        self.sync_node_visibility(window, cx);
        self.attention_expanded = false;
        self.object_open_job = None;
        self.object_open_task = None;
        self.object_open_sequence = self.object_open_sequence.wrapping_add(1);
        self.services = Snapshot::default();
        self.rebuild_service_rows(cx);
        self.logs
            .update(cx, |logs, cx| logs.set_target(None, Vec::new(), window, cx));
        self.push_source(window, cx);
        self.restore_parked_summary(window, cx);
    }

    fn load_configuration(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.config_task = None;
        self.config_job = None;
        self.config_generation = self.config_generation.wrapping_add(1);
        let generation = self.config_generation;
        // Before the reset publishes the overview's state, so the old
        // error isn't shown for the new path.
        self.config_error = None;
        self.invalidate_target(window, cx);
        self.contexts.clear();
        self.context_nodes.clear();
        self.loaded_config_path = None;
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
                            if view.entry_open.is_some() {
                                // An entry names its Talos context or has the
                                // one its `context` names; the file's current
                                // context never stands in.
                                let names = view.contexts.clone();
                                let file = view.loaded_config_path.clone().unwrap_or_default();
                                if !view.talos_context_for_entry(&names, &file, window, cx) {
                                    cx.notify();
                                    return;
                                }
                            } else if view.contexts.contains(&catalog.current) {
                                view.applied.context = Some(catalog.current);
                            } else {
                                view.pending_link = None;
                                view.config_error = Some(format!(
                                    "Talos context '{}' was not found",
                                    catalog.current
                                ));
                                view.rebuild_joined_nodes(cx);
                                cx.notify();
                                return;
                            }
                        }
                        view.remember_connection(window, cx);
                        view.refresh(window, cx);
                    }
                    Err(error) => {
                        view.pending_link = None;
                        view.config_error = Some(error);
                        view.rebuild_joined_nodes(cx);
                        view.prepare_context_display(window, cx);
                    }
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
        self.leave_entry(cx);
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
        ScreenHandle::new(Self::screen_entity::<T>(runtime, subscriptions, window, cx))
    }

    fn screen_entity<T: ScreenPanel>(
        runtime: Handle,
        subscriptions: &mut Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<T> {
        let screen = cx.new(|cx| T::new(runtime, window, cx));
        subscriptions.push(cx.subscribe_in(&screen, window, Self::screen_event));
        screen
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
            // With nothing reading yet, Health's Retry reads the kubeconfig
            // and connects again too.
            ScreenEvent::RefreshSummary
                if self.kubernetes_only.is_some()
                    && self.registry.active().summary_session.is_none() =>
            {
                self.refresh(window, cx)
            }
            ScreenEvent::RefreshSummary => self.refresh_summary(window, cx),
            ScreenEvent::RetryCluster => self.refresh(window, cx),
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
                applied: self.applied_access(),
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

    /// The local access the last overview was collected with, which a live
    /// source carries for Operations to pin its clients to.
    fn applied_access(&self) -> Option<resources::talos::AppliedAccess> {
        Some(resources::talos::AppliedAccess {
            config_path: self.applied.path.clone(),
            selection: self.kubeconfig.clone(),
            configuration: self.registry.active().access_configuration?,
            identity: self.registry.active().access?,
        })
    }

    /// Whose Kubernetes access this is, as `kube_source`'s id, even while
    /// that source is missing for a while: none once another context,
    /// kubeconfig or talosconfig is chosen, until it has loaded.
    fn kube_identity(&self) -> Option<String> {
        if self.fixture {
            return Some(resources::example::connection(
                self.applied.context.as_ref()?,
            ));
        }
        if let Some(kube) = &self.kubernetes_only {
            return kube.access().map(|access| access.id());
        }
        self.registry.active().access.map(|access| access.key())
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
        Some(KubeSource {
            id: self.registry.active().access?.key(),
            context: cluster.name.clone(),
            access: KubeAccess::Talos(Box::new(resources::talos::TalosAccess {
                configuration: self.registry.active().access_configuration?,
                selection: self.kubeconfig.clone(),
                live: LiveSource {
                    client: cluster.client.clone()?,
                    cluster: Arc::new(cluster.clone()),
                    collector,
                    config_path: self.applied.path.clone(),
                    applied: self.applied_access(),
                },
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
        self.sync_node_metrics(window, cx);
        self.deliver_summary_nodes(cx);
        let source = self.screen_source();
        for (_, screen) in &self.screens {
            screen.set_source(source.clone(), window, cx);
        }
        if let Some(data) = self.registry.active().summary_health.clone() {
            self.deliver_workloads(data, cx);
        }
        self.sync_unread_health(cx);
        if let Some(screen) = self.active_screen() {
            screen.activate(window, cx);
        }
        let source = self.kube_source();
        self.sync_search_source(source.clone(), window, cx);
        self.custom
            .update(cx, |custom, cx| custom.set_source(source.clone(), cx));
        self.node_pods.update(cx, |resources, cx| {
            resources.set_source(source.clone(), window, cx)
        });
        let cluster = source.as_ref().map(KubeSource::cluster);
        self.monitoring
            .update(cx, |monitoring, cx| monitoring.set_source(cluster, cx));
        let id = source.as_ref().map(|source| source.id.clone());
        self.observability
            .update(cx, |page, cx| page.set_source(id, cx));
        self.applications
            .update(cx, |page, cx| page.set_source(source.clone(), cx));
        let identity = self.kube_identity();
        self.dock.update(cx, |dock, cx| {
            dock.set_source(source.clone(), identity.as_deref(), window, cx)
        });
        self.resources
            .update(cx, |resources, cx| resources.set_source(source, window, cx));
    }

    /// Hands pod and node history the Monitoring page's Prometheus.
    fn push_history(&mut self, cx: &mut Context<Self>) {
        let history = self.monitoring.read(cx).history();
        self.node_history
            .update(cx, |view, cx| view.set_source(history.clone(), cx));
        self.resources
            .update(cx, |resources, cx| resources.set_history(history, cx));
        cx.notify();
    }

    /// A refresh the user asked for. Unlike the automatic one it also lists
    /// the Resources page again; its watch keeps it current otherwise.
    fn refresh_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.registry.active_mut().prompted_access = None;
        self.refresh_summary(window, cx);
        self.refresh(window, cx);
        if self.page == Page::Resources {
            self.resources
                .update(cx, |resources, cx| resources.refresh(window, cx));
        }
        if self.page == Page::Monitoring {
            self.monitoring
                .update(cx, |monitoring, cx| monitoring.refresh(cx));
        }
        if self.page == Page::Observability {
            self.observability
                .update(cx, |page, cx| page.refresh_current(cx));
        }
        if self.page == Page::Applications {
            self.applications.update(cx, |page, cx| page.refresh(cx));
        }
        if self.page == Page::Nodes {
            self.node_history
                .update(cx, |history, cx| history.refresh(cx));
        }
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.kubernetes_only.is_some() {
            self.refresh_kubernetes(window, cx);
            self.ensure_summary(window, cx);
            return;
        }
        if self.config_loading || self.overview.is_loading() {
            return;
        }
        self.elapsed = Duration::ZERO;
        let request = self.overview.begin(self.applied.clone());
        self.publish_reading(cx);
        if self.fixture && self.fixture_hold {
            // Loading until the hold is released, which only a test does.
            cx.notify();
            return;
        }
        if self.fixture {
            self.fixture_tick += 1;
            let context = self.applied.context.clone().unwrap_or_default();
            self.overview
                .apply(&request, Ok(fixture::cluster(&context, self.fixture_tick)));
            self.overview_succeeded();
            self.sync_nodes(window, cx);
            self.refresh_services(window, cx);
            self.ensure_summary(window, cx);
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
            self.observed_nodes(),
        );
        self.overview_job = Some(job);
        self.overview_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Overview worker stopped".into()));
            _ = this.update_in(cx, |view, window, cx| {
                view.overview_received(epoch, request, result, window, cx);
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
        self.rebuild_joined_nodes(cx);
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

    /// Marks every cached view dirty, for state they read from outside their
    /// own entity.
    fn notify_cached(&self, cx: &mut App) {
        for (_, screen) in &self.screens {
            App::notify(cx, screen.view().entity_id());
        }
        App::notify(cx, self.observability.entity_id());
        App::notify(cx, self.monitoring.entity_id());
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
            self.automatic
                && self.kubernetes_only.is_none()
                && self.overview.data().is_some()
                && !loading,
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
