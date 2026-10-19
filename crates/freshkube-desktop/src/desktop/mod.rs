mod access;
mod app_menu;
#[cfg(test)]
mod applications_column_tests;
#[cfg(test)]
mod applications_tests;
mod connection;
pub(crate) mod dock;
mod entity_pages;
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
mod page_map;
mod pages;
#[cfg(test)]
mod proof_tests;
mod search;
mod services;
mod session;
mod settings;
#[path = "startup/setup.rs"]
mod setup_startup;
mod shell;
mod source;
#[cfg(any(debug_assertions, feature = "stress"))]
mod startup;
mod states;
mod switch;
#[cfg(test)]
mod switch_tests;
mod system_services;
mod target;
#[cfg(test)]
mod testing;
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
    logs::{LogPanel, TalosPanel},
    maintenance::MaintenanceView,
    monitoring::page::TalosNodes,
    mutation::{self, Operations},
    presentation::{self, NodeSummary},
    resources::{self, navigation, shell as pod_shell},
    screens::{ScreenEvent, ScreenHandle},
    state::Snapshot,
    text_size, theme,
    ui::clock,
};
use freshkube_core::cluster_overview::{ClusterOverview, KubeconfigSelection};
use freshkube_core::resources::{ResourceKind, builtin};
use freshkube_reads::Loader;
use gpui_kit::component::{ActiveTheme, Theme, TitleBar, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
use talos_rs::{ServiceInfo, TalosClient};
use tokio::runtime::Handle;

pub(crate) use freshkube_probe::probe;

pub(crate) use overview::OverviewEvent;
pub(crate) use pages::Page;
use pages::{Area, ColumnReveal, PageId};
pub(crate) use settings::SettingsEvent;
pub(crate) use system_services::ServiceEvent;

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

pub(crate) fn run(mut options: GpuiOptions, runtime: Handle) -> color_eyre::Result<()> {
    // Before anything reads the workspace.
    options.setup_notice = startup::migrate_setup(&options);
    let error = std::sync::Arc::new(std::sync::Mutex::new(None));
    let launch_error = error.clone();
    gpui_kit::application()
        .with_assets(theme::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::install(cx);
            text_size::install_with(options.preferences.clone(), options.debug.text_size, cx);
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
            let window_size = startup::window_size(options.debug.window_size.as_deref());
            // Debug fixture checks open maintenance mode on an invented node
            // that stops at the review (`maintenance/example.rs`).
            #[cfg(debug_assertions)]
            let example_maintenance =
                options.starts_with_examples() && options.debug.page() == Some("maintenance");
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
                        cx.new(|cx| {
                            MaintenanceView::example(runtime, options.debug.theme, window, cx)
                        })
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

/// Puts the window's controls on the header at the current text size: the
/// header is in dp and grows with it, macOS's traffic lights don't.
pub(crate) fn place_window_controls(window: &mut Window, cx: &App) {
    freshkube_ui::platform::place_window_controls(window, crate::text_size::current(cx));
}

/// What a cached page view is laid out as: the whole space it is given.
fn cached_page_style() -> StyleRefinement {
    StyleRefinement::default().size_full()
}

/// What `Pilot::new` makes the shell of, beside the session.
struct Owners {
    pages: page_map::Pages,
    chrome: shell::Chrome,
    navigation: shell::Navigation,
    switch: switch::Switcher,
    subscriptions: Vec<Subscription>,
    tick: Task<()>,
}

/// Configuration discovery serves either the Talos file or the Kubernetes files.
#[derive(Clone, PartialEq, Eq)]
enum ConfigurationRead {
    Talos(AppliedConfig),
    Kubernetes(Option<PathBuf>),
}

pub(crate) struct Pilot {
    runtime: Handle,
    connection_store: Option<ConnectionStore>,
    /// The absolute path of the last successfully parsed Talos configuration.
    loaded_config_path: Option<PathBuf>,
    applied: AppliedConfig,
    contexts: Vec<String>,
    /// Node counts for contexts that loaded at least once this session.
    context_nodes: BTreeMap<String, usize>,
    context_display: shell::ContextDisplay,
    config_error: Option<String>,
    config_loading: bool,
    configuration_read: Loader<(), Option<ConfigurationRead>>,
    epoch: u64,
    overview: Snapshot<ClusterOverview, AppliedConfig, String>,
    /// The workspace's entries and what the header's switcher shows of them.
    switch: switch::Switcher,
    /// What the notices about links and switches said, for tests: the
    /// window offers no way to read a notice back.
    #[cfg(test)]
    pub(crate) told: Vec<String>,
    /// The Talos nodes, the one selected and what is read of it.
    target: target::Selected,
    chrome: shell::Chrome,
    /// The pages: the map of every page's handle, and the typed handles
    /// the shell observes and pushes into.
    pages: page_map::Pages,
    /// Settings' handle, also held by the pages: `settings/` and the
    /// start-up code read it here.
    settings_page: Entity<settings::SettingsPage>,
    /// `workspace.json` beside the preferences; none without preferences.
    workspace_file: Option<PathBuf>,
    setup_file: Entity<crate::setup_file::SetupFile>,
    /// Kubernetes credentials source for the overview roster and screens.
    kubeconfig: KubeconfigSelection,
    kubeconfig_draft: kubeconfig::KubeconfigDraft,
    /// Where the shell is: the page, the area, the column and the rail.
    navigation: shell::Navigation,
    /// Example data or a live cluster, and Talos or a kubeconfig alone
    /// (`source::Mode`): what the window shows.
    mode: source::Mode,
    focus: FocusHandle,
    /// The window the shell draws in, for what changes the shown page
    /// without one at hand (`Pilot::sync_visible_soon`).
    window: AnyWindowHandle,
    _subscriptions: Vec<Subscription>,
    // The snapshots keep the display/freshness state and its existing request
    // tokens; these completion owners retain the jobs and their deliveries.
    overview_read: Loader<(), AppliedConfig>,
    _tick: Task<()>,
}

impl Pilot {
    /// The Workloads screen, for its tests to read its rows.
    #[cfg(test)]
    pub(crate) fn workloads(&self) -> Entity<crate::screens::WorkloadsScreen> {
        self.pages.health.clone()
    }

    /// The Applications page and the page shown, for its tests.
    #[cfg(test)]
    pub(crate) fn applications(&self) -> (Entity<crate::applications::ApplicationsPage>, Page) {
        (self.pages.applications.clone(), self.navigation.page)
    }

    /// The Observability page, for tests of the links that open it.
    #[cfg(test)]
    pub(crate) fn observability(&self) -> Entity<crate::observability::ObservabilityPage> {
        self.pages.observability.clone()
    }

    /// The object Resources shows in its drawer, for tests of the links
    /// that open one.
    #[cfg(test)]
    pub(crate) fn opened_object(&self, cx: &App) -> Option<resources::model::ResourceIdentity> {
        self.pages.resources.read(cx).detail_identity(cx).cloned()
    }

    /// The tab the Resources pane shows, for tests of the links that open it.
    #[cfg(test)]
    pub(crate) fn opened_tab(&self, cx: &App) -> resources::Tab {
        self.pages.resources.read(cx).detail_tab(cx)
    }

    /// What example sources hold from now on, as `FRESHKUBE_FIXTURE_HOLD`
    /// says at launch; every page hears it with the source pushed here. For
    /// the Applications column, Applications and Applications page tests.
    #[cfg(test)]
    pub(crate) fn set_hold(
        &mut self,
        hold: freshkube_core::cluster_source::Hold,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        *self.mode.hold_mut() = hold;
        self.push_source(window, cx);
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
        cx.bind_keys([KeyBinding::new(
            "secondary-r",
            Refresh,
            Some(freshkube_ui::page::SHELL_CONTEXT),
        )]);
        shell::Navigation::bind_keys(cx);
        cx.bind_keys([
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
        cx.bind_keys(settings::destinations::key_bindings());
        cx.bind_keys(settings::sources::key_bindings());
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

    /// What the shell observes of its own, before any page is made: itself,
    /// the operations slot and the window's close.
    fn subscribe_shell(window: &mut Window, cx: &mut Context<Self>) -> Vec<Subscription> {
        let mut subscriptions = Vec::new();
        // Overview and Nodes hear where things stand whenever the shell
        // notifies, and redraw only when that changed; the countdown tick
        // notifies only its ring.
        subscriptions.push(cx.observe(&cx.entity(), move |pilot, _, cx| {
            pilot.push_overview_state(cx);
            pilot.push_node_services_state(cx);
            pilot.push_attention_stale(cx);
        }));
        // The status bar shows the running operation; the window refuses to
        // close under it. Cached screens may read the slot too.
        let operations = Operations::global(cx);
        subscriptions.push(cx.observe(&operations, |view, _, cx| {
            view.pages.notify_cached(cx);
            cx.notify();
        }));
        window.on_window_should_close(cx, |window, cx| {
            mutation::may_close(window, cx)
                && pod_shell::may_close(window, cx, |window, _| window.remove_window())
        });
        subscriptions
    }

    /// Cached views keep their last frame; a font size or palette change
    /// that doesn't refresh the window by itself must still redraw them.
    /// Made after the pages, so Observability hears the theme first.
    fn observe_theme(window: &mut Window, cx: &mut Context<Self>) -> Subscription {
        cx.observe_global_in::<Theme>(window, |view, window, cx| {
            place_window_controls(window, cx);
            view.prepare_context_display(window, cx);
            view.pages.notify_cached(cx);
            cx.notify();
        })
    }

    /// The one-second tick: the countdown, and the automatic refresh.
    fn start_tick(window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this
                    .update_in(cx, |view, window, cx| {
                        view.chrome.elapsed += Duration::from_secs(1);
                        if view.chrome.automatic
                            && view.mode().has_talos()
                            && view.chrome.elapsed >= AUTO_REFRESH
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
        if self.mode().is_example() {
            self.contexts = fixture::CONTEXTS
                .iter()
                .map(|name| (*name).into())
                .collect();
            self.applied.context = Some(self.contexts[0].clone());
            self.seed_fixture_history(cx);
            self.refresh(window, cx);
        } else if kubernetes_only {
            let kube = kubernetes_only::KubernetesOnly::new(kubeconfig_path, kube_context);
            self.mode.set_kubernetes_only(if remembered_kubernetes {
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
        // The window opens on Overview, which has no navigation column.
        crate::screens::set_chrome_width(RAIL_WIDTH);
        // Opened before the pages, which read their splits' sizes from it.
        let navigation_file =
            crate::navigation_file::NavigationFile::open(options.preferences.as_deref());
        navigation_file.clone().install(cx);
        Self::bind_shell_keys(cx);
        let mut subscriptions = Self::subscribe_shell(window, cx);
        let age_label = cx.new(|_| switch::AgeLabel::new());
        let switch = switch::Switcher::new(age_label);
        let pages = page_map::Pages::new(&options, &runtime, &switch.age_label, window, cx);
        subscriptions.push(Self::observe_theme(window, cx));
        subscriptions.extend(pages.subscribe(window, cx));
        let tick = Self::start_tick(window, cx);
        let chrome = shell::Chrome::new(
            app_menu::menu_platform(options.debug.app_menu_button),
            options.preferences.clone(),
            window,
            cx,
        );
        let navigation = shell::Navigation::new(navigation_file, cx);
        let mut view = Self::with_session(
            &options,
            runtime,
            Owners {
                pages,
                chrome,
                navigation,
                switch,
                subscriptions,
                tick,
            },
            window,
            cx,
        );
        view.start_with_setup(options, window, cx);
        view
    }

    /// Starts the shell once it is made: reads the workspace, opens the
    /// first source, hears what needs the shell itself, takes focus and
    /// gives Overview its first frame.
    fn start(&mut self, mut options: GpuiOptions, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(any(debug_assertions, feature = "stress"))]
        let debug = options.debug.clone();
        let names_source = options.starts_with_examples()
            || options.named_source
            || options.maintenance_endpoint.is_some();
        let launched = switch::Launched {
            kubernetes_only: options.kubernetes_only,
            kubeconfig: options.kubeconfig_path.clone(),
            kube_context: options.kube_context.clone(),
            talosconfig: options.config_path.clone(),
            talos_context: options.context.clone(),
        };
        if let Some(notice) = options.setup_notice.take() {
            self.tell(notice, window, cx);
        }
        self.load_workspace(cx);
        self.rebuild_switcher(cx);
        let workspace_entry = self.launch_entry(names_source, cx);
        let started_entry = workspace_entry.is_some();
        if let Some((id, note)) = workspace_entry {
            self.activate_entry(&id, window, cx);
            if let Some(note) = note {
                gpui_kit::component::WindowExt::push_notification(window, note, cx);
            }
        } else {
            self.open_initial_source(
                options.kubernetes_only,
                options.kubeconfig_path,
                options.kube_context,
                options.remembered_kubernetes,
                window,
                cx,
            );
        }
        if !started_entry {
            self.adopt_launched_entry(&launched, cx);
        }
        self._subscriptions.push(cx.observe_in(
            &self.settings_page,
            window,
            |this, _, window, cx| {
                this.rebuild_switcher(cx);
                this.sync_setup_connection(window, cx);
                cx.notify();
            },
        ));
        self.watch_menus(window, cx);
        self.publish_reading(cx);
        self.chrome.parts.watch(self, cx);
        self.prepare_context_display(window, cx);
        // Initial shell focus makes contextual commands available without a click.
        window.focus(&self.focus, cx);
        // Without Talos, the one page that reads is the first one shown.
        if !self.mode().has_talos() {
            self.navigate_from_keyboard(Page::Overview, window, cx);
        }
        #[cfg(any(debug_assertions, feature = "stress"))]
        self.open_startup_page(&debug, window, cx);
        #[cfg(feature = "stress")]
        crate::stress::start(window, cx);
        // Overview's first frame needs no notify of the shell to know where
        // things stand.
        self.push_overview_state(cx);
        self.push_attention_stale(cx);
    }

    /// The shell around the owners `Pilot::new` made, with the session as
    /// it starts, before anything is read. Overview owns the age label it draws.
    fn with_session(
        options: &GpuiOptions,
        runtime: Handle,
        owners: Owners,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let Owners {
            pages,
            chrome,
            navigation,
            switch,
            subscriptions,
            tick,
        } = owners;
        Self {
            runtime,
            connection_store: options.preferences.as_deref().map(ConnectionStore::new),
            loaded_config_path: None,
            applied: AppliedConfig {
                path: options.config_path.clone(),
                context: options.context.clone(),
            },
            contexts: Vec::new(),
            context_nodes: BTreeMap::new(),
            context_display: shell::ContextDisplay::default(),
            config_error: None,
            config_loading: false,
            configuration_read: Loader::default(),
            epoch: 0,
            overview: Snapshot::default(),
            switch,
            target: target::Selected::default(),
            chrome,
            settings_page: pages.settings_page.clone(),
            pages,
            workspace_file: options
                .preferences
                .as_deref()
                .map(|preferences| preferences.with_file_name("workspace.json")),
            setup_file: cx.new(|cx| {
                crate::setup_file::SetupFile::new(
                    options.preferences.clone(),
                    options.starts_with_examples(),
                    cx,
                )
            }),
            kubeconfig: KubeconfigSelection::Automatic,
            #[cfg(test)]
            told: Vec::new(),
            kubeconfig_draft: Default::default(),
            navigation,
            // The one place a hold is set: example data's, from
            // `DebugOptions::hold`.
            mode: source::Mode::new(options.starts_with_examples().then_some(options.debug.hold)),
            focus: cx.focus_handle(),
            window: window.window_handle(),
            _subscriptions: subscriptions,
            overview_read: Loader::default(),
            _tick: tick,
        }
    }

    fn invalidate_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.epoch = self.epoch.wrapping_add(1);
        // The whole session goes, with its summary tasks, before anything
        // below rebuilds from it.
        self.switch.registry.reset_active();
        self.pages
            .search
            .update(cx, |search, cx| search.invalidate(cx));
        self.overview_read.resolve(self.applied.clone(), Ok(()));
        self.target.clear_nodes();
        self.pages
            .system_services
            .update(cx, |services, cx| services.set_nodes(&[], cx));
        self.pages
            .node_workspace
            .update(cx, |nodes, _| nodes.load_history.clear());
        self.overview = Snapshot::default();
        self.pages.system_services.update(cx, |services, cx| {
            services.set_nodes(&self.target.nodes, cx)
        });
        self.rebuild_joined_nodes(cx);
        self.pages.push_node_facts(cx);
        self.prepare_context_display(window, cx);
        let document = self.pages.node_workspace.read(cx).document.clone();
        document.update(cx, |pane, cx| pane.close(cx));
        self.sync_node_visibility(window, cx);
        self.pages
            .overview_page
            .update(cx, |overview, cx| overview.collapse(cx));
        self.target.cancel_object_open();
        self.target.services = Snapshot::default();
        self.push_services(cx);
        self.pages
            .logs
            .update(cx, |logs, cx| logs.set_target(None, Vec::new(), window, cx));
        self.push_source(window, cx);
        self.restore_parked_summary(window, cx);
    }

    fn load_configuration(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let request = self
            .configuration_read
            .start(Some(ConfigurationRead::Talos(self.applied.clone())));
        // Before the reset publishes the overview's state, so the old
        // error isn't shown for the new path.
        self.config_error = None;
        self.invalidate_target(window, cx);
        self.contexts.clear();
        self.context_nodes.clear();
        self.loaded_config_path = None;
        self.config_loading = true;
        let (job, receiver) = backend::load_contexts(self.runtime.clone(), self.applied.clone());
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Configuration worker stopped".into()));
            _ = this.update_in(cx, |view, window, cx| {
                if !view.configuration_read.land(&request, Ok(()), cx) {
                    return;
                }
                view.config_loading = false;
                match result {
                    Ok(catalog) => {
                        view.loaded_config_path = Some(catalog.path);
                        view.contexts = catalog.names;
                        if view.applied.context.is_none() {
                            if view.switch.entry_open.is_some() {
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
                                view.switch.pending_link = None;
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
                        view.switch.pending_link = None;
                        view.config_error = Some(error);
                        view.rebuild_joined_nodes(cx);
                        view.prepare_context_display(window, cx);
                    }
                }
                cx.notify();
            });
        });
        self.configuration_read.run(Some(job), task);
        cx.notify();
    }

    /// Applies a Talos file and context from Connection or a state's picker.
    /// A picker passes no context, so choosing a file again reloads it.
    fn apply_talos_config(
        &mut self,
        value: AppliedConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mode().is_example() || self.config_loading {
            return;
        }
        if self.mode().has_talos()
            && value.path == self.applied.path
            && let Some(context) = value.context.clone()
        {
            self.select_context(context, window, cx);
            return;
        }
        self.unless_shell(window, cx, move |this, window, cx| {
            this.leave_entry(cx);
            if !this.mode().has_talos() {
                // The default talosconfig is what's missing in this mode.
                if value.path.is_none() {
                    return;
                }
                this.leave_kubernetes_only(window, cx);
            }
            this.applied = value;
            this.load_configuration(window, cx);
        });
    }

    /// Opens the native file picker and applies its chosen file at once.
    /// Cancelling leaves everything unchanged.
    fn browse_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode().is_example() || self.config_loading {
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
                view.apply_talos_config(
                    AppliedConfig {
                        path: Some(path),
                        context: None,
                    },
                    window,
                    cx,
                );
            });
        })
        .detach();
    }

    fn target(&self) -> Option<(Target, TalosClient)> {
        let cluster = self.overview.data()?;
        let node = self.target.summary()?;
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

    /// What a Talos screen asks of the shell.
    fn screen_event(&mut self, event: &ScreenEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            ScreenEvent::Back => self.node_back(window, cx),
            // With nothing reading yet, Health's Retry reads the kubeconfig
            // and connects again too.
            ScreenEvent::RefreshSummary
                if !self.mode().has_talos()
                    && self.switch.registry.active().summary_session.is_none() =>
            {
                self.refresh(window, cx)
            }
            ScreenEvent::RefreshSummary => self.refresh_summary(window, cx),
            ScreenEvent::RetryCluster => self.refresh(window, cx),
            ScreenEvent::OpenLogs(service) => {
                self.pages.select_service(Some(service.clone()), cx);
                self.open_logs(window, cx);
            }
            ScreenEvent::SelectNode(node) => {
                self.open_node_by_name(node, nodes::NodeTab::Overview, window, cx)
            }
            ScreenEvent::OpenLogsOn { node, service } => {
                if !self.target.has_node(node) {
                    return;
                }
                self.open_node_by_name(node, nodes::NodeTab::Logs, window, cx);
                self.pages.select_service(Some(service.clone()), cx);
                self.open_logs(window, cx);
            }
        }
    }

    fn active_screen(&self, cx: &App) -> Option<ScreenHandle> {
        let page = if self.navigation.page == Page::Nodes {
            self.pages.node_workspace.read(cx).pane_screen()?
        } else {
            self.navigation.page.screen()?
        };
        self.pages.map.get(&page).cloned()
    }

    /// Hands every page the current source; the visible screen loads if
    /// empty. The screens hear first, then the entities beside the pages,
    /// then the other pages, in key order.
    fn push_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.deliver_summary_nodes(cx);
        let source = self.source();
        for (_, screen) in self.pages.map.iter().filter(|(id, _)| id.is_screen()) {
            screen.set_page_source(source.clone(), window, cx);
        }
        if let Some(data) = self.switch.registry.active().summary_health.clone() {
            self.deliver_workloads(data, cx);
        }
        self.sync_unread_health(cx);
        if let Some(screen) = self.active_screen(cx) {
            screen.activate(window, cx);
        }
        let kube = source.kube.clone();
        self.sync_search_source(kube.clone(), window, cx);
        self.pages
            .custom
            .update(cx, |custom, cx| custom.set_source(kube.clone(), cx));
        self.pages.dock.update(cx, |dock, cx| {
            dock.set_source(kube, source.kube_identity.as_deref(), window, cx)
        });
        for (_, page) in self.pages.map.iter().filter(|(id, _)| !id.is_screen()) {
            page.set_page_source(source.clone(), window, cx);
        }
    }

    /// Hands pod and node history the Monitoring page's Prometheus.
    fn push_history(&mut self, cx: &mut Context<Self>) {
        let history = self.pages.monitoring.read(cx).history();
        let node_history = self.pages.node_workspace.read(cx).history.clone();
        node_history.update(cx, |view, cx| view.set_source(history.clone(), cx));
        self.pages
            .resources
            .update(cx, |resources, cx| resources.set_history(history, cx));
        cx.notify();
    }

    /// A refresh the user asked for. Unlike the automatic one it also lists
    /// the Resources page again; its watch keeps it current otherwise.
    fn refresh_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switch.registry.active_mut().prompted_access = None;
        self.refresh_summary(window, cx);
        self.refresh(window, cx);
        // The screens refresh with the shell, above.
        let id = self.navigation.page.id();
        if !id.is_screen()
            && let Some(page) = self.pages.map.get(&id)
        {
            page.refresh(window, cx);
        }
        if self.navigation.page == Page::Nodes {
            let history = self.pages.node_workspace.read(cx).history.clone();
            history.update(cx, |history, cx| history.refresh(cx));
        }
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.mode().has_talos() {
            self.refresh_kubernetes(window, cx);
            self.ensure_summary(window, cx);
            return;
        }
        if self.config_loading || self.overview.is_loading() {
            return;
        }
        self.chrome.elapsed = Duration::ZERO;
        let request = self.overview.begin(self.applied.clone());
        self.publish_reading(cx);
        if self.mode().holds_talos() {
            // Loading until the hold is released, which only a test does.
            // Coroot's examples don't wait for Talos, so Observability has
            // its source, and its own hold, at once.
            if let Some(page) = self.pages.map.get(&PageId::Observability) {
                page.set_page_source(self.source(), window, cx);
            }
            cx.notify();
            return;
        }
        if self.mode().is_example() {
            let tick = self.mode.next_tick();
            let context = self.applied.context.clone().unwrap_or_default();
            self.overview
                .apply(&request, Ok(fixture::cluster(&context, tick)));
            self.overview_succeeded(cx);
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
            self.switch.observed_nodes(),
        );
        let completion = self.overview_read.start(self.applied.clone());
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Overview worker stopped".into()));
            _ = this.update_in(cx, |view, window, cx| {
                if epoch == view.epoch
                    && view.overview.is_loading()
                    && view.overview.is_current(&request)
                    && view.overview_read.land(&completion, Ok(()), cx)
                {
                    view.overview_received(epoch, request, result, window, cx);
                }
            });
        });
        self.overview_read.run(Some(job), task);
        cx.notify();
    }

    fn refresh_screen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.navigation.page == Page::Health {
            return;
        }
        if let Some(screen) = self.active_screen(cx) {
            screen.refresh(window, cx);
        }
    }

    /// Example data starts with five minutes of load history so the
    /// sparklines show a trend immediately.
    fn seed_fixture_history(&mut self, cx: &mut App) {
        let context = self.applied.context.clone().unwrap_or_default();
        for _ in 1..presentation::LOAD_HISTORY_LEN {
            let tick = self.mode.next_tick();
            let nodes = presentation::node_summaries(&fixture::cluster(&context, tick));
            self.pages
                .node_workspace
                .update(cx, |workspace, _| workspace.load_history.record(&nodes));
        }
    }

    /// Records per-refresh history once a snapshot for the applied target lands.
    fn overview_succeeded(&mut self, cx: &mut App) {
        let Some(cluster) = self.overview.data() else {
            return;
        };
        let nodes = presentation::node_summaries(cluster);
        self.pages
            .node_workspace
            .update(cx, |workspace, _| workspace.load_history.record(&nodes));
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
        self.chrome.set_appearance(appearance, window, cx);
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

    fn status<T, I: Clone + Eq, E>(snapshot: &Snapshot<T, I, E>) -> String {
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

    /// Whether contexts or a connection are loading, for the refresh button.
    fn loading(&self) -> bool {
        let connecting = self
            .mode()
            .kubernetes_only()
            .is_some_and(|kube| kube.connection == kubernetes_only::KubeConnection::Connecting);
        self.config_loading || self.overview.is_loading() || connecting
    }

    /// How full the countdown ring is, and whether it shows at all.
    fn countdown_state(&self) -> (f32, bool) {
        let loading = self.loading();
        (
            1. - self.chrome.elapsed.as_secs_f32() / AUTO_REFRESH.as_secs_f32(),
            self.chrome.automatic
                && self.mode().has_talos()
                && self.overview.data().is_some()
                && !loading,
        )
    }

    /// Moves the ring on without redrawing anything else.
    fn tick_countdown(&self, cx: &mut Context<Self>) {
        let (remaining, visible) = self.countdown_state();
        self.chrome.countdown.update(cx, |countdown, cx| {
            if countdown.set(remaining, visible) {
                cx.notify();
            }
        });
    }
}
