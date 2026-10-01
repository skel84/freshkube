use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use color_eyre::Result;
use dioxus::prelude::*;
use futures::StreamExt;
use talos_pilot_core::cluster_overview::{
    ClusterConnectionStatus, ClusterOverview, ClusterOverviewCollector, KubeconfigFileInfo,
    KubeconfigSelection, inspect_kubeconfig,
};
use talos_pilot_core::formatting::format_bytes;
use talos_pilot_core::types::LogLevel;
use tokio::{runtime::Handle, sync::mpsc, task::AbortHandle};

use crate::{
    DioxusOptions,
    diagnostics::DiagnosticsPanel,
    etcd_workloads::{EtcdPanel, WorkloadsPanel},
    feature::{FeatureContext, LogRequest, run_on_runtime},
    logs::LogsPanel,
    maintenance::MaintenancePanel,
    model::{BATCH_LIMIT, Identity, LogView, Target, Ticket},
    network::{NetworkPanel, ServiceRestart},
    operations::{OperationsPanel, cancel_current_mutation, operation_busy as registry_busy},
    processes_storage::{ProcessesPanel, StoragePanel},
    security_lifecycle::{LifecyclePanel, SecurityPanel},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);
const QUEUE_CAPACITY: usize = 8;
const STYLE: &str = include_str!("style.css");
const REFRESH_INTERVAL: Duration = Duration::from_secs(10);

#[cfg(test)]
thread_local! {
    static TEST_BUSY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn operation_busy() -> bool {
    #[cfg(test)]
    if TEST_BUSY.with(std::cell::Cell::get) {
        return true;
    }
    registry_busy()
}

#[derive(Clone)]
struct Bootstrap {
    options: DioxusOptions,
    runtime: Handle,
}

const CLOSE_MENU_ID: &str = "talos-pilot-safe-close";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseAction {
    CancelAndWait,
    Close,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CloseState {
    requested: bool,
    queued: bool,
}

impl CloseState {
    fn request(&mut self, busy: bool, cancel: impl FnOnce()) -> CloseAction {
        self.requested = true;
        self.queued = !busy;
        if busy {
            cancel();
            CloseAction::CancelAndWait
        } else {
            CloseAction::Close
        }
    }

    fn poll(&mut self, busy: bool) -> bool {
        if self.requested && !self.queued && !busy {
            self.queued = true;
            true
        } else {
            false
        }
    }

    fn waiting(&self) -> bool {
        self.requested && !self.queued
    }
}

#[derive(Default)]
struct NativeCloseInner {
    state: CloseState,
    desktop: Option<dioxus::desktop::WeakDesktopContext>,
}

#[derive(Clone, Default)]
struct NativeClose(Rc<RefCell<NativeCloseInner>>);

thread_local! {
    // The launch provider is Send + Sync, but its native context is not.
    // Keep only a weak registration here: the event handler/root own the state,
    // and the state itself holds only a weak reference to the desktop window.
    static NATIVE_CLOSE_CONTEXT: RefCell<std::rc::Weak<RefCell<NativeCloseInner>>> =
        const { RefCell::new(std::rc::Weak::new()) };
}

impl NativeClose {
    fn context_provider(&self) -> Box<dyn Fn() -> Box<dyn std::any::Any> + Send + Sync + 'static> {
        NATIVE_CLOSE_CONTEXT.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(
                slot.upgrade().is_none(),
                "a native-close context is already registered on this thread"
            );
            *slot = Rc::downgrade(&self.0);
        });
        let owner = std::thread::current().id();
        Box::new(move || {
            // Desktop 0.7.4 evaluates context providers on the launch thread,
            // before starting its native event loop. Do not accidentally build
            // an unrelated thread-local context if that contract ever changes.
            assert_eq!(
                std::thread::current().id(),
                owner,
                "native-close context must be provided on the launch thread"
            );
            let inner = NATIVE_CLOSE_CONTEXT
                .with(|slot| slot.borrow().upgrade())
                .expect("native-close event handler must outlive context provisioning");
            Box::new(Self(inner))
        })
    }

    fn request(&self, busy: bool) -> CloseAction {
        self.0
            .borrow_mut()
            .state
            .request(busy, cancel_current_mutation)
    }

    fn desktop(&self) -> Option<dioxus::desktop::DesktopContext> {
        self.0
            .borrow()
            .desktop
            .as_ref()
            .and_then(std::rc::Weak::upgrade)
    }

    fn request_and_queue(&self) {
        let action = self.request(operation_busy());
        if let Some(desktop) = self.desktop() {
            desktop.set_close_behavior(match action {
                CloseAction::CancelAndWait => dioxus::desktop::WindowCloseBehaviour::WindowHides,
                CloseAction::Close => dioxus::desktop::WindowCloseBehaviour::WindowCloses,
            });
            if action == CloseAction::Close {
                desktop.close();
            }
        }
    }

    fn event<T: 'static>(&self, event: &dioxus::desktop::tao::event::Event<'_, T>) {
        use dioxus::desktop::{
            WindowCloseBehaviour,
            tao::event::{Event as NativeEvent, WindowEvent},
        };
        let desktop = self.desktop();
        let native_close = matches!(event,
            NativeEvent::WindowEvent { event: WindowEvent::CloseRequested, window_id, .. }
                if desktop.as_ref().is_none_or(|desktop| desktop.id() == *window_id));
        let busy = operation_busy();
        if native_close {
            // This callback runs before Dioxus handles CloseRequested.
            // WindowHides preserves the WebView, root scopes and caller runtime;
            // the next native event restores visibility while compensation runs.
            let action = self.request(busy);
            if let Some(desktop) = desktop {
                desktop.set_close_behavior(match action {
                    CloseAction::CancelAndWait => WindowCloseBehaviour::WindowHides,
                    CloseAction::Close => WindowCloseBehaviour::WindowCloses,
                });
            } else {
                // Bootstrap has not registered the window yet. Config defaults
                // to WindowHides; the root waiter issues a guarded close later.
                self.0.borrow_mut().state.queued = false;
            }
        } else if let Some(desktop) = desktop {
            // Dioxus's UserWindowEvent type is private. Set the safe behavior
            // before EVERY event so even its private CloseWindow event cannot
            // drop the root if a mutation started after close was queued.
            if busy {
                if self.0.borrow().state.requested {
                    self.request(true);
                }
                desktop.set_close_behavior(WindowCloseBehaviour::WindowHides);
            } else {
                desktop.set_close_behavior(if self.0.borrow().state.queued {
                    WindowCloseBehaviour::WindowCloses
                } else {
                    WindowCloseBehaviour::WindowHides
                });
            }
            if self.0.borrow().state.waiting() {
                desktop.window.set_visible(true);
            }
        }
    }
}

fn guarded_menu() -> Result<dioxus::desktop::muda::Menu> {
    use dioxus::desktop::muda::{
        Menu, MenuItem, PredefinedMenuItem, Submenu,
        accelerator::{Accelerator, Code, Modifiers},
    };
    let menu = Menu::new();
    let application = Submenu::new("Talos Pilot", true);
    let quit = MenuItem::with_id(
        CLOSE_MENU_ID,
        "Quit Talos Pilot",
        true,
        Some(Accelerator::new(
            Some(if cfg!(target_os = "macos") {
                Modifiers::SUPER
            } else {
                Modifiers::CONTROL
            }),
            Code::KeyQ,
        )),
    );
    application.append_items(&[
        &PredefinedMenuItem::hide(None),
        &PredefinedMenuItem::hide_others(None),
        &PredefinedMenuItem::show_all(None),
        &PredefinedMenuItem::separator(),
        &quit,
    ])?;
    let edit = Submenu::new("Edit", true);
    edit.append_items(&[
        &PredefinedMenuItem::undo(None),
        &PredefinedMenuItem::redo(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::cut(None),
        &PredefinedMenuItem::copy(None),
        &PredefinedMenuItem::paste(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::select_all(None),
    ])?;
    let window = Submenu::new("Window", true);
    window.append_items(&[
        &PredefinedMenuItem::fullscreen(None),
        &PredefinedMenuItem::maximize(None),
        &PredefinedMenuItem::minimize(None),
        &PredefinedMenuItem::close_window(None),
    ])?;
    menu.append_items(&[&application, &edit, &window])?;
    Ok(menu)
}

pub(crate) fn run(options: DioxusOptions, runtime: Handle) -> Result<()> {
    let window = dioxus::desktop::WindowBuilder::new()
        .with_title("Talos Pilot · Experimental Desktop")
        .with_inner_size(dioxus::desktop::tao::dpi::LogicalSize::new(1280.0, 840.0));
    let close = NativeClose::default();
    let close_context = close.context_provider();
    let events = close.clone();
    let config = dioxus::desktop::Config::new()
        .with_window(window)
        .with_background_color((12, 17, 27, 255))
        .with_close_behaviour(dioxus::desktop::WindowCloseBehaviour::WindowHides)
        .with_menu(guarded_menu()?)
        .with_custom_event_handler(move |event, _| events.event(event));
    dioxus::LaunchBuilder::new()
        .with_context(Bootstrap { options, runtime })
        .with_context_provider(close_context)
        .with_cfg(config)
        .launch(app);
    Ok(())
}

/// Abort-on-drop also applies when Dioxus unmounts the root signal.
#[derive(Default)]
struct Worker(Option<AbortHandle>);

impl Worker {
    fn cancel(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
    fn replace(&mut self, task: AbortHandle) {
        self.cancel();
        self.0 = Some(task);
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
    }
}

enum Event {
    Configured(u64, std::result::Result<Vec<ClusterOverview>, String>),
    InitialSnapshot(u64, String, std::result::Result<ClusterOverview, String>),
    LoadFinished(u64),
    Snapshot(Ticket, std::result::Result<ClusterOverview, String>),
    StreamStarted(Ticket),
    Lines(Ticket, Vec<String>, usize),
    StreamEnded(Ticket, String),
    Picked(u64, Option<PathBuf>),
    KubeconfigPicked(u64, Option<PathBuf>),
    KubeconfigInspected(u64, std::result::Result<KubeconfigFileInfo, String>),
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum KubeconfigMode {
    #[default]
    Automatic,
    TalosControlPlane,
    File,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum NodeTab {
    #[default]
    Overview,
    Services,
    Logs,
    Processes,
    Storage,
    Network,
    Etcd,
    Workloads,
    Diagnostics,
    Security,
    Lifecycle,
    Operations,
}

impl NodeTab {
    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Services => "Services",
            Self::Logs => "Logs",
            Self::Processes => "Processes",
            Self::Storage => "Storage",
            Self::Network => "Network",
            Self::Etcd => "etcd",
            Self::Workloads => "Workloads",
            Self::Diagnostics => "Diagnostics",
            Self::Security => "Security",
            Self::Lifecycle => "Lifecycle",
            Self::Operations => "Operations",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Overview => "node-overview",
            Self::Services => "node-services",
            Self::Logs => "node-logs",
            Self::Processes => "node-processes",
            Self::Storage => "node-storage",
            Self::Network => "node-network",
            Self::Etcd => "node-etcd",
            Self::Workloads => "node-workloads",
            Self::Diagnostics => "node-diagnostics",
            Self::Security => "node-security",
            Self::Lifecycle => "node-lifecycle",
            Self::Operations => "node-operations",
        }
    }

    fn all() -> [Self; 12] {
        [
            Self::Overview,
            Self::Services,
            Self::Logs,
            Self::Processes,
            Self::Storage,
            Self::Network,
            Self::Etcd,
            Self::Workloads,
            Self::Diagnostics,
            Self::Security,
            Self::Lifecycle,
            Self::Operations,
        ]
    }
}

fn kubeconfig_selection_label(selection: &KubeconfigSelection) -> String {
    match selection {
        KubeconfigSelection::Automatic => "Automatic".into(),
        KubeconfigSelection::TalosControlPlane => "Talos control plane".into(),
        KubeconfigSelection::File { path, context } => format!(
            "Selected kubeconfig: {} · {}",
            path.display(),
            context.as_deref().unwrap_or("file current-context")
        ),
    }
}

struct Controller {
    runtime: Handle,
    options: DioxusOptions,
    path: String,
    exact_path: Option<PathBuf>,
    applied_config_path: Option<PathBuf>,
    collector: ClusterOverviewCollector,
    identity: Identity,
    clusters: Vec<ClusterOverview>,
    errors: std::collections::HashMap<String, String>,
    attempts: std::collections::HashMap<String, Instant>,
    successes: std::collections::HashMap<String, Instant>,
    revisions: std::collections::HashMap<String, u64>,
    auto_refresh: bool,
    next_refresh: Instant,
    requested_logs: Vec<String>,
    navigation_error: Option<String>,
    loading: bool,
    refreshing: bool,
    config_error: Option<String>,
    node_tab: NodeTab,
    service: String,
    logs: Option<LogView>,
    stream_status: String,
    streaming: bool,
    picking: bool,
    picker_generation: u64,
    load_worker: Worker,
    snapshot_worker: Worker,
    stream_worker: Worker,
    picker_worker: Worker,
    kubeconfig_mode: KubeconfigMode,
    kubeconfig_path: String,
    kubeconfig_exact_path: Option<PathBuf>,
    kubeconfig_context: Option<String>,
    kubeconfig_info: Option<KubeconfigFileInfo>,
    kubeconfig_error: Option<String>,
    kubeconfig_applied: KubeconfigSelection,
    kubeconfig_generation: u64,
    kubeconfig_picking: bool,
    kubeconfig_parsing: bool,
    kubeconfig_picker_worker: Worker,
    kubeconfig_parser_worker: Worker,
    tx: mpsc::Sender<Event>,
    rx: Option<mpsc::Receiver<Event>>,
}

impl Controller {
    fn new(bootstrap: Bootstrap) -> Self {
        let (tx, rx) = mpsc::channel(QUEUE_CAPACITY);
        let path = bootstrap
            .options
            .config_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut controller = Self {
            runtime: bootstrap.runtime,
            exact_path: bootstrap.options.config_path.clone(),
            applied_config_path: None,
            collector: ClusterOverviewCollector::new(
                bootstrap.options.config_path.clone(),
                bootstrap.options.context.clone(),
            ),
            options: bootstrap.options,
            path,
            identity: Identity::default(),
            clusters: Vec::new(),
            errors: Default::default(),
            attempts: Default::default(),
            successes: Default::default(),
            revisions: Default::default(),
            auto_refresh: true,
            next_refresh: Instant::now() + REFRESH_INTERVAL,
            requested_logs: Vec::new(),
            navigation_error: None,
            loading: false,
            refreshing: false,
            config_error: None,
            node_tab: NodeTab::default(),
            service: String::new(),
            logs: None,
            stream_status: "Choose a node and service to start live logs.".into(),
            streaming: false,
            picking: false,
            picker_generation: 0,
            load_worker: Worker::default(),
            snapshot_worker: Worker::default(),
            stream_worker: Worker::default(),
            picker_worker: Worker::default(),
            kubeconfig_mode: KubeconfigMode::Automatic,
            kubeconfig_path: String::new(),
            kubeconfig_exact_path: None,
            kubeconfig_context: None,
            kubeconfig_info: None,
            kubeconfig_error: None,
            kubeconfig_applied: KubeconfigSelection::Automatic,
            kubeconfig_generation: 0,
            kubeconfig_picking: false,
            kubeconfig_parsing: false,
            kubeconfig_picker_worker: Worker::default(),
            kubeconfig_parser_worker: Worker::default(),
            tx,
            rx: Some(rx),
        };
        controller.load();
        controller
    }

    fn load(&mut self) {
        if operation_busy() {
            return;
        }
        self.load_worker.cancel();
        self.snapshot_worker.cancel();
        self.stream_worker.cancel();
        self.picker_worker.cancel();
        self.picker_generation += 1;
        self.picking = false;
        self.cancel_kubeconfig_work();
        if self.kubeconfig_mode == KubeconfigMode::File {
            self.inspect_kubeconfig_draft();
        }
        let epoch = self.identity.reload();
        self.node_tab = NodeTab::default();
        crate::logs::retain_log_target(None);
        self.clusters.clear();
        self.errors.clear();
        self.attempts.clear();
        self.successes.clear();
        self.revisions.clear();
        self.requested_logs.clear();
        self.navigation_error = None;
        self.next_refresh = Instant::now() + REFRESH_INTERVAL;
        self.config_error = None;
        self.loading = true;
        self.refreshing = false;
        self.service.clear();
        self.logs = None;
        self.streaming = false;
        self.stream_status = "Choose a node and service to start live logs.".into();
        let path = self
            .exact_path
            .clone()
            .or_else(|| (!self.path.trim().is_empty()).then(|| PathBuf::from(self.path.trim())))
            .or_else(|| talos_rs::TalosConfig::default_path().ok());
        self.applied_config_path = path.clone();
        self.collector = ClusterOverviewCollector::new(path, self.options.context.clone());
        self.collector
            .set_kubeconfig_selection(self.kubeconfig_applied.clone());
        let collector = self.collector.clone();
        let tx = self.tx.clone();
        let task = self.runtime.spawn(async move {
            // Connections are independently bounded by the shared client. Do not
            // discard every configured context because their sequential total
            // exceeds the timeout used for an individual snapshot or log request.
            let configured = match collector.connect().await {
                Ok(clusters) => clusters,
                Err(error) => {
                    let _ = tx
                        .send(Event::Configured(epoch, Err(error.to_string())))
                        .await;
                    return;
                }
            };
            if tx
                .send(Event::Configured(epoch, Ok(configured.clone())))
                .await
                .is_err()
            {
                return;
            }
            // Independent contexts collect concurrently; no native I/O runs on the UI thread.
            let jobs = configured.into_iter().map(|cluster| {
                let collector = collector.clone();
                let tx = tx.clone();
                async move {
                    let name = cluster.name.clone();
                    let result = collect_snapshot(collector, cluster).await;
                    let _ = tx.send(Event::InitialSnapshot(epoch, name, result)).await;
                }
            });
            futures::future::join_all(jobs).await;
            let _ = tx.send(Event::LoadFinished(epoch)).await;
        });
        self.load_worker.replace(task.abort_handle());
    }

    fn edit_path(&mut self, path: String) {
        if operation_busy() {
            return;
        }
        self.path = path;
        self.exact_path = None;
        self.picker_generation += 1;
        self.picker_worker.cancel();
        self.picking = false;
    }

    fn pick(&mut self) {
        if operation_busy() {
            return;
        }
        self.picker_generation += 1;
        let generation = self.picker_generation;
        self.picking = true;
        let tx = self.tx.clone();
        let task = self.runtime.spawn(async move {
            let selected = rfd::AsyncFileDialog::new()
                .set_title("Select talosconfig")
                .pick_file()
                .await;
            let _ = tx
                .send(Event::Picked(
                    generation,
                    selected.map(|file| file.path().to_owned()),
                ))
                .await;
        });
        self.picker_worker.replace(task.abort_handle());
    }

    fn cancel_kubeconfig_work(&mut self) {
        self.kubeconfig_generation += 1;
        self.kubeconfig_picker_worker.cancel();
        self.kubeconfig_parser_worker.cancel();
        self.kubeconfig_picking = false;
        self.kubeconfig_parsing = false;
    }

    fn set_kubeconfig_mode(&mut self, mode: KubeconfigMode) {
        if operation_busy() {
            return;
        }
        self.cancel_kubeconfig_work();
        self.kubeconfig_mode = mode;
        if mode == KubeconfigMode::File {
            self.inspect_kubeconfig_draft();
        }
    }

    fn edit_kubeconfig_path(&mut self, path: String) {
        if operation_busy() {
            return;
        }
        self.cancel_kubeconfig_work();
        self.kubeconfig_path = path;
        self.kubeconfig_exact_path = None;
        self.kubeconfig_context = None;
        self.inspect_kubeconfig_draft();
    }

    fn kubeconfig_file_path(&self) -> Option<PathBuf> {
        self.kubeconfig_exact_path.clone().or_else(|| {
            (!self.kubeconfig_path.trim().is_empty()).then(|| PathBuf::from(&self.kubeconfig_path))
        })
    }

    fn inspect_kubeconfig_draft(&mut self) {
        self.kubeconfig_info = None;
        self.kubeconfig_error = None;
        let Some(path) = self.kubeconfig_file_path() else {
            self.kubeconfig_error = Some("Select or enter a kubeconfig file path.".into());
            return;
        };
        self.kubeconfig_parsing = true;
        let generation = self.kubeconfig_generation;
        let tx = self.tx.clone();
        let task = self.runtime.spawn(async move {
            // Parsing is local only; no client, auth plugin, or environment mutation.
            let result = tokio::task::spawn_blocking(move || {
                inspect_kubeconfig(&path).map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|error| Err(format!("Could not inspect kubeconfig: {error}")));
            let _ = tx
                .send(Event::KubeconfigInspected(generation, result))
                .await;
        });
        self.kubeconfig_parser_worker.replace(task.abort_handle());
    }

    fn pick_kubeconfig(&mut self) {
        if operation_busy() {
            return;
        }
        self.cancel_kubeconfig_work();
        self.kubeconfig_picking = true;
        let generation = self.kubeconfig_generation;
        let tx = self.tx.clone();
        let task = self.runtime.spawn(async move {
            let selected = rfd::AsyncFileDialog::new()
                .set_title("Select kubeconfig")
                .pick_file()
                .await;
            let _ = tx
                .send(Event::KubeconfigPicked(
                    generation,
                    selected.map(|file| file.path().to_owned()),
                ))
                .await;
        });
        self.kubeconfig_picker_worker.replace(task.abort_handle());
    }

    fn kubeconfig_draft_error(&self) -> Option<String> {
        if self.kubeconfig_mode != KubeconfigMode::File {
            return None;
        }
        if self.kubeconfig_picking || self.kubeconfig_parsing {
            return Some("Waiting for kubeconfig file inspection.".into());
        }
        if let Some(error) = &self.kubeconfig_error {
            return Some(error.clone());
        }
        let info = self.kubeconfig_info.as_ref()?;
        let context = self
            .kubeconfig_context
            .as_ref()
            .or(info.current_context.as_ref());
        match context {
            None => Some("This file has no current-context. Choose an explicit context.".into()),
            Some(context) if !info.contexts.contains(context) => Some(format!(
                "Context {context:?} is not in this file. Choose a valid context."
            )),
            Some(_) => None,
        }
    }

    fn kubeconfig_draft(&self) -> Option<KubeconfigSelection> {
        match self.kubeconfig_mode {
            KubeconfigMode::Automatic => Some(KubeconfigSelection::Automatic),
            KubeconfigMode::TalosControlPlane => Some(KubeconfigSelection::TalosControlPlane),
            KubeconfigMode::File => {
                if self.kubeconfig_draft_error().is_some() || self.kubeconfig_info.is_none() {
                    return None;
                }
                Some(KubeconfigSelection::File {
                    path: self.kubeconfig_file_path()?,
                    context: self.kubeconfig_context.clone(),
                })
            }
        }
    }

    fn kubeconfig_has_pending_edits(&self) -> bool {
        let requested = match self.kubeconfig_mode {
            KubeconfigMode::Automatic => Some(KubeconfigSelection::Automatic),
            KubeconfigMode::TalosControlPlane => Some(KubeconfigSelection::TalosControlPlane),
            KubeconfigMode::File => {
                self.kubeconfig_file_path()
                    .map(|path| KubeconfigSelection::File {
                        path,
                        context: self.kubeconfig_context.clone(),
                    })
            }
        };
        requested.as_ref() != Some(&self.kubeconfig_applied)
    }

    fn apply_kubeconfig(&mut self) {
        if operation_busy() {
            return;
        }
        if let Some(selection) = self.kubeconfig_draft() {
            self.kubeconfig_applied = selection;
            self.collector
                .set_kubeconfig_selection(self.kubeconfig_applied.clone());
            self.load();
        }
    }

    fn reset_kubeconfig(&mut self) {
        if operation_busy() {
            return;
        }
        self.set_kubeconfig_mode(KubeconfigMode::Automatic);
        self.apply_kubeconfig();
    }

    fn select(&mut self, target: Target) {
        if operation_busy() {
            return;
        }
        if self.identity.select(Some(target)) {
            self.node_tab = NodeTab::default();
            self.snapshot_worker.cancel();
            self.stream_worker.cancel();
            self.refreshing = false;
            self.streaming = false;
            self.service.clear();
            self.requested_logs.clear();
            self.navigation_error = None;
            self.next_refresh = Instant::now() + REFRESH_INTERVAL;
            self.logs = None;
            self.stream_status = "Choose a service and start a live stream.".into();
            self.ensure_service();
            let log_key = self.feature_context().map(|ctx| ctx.key());
            crate::logs::retain_log_target(log_key.as_deref());
        }
    }

    fn selected(&self) -> Option<&ClusterOverview> {
        let target = self.identity.target.as_ref()?;
        self.clusters
            .iter()
            .find(|cluster| cluster.name == target.context)
    }

    fn services(&self) -> Vec<talos_rs::ServiceInfo> {
        let Some(target) = self.identity.target.as_ref() else {
            return Vec::new();
        };
        let Some(node) = target.node.as_ref() else {
            return Vec::new();
        };
        self.selected()
            .into_iter()
            .flat_map(|cluster| &cluster.services)
            .filter(|values| &values.node == node)
            .flat_map(|values| values.services.clone())
            .collect()
    }

    fn ensure_service(&mut self) {
        let services = self.services();
        if services.iter().any(|service| service.id == self.service) {
            return;
        }
        if self.streaming {
            self.stop();
        }
        self.service = services
            .first()
            .map(|service| service.id.clone())
            .unwrap_or_default();
        self.logs = None;
    }

    fn change_service(&mut self, service: String) {
        if self.service != service {
            self.stop();
            self.service = service;
            self.logs = None;
            self.stream_status = "Service changed. Start to open its stream.".into();
        }
    }

    fn select_node_tab(&mut self, tab: NodeTab) {
        if operation_busy() {
            return;
        }
        self.node_tab = tab;
    }

    fn open_service_logs(&mut self, service: String) {
        self.navigate_logs(LogRequest {
            node: None,
            address: None,
            services: vec![service],
        });
    }

    fn navigate_logs(&mut self, request: LogRequest) -> bool {
        if operation_busy() {
            return false;
        }
        let Some(cluster) = self.selected() else {
            return false;
        };
        let context = cluster.name.clone();
        let target = match (&request.node, &request.address) {
            (None, None) => self.identity.target.clone().filter(|target| {
                target
                    .node
                    .as_ref()
                    .zip(target.address.as_ref())
                    .is_some_and(|(name, address)| cluster.node_ips.get(name) == Some(address))
            }),
            (Some(name), Some(address)) if cluster.node_ips.get(name) == Some(address) => {
                Some(Target {
                    context,
                    node: Some(name.clone()),
                    address: Some(address.clone()),
                })
            }
            _ => None,
        };
        let Some(target) = target else {
            self.navigation_error =
                Some("Log target is not in the selected context's current node roster.".into());
            return false;
        };
        self.select(target);
        self.requested_logs = request
            .services
            .into_iter()
            .filter(|service| !service.trim().is_empty())
            .take(32)
            .collect();
        self.requested_logs.sort();
        self.requested_logs.dedup();
        if let Some(service) = self.requested_logs.first().cloned() {
            self.change_service(service);
        }
        self.navigation_error = None;
        self.node_tab = NodeTab::Logs;
        true
    }

    fn feature_context(&self) -> Option<FeatureContext> {
        let target = self.identity.target.as_ref()?;
        let cluster = self.selected()?;
        let node = target.node.as_ref()?;
        let address = target.address.as_ref()?;
        if cluster.node_ips.get(node) != Some(address) {
            return None;
        }
        Some(FeatureContext {
            epoch: self.identity.config,
            revision: self
                .revisions
                .get(&cluster.name)
                .copied()
                .unwrap_or_default(),
            context: cluster.name.clone(),
            node: node.clone(),
            address: address.clone(),
            config_path: self.applied_config_path.clone(),
            client: cluster.client.as_ref()?.with_node(address),
            collector: self.collector.clone(),
            cluster: cluster.clone(),
            runtime: self.runtime.clone(),
        })
    }

    fn auto_refresh_due(&self, now: Instant) -> bool {
        self.auto_refresh
            && now >= self.next_refresh
            && !self.loading
            && !self.refreshing
            && !operation_busy()
            && self.selected().is_some()
    }

    fn tick(&mut self, now: Instant) {
        if self.auto_refresh_due(now) {
            self.refresh();
        }
    }

    fn refresh(&mut self) {
        if operation_busy() {
            return;
        }
        if self.loading || self.refreshing {
            return;
        }
        let Some(cluster) = self.selected().cloned() else {
            return;
        };
        self.refreshing = true;
        self.next_refresh = Instant::now() + REFRESH_INTERVAL;
        let ticket = self.identity.snapshot();
        let collector = self.collector.clone();
        let tx = self.tx.clone();
        let task = self.runtime.spawn(async move {
            let result = collect_snapshot(collector, cluster).await;
            let _ = tx.send(Event::Snapshot(ticket, result)).await;
        });
        self.snapshot_worker.replace(task.abort_handle());
    }

    fn stop(&mut self) {
        self.identity.stop_stream();
        self.stream_worker.cancel();
        self.streaming = false;
        self.stream_status = "Stopped · retained lines are historical, not live.".into();
    }

    #[allow(dead_code)] // Legacy controller API retained for compatibility tests.
    fn start(&mut self) {
        let Some(target) = self.identity.target.clone() else {
            return;
        };
        let Some(address) = target.address else {
            return;
        };
        let Some(client) = self.selected().and_then(|cluster| cluster.client.clone()) else {
            return;
        };
        if self.service.is_empty() {
            return;
        }
        self.stream_worker.cancel();
        let service = self.service.clone();
        let ticket = self.identity.start_stream(service.clone());
        self.logs = Some(LogView::new(address.clone(), service.clone()));
        self.streaming = true;
        self.stream_status = "Connecting · waiting for the service log stream…".into();
        let client = client.with_node(&address);
        let tx = self.tx.clone();
        let tail = self.options.tail.clamp(0, 10_000);
        let task = self.runtime.spawn(async move {
            let stream = match tokio::time::timeout(REQUEST_TIMEOUT, client.logs_follow(&service, tail)).await {
                Ok(Ok(stream)) => stream,
                result => {
                    let error = match result {
                        Ok(Err(error)) => error.to_string(),
                        Err(_) => "Opening the log stream timed out.".into(),
                        Ok(Ok(_)) => unreachable!(),
                    };
                    let _ = tx.send(Event::StreamEnded(ticket, error)).await;
                    return;
                }
            };
            if tx.send(Event::StreamStarted(ticket.clone())).await.is_err() { return; }
            futures::pin_mut!(stream);
            let mut pending = Vec::with_capacity(BATCH_LIMIT);
            let mut dropped = 0;
            let mut timer = tokio::time::interval(Duration::from_millis(100));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let end = loop {
                tokio::select! {
                    _ = timer.tick() => {
                        if !pending.is_empty() || dropped > 0 {
                            let event = Event::Lines(ticket.clone(), std::mem::take(&mut pending), dropped);
                            match tx.try_send(event) {
                                Ok(()) => dropped = 0,
                                Err(mpsc::error::TrySendError::Full(Event::Lines(_, lines, _))) => dropped += lines.len(),
                                Err(mpsc::error::TrySendError::Closed(_)) => return,
                                Err(_) => unreachable!(),
                            }
                        }
                    }
                    next = stream.next() => {
                        match next {
                            Some(Ok(line)) => {
                                if pending.len() < BATCH_LIMIT { pending.push(line); }
                                else { dropped += 1; }
                            }
                            Some(Err(error)) => break format!("Stream failed: {error}. Reconnect to retry."),
                            None => break "Stream ended · reconnect to resume.".into(),
                        }
                    }
                }
            };
            // Final control events use backpressure, so failure/end cannot be silently lost.
            if (!pending.is_empty() || dropped > 0)
                && tx.send(Event::Lines(ticket.clone(), pending, dropped)).await.is_err()
            {
                return;
            }
            let _ = tx.send(Event::StreamEnded(ticket, end)).await;
        });
        self.stream_worker.replace(task.abort_handle());
    }

    fn apply_snapshot(
        &mut self,
        name: String,
        result: std::result::Result<ClusterOverview, String>,
    ) {
        // Do not rebind or unmount a compensating mutation's target.
        if operation_busy() {
            return;
        }
        self.attempts.insert(name.clone(), Instant::now());
        match result {
            Ok(snapshot) => {
                if snapshot.connection.is_connected() {
                    self.successes.insert(name.clone(), Instant::now());
                }
                *self.revisions.entry(name.clone()).or_default() += 1;
                self.errors.remove(&name);
                if let Some(cluster) = self
                    .clusters
                    .iter_mut()
                    .find(|cluster| cluster.name == name)
                {
                    *cluster = snapshot;
                }
                if let Some(target) = self
                    .identity
                    .target
                    .clone()
                    .filter(|target| target.context == name)
                    && let Some(node) = &target.node
                    && !operation_busy()
                {
                    let address = self
                        .selected()
                        .and_then(|cluster| cluster.node_ips.get(node))
                        .cloned();
                    let rebound = match address {
                        Some(address) => Target {
                            context: target.context,
                            node: target.node,
                            address: Some(address),
                        },
                        None => Target {
                            context: target.context,
                            node: None,
                            address: None,
                        },
                    };
                    // Same name does not imply the same management endpoint.
                    // select invalidates generations and aborts old workers.
                    self.select(rebound);
                }
                self.ensure_service();
            }
            Err(error) => {
                self.errors.insert(name, error);
            }
        }
    }

    fn apply(&mut self, event: Event) -> bool {
        match event {
            Event::Configured(epoch, result) if epoch == self.identity.config => match result {
                Ok(mut clusters) => {
                    clusters.sort_by(|a, b| a.name.cmp(&b.name));
                    self.clusters = clusters;
                    if let Some(first) = self.clusters.first() {
                        self.identity.select(Some(Target {
                            context: first.name.clone(),
                            node: None,
                            address: None,
                        }));
                    }
                }
                Err(error) => {
                    self.config_error = Some(error);
                    self.loading = false;
                }
            },
            Event::InitialSnapshot(epoch, name, result) if epoch == self.identity.config => {
                self.apply_snapshot(name, result)
            }
            Event::LoadFinished(epoch) if epoch == self.identity.config => {
                self.loading = false;
            }
            Event::Snapshot(ticket, result) if self.identity.accepts_snapshot(&ticket) => {
                self.refreshing = false;
                if let Some(target) = ticket.target {
                    self.apply_snapshot(target.context, result);
                }
            }
            Event::StreamStarted(ticket) if self.identity.accepts_stream(&ticket) => {
                self.stream_status = "Live · newest matching lines first".into();
            }
            Event::Lines(ticket, lines, dropped) if self.identity.accepts_stream(&ticket) => {
                if let Some(logs) = &mut self.logs {
                    logs.append(lines, dropped);
                }
            }
            Event::StreamEnded(ticket, error) if self.identity.accepts_stream(&ticket) => {
                self.streaming = false;
                self.stream_status = error;
            }
            Event::Picked(generation, path) if generation == self.picker_generation => {
                self.picking = false;
                if operation_busy() {
                    self.navigation_error = Some("Talosconfig selection discarded while a mutation is in progress. Browse again after it completes.".into());
                } else if let Some(path) = path {
                    self.path = path.to_string_lossy().into_owned();
                    self.exact_path = Some(path);
                }
            }
            Event::KubeconfigPicked(generation, path)
                if generation == self.kubeconfig_generation =>
            {
                self.kubeconfig_picking = false;
                if operation_busy() {
                    self.kubeconfig_error = Some("File selection discarded while a mutation is in progress. Browse again after it completes.".into());
                } else if let Some(path) = path {
                    self.cancel_kubeconfig_work();
                    self.kubeconfig_path = path.to_string_lossy().into_owned();
                    self.kubeconfig_exact_path = Some(path);
                    self.kubeconfig_context = None;
                    self.inspect_kubeconfig_draft();
                } else if self.kubeconfig_mode == KubeconfigMode::File {
                    self.inspect_kubeconfig_draft();
                }
            }
            Event::KubeconfigInspected(generation, result)
                if generation == self.kubeconfig_generation =>
            {
                self.kubeconfig_parsing = false;
                match result {
                    Ok(info) => {
                        self.kubeconfig_info = Some(info);
                        self.kubeconfig_error = None;
                    }
                    Err(error) => {
                        self.kubeconfig_info = None;
                        self.kubeconfig_error = Some(error);
                    }
                }
            }
            _ => return false,
        }
        true
    }
}

/// Start with an empty data snapshot, not retained fields: missing API data is unknown.
async fn collect_snapshot(
    collector: ClusterOverviewCollector,
    cluster: ClusterOverview,
) -> std::result::Result<ClusterOverview, String> {
    if cluster.client.is_none() {
        return Ok(cluster);
    }
    let mut fresh = ClusterOverview {
        name: cluster.name,
        endpoints: cluster.endpoints,
        client: cluster.client,
        ..Default::default()
    };
    tokio::time::timeout(REQUEST_TIMEOUT, collector.refresh(&mut fresh)).await
        .map_err(|_| "Snapshot timed out. The displayed snapshot is retained and may be stale; refresh or reload to retry.".to_owned())?;
    Ok(fresh)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CloseDisplay {
    state: CloseState,
    busy: bool,
}

#[component]
fn NativeClosePanel() -> Element {
    let Some(close) = try_consume_context::<NativeClose>() else {
        return rsx! {};
    };
    let bootstrap = use_context::<Bootstrap>();
    let window = try_consume_context::<dioxus::desktop::DesktopContext>();
    let has_window = window.is_some();
    let registration = close.clone();
    use_hook(move || {
        if let Some(window) = window {
            registration.0.borrow_mut().desktop = Some(Rc::downgrade(&window));
        }
    });
    if has_window {
        let menu_close = close.clone();
        dioxus::desktop::use_muda_event_handler(move |event| {
            if event.id().0 == CLOSE_MENU_ID {
                menu_close.request_and_queue();
            }
        });
    }
    let initial = close.clone();
    let mut display = use_signal(move || CloseDisplay {
        state: initial.0.borrow().state,
        busy: operation_busy(),
    });
    let poll_close = close.clone();
    use_future(move || {
        let close = poll_close.clone();
        let runtime = bootstrap.runtime.clone();
        async move {
            loop {
                let busy = operation_busy();
                let should_close = close.0.borrow_mut().state.poll(busy);
                let next = CloseDisplay {
                    state: close.0.borrow().state,
                    busy,
                };
                if *display.peek() != next {
                    display.set(next);
                }
                if should_close {
                    if let Some(window) = close.desktop() {
                        // CloseWindow is rechecked synchronously by the config
                        // event handler before Dioxus can drop the root scope.
                        window.set_close_behavior(
                            dioxus::desktop::WindowCloseBehaviour::WindowCloses,
                        );
                        window.close();
                    }
                } else if next.state.waiting()
                    && let Some(window) = close.desktop()
                {
                    window.window.set_visible(true);
                }
                let _ = run_on_runtime(&runtime, async {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                })
                .await;
            }
        }
    });
    let current = *display.read();
    if !current.busy && !current.state.requested {
        return rsx! {};
    }
    rsx! {
        aside { class: "banner feature-status warning", role: "status", aria_live: "polite",
            if current.state.waiting() {
                strong { "Close requested — waiting for the mutation to finish safely." }
                p { "Cooperative cancellation has been requested. An accepted RPC, compensating uncordon and audit must finish before this window can close. Do not force-quit the application." }
            } else if current.busy {
                strong { "A node mutation is in progress." }
                p { "Native close and Quit requests cancel before the next safe step and wait; they do not interrupt an accepted RPC." }
            } else {
                p { "The mutation has finished. Closing the window…" }
            }
            if current.busy {
                button { onclick: move |_| {
                    close.request_and_queue();
                    let next = CloseDisplay { state: close.0.borrow().state, busy: operation_busy() };
                    display.set(next);
                }, "Cancel mutation and close when safe" }
            }
        }
    }
}

fn app() -> Element {
    let bootstrap = use_context::<Bootstrap>();
    if bootstrap.options.maintenance_endpoint.is_some() {
        return rsx! {
            style { "{STYLE}" }
            div { class: "shell",
                header { class: "topbar", div { class: "brand", strong { "Talos Pilot" } } span { class: "experiment", "MAINTENANCE · INSECURE" } }
                NativeClosePanel {}
                main { class: "content maintenance-content",
                    MaintenancePanel { options: bootstrap.options, runtime: bootstrap.runtime }
                }
            }
        };
    }
    let mut state = use_signal(|| Controller::new(bootstrap));
    #[cfg(test)]
    use_context_provider(|| state);
    use_future(move || {
        let mut rx = state
            .write()
            .rx
            .take()
            .expect("event receiver is owned by one root future");
        async move {
            while let Some(event) = rx.recv().await {
                // One signal write per bounded batch of events, never a write per line.
                let mut controller = state.write();
                controller.apply(event);
                for _ in 0..QUEUE_CAPACITY - 1 {
                    match rx.try_recv() {
                        Ok(event) => {
                            controller.apply(event);
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    });
    use_future(move || async move {
        loop {
            // No Tokio reactor is required on the WebView thread.
            let runtime = state.read().runtime.clone();
            let _ = run_on_runtime(&runtime, async {
                tokio::time::sleep(Duration::from_secs(1)).await
            })
            .await;
            state.write().tick(Instant::now());
        }
    });
    rsx! {
        style { "{STYLE}" }
        div { class: "shell",
            header { class: "topbar",
                div { class: "brand", span { class: "brand-mark", "T" } div { strong { "Talos Pilot" } small { "CLUSTER OBSERVATORY" } } }
                span { class: "experiment", "EXPERIMENTAL DESKTOP" }
            }
            NativeClosePanel {}
            Settings { state }
            div { class: "workspace",
                Sidebar { state }
                main { class: "content", Dashboard { state } }
            }
        }
    }
}

#[component]
fn Settings(mut state: Signal<Controller>) -> Element {
    let controller = state.read();
    let path = controller.path.clone();
    let picking = controller.picking;
    let loading = controller.loading;
    let busy = operation_busy();
    let error = controller.config_error.clone();
    let filter = controller.options.context.clone();
    let mode = controller.kubeconfig_mode;
    let mode_value = match mode {
        KubeconfigMode::Automatic => "automatic",
        KubeconfigMode::TalosControlPlane => "talos",
        KubeconfigMode::File => "file",
    };
    let kubeconfig_path = controller.kubeconfig_path.clone();
    let kubeconfig_picking = controller.kubeconfig_picking;
    let kubeconfig_parsing = controller.kubeconfig_parsing;
    let kubeconfig_error = controller.kubeconfig_draft_error();
    let contexts = controller
        .kubeconfig_info
        .as_ref()
        .map(|info| info.contexts.clone())
        .unwrap_or_default();
    let current_context = controller
        .kubeconfig_info
        .as_ref()
        .and_then(|info| info.current_context.as_ref())
        .map(|context| format!("File current-context ({context})"))
        .unwrap_or_else(|| "File current-context (missing)".into());
    // Prefix named choices so an actual empty context name cannot alias the default.
    let context_value = controller
        .kubeconfig_context
        .as_ref()
        .map(|context| format!("named:{context}"))
        .unwrap_or_default();
    let can_apply = controller.kubeconfig_draft().is_some();
    let pending = controller.kubeconfig_has_pending_edits();
    let applied = kubeconfig_selection_label(&controller.kubeconfig_applied);
    drop(controller);
    rsx! {
        section { class: "configuration",
            label { r#for: "config-path", "TALOSCONFIG" }
            input { id: "config-path", class: "path", value: "{path}", placeholder: "Default talosconfig (leave blank)", disabled: busy,
                oninput: move |event| state.write().edit_path(event.value()) }
            button { disabled: picking || busy, onclick: move |_| state.write().pick(), if picking { "Selecting…" } else { "Browse…" } }
            button { class: "primary", disabled: busy, onclick: move |_| state.write().load(), if loading { "Reload config" } else { "Load config" } }
            if let Some(filter) = filter { span { class: "muted", "Context filter: {filter}" } }
        }
        section { class: "configuration kubeconfig-settings",
            label { r#for: "kubeconfig-source", "KUBERNETES SOURCE" }
            select { id: "kubeconfig-source", value: mode_value, disabled: busy,
                onchange: move |event| state.write().set_kubeconfig_mode(match event.value().as_str() {
                    "talos" => KubeconfigMode::TalosControlPlane,
                    "file" => KubeconfigMode::File,
                    _ => KubeconfigMode::Automatic,
                }),
                option { value: "automatic", "Automatic" }
                option { value: "talos", "Talos control plane" }
                option { value: "file", "Selected kubeconfig" }
            }
            if mode == KubeconfigMode::File {
                label { r#for: "kubeconfig-path", "FILE" }
                input { id: "kubeconfig-path", class: "path", value: "{kubeconfig_path}", disabled: busy,
                    placeholder: "Select or enter kubeconfig path",
                    oninput: move |event| state.write().edit_kubeconfig_path(event.value()) }
                button { disabled: kubeconfig_picking || busy, onclick: move |_| state.write().pick_kubeconfig(),
                    if kubeconfig_picking { "Selecting…" } else { "Browse kubeconfig…" } }
                label { r#for: "kubeconfig-context", "CONTEXT" }
                select { id: "kubeconfig-context", value: "{context_value}", disabled: busy,
                    onchange: move |event| {
                        if !operation_busy() {
                            state.write().kubeconfig_context = event.value()
                                .strip_prefix("named:").map(str::to_owned);
                        }
                    },
                    option { value: "", "{current_context}" }
                    for context in contexts {
                        option { value: "named:{context}", "{context}" }
                    }
                }
            }
            button { class: "primary", disabled: !can_apply || busy,
                onclick: move |_| state.write().apply_kubeconfig(), "Apply source" }
            button { disabled: busy, onclick: move |_| state.write().reset_kubeconfig(), "Reset to Automatic" }
            div { class: "source-status", role: "status",
                span { class: "muted", "Applied request: {applied}. " }
                if pending { strong { "Pending edits — not applied." } }
                else { span { "No pending edits." } }
                p { class: "muted", "Session-only. Unverified file credentials are never used; a pinned same-cluster Talos fallback is explicitly labelled. No ambient fallback." }
                if kubeconfig_parsing { p { "Inspecting kubeconfig…" } }
                if let Some(error) = kubeconfig_error { p { class: "source-error", "{error}" } }
                if mode == KubeconfigMode::Automatic {
                    p { class: "muted", "Uses the existing Talos overview defaults and checks KUBECONFIG for a cluster mismatch." }
                }
                if mode == KubeconfigMode::TalosControlPlane {
                    p { class: "muted", "Uses the Talos control-plane kubeconfig and ignores ambient KUBECONFIG." }
                }
            }
        }
        if busy { div { class: "banner warning", role: "status", "Operation in progress. Context, node, settings and navigation are locked until cancellation, compensation and audit complete." } }
        if let Some(error) = error { div { class: "banner error", role: "alert", "{error}" } }
    }
}

#[derive(Clone)]
struct SidebarCluster {
    name: String,
    status: String,
    active: bool,
    nodes: Vec<(String, String, bool, bool)>,
}

#[component]
fn Sidebar(mut state: Signal<Controller>) -> Element {
    let controller = state.read();
    let loading = controller.loading;
    let busy = operation_busy();
    let clusters =
        controller
            .clusters
            .iter()
            .map(|cluster| {
                let mut nodes = cluster
                    .node_ips
                    .iter()
                    .map(|(node, address)| {
                        let role = cluster.node_is_controlplane(node);
                        let selected = controller.identity.target.as_ref().is_some_and(|target| {
                            target.context == cluster.name && target.node.as_ref() == Some(node)
                        });
                        (node.clone(), address.clone(), role, selected)
                    })
                    .collect::<Vec<_>>();
                nodes.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
                SidebarCluster {
                    name: cluster.name.clone(),
                    status: connection_label(&cluster.connection).into(),
                    active: controller.identity.target.as_ref().is_some_and(|target| {
                        target.context == cluster.name && target.node.is_none()
                    }),
                    nodes,
                }
            })
            .collect::<Vec<_>>();
    drop(controller);
    rsx! {
        aside { class: "sidebar",
            div { class: "section-label", "CONFIGURED CONTEXTS" }
            if loading { p { class: "muted loading", "Collecting snapshots…" } }
            if clusters.is_empty() { p { class: "empty", "No configured clusters. Select a talosconfig and load it." } }
            for cluster in clusters {
                div { class: "cluster-nav", key: "{cluster.name}",
                    button { class: if cluster.active { "context active" } else { "context" }, disabled: busy,
                        onclick: { let name = cluster.name.clone(); move |_| state.write().select(Target { context: name.clone(), node: None, address: None }) },
                        span { "{cluster.name}" } small { "{cluster.status}" }
                    }
                    for controlplane in [true, false] {
                        if cluster.nodes.iter().any(|node| node.2 == controlplane) {
                            div { class: "role-label", if controlplane { "CONTROL PLANE" } else { "WORKERS / OTHER" } }
                            for (node, address, _, selected) in cluster.nodes.iter().filter(|node| node.2 == controlplane).cloned() {
                                button { class: if selected { "node active" } else { "node" }, key: "{address}-{node}", disabled: busy,
                                    onclick: { let context = cluster.name.clone(); let node = node.clone(); let address = address.clone(); move |_| state.write().select(Target { context: context.clone(), node: Some(node.clone()), address: Some(address.clone()) }) },
                                    span { class: "node-dot", "•" } div { span { "{node}" } small { "{address}" } }
                                }
                            }
                        }
                    }
                    if cluster.nodes.is_empty() { p { class: "muted", "Node roster unavailable" } }
                }
            }
            div { class: "sidebar-footer", "Native Dioxus 0.7 · TLS authenticated" }
        }
    }
}

fn connection_label(status: &ClusterConnectionStatus) -> &'static str {
    match status {
        ClusterConnectionStatus::Connected => "Connected",
        ClusterConnectionStatus::Disconnected => "Not queried",
        ClusterConnectionStatus::Unreachable(_) => "Unreachable",
    }
}

#[component]
fn Dashboard(mut state: Signal<Controller>) -> Element {
    let controller = state.read();
    let Some(cluster) = controller.selected() else {
        return rsx! { section { class: "panel welcome", h1 { "Your clusters, in focus." } p { "Load a talosconfig to inspect live system state. Mutations require explicit target confirmation." } } };
    };
    let name = cluster.name.clone();
    let node = controller
        .identity
        .target
        .as_ref()
        .and_then(|target| target.node.clone());
    let heading = node.clone().unwrap_or_else(|| name.clone());
    let tab = controller.node_tab;
    let tab_id = tab.id();
    let address = controller
        .identity
        .target
        .as_ref()
        .and_then(|target| target.address.clone())
        .unwrap_or_else(|| "Address unavailable".into());
    let status = connection_label(&cluster.connection);
    let status_class = if cluster.connection.is_connected() {
        "badge connected"
    } else {
        "badge unknown"
    };
    let mut warnings = Vec::new();
    if let Some(error) = cluster.connection.error() {
        warnings.push(error.to_owned());
    }
    if let Some(warning) = &cluster.discovery_warning {
        warnings.push(warning.clone());
    }
    if let Some(warning) = &cluster.kubeconfig_warning {
        warnings.push(warning.clone());
    }
    if let Some(error) = controller.errors.get(&name) {
        warnings.push(error.clone());
    }
    let queried = controller
        .attempts
        .get(&name)
        .map(|time| format!("Last collection attempt {}s ago", time.elapsed().as_secs()))
        .unwrap_or_else(|| "No snapshot collected yet".into());
    let succeeded = controller
        .successes
        .get(&name)
        .map(|time| format!("Last successful snapshot {}s ago", time.elapsed().as_secs()))
        .unwrap_or_else(|| "No successful snapshot".into());
    let stale = !cluster.connection.is_connected()
        || controller.errors.contains_key(&name)
        || controller
            .successes
            .get(&name)
            .is_none_or(|time| time.elapsed() > REFRESH_INTERVAL * 2);
    let auto_refresh = controller.auto_refresh;
    let busy = operation_busy();
    let feature = controller.feature_context();
    let feature_key = feature
        .as_ref()
        .map(FeatureContext::key)
        .unwrap_or_else(|| format!("{}:{name}:{heading}", controller.identity.config));
    let requested_logs = controller.requested_logs.clone();
    let tail = controller.options.tail;
    let navigation_error = controller.navigation_error.clone();
    let refreshing = controller.refreshing;
    let loading = controller.loading;
    let endpoints = cluster.endpoints.join(", ");
    let quorum = cluster
        .etcd_summary
        .as_ref()
        .map(|etcd| {
            format!(
                "{} / {} responding · {}",
                etcd.healthy,
                etcd.total,
                if etcd.has_quorum {
                    "quorum reported"
                } else {
                    "quorum unavailable"
                }
            )
        })
        .unwrap_or_else(|| "Unknown / unavailable".into());
    let mut metrics = Vec::new();
    let mut nodes = cluster.node_ips.keys().cloned().collect::<Vec<_>>();
    nodes.sort();
    if let Some(node) = &node {
        nodes.retain(|value| value == node);
    }
    for node in nodes {
        let memory = cluster
            .memory
            .iter()
            .find(|value| value.node == node)
            .and_then(|value| value.meminfo.as_ref())
            .map(|value| {
                let used = value.mem_total.saturating_sub(value.mem_available);
                let percent = if value.mem_total == 0 {
                    0.0
                } else {
                    used as f64 / value.mem_total as f64 * 100.0
                };
                format!(
                    "{} / {} ({percent:.1}%)",
                    format_bytes(used),
                    format_bytes(value.mem_total)
                )
            })
            .unwrap_or_else(|| "Unknown / unavailable".into());
        let load = cluster
            .load_avg
            .iter()
            .find(|value| value.node == node)
            .map(|value| {
                format!(
                    "{:.2} / {:.2} / {:.2}",
                    value.load1, value.load5, value.load15
                )
            })
            .unwrap_or_else(|| "Unknown / unavailable".into());
        let cpu = cluster
            .cpu_info
            .iter()
            .find(|value| value.node == node)
            .map(|value| format!("{} cores · {}", value.cpu_count, value.model_name))
            .unwrap_or_else(|| "Unknown / unavailable".into());
        let version = cluster
            .versions
            .iter()
            .find(|value| value.node == node)
            .map(|value| value.version.clone())
            .unwrap_or_else(|| "Unknown".into());
        metrics.push((node, memory, load, cpu, version));
    }
    let services = controller.services();
    let kubeconfig_source = cluster
        .kubeconfig_source
        .clone()
        .unwrap_or_else(|| "Not collected / unavailable".into());
    drop(controller);
    rsx! {
        section { class: "overview",
            div { class: "dashboard-header",
                div { class: "page-heading", div { div { class: "eyebrow", if node.is_some() { "{name} / NODE DETAIL" } else { "CLUSTER OVERVIEW" } } h1 { "{heading}" } }
                    div { class: "heading-actions", span { class: status_class, "{status}" } button { disabled: loading || refreshing || busy, onclick: move |_| state.write().refresh(), if refreshing { "Refreshing…" } else { "Refresh snapshot" } }
                        button { disabled: busy, onclick: move |_| { let value = state.read().auto_refresh; state.write().auto_refresh = !value; }, if auto_refresh { "Auto refresh: on" } else { "Auto refresh: paused" } }
                    }
                }
                if node.is_some() { p { class: "muted node-address", "{address}" } }
                p { class: "muted freshness", "{queried} · {succeeded} · Auto interval 10s; unavailable fields are unknown. Connectivity is not a health verdict." }
                p { class: "muted freshness", "Effective Kubernetes source: {kubeconfig_source}" }
                if refreshing { div { class: "banner", "Refreshing… previously displayed data may be stale until this request completes." } }
                if stale && !loading { div { class: "banner warning", role: "status", "Snapshot stale or unavailable. Retained values are not current health evidence." } }
                if let Some(error) = navigation_error { div { class: "banner error", role: "alert", "{error}" } }
                for warning in warnings { div { class: "banner warning", role: "status", "{warning}" } }
            }
            if node.is_some() {
                div { class: "node-tabs", role: "tablist", aria_label: "Node details",
                    for item in NodeTab::all() {
                        button { class: if tab == item { "node-tab active" } else { "node-tab" }, disabled: busy,
                            id: "{item.id()}-tab", role: "tab",
                            aria_selected: if tab == item { "true" } else { "false" },
                            aria_controls: "{item.id()}-panel",
                            onclick: move |_| state.write().select_node_tab(item), "{item.label()}"
                        }
                    }
                }
            }
            {rsx! {
            div { class: if node.is_some() && tab == NodeTab::Logs { "dashboard-body logs-body" } else { "dashboard-body" },
                key: "{feature_key}-{tab_id}",
                id: "{tab_id}-panel",
                role: if node.is_some() { "tabpanel" } else { "region" },
                aria_labelledby: node.as_ref().map(|_| format!("{tab_id}-tab")),
                tabindex: node.as_ref().map(|_| "0"),
                if node.is_none() {
                    div { class: "summary-grid",
                        article { class: "panel", h3 { "API ENDPOINTS" } p { "{endpoints}" } }
                        article { class: "panel", h3 { "ETCD SNAPSHOT" } p { "{quorum}" } }
                    }
                }
                if node.is_none() || tab == NodeTab::Overview {
                    if metrics.is_empty() { div { class: "panel empty", "No node metrics available. Check connection warnings or refresh the snapshot." } }
                    div { class: "metrics-grid",
                        for (node_name, memory, load, cpu, version) in metrics {
                            article { class: "panel node-card", key: "{node_name}",
                                div { class: "card-heading", h2 { "{node_name}" } span { class: "muted", "{version}" } }
                                dl { dt { "Memory used" } dd { "{memory}" } dt { "Load 1 / 5 / 15m" } dd { "{load}" } dt { "Processor" } dd { "{cpu}" } }
                            }
                        }
                    }
                }
                if node.is_some() && tab == NodeTab::Services {
                    section { class: "panel services", h2 { "Services" }
                        if services.is_empty() { p { class: "empty", "Services unavailable or empty for this node. Refresh to retry." } }
                        else { table { thead { tr { th { "Service" } th { "State" } th { "Health snapshot" } th { "Actions" } } }
                            tbody { for service in services {
                                tr { key: "{service.id}",
                                    td { button { class: "service-link", disabled: busy, onclick: { let id = service.id.clone(); move |_| state.write().open_service_logs(id.clone()) }, "{service.id}" } }
                                    td { "{service.state}" }
                                    td { {service.health.map(|health| if health.unknown { "Unknown".to_owned() } else if health.healthy { "Healthy".to_owned() } else { format!("Unhealthy · {}", health.last_message) }).unwrap_or_else(|| "Unknown".into())} }
                                    td { if let Some(ctx) = feature.clone() { ServiceRestart { ctx, service: service.id.clone() } } }
                                }
                            } }
                        } }
                    }
                }
                if node.is_none() {
                    section { class: "panel logs-hint", h2 { "Node inspection" } p { class: "muted", "Select a node in the sidebar to inspect logs, processes, storage, networking and cluster operations." } }
                } else if tab != NodeTab::Overview && tab != NodeTab::Services {
                    if let Some(ctx) = feature.clone() {
                        FeaturePanel { key: "{feature_key}-{tab_id}", ctx, tab, initial_services: requested_logs, tail,
                            on_logs: move |request| { state.write().navigate_logs(request); } }
                    } else {
                        section { class: "panel empty", h2 { "{tab.label()}" } p { "Selected node unavailable: no authenticated client or current roster identity. Refresh or reload to retry." } }
                    }
                }
            }
            }}
        }
    }
}

#[component]
fn FeaturePanel(
    ctx: FeatureContext,
    tab: NodeTab,
    initial_services: Vec<String>,
    tail: i32,
    on_logs: EventHandler<LogRequest>,
) -> Element {
    match tab {
        NodeTab::Logs => rsx! { LogsPanel { ctx, on_logs, initial_services, tail } },
        NodeTab::Processes => rsx! { ProcessesPanel { ctx, on_logs } },
        NodeTab::Storage => rsx! { StoragePanel { ctx, on_logs } },
        NodeTab::Network => rsx! { NetworkPanel { ctx, on_logs } },
        NodeTab::Etcd => rsx! { EtcdPanel { ctx, on_logs } },
        NodeTab::Workloads => rsx! { WorkloadsPanel { ctx, on_logs } },
        NodeTab::Diagnostics => rsx! { DiagnosticsPanel { ctx, on_logs } },
        NodeTab::Security => rsx! { SecurityPanel { ctx, on_logs } },
        NodeTab::Lifecycle => rsx! { LifecyclePanel { ctx, on_logs } },
        NodeTab::Operations => rsx! { OperationsPanel { ctx, on_logs } },
        NodeTab::Overview | NodeTab::Services => rsx! { div {} },
    }
}

#[allow(dead_code)] // Reachable logs use LogsPanel; legacy presentation retained for fixtures.
#[component]
fn Logs(mut state: Signal<Controller>) -> Element {
    let controller = state.read();
    let node_selected = controller
        .identity
        .target
        .as_ref()
        .is_some_and(|target| target.address.is_some());
    if !node_selected {
        return rsx! { section { class: "panel logs-hint", h2 { "Live service logs" } p { class: "muted", "Select a node in the sidebar to inspect its services and stream logs." } } };
    }
    let services = controller.services();
    let services_empty = services.is_empty();
    let service = controller.service.clone();
    let streaming = controller.streaming;
    let status = controller.stream_status.clone();
    let query = controller
        .logs
        .as_ref()
        .map(|view| view.logs.buffer().query().to_owned())
        .unwrap_or_default();
    let levels = [
        LogLevel::Error,
        LogLevel::Warning,
        LogLevel::Info,
        LogLevel::Debug,
        LogLevel::Unknown,
    ]
    .into_iter()
    .map(|level| {
        let enabled = controller
            .logs
            .as_ref()
            .is_none_or(|view| view.logs.buffer().filters().levels.accepts(&level));
        (level, enabled)
    })
    .collect::<Vec<_>>();
    let (rows, counter) = if let Some(view) = &controller.logs {
        let (indices, matching) = view.rendered_indices();
        let rows = indices
            .into_iter()
            .map(|index| {
                let entry = &view.logs.buffer().entries()[index];
                (
                    index,
                    entry.level.to_string().to_lowercase(),
                    entry
                        .timestamp
                        .as_ref()
                        .map(|timestamp| timestamp.display.clone())
                        .unwrap_or_default(),
                    entry.raw.clone(),
                )
            })
            .collect::<Vec<_>>();
        let retained = view.logs.buffer().entries().len();
        let counter = format!(
            "Showing newest {} of {matching} matching · {retained} retained / {} received · {} overload drops · {} truncated",
            rows.len(),
            view.received,
            view.dropped,
            view.truncated
        );
        (rows, counter)
    } else {
        (Vec::new(), "No log data received yet.".into())
    };
    let has_logs = controller.logs.is_some();
    drop(controller);
    rsx! {
        section { class: "panel logs",
            div { class: "card-heading", h2 { "Live service logs" } span { class: "muted", "Text only · newest first" } }
            div { class: "log-toolbar",
                select { aria_label: "Service", value: "{service}", disabled: services_empty,
                    onchange: move |event| state.write().change_service(event.value()),
                    if services_empty { option { value: "", "No services available" } }
                    for service in services { option { value: "{service.id}", "{service.id}" } }
                }
                button { class: "primary", disabled: service.is_empty() || streaming, onclick: move |_| state.write().start(), "Start / reconnect" }
                button { disabled: !streaming, onclick: move |_| state.write().stop(), "Stop" }
                input { class: "search", aria_label: "Search logs", placeholder: "Search retained lines (case insensitive)", value: "{query}", disabled: !has_logs,
                    oninput: move |event| { if let Some(view) = &mut state.write().logs { view.query(event.value()); } } }
            }
            div { class: "filter-bar",
                span { class: "section-label", "LEVELS" }
                for (level, enabled) in levels {
                    button { class: if enabled { "filter enabled" } else { "filter" }, disabled: !has_logs, aria_pressed: "{enabled}",
                        onclick: { let level = level.clone(); move |_| { if let Some(view) = &mut state.write().logs { view.toggle(level.clone()); } } }, "{level}" }
                }
            }
            p { class: if streaming { "stream-status live" } else { "stream-status muted" }, role: "status", "{status}" }
            p { class: "log-counter muted", "{counter} · DOM capped at 300 rows; search applies to the retained buffer." }
            div { class: "log-output", aria_label: "Service log lines",
                if rows.is_empty() { p { class: "empty", "No matching lines. Start the stream, wait for events, or adjust search and level filters." } }
                for (index, level, timestamp, text) in rows {
                    div { class: "log-row {level}", key: "{index}", span { class: "log-time", "{timestamp}" } pre { "{text}" } }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::dioxus_core::{DynamicNode, TemplateNode};

    fn stylesheet_rule(selector: &str) -> &str {
        STYLE
            .split_once(&format!("{selector}{{"))
            .unwrap_or_else(|| panic!("missing stylesheet selector: {selector}"))
            .1
            .split_once('}')
            .unwrap()
            .0
    }

    #[test]
    fn shared_styles_distinguish_pressed_buttons_and_focusable_text_regions() {
        let selected = stylesheet_rule("button[aria-pressed=\"true\"]");
        assert!(selected.contains("background:#1c3047"));
        assert!(selected.contains("border-color:#2a4965"));
        assert!(selected.contains("color:var(--accent)"));
        let focus = stylesheet_rule(".text-review-region:focus-visible,[tabindex]:focus-visible");
        assert!(focus.contains("outline:2px solid var(--accent)"));
        assert!(focus.contains("outline-offset:2px"));
        assert!(!stylesheet_rule("button").contains("color:var(--accent)"));
    }

    #[test]
    fn shared_styles_color_actual_log_levels_without_overriding_log_wrap() {
        for (selector, token) in [
            (".log-level-error", "--error"),
            (".log-level-warn", "--warn"),
            (".log-level-info", "--text"),
            (".log-level-debug,.log-level-unknown", "--muted"),
        ] {
            assert_eq!(stylesheet_rule(selector), format!("color:var({token})"));
        }
        assert!(!STYLE.contains("white-space:pre-wrap!important"));
        assert!(!stylesheet_rule(".log-lines pre").contains("white-space"));
        assert_eq!(
            stylesheet_rule(".panel :is(p,h1,h2,h3,h4,h5,h6)"),
            "overflow-wrap:anywhere"
        );
    }

    #[test]
    fn shared_styles_preserve_narrow_tabs_and_responsive_scroll_panes() {
        assert!(STYLE.contains("@media(max-width:900px){.feature-split,.split-pane{grid-template-columns:minmax(0,1fr)}"));
        assert!(STYLE.contains(".feature-scroll,.table-scroll,.scroll-pane,.feature-detail,.detail-pane{max-height:420px}"));
        assert!(STYLE.contains("@media(max-width:600px)"));
        assert!(STYLE.contains(".node-tabs{flex-wrap:nowrap}"));
    }

    fn render_shell_smoke(dom: &VirtualDom, name: &str) -> String {
        let markup = dioxus_ssr::render(dom);
        assert!(markup.contains("class=\"shell\""));
        assert!(markup.contains(STYLE));
        if let Some(directory) = std::env::var_os("TALOS_PILOT_UI_SMOKE_DIR") {
            let directory = PathBuf::from(directory);
            let temporary = directory.starts_with(std::env::temp_dir())
                || directory.starts_with("/tmp")
                || directory.starts_with("/private/tmp");
            assert!(
                directory.is_absolute()
                    && temporary
                    && !directory
                        .components()
                        .any(|part| part == std::path::Component::ParentDir),
                "UI smoke fixtures must be exported to an absolute temporary directory"
            );
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                directory.join(format!("{name}.html")),
                format!(
                    "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Talos Pilot UI smoke fixture</title></head><body>{markup}</body></html>"
                ),
            )
            .unwrap();
        }
        markup
    }

    #[test]
    fn renderer_backed_shell_smoke_fixtures_cover_tabs_long_targets_and_busy() {
        struct ResetBusy;
        impl Drop for ResetBusy {
            fn drop(&mut self) {
                TEST_BUSY.with(|busy| busy.set(false));
            }
        }
        let _reset = ResetBusy;
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut dom = VirtualDom::new(app);
        dom.provide_root_context(Bootstrap {
            options: DioxusOptions {
                config_path: Some("/nonexistent-talos-pilot-ui-smoke/talosconfig".into()),
                context: None,
                tail: 100,
                maintenance_endpoint: None,
            },
            runtime: runtime.handle().clone(),
        });
        dom.rebuild_in_place();
        let mut mounted = dom.in_scope(ScopeId::APP, consume_context::<Signal<Controller>>);
        {
            let mut controller = mounted.write();
            controller.load_worker.cancel();
            controller.loading = false;
        }
        let _ = dom.render_immediate_to_vec();
        let welcome = render_shell_smoke(&dom, "shell-welcome");
        assert!(welcome.contains("Your clusters, in focus."));

        let context = format!("prod-{}", "long-context-".repeat(20));
        let node = format!("node-{}", "long-node-".repeat(30));
        let address = format!("unavailable-{}", "long-address-".repeat(20));
        {
            let mut controller = mounted.write();
            let mut snapshot = node_snapshot(Some(&address));
            snapshot.name = context.clone();
            snapshot.node_ips.clear();
            snapshot.node_ips.insert(node.clone(), address.clone());
            snapshot.services[0].node = node.clone();
            snapshot.endpoints = vec![address.clone()];
            snapshot.discovery_warning = Some(format!(
                "Unavailable fixture source: {}",
                "long-warning-".repeat(30)
            ));
            assert!(snapshot.client.is_none());
            controller.clusters = vec![snapshot];
            controller.select(Target {
                context: context.clone(),
                node: Some(node.clone()),
                address: Some(address.clone()),
            });
        }
        for tab in NodeTab::all() {
            mounted.write().select_node_tab(tab);
            let _ = dom.render_immediate_to_vec();
            let markup = render_shell_smoke(&dom, &format!("shell-{}", tab.id()));
            assert!(markup.contains(&context));
            assert!(markup.contains(&node));
            assert!(markup.contains(&address));
            assert!(markup.contains("role=\"tablist\""));
            assert!(markup.contains("role=\"tabpanel\""));
            assert!(markup.contains("tabindex=\"0\""));
            assert!(markup.contains(&format!("aria-controls=\"{}-panel\"", tab.id())));
            assert_eq!(
                markup.contains("Selected node unavailable"),
                !matches!(tab, NodeTab::Overview | NodeTab::Services)
            );
        }
        mounted.write().select_node_tab(NodeTab::Operations);
        TEST_BUSY.with(|busy| busy.set(true));
        let _ = dom.render_immediate_to_vec();
        let busy = render_shell_smoke(&dom, "shell-busy");
        assert!(busy.contains("navigation are locked"));
        assert!(busy.contains("disabled"));
        TEST_BUSY.with(|busy| busy.set(false));

        mounted.write().select(Target {
            context,
            node: None,
            address: None,
        });
        let _ = dom.render_immediate_to_vec();
        let overview = render_shell_smoke(&dom, "shell-cluster-overview");
        assert!(overview.contains("API ENDPOINTS"));
        assert!(!overview.contains("class=\"node-tabs\""));
    }

    fn test_controller(runtime: &tokio::runtime::Runtime) -> Controller {
        let mut controller = Controller::new(Bootstrap {
            options: DioxusOptions {
                config_path: Some(PathBuf::from(
                    "/nonexistent-talos-pilot-dioxus-unit/talosconfig",
                )),
                context: None,
                tail: 100,
                maintenance_endpoint: None,
            },
            runtime: runtime.handle().clone(),
        });
        controller.load_worker.cancel();
        controller.loading = false;
        controller
    }

    fn parsed_contexts(controller: &mut Controller, current: Option<&str>) {
        controller.kubeconfig_parser_worker.cancel();
        assert!(controller.apply(Event::KubeconfigInspected(
            controller.kubeconfig_generation,
            Ok(KubeconfigFileInfo {
                contexts: vec!["dev".into(), "prod".into()],
                current_context: current.map(str::to_owned),
            }),
        )));
    }

    #[test]
    fn applying_file_source_persists_through_talos_reload_and_reset() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut controller = test_controller(&runtime);
        controller.set_kubeconfig_mode(KubeconfigMode::File);
        controller.edit_kubeconfig_path("/selected/kubeconfig".into());
        parsed_contexts(&mut controller, Some("dev"));
        controller.kubeconfig_context = Some("prod".into());
        assert!(controller.kubeconfig_has_pending_edits());
        controller.apply_kubeconfig();
        let selected = KubeconfigSelection::File {
            path: PathBuf::from("/selected/kubeconfig"),
            context: Some("prod".into()),
        };
        assert_eq!(controller.kubeconfig_applied, selected);
        assert!(!controller.kubeconfig_has_pending_edits());
        controller.edit_path("/another/talosconfig".into());
        controller.load();
        assert_eq!(controller.kubeconfig_applied, selected);
        assert_eq!(controller.kubeconfig_context.as_deref(), Some("prod"));
        controller.set_kubeconfig_mode(KubeconfigMode::TalosControlPlane);
        assert_eq!(controller.kubeconfig_applied, selected);
        controller.apply_kubeconfig();
        controller.load();
        assert_eq!(
            controller.kubeconfig_applied,
            KubeconfigSelection::TalosControlPlane
        );
        controller.reset_kubeconfig();
        assert_eq!(
            controller.kubeconfig_applied,
            KubeconfigSelection::Automatic
        );
        assert!(!controller.kubeconfig_has_pending_edits());
    }

    #[test]
    fn source_apply_and_reload_abort_stream_and_reject_old_snapshot_events() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for apply_source in [true, false] {
            let (mut controller, stream, worker) = active_fake_stream(&runtime);
            let snapshot = controller.identity.snapshot();
            let epoch = controller.identity.config;
            let snapshot_task = runtime.spawn(std::future::pending::<()>());
            controller
                .snapshot_worker
                .replace(snapshot_task.abort_handle());
            if apply_source {
                controller.set_kubeconfig_mode(KubeconfigMode::TalosControlPlane);
                // Draft edits alone never invalidate an active stream.
                assert!(controller.identity.accepts_stream(&stream));
                controller.apply_kubeconfig();
            } else {
                controller.load();
            }
            assert!(controller.clusters.is_empty());
            assert!(controller.logs.is_none());
            assert!(!controller.streaming);
            assert!(!controller.apply(Event::Snapshot(
                snapshot,
                Ok(node_snapshot(Some("10.0.0.9")))
            )));
            assert!(!controller.apply(Event::InitialSnapshot(
                epoch,
                "prod".into(),
                Ok(node_snapshot(None))
            )));
            assert!(!controller.apply(Event::Configured(epoch, Ok(vec![node_snapshot(None)]))));
            assert!(!controller.apply(Event::LoadFinished(epoch)));
            assert!(!controller.apply(Event::Lines(stream.clone(), vec!["stale".into()], 0)));
            assert!(!controller.apply(Event::StreamEnded(stream, "stale".into())));
            assert!(runtime.block_on(worker).unwrap_err().is_cancelled());
            assert!(runtime.block_on(snapshot_task).unwrap_err().is_cancelled());
        }
    }

    #[test]
    fn picker_and_parser_generations_reject_typed_path_mode_and_reload_races() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut controller = test_controller(&runtime);
        controller.set_kubeconfig_mode(KubeconfigMode::File);
        for change in 0..3 {
            let old = controller.kubeconfig_generation;
            let picker = runtime.spawn(std::future::pending::<()>());
            let parser = runtime.spawn(std::future::pending::<()>());
            controller
                .kubeconfig_picker_worker
                .replace(picker.abort_handle());
            controller
                .kubeconfig_parser_worker
                .replace(parser.abort_handle());
            match change {
                0 => controller.edit_kubeconfig_path("/typed/new".into()),
                1 => controller.set_kubeconfig_mode(KubeconfigMode::Automatic),
                _ => controller.load(),
            }
            let path = controller.kubeconfig_path.clone();
            assert!(!controller.apply(Event::KubeconfigPicked(old, Some("/stale/picked".into()))));
            assert!(!controller.apply(Event::KubeconfigInspected(
                old,
                Err("stale parser error".into())
            )));
            assert!(!controller.apply(Event::KubeconfigInspected(
                old,
                Ok(KubeconfigFileInfo {
                    contexts: vec!["stale".into()],
                    current_context: Some("stale".into()),
                })
            )));
            assert_eq!(controller.kubeconfig_path, path);
            assert!(runtime.block_on(picker).unwrap_err().is_cancelled());
            assert!(runtime.block_on(parser).unwrap_err().is_cancelled());
        }
    }

    #[test]
    fn file_errors_clear_and_context_choices_do_not_silently_apply() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut controller = test_controller(&runtime);
        controller.set_kubeconfig_mode(KubeconfigMode::File);
        assert!(
            controller
                .kubeconfig_draft_error()
                .unwrap()
                .contains("path")
        );
        assert!(controller.kubeconfig_file_path().is_none());
        controller.edit_kubeconfig_path("/selected/kubeconfig".into());
        controller.kubeconfig_parser_worker.cancel();
        assert!(controller.apply(Event::KubeconfigInspected(
            controller.kubeconfig_generation,
            Err("Invalid kubeconfig YAML".into()),
        )));
        assert!(
            controller
                .kubeconfig_draft_error()
                .unwrap()
                .contains("Invalid")
        );
        controller.apply_kubeconfig();
        assert_eq!(
            controller.kubeconfig_applied,
            KubeconfigSelection::Automatic
        );
        parsed_contexts(&mut controller, None);
        assert!(
            controller
                .kubeconfig_draft_error()
                .unwrap()
                .contains("no current-context")
        );
        assert!(controller.kubeconfig_draft().is_none());
        controller.kubeconfig_context = Some("prod".into());
        assert!(controller.kubeconfig_draft().is_some());
        // A queued result for the same file must not reset a user's context choice.
        parsed_contexts(&mut controller, Some("dev"));
        assert_eq!(controller.kubeconfig_context.as_deref(), Some("prod"));
        controller.kubeconfig_context = None;
        assert!(matches!(
            controller.kubeconfig_draft(),
            Some(KubeconfigSelection::File { context: None, .. })
        ));
        parsed_contexts(&mut controller, Some("missing"));
        assert!(
            controller
                .kubeconfig_draft_error()
                .unwrap()
                .contains("not in this file")
        );
        controller.kubeconfig_context = Some("dev".into());
        assert!(controller.kubeconfig_draft().is_some());
        controller.edit_kubeconfig_path("  ".into());
        assert!(controller.kubeconfig_file_path().is_none());
        assert!(controller.kubeconfig_info.is_none());
        assert!(controller.kubeconfig_context.is_none());
        assert!(controller.kubeconfig_draft().is_none());
        assert_eq!(
            controller.kubeconfig_applied,
            KubeconfigSelection::Automatic
        );
    }

    #[cfg(unix)]
    #[test]
    fn picker_preserves_non_utf8_path_until_user_edits_it() {
        use std::os::unix::ffi::OsStringExt;
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut controller = test_controller(&runtime);
        controller.set_kubeconfig_mode(KubeconfigMode::File);
        let path = PathBuf::from(std::ffi::OsString::from_vec(
            b"/selected/\xffconfig".to_vec(),
        ));
        assert!(controller.apply(Event::KubeconfigPicked(
            controller.kubeconfig_generation,
            Some(path.clone())
        )));
        parsed_contexts(&mut controller, Some("prod"));
        assert_eq!(controller.kubeconfig_file_path(), Some(path.clone()));
        assert_eq!(
            controller.kubeconfig_draft(),
            Some(KubeconfigSelection::File {
                path,
                context: None
            })
        );
        controller.edit_kubeconfig_path("/typed/config".into());
        assert!(controller.kubeconfig_exact_path.is_none());
        assert_eq!(
            controller.kubeconfig_file_path(),
            Some("/typed/config".into())
        );
    }

    fn node_snapshot(address: Option<&str>) -> ClusterOverview {
        let mut snapshot = ClusterOverview {
            name: "prod".into(),
            connection: ClusterConnectionStatus::Connected,
            ..Default::default()
        };
        if let Some(address) = address {
            snapshot.node_ips.insert("node-a".into(), address.into());
            snapshot.services.push(talos_rs::NodeServices {
                node: "node-a".into(),
                services: vec![talos_rs::ServiceInfo {
                    id: "kubelet".into(),
                    state: "Running".into(),
                    health: None,
                }],
            });
        }
        snapshot
    }

    fn active_fake_stream(
        runtime: &tokio::runtime::Runtime,
    ) -> (Controller, Ticket, tokio::task::JoinHandle<()>) {
        let mut controller = test_controller(runtime);
        controller.clusters = vec![node_snapshot(Some("10.0.0.1"))];
        controller.select(Target {
            context: "prod".into(),
            node: Some("node-a".into()),
            address: Some("10.0.0.1".into()),
        });
        let ticket = controller.identity.start_stream("kubelet".into());
        controller.streaming = true;
        controller.logs = Some(LogView::new("10.0.0.1".into(), "kubelet".into()));
        controller
            .logs
            .as_mut()
            .unwrap()
            .append(vec!["info retained".into()], 0);
        let worker = runtime.spawn(std::future::pending::<()>());
        controller.stream_worker.replace(worker.abort_handle());
        (controller, ticket, worker)
    }

    #[test]
    fn node_tabs_reset_on_different_target_and_preserve_same_target_refresh() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut controller = test_controller(&runtime);
        let mut other_context = node_snapshot(Some("10.0.0.1"));
        other_context.name = "dev".into();
        controller.clusters = vec![node_snapshot(Some("10.0.0.1")), other_context];
        let target = Target {
            context: "prod".into(),
            node: Some("node-a".into()),
            address: Some("10.0.0.1".into()),
        };
        controller.select(target.clone());
        assert_eq!(controller.node_tab, NodeTab::Overview);
        controller.select_node_tab(NodeTab::Services);
        controller.select(target.clone());
        assert_eq!(controller.node_tab, NodeTab::Services);
        let snapshot = controller.identity.snapshot();
        assert!(controller.apply(Event::Snapshot(
            snapshot,
            Ok(node_snapshot(Some("10.0.0.1")))
        )));
        assert_eq!(controller.node_tab, NodeTab::Services);
        controller.select_node_tab(NodeTab::Logs);
        controller.select(Target {
            node: Some("node-b".into()),
            address: Some("10.0.0.2".into()),
            ..target.clone()
        });
        assert_eq!(controller.node_tab, NodeTab::Overview);
        controller.select_node_tab(NodeTab::Logs);
        controller.select(Target {
            context: "dev".into(),
            ..target
        });
        assert_eq!(controller.node_tab, NodeTab::Overview);
        controller.select_node_tab(NodeTab::Services);
        controller.select(Target {
            context: "dev".into(),
            node: None,
            address: None,
        });
        assert_eq!(controller.node_tab, NodeTab::Overview);
        controller.select_node_tab(NodeTab::Logs);
        controller.load();
        assert_eq!(controller.node_tab, NodeTab::Overview);
    }

    #[test]
    fn pure_tab_switch_preserves_stream_snapshot_filters_and_retained_logs() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (mut controller, stream, worker) = active_fake_stream(&runtime);
        let snapshot = controller.identity.snapshot();
        let snapshot_task = runtime.spawn(std::future::pending::<()>());
        controller
            .snapshot_worker
            .replace(snapshot_task.abort_handle());
        controller.refreshing = true;
        controller.logs.as_mut().unwrap().query("info".into());
        controller.logs.as_mut().unwrap().toggle(LogLevel::Debug);
        let status = controller.stream_status.clone();
        for tab in [
            NodeTab::Logs,
            NodeTab::Overview,
            NodeTab::Services,
            NodeTab::Logs,
        ] {
            controller.select_node_tab(tab);
            assert_eq!(controller.node_tab, tab);
            assert!(controller.streaming);
            assert!(controller.refreshing);
            assert!(controller.identity.accepts_stream(&stream));
            assert!(controller.identity.accepts_snapshot(&snapshot));
            assert!(!worker.is_finished());
            assert!(!snapshot_task.is_finished());
            assert_eq!(controller.stream_status, status);
            let view = controller.logs.as_ref().unwrap();
            assert_eq!(view.logs.buffer().query(), "info");
            assert!(
                !view
                    .logs
                    .buffer()
                    .filters()
                    .levels
                    .accepts(&LogLevel::Debug)
            );
            assert!(controller.apply(Event::Lines(
                stream.clone(),
                vec!["info still live while hidden".into()],
                0
            )));
        }
        assert_eq!(
            controller
                .logs
                .as_ref()
                .unwrap()
                .logs
                .buffer()
                .entries()
                .len(),
            5
        );
        drop(controller);
        assert!(runtime.block_on(worker).unwrap_err().is_cancelled());
        assert!(runtime.block_on(snapshot_task).unwrap_err().is_cancelled());
    }

    #[test]
    fn service_link_opens_logs_without_starting_a_new_stream() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (mut controller, stream, worker) = active_fake_stream(&runtime);
        controller.clusters[0].services[0]
            .services
            .push(talos_rs::ServiceInfo {
                id: "apid".into(),
                state: "Running".into(),
                health: None,
            });
        controller.select_node_tab(NodeTab::Services);
        controller.open_service_logs("kubelet".into());
        assert_eq!(controller.node_tab, NodeTab::Logs);
        assert_eq!(controller.service, "kubelet");
        assert!(controller.identity.accepts_stream(&stream));
        assert!(controller.streaming);
        assert!(!worker.is_finished());
        controller.select_node_tab(NodeTab::Services);
        controller.open_service_logs("apid".into());
        assert_eq!(controller.node_tab, NodeTab::Logs);
        assert_eq!(controller.service, "apid");
        assert!(!controller.streaming);
        assert!(controller.logs.is_none());
        assert!(controller.stream_worker.0.is_none());
        assert!(!controller.identity.accepts_stream(&stream));
        assert!(runtime.block_on(worker).unwrap_err().is_cancelled());
    }

    fn rendered_text(dom: &VirtualDom, node: &VNode) -> Vec<String> {
        fn static_text(nodes: &[TemplateNode], text: &mut Vec<String>) {
            for node in nodes {
                match node {
                    TemplateNode::Text { text: value } => text.push(value.to_string()),
                    TemplateNode::Element { children, .. } => static_text(children, text),
                    TemplateNode::Dynamic { .. } => {}
                }
            }
        }
        let mut text = Vec::new();
        static_text(node.template.roots, &mut text);
        for (index, dynamic) in node.dynamic_nodes.iter().enumerate() {
            match dynamic {
                DynamicNode::Text(value) => text.push(value.value.clone()),
                DynamicNode::Fragment(nodes) => {
                    for child in nodes {
                        text.extend(rendered_text(dom, child));
                    }
                }
                DynamicNode::Component(component) => {
                    if let Some(scope) = component.mounted_scope(index, node, dom) {
                        text.extend(rendered_text(dom, scope.root_node()));
                    }
                }
                DynamicNode::Placeholder(_) => {}
            }
        }
        text
    }

    #[test]
    fn headless_all_node_tabs_render_active_content_and_unavailable_targets() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut dom = VirtualDom::new(app);
        dom.provide_root_context(Bootstrap {
            options: DioxusOptions {
                config_path: Some(PathBuf::from(
                    "/nonexistent-talos-pilot-dioxus-tabs/talosconfig",
                )),
                context: None,
                tail: 100,
                maintenance_endpoint: None,
            },
            runtime: runtime.handle().clone(),
        });
        dom.rebuild_in_place();
        let mut mounted = dom.in_scope(ScopeId::APP, consume_context::<Signal<Controller>>);
        let worker = runtime.spawn(std::future::pending::<()>());
        let stream = {
            let mut controller = mounted.write();
            controller.load_worker.cancel();
            controller.identity.reload();
            controller.loading = false;
            controller.clusters = vec![node_snapshot(Some("10.0.0.1"))];
            controller.select(Target {
                context: "prod".into(),
                node: Some("node-a".into()),
                address: Some("10.0.0.1".into()),
            });
            controller.streaming = true;
            controller.logs = Some(LogView::new("10.0.0.1".into(), "kubelet".into()));
            controller.stream_worker.replace(worker.abort_handle());
            controller.identity.start_stream("kubelet".into())
        };
        for tab in NodeTab::all() {
            mounted.write().select_node_tab(tab);
            let _ = dom.render_immediate_to_vec();
            let text = rendered_text(&dom, dom.base_scope().root_node());
            assert_eq!(
                text.iter().any(|value| value == "Memory used"),
                tab == NodeTab::Overview
            );
            assert_eq!(
                text.iter().any(|value| value == "Health snapshot"),
                tab == NodeTab::Services
            );
            assert_eq!(
                text.iter()
                    .any(|value| value.contains("Selected node unavailable")),
                tab != NodeTab::Overview && tab != NodeTab::Services
            );
            assert!(text.iter().any(|value| value == "Refresh snapshot"));
            assert!(mounted.peek().identity.accepts_stream(&stream));
            assert!(!worker.is_finished());
            assert!(mounted.write().apply(Event::Lines(
                stream.clone(),
                vec!["info arrives while tabs change".into()],
                0
            )));
        }
        assert_eq!(
            mounted
                .peek()
                .logs
                .as_ref()
                .unwrap()
                .logs
                .buffer()
                .entries()
                .len(),
            NodeTab::all().len()
        );
        mounted.write().select(Target {
            context: "prod".into(),
            node: None,
            address: None,
        });
        let _ = dom.render_immediate_to_vec();
        let text = rendered_text(&dom, dom.base_scope().root_node());
        assert!(text.iter().any(|value| value == "API ENDPOINTS"));
        assert!(text.iter().any(|value| value == "Memory used"));
        assert!(!text.iter().any(|value| value == "Overview"));
        assert!(!text.iter().any(|value| value == "Health snapshot"));
        drop(dom);
        assert!(runtime.block_on(worker).unwrap_err().is_cancelled());
    }

    #[test]
    fn changed_or_removed_node_cancels_old_address_stream_and_rejects_events() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for address in [Some("10.0.0.2"), None] {
            let (mut controller, old_stream, worker) = active_fake_stream(&runtime);
            let request = controller.identity.snapshot();
            assert!(controller.apply(Event::Snapshot(request, Ok(node_snapshot(address)))));
            let target = controller.identity.target.as_ref().unwrap();
            assert_eq!(target.address.as_deref(), address);
            assert_eq!(target.node.as_deref(), address.map(|_| "node-a"));
            assert_eq!(target.context, "prod");
            assert!(!controller.streaming);
            assert!(controller.logs.is_none());
            assert!(!controller.apply(Event::Lines(
                old_stream.clone(),
                vec!["info stale address".into()],
                0
            )));
            assert!(!controller.apply(Event::StreamEnded(old_stream, "old failure".into())));
            assert!(runtime.block_on(worker).unwrap_err().is_cancelled());
        }
    }

    #[test]
    fn unchanged_node_address_preserves_active_stream_and_retained_logs() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (mut controller, stream, worker) = active_fake_stream(&runtime);
        let request = controller.identity.snapshot();
        assert!(controller.apply(Event::Snapshot(
            request,
            Ok(node_snapshot(Some("10.0.0.1")))
        )));
        assert!(controller.streaming);
        assert!(!worker.is_finished());
        assert!(controller.apply(Event::Lines(stream, vec!["info still live".into()], 0)));
        assert_eq!(
            controller
                .logs
                .as_ref()
                .unwrap()
                .logs
                .buffer()
                .entries()
                .len(),
            2
        );
        drop(controller);
        assert!(runtime.block_on(worker).unwrap_err().is_cancelled());
    }

    #[test]
    fn failed_refresh_retains_target_stream_and_snapshot_with_stale_warning() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (mut controller, stream, worker) = active_fake_stream(&runtime);
        let request = controller.identity.snapshot();
        let previous_target = controller.identity.target.clone();
        assert!(controller.apply(Event::Snapshot(
            request,
            Err("Snapshot timed out; retained data may be stale".into())
        )));
        assert_eq!(controller.identity.target, previous_target);
        assert_eq!(
            controller.clusters[0]
                .node_ips
                .get("node-a")
                .map(String::as_str),
            Some("10.0.0.1")
        );
        assert!(controller.errors["prod"].contains("stale"));
        assert!(controller.streaming);
        assert!(controller.identity.accepts_stream(&stream));
        assert_eq!(
            controller
                .logs
                .as_ref()
                .unwrap()
                .logs
                .buffer()
                .entries()
                .len(),
            1
        );
        drop(controller);
        assert!(runtime.block_on(worker).unwrap_err().is_cancelled());
    }

    #[test]
    fn mixed_context_results_keep_failed_and_successful_contexts_independent() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut controller = test_controller(&runtime);
        let epoch = controller.identity.config;
        let successful = node_snapshot(Some("10.0.0.1"));
        let failed = ClusterOverview {
            name: "unreachable".into(),
            connection: ClusterConnectionStatus::Unreachable(
                "endpoint connection timed out".into(),
            ),
            ..Default::default()
        };
        assert!(controller.apply(Event::Configured(
            epoch,
            Ok(vec![failed.clone(), successful.clone()])
        )));
        assert!(controller.apply(Event::InitialSnapshot(epoch, "prod".into(), Ok(successful))));
        assert!(controller.apply(Event::InitialSnapshot(
            epoch,
            "unreachable".into(),
            Ok(failed)
        )));
        assert!(controller.apply(Event::LoadFinished(epoch)));
        assert_eq!(controller.clusters.len(), 2);
        assert!(
            controller
                .clusters
                .iter()
                .find(|cluster| cluster.name == "prod")
                .unwrap()
                .connection
                .is_connected()
        );
        assert!(
            controller
                .clusters
                .iter()
                .find(|cluster| cluster.name == "unreachable")
                .unwrap()
                .connection
                .error()
                .unwrap()
                .contains("timed out")
        );
        assert!(controller.config_error.is_none());
        assert!(!controller.loading);
    }

    #[test]
    fn worker_drop_aborts_pending_native_task() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let task = tokio::spawn(std::future::pending::<()>());
            let worker = Worker(Some(task.abort_handle()));
            drop(worker);
            assert!(task.await.unwrap_err().is_cancelled());
        });
    }

    #[test]
    fn headless_mount_and_missing_config_error() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let options = DioxusOptions {
            config_path: Some(PathBuf::from(
                "/nonexistent-talos-pilot-dioxus-smoke/talosconfig",
            )),
            context: None,
            tail: 100,
            maintenance_endpoint: None,
        };
        let bootstrap = Bootstrap {
            options,
            runtime: runtime.handle().clone(),
        };
        let mut dom = VirtualDom::new(app);
        dom.provide_root_context(bootstrap.clone());
        dom.rebuild_in_place();
        let mounted = dom.in_scope(ScopeId::APP, consume_context::<Signal<Controller>>);
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                while mounted.peek().loading {
                    dom.wait_for_work().await;
                    let _ = dom.render_immediate_to_vec();
                }
            })
            .await
            .expect("missing configuration should reach the mounted error state");
        });
        assert!(
            mounted
                .peek()
                .config_error
                .as_deref()
                .unwrap()
                .contains("failed to load talosconfig")
        );
        assert!(mounted.peek().clusters.is_empty());
        let mut mounted = mounted;
        mounted.write().set_kubeconfig_mode(KubeconfigMode::File);
        mounted
            .write()
            .edit_kubeconfig_path("/nonexistent-talos-pilot-dioxus-smoke/kubeconfig".into());
        let _ = dom.render_immediate_to_vec();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                while mounted.peek().kubeconfig_parsing {
                    dom.wait_for_work().await;
                    let _ = dom.render_immediate_to_vec();
                }
            })
            .await
            .expect("file inspection should reach the mounted error state");
        });
        assert!(mounted.peek().kubeconfig_error.is_some());
        assert!(mounted.peek().kubeconfig_draft().is_none());
        assert_eq!(
            mounted.peek().kubeconfig_applied,
            KubeconfigSelection::Automatic
        );
        mounted.write().edit_kubeconfig_path(String::new());
        let _ = dom.render_immediate_to_vec();
        assert!(mounted.peek().kubeconfig_file_path().is_none());
        let picker = runtime.spawn(std::future::pending::<()>());
        let parser = runtime.spawn(std::future::pending::<()>());
        mounted
            .write()
            .kubeconfig_picker_worker
            .replace(picker.abort_handle());
        mounted
            .write()
            .kubeconfig_parser_worker
            .replace(parser.abort_handle());
        drop(dom);
        assert!(runtime.block_on(picker).unwrap_err().is_cancelled());
        assert!(runtime.block_on(parser).unwrap_err().is_cancelled());
    }

    #[test]
    fn automatic_refresh_is_selected_context_bounded_and_pauseable() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut controller = test_controller(&runtime);
        controller.clusters = vec![node_snapshot(Some("10.0.0.1"))];
        controller.select(Target {
            context: "prod".into(),
            node: None,
            address: None,
        });
        let due = controller.next_refresh;
        assert!(!controller.auto_refresh_due(due - Duration::from_millis(1)));
        assert!(controller.auto_refresh_due(due));
        controller.auto_refresh = false;
        assert!(!controller.auto_refresh_due(due));
        controller.auto_refresh = true;
        controller.refreshing = true;
        assert!(!controller.auto_refresh_due(due));
        controller.refreshing = false;
        controller.loading = true;
        assert!(!controller.auto_refresh_due(due));
        controller.loading = false;
        controller.tick(due);
        assert!(controller.refreshing);
        assert!(controller.snapshot_worker.0.is_some());
        controller.snapshot_worker.cancel();
    }

    #[test]
    fn failed_or_stale_snapshot_never_advances_success_or_revision() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut controller = test_controller(&runtime);
        controller.clusters = vec![node_snapshot(Some("10.0.0.1"))];
        controller.select(Target {
            context: "prod".into(),
            node: Some("node-a".into()),
            address: Some("10.0.0.1".into()),
        });
        let first = controller.identity.snapshot();
        assert!(controller.apply(Event::Snapshot(first, Ok(node_snapshot(Some("10.0.0.1"))))));
        let success = controller.successes["prod"];
        let revision = controller.revisions["prod"];
        let stale = controller.identity.snapshot();
        let current = controller.identity.snapshot();
        assert!(!controller.apply(Event::Snapshot(stale, Ok(node_snapshot(Some("10.0.0.9"))))));
        assert!(controller.apply(Event::Snapshot(
            current,
            Err("offline; retained snapshot".into())
        )));
        assert_eq!(controller.successes["prod"], success);
        assert_eq!(controller.revisions["prod"], revision);
        assert_eq!(
            controller
                .identity
                .target
                .as_ref()
                .unwrap()
                .address
                .as_deref(),
            Some("10.0.0.1")
        );
        assert!(controller.errors["prod"].contains("offline"));
        controller.edit_path("/edited/not-applied".into());
        assert_ne!(
            controller.applied_config_path,
            Some(PathBuf::from("/edited/not-applied"))
        );
    }

    #[test]
    fn log_navigation_requires_authoritative_same_context_name_and_address() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut controller = test_controller(&runtime);
        let mut snapshot = node_snapshot(Some("10.0.0.1"));
        snapshot.node_ips.insert("node-b".into(), "10.0.0.2".into());
        controller.clusters = vec![snapshot];
        controller.select(Target {
            context: "prod".into(),
            node: Some("node-a".into()),
            address: Some("10.0.0.1".into()),
        });
        let original = controller.identity.target.clone();
        for (node, address) in [
            (Some("node-a"), Some("10.0.0.9")),
            (Some("node-b"), None),
            (None, Some("10.0.0.2")),
            (Some("foreign"), Some("10.0.0.2")),
        ] {
            assert!(!controller.navigate_logs(LogRequest {
                node: node.map(str::to_owned),
                address: address.map(str::to_owned),
                services: vec!["kubelet".into()]
            }));
            assert_eq!(controller.identity.target, original);
        }
        assert!(controller.navigate_logs(LogRequest {
            node: None,
            address: None,
            services: vec!["kubelet".into()]
        }));
        assert_eq!(controller.identity.target, original);
        assert!(controller.navigate_logs(LogRequest {
            node: Some("node-b".into()),
            address: Some("10.0.0.2".into()),
            services: vec!["etcd".into(), "etcd".into()]
        }));
        assert_eq!(
            controller.identity.target.as_ref().unwrap().node.as_deref(),
            Some("node-b")
        );
        assert_eq!(controller.requested_logs, vec!["etcd"]);
        assert_eq!(controller.node_tab, NodeTab::Logs);
    }

    #[test]
    fn headless_busy_operation_locks_navigation_settings_and_snapshot_rebind() {
        struct ResetBusy;
        impl Drop for ResetBusy {
            fn drop(&mut self) {
                TEST_BUSY.with(|busy| busy.set(false));
            }
        }
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut dom = VirtualDom::new(app);
        dom.provide_root_context(Bootstrap {
            options: DioxusOptions {
                config_path: Some("/nonexistent-shell-busy/talosconfig".into()),
                context: None,
                tail: 100,
                maintenance_endpoint: None,
            },
            runtime: runtime.handle().clone(),
        });
        dom.rebuild_in_place();
        let mut mounted = dom.in_scope(ScopeId::APP, consume_context::<Signal<Controller>>);
        {
            let mut controller = mounted.write();
            controller.load_worker.cancel();
            controller.loading = false;
            controller.clusters = vec![node_snapshot(Some("10.0.0.1"))];
            controller.select(Target {
                context: "prod".into(),
                node: Some("node-a".into()),
                address: Some("10.0.0.1".into()),
            });
            controller.node_tab = NodeTab::Operations;
        }
        TEST_BUSY.with(|busy| busy.set(true));
        let _reset = ResetBusy;
        {
            let mut controller = mounted.write();
            let target = controller.identity.target.clone();
            let epoch = controller.identity.config;
            let path = controller.path.clone();
            controller.edit_path("/must-not-change".into());
            controller.set_kubeconfig_mode(KubeconfigMode::File);
            controller.load();
            controller.select(Target {
                context: "prod".into(),
                node: None,
                address: None,
            });
            controller.select_node_tab(NodeTab::Logs);
            controller.apply_snapshot("prod".into(), Ok(node_snapshot(Some("10.0.0.9"))));
            let picker = controller.picker_generation;
            let kube_picker = controller.kubeconfig_generation;
            assert!(controller.apply(Event::Picked(picker, Some("/late-dialog/path".into()))));
            assert!(controller.apply(Event::KubeconfigPicked(
                kube_picker,
                Some("/late-dialog/kubeconfig".into())
            )));
            assert!(controller.kubeconfig_path.is_empty());
            assert_eq!(controller.path, path);
            assert_eq!(controller.identity.config, epoch);
            assert_eq!(controller.identity.target, target);
            assert_eq!(controller.node_tab, NodeTab::Operations);
            assert!(controller.kubeconfig_mode == KubeconfigMode::Automatic);
            assert!(!controller.auto_refresh_due(Instant::now() + REFRESH_INTERVAL * 10));
        }
        let _ = dom.render_immediate_to_vec();
        let text = rendered_text(&dom, dom.base_scope().root_node());
        assert!(
            text.iter()
                .any(|value| value.contains("navigation are locked"))
        );
    }

    #[test]
    fn native_close_waits_for_busy_worker_and_requests_only_cooperative_cancel() {
        let mut state = CloseState::default();
        let cancellations = std::cell::Cell::new(0);
        let cancel = || cancellations.set(cancellations.get() + 1);
        assert!(!state.poll(true));
        assert_eq!(state.request(true, cancel), CloseAction::CancelAndWait);
        assert_eq!(cancellations.get(), 1);
        assert!(state.waiting());
        // The cancellation request is not proof the RPC/compensation/audit ended.
        for _ in 0..5 {
            assert!(!state.poll(true));
        }
        assert!(state.waiting());
        assert!(state.poll(false));
        assert!(!state.waiting());
        assert!(!state.poll(false)); // Queue exactly one automatic close.
    }

    #[test]
    fn idle_native_close_is_immediate_and_never_cancels_an_unrelated_operation() {
        let mut state = CloseState::default();
        assert_eq!(
            state.request(false, || panic!("idle close must not cancel")),
            CloseAction::Close
        );
        assert!(!state.waiting());
        assert!(!state.poll(false));
    }

    #[test]
    fn repeated_close_and_new_busy_transition_are_rechecked_before_root_exit() {
        let mut state = CloseState::default();
        let cancellations = std::cell::Cell::new(0);
        let cancel = || cancellations.set(cancellations.get() + 1);
        assert_eq!(state.request(true, cancel), CloseAction::CancelAndWait);
        assert_eq!(state.request(true, cancel), CloseAction::CancelAndWait);
        assert!(state.poll(false));
        // A queued CloseWindow event must re-check busy rather than treating
        // the earlier idle poll as authorization to drop a new runtime worker.
        assert_eq!(state.request(true, cancel), CloseAction::CancelAndWait);
        assert!(!state.poll(true));
        assert_eq!(cancellations.get(), 3);
        assert!(state.poll(false));
        assert_eq!(
            state.request(false, || panic!("finished worker must not cancel")),
            CloseAction::Close
        );
    }

    #[test]
    fn native_close_provider_is_send_sync_but_shares_only_launch_thread_state() {
        fn assert_send_sync<T: Send + Sync>(_: &T) {}
        let close = NativeClose::default();
        let provider = close.context_provider();
        assert_send_sync(&provider);
        let context = provider().downcast::<NativeClose>().unwrap();
        assert!(Rc::ptr_eq(&close.0, &context.0));
        assert_eq!(context.request(false), CloseAction::Close);
        assert!(close.0.borrow().state.queued);
        assert_eq!(Rc::strong_count(&close.0), 2);
        let weak = Rc::downgrade(&close.0);
        drop(context);
        drop(close);
        // Neither the Send + Sync provider nor the thread-local registration
        // keeps idle close state (or a native window) alive.
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn native_close_provider_rejects_a_different_thread() {
        let close = NativeClose::default();
        let provider = close.context_provider();
        assert!(
            std::thread::spawn(move || {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = provider();
                }))
                .is_err()
            })
            .join()
            .unwrap()
        );
        assert_eq!(close.0.borrow().state, CloseState::default());
    }

    #[test]
    fn headless_close_wait_notice_is_reachable_without_a_native_window() {
        fn close_fixture() -> Element {
            rsx! { NativeClosePanel {} }
        }
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let close = NativeClose::default();
        close.0.borrow_mut().state.request(true, || {});
        let mut dom = VirtualDom::new(close_fixture);
        dom.provide_root_context(close);
        dom.provide_root_context(Bootstrap {
            options: DioxusOptions {
                config_path: None,
                context: None,
                tail: 100,
                maintenance_endpoint: None,
            },
            runtime: runtime.handle().clone(),
        });
        TEST_BUSY.with(|busy| busy.set(true));
        struct ResetBusy;
        impl Drop for ResetBusy {
            fn drop(&mut self) {
                TEST_BUSY.with(|busy| busy.set(false));
            }
        }
        let _reset = ResetBusy;
        dom.rebuild_in_place();
        let text = rendered_text(&dom, dom.base_scope().root_node());
        assert!(
            text.iter()
                .any(|value| value.contains("waiting for the mutation to finish safely"))
        );
        assert!(
            text.iter()
                .any(|value| value.contains("Cancel mutation and close when safe"))
        );
    }
}
