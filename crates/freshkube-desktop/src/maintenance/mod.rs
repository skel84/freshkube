//! Maintenance mode: `--insecure --endpoint <ip>`.
//!
//! A node that has no machine configuration yet answers only on Talos's
//! insecure API. This view inspects it (version, disks, volumes), generates a
//! machine configuration for an explicitly chosen install disk, applies the
//! reviewed bytes after a confirmation, then bootstraps etcd after a second
//! one and polls the secure APIs until the cluster answers. It replaces the
//! cluster shell: no talosconfig, no ambient cluster, no kubeconfig.
//!
//! The workflow itself lives in `freshkube_core::maintenance`; this file is
//! the GPUI presentation of it. Anything that changes a node holds the shared
//! operation slot, is confirmed through `mutation::confirm` against a
//! fingerprint of what was reviewed, and runs through core's cooperative
//! cancellation. Generated configurations and talosconfigs contain secrets:
//! they are shown only in the review pane, written only to the output
//! directory the user chose, and never logged.
use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use freshkube_core::maintenance::{
    BOOTSTRAP_POLL_INTERVAL, BootstrapPhase, BootstrapReadinessEvidence, BootstrapSession,
    EtcdReadinessEvidence, InsecureMaintenanceSnapshot, KubernetesReadinessEvidence, MachineRole,
    MaintenanceAction, MaintenanceDraft, MaintenanceError, MaintenanceEvent, ProgressLog,
    READ_TIMEOUT, SourceAvailability, TalosReadinessEvidence, execute_maintenance_action_bounded,
    frozen_review, is_mutation,
};
use futures::future::BoxFuture;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Sizable, TitleBar,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::{runtime::Handle, task::AbortHandle};

use crate::{
    actions,
    mutation::{self, Confirmation, Operations},
    palette::palette,
    screens::{field, mono, page_body, page_scroll, panel},
    ui::{self, MONO_FONT, Tone, dp},
};

mod review;
mod view;

use review::Review;

#[cfg(test)]
mod tests;

/// Cooperative cancellation handed to a runner; checked before each step.
pub(crate) type Cancelled = Arc<dyn Fn() -> bool + Send + Sync>;

/// Executes one maintenance action. Production runs core's bounded executor;
/// tests substitute fakes, so nothing needs a node.
pub(crate) type Runner =
    Arc<dyn Fn(MaintenanceAction, Cancelled) -> BoxFuture<'static, MaintenanceEvent> + Send + Sync>;

pub(crate) fn live_runner() -> Runner {
    Arc::new(|action, cancelled| {
        Box::pin(execute_maintenance_action_bounded(
            action,
            move || cancelled(),
            READ_TIMEOUT,
        ))
    })
}

/// The workflow input fields, one editable entity each.
struct Fields {
    endpoint: Entity<InputState>,
    cluster: Entity<InputState>,
    kubernetes_api: Entity<InputState>,
    output: Entity<InputState>,
    context: Entity<InputState>,
    bootstrap_node: Entity<InputState>,
    kubernetes_node: Entity<InputState>,
    kubeconfig: Entity<InputState>,
}

/// Work in flight: a read or a mutation holding the operation slot.
struct Work {
    mutation: bool,
    label: SharedString,
    cancel: Arc<AtomicBool>,
    /// Reads can be abandoned; a submitted mutation is always awaited.
    abort: Option<AbortHandle>,
}

pub(crate) struct MaintenanceView {
    runtime: Handle,
    runner: Runner,
    draft: MaintenanceDraft,
    fields: Fields,
    session: Option<BootstrapSession>,
    work: Option<Work>,
    /// Which run's result may still land.
    generation: u64,
    auto_poll: bool,
    review: Option<Review>,
    error: Option<String>,
    progress: ProgressLog,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
    _tick: Task<()>,
}

impl MaintenanceView {
    pub(crate) fn new(
        endpoint: String,
        runtime: Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_runner(endpoint, runtime, live_runner(), window, cx)
    }

    pub(crate) fn with_runner(
        endpoint: String,
        runtime: Handle,
        runner: Runner,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = MaintenanceDraft::new(&endpoint);
        let input = |value: &str, placeholder: &'static str, window: &mut Window, cx: &mut App| {
            let state = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            state.update(cx, |input, cx| {
                input.set_value(value.to_owned(), window, cx)
            });
            state
        };
        let fields = Fields {
            endpoint: input(&draft.endpoint, "192.0.2.10", window, cx),
            cluster: input(&draft.cluster, "Cluster name", window, cx),
            kubernetes_api: input(&draft.kubernetes_api, "https://192.0.2.10:6443", window, cx),
            output: input(&draft.output, "Output directory", window, cx),
            context: input(&draft.context, "Talos context", window, cx),
            bootstrap_node: input(&draft.bootstrap_node, "192.0.2.10", window, cx),
            kubernetes_node: input(&draft.kubernetes_node, "Optional", window, cx),
            kubeconfig: input(&draft.kubeconfig, "Optional", window, cx),
        };
        let mut subscriptions = Vec::new();
        // Bound after the initial values are set, so only edits reach the draft.
        macro_rules! bind {
            ($field:ident, $apply:expr) => {
                subscriptions.push(cx.subscribe_in(
                    &fields.$field,
                    window,
                    |view, state, event, window, cx| {
                        if matches!(event, InputEvent::Change) {
                            let value = state.read(cx).value().to_string();
                            #[allow(clippy::redundant_closure_call)]
                            ($apply)(&mut view.draft, value);
                            view.sync_context(window, cx);
                        }
                    },
                ));
            };
        }
        bind!(endpoint, |d: &mut MaintenanceDraft, v| d.endpoint = v);
        bind!(cluster, |d: &mut MaintenanceDraft, v| d.set_cluster_name(v));
        bind!(kubernetes_api, |d: &mut MaintenanceDraft, v| d
            .kubernetes_api =
            v);
        bind!(output, |d: &mut MaintenanceDraft, v| d.output = v);
        bind!(context, |d: &mut MaintenanceDraft, v| d.context = v);
        bind!(bootstrap_node, |d: &mut MaintenanceDraft, v| d
            .bootstrap_node =
            v);
        bind!(kubernetes_node, |d: &mut MaintenanceDraft, v| d
            .kubernetes_node =
            v);
        bind!(kubeconfig, |d: &mut MaintenanceDraft, v| d.kubeconfig = v);
        // The operation slot's holder shows in the footer.
        let operations = Operations::global(cx);
        subscriptions.push(cx.observe(&operations, |_, _, cx| cx.notify()));
        subscriptions.push(cx.observe_window_appearance(window, |_, window, cx| {
            gpui_kit::component::Theme::sync_system_appearance(Some(window), cx);
        }));
        window.on_window_should_close(cx, mutation::may_close);
        let tick = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(BOOTSTRAP_POLL_INTERVAL)
                    .await;
                if this
                    .update_in(cx, |view, window, cx| view.automatic_poll(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            runtime,
            runner,
            draft,
            fields,
            session: None,
            work: None,
            generation: 0,
            auto_poll: false,
            review: None,
            error: None,
            progress: ProgressLog::default(),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
            _tick: tick,
        }
    }

    /// Keeps the context field in step when renaming the cluster moved it.
    fn sync_context(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let context = self.draft.context.clone();
        if self.fields.context.read(cx).value() != context.as_str() {
            self.fields
                .context
                .update(cx, |input, cx| input.set_value(context, window, cx));
        }
    }

    fn slot_busy(cx: &App) -> Option<SharedString> {
        Operations::current(cx).map(|running| running.label)
    }

    /// Whether the node, role and credentials form is frozen.
    fn locked(&self, cx: &App) -> bool {
        self.work.is_some() || self.session.is_some() || Self::slot_busy(cx).is_some()
    }

    fn record(&mut self, message: impl Into<String>) {
        self.progress.record(message);
    }

    fn reduce(&mut self, event: MaintenanceEvent) {
        let Some(session) = &self.session else {
            return;
        };
        match session.reduce(event) {
            Ok(next) => {
                self.progress.record(next.progress_line());
                self.session = Some(next);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    // ---- workflow steps ---------------------------------------------------

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_some() || self.work.is_some() {
            return;
        }
        if let Some(label) = Self::slot_busy(cx) {
            self.error = Some(format!(
                "{label} is still running; try again when it has finished."
            ));
            cx.notify();
            return;
        }
        let plan = match self.draft.plan() {
            Ok(plan) => plan,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let session = BootstrapSession::new(plan);
        let Some(action) = session.initial_collection_action() else {
            return;
        };
        self.review = None;
        self.progress = ProgressLog::default();
        self.record(
            "Collecting insecure Talos version, hardware/disks and volumes; no credentials used.",
        );
        self.auto_poll = true;
        self.run(action, Some(session), window, cx);
    }

    fn select_disk(&mut self, device_path: String, cx: &mut Context<Self>) {
        if self.work.is_some() {
            return;
        }
        let Some(session) = &self.session else {
            return;
        };
        match session.select_install_target(device_path) {
            Ok(event) => {
                self.error = None;
                self.reduce(event);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn generate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = &self.session else {
            return;
        };
        match session.begin_configuration_generation() {
            Ok((next, action)) => {
                self.run(action, Some(next), window, cx);
            }
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    /// Reads the generated configuration for review. These bytes, and only
    /// these, are what apply sends.
    fn read_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.work.is_some() {
            return;
        }
        let Some(path) = self
            .session
            .as_ref()
            .and_then(|session| session.generated_configuration.as_ref())
            .map(|configuration| configuration.machine_config_path.clone())
        else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        let handle = self.runtime.spawn(async move {
            tokio::task::spawn_blocking(move || frozen_review(&path))
                .await
                .map_err(|error| error.to_string())?
        });
        self.work = Some(Work {
            mutation: false,
            label: "Reading the generated configuration".into(),
            cancel: Arc::new(AtomicBool::new(false)),
            abort: Some(handle.abort_handle()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = handle
                .await
                .unwrap_or_else(|error| Err(format!("Review worker stopped: {error}")));
            _ = this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.work = None;
                match result {
                    Ok(text) => {
                        view.review = Some(Review::new(text));
                        view.error = None;
                    }
                    Err(error) => {
                        view.review = None;
                        view.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn prepare_apply(&mut self, cx: &mut Context<Self>) {
        if self.work.is_some() {
            return;
        }
        let (Some(session), Some(review)) = (&self.session, &self.review) else {
            self.error = Some("Read and review the configuration before requesting apply".into());
            cx.notify();
            return;
        };
        match session.request_reviewed_configuration_application(review.text.clone()) {
            Ok((next, _)) => {
                self.error = None;
                self.progress.record(next.progress_line());
                self.session = Some(next);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    /// What the apply confirmation was decided on. Changes if the target,
    /// disk, configuration file or reviewed bytes change, or the phase moves.
    fn apply_fingerprint(&self) -> Option<u64> {
        let session = self.session.as_ref()?;
        if session.phase != BootstrapPhase::AwaitingApplyConfirmation {
            return None;
        }
        let confirmation = session.pending_confirmation.as_ref()?;
        let review = self.review.as_ref()?;
        Some(actions::fingerprint((
            confirmation.endpoint().address(),
            confirmation.install_target().device_path(),
            confirmation.install_target().disk_id(),
            confirmation.config_path(),
            &review.text,
        )))
    }

    fn bootstrap_fingerprint(&self) -> Option<u64> {
        let session = self.session.as_ref()?;
        session.bootstrap_confirmation_phrase()?;
        Some(actions::fingerprint((
            &session.plan.authenticated_target.talos_context,
            session.plan.authenticated_target.node.address(),
        )))
    }

    /// Opens the destructive install confirmation.
    fn confirm_apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(fingerprint), Some(session)) = (self.apply_fingerprint(), &self.session) else {
            return;
        };
        let Some(confirmation) = session.pending_confirmation.as_ref() else {
            return;
        };
        let target = confirmation.install_target();
        let role = match session.plan.role {
            MachineRole::ControlPlane => "Control plane",
            MachineRole::Worker => "Worker",
        };
        let node = confirmation.endpoint().to_string();
        let this = cx.weak_entity();
        let (current, run) = (this.clone(), this);
        mutation::confirm(
            Confirmation {
                title: format!("Install Talos on {node}?").into(),
                summary: format!(
                    "Applies the reviewed machine configuration to {node} over the insecure Talos API. The node installs Talos and reboots."
                )
                .into(),
                facts: vec![
                    ("Node".into(), node.into()),
                    (
                        "Install disk".into(),
                        format!("{} · {}", target.device_path(), target.disk_id()).into(),
                    ),
                    ("Role".into(), role.into()),
                    (
                        "Configuration".into(),
                        confirmation.config_path().display().to_string().into(),
                    ),
                ],
                warnings: vec![
                    "This can erase the selected install disk.".into(),
                    "The target, role, disk and credentials are locked once applied.".into(),
                ],
                confirm_label: "Install and apply".into(),
                destructive: true,
                fingerprint,
                current: std::rc::Rc::new(move |cx: &App| {
                    current
                        .upgrade()
                        .and_then(|view| view.read(cx).apply_fingerprint())
                }),
                on_confirm: std::rc::Rc::new(move |window: &mut Window, cx: &mut App| {
                    if let Some(view) = run.upgrade() {
                        view.update(cx, |view, cx| view.begin_apply(window, cx));
                    }
                }),
            },
            window,
            cx,
        );
    }

    fn begin_apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = &self.session else {
            return;
        };
        let Some(confirmation) = session.pending_confirmation.clone() else {
            return;
        };
        match session.confirm_configuration_application(confirmation) {
            Ok((next, action)) => {
                if self.run(action, Some(next), window, cx) {
                    // The reviewed bytes are sent; stop holding secrets here.
                    self.review = None;
                }
            }
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn confirm_bootstrap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(fingerprint), Some(session)) = (self.bootstrap_fingerprint(), &self.session)
        else {
            return;
        };
        let target = &session.plan.authenticated_target;
        let node = target.node.to_string();
        let context = target.talos_context.clone();
        let this = cx.weak_entity();
        let (current, run) = (this.clone(), this);
        mutation::confirm(
            Confirmation {
                title: format!("Bootstrap etcd on {node}?").into(),
                summary: format!(
                    "Initializes etcd and the cluster on {node} through its secure Talos API."
                )
                .into(),
                facts: vec![
                    ("Node".into(), node.into()),
                    ("Talos context".into(), context.into()),
                    ("Role".into(), "Control plane".into()),
                ],
                warnings: vec![
                    "Bootstrap must run once, on the intended first control-plane node only."
                        .into(),
                ],
                confirm_label: "Bootstrap this node".into(),
                destructive: true,
                fingerprint,
                current: std::rc::Rc::new(move |cx: &App| {
                    current
                        .upgrade()
                        .and_then(|view| view.read(cx).bootstrap_fingerprint())
                }),
                on_confirm: std::rc::Rc::new(move |window: &mut Window, cx: &mut App| {
                    if let Some(view) = run.upgrade() {
                        view.update(cx, |view, cx| view.begin_bootstrap(window, cx));
                    }
                }),
            },
            window,
            cx,
        );
    }

    fn begin_bootstrap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = &self.session else {
            return;
        };
        if session.plan.role != MachineRole::ControlPlane {
            self.error = Some("Bootstrap is only valid for a control-plane node".into());
            cx.notify();
            return;
        }
        match session.begin_bootstrap() {
            Ok((next, action)) => {
                self.run(action, Some(next), window, cx);
            }
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn poll_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.work.is_some() {
            return;
        }
        let Some(action) = self
            .session
            .as_ref()
            .and_then(BootstrapSession::polling_action)
        else {
            return;
        };
        self.auto_poll = true;
        self.run(action, None, window, cx);
    }

    fn automatic_poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.auto_poll
            && self.work.is_none()
            && self
                .session
                .as_ref()
                .and_then(BootstrapSession::polling_action)
                .is_some()
        {
            self.poll_now(window, cx);
        }
    }

    /// Stops at the next checkpoint. A request already sent to the node still
    /// completes and its real result is kept; automatic polling stops.
    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.auto_poll = false;
        let mut idle = true;
        if let Some(work) = &mut self.work {
            idle = false;
            work.cancel.store(true, Ordering::SeqCst);
            if let Some(abort) = work.abort.take() {
                abort.abort();
            }
            if work.mutation {
                Operations::global(cx).update(cx, |operations, cx| operations.request_cancel(cx));
            }
        }
        if idle {
            self.reduce(MaintenanceEvent::Cancelled);
        }
        self.record(
            "Cancellation requested. Submitted mutations are not aborted; their actual result is retained. Automatic polling stopped.",
        );
        cx.notify();
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        if self.work.is_some() {
            return;
        }
        self.session = None;
        self.review = None;
        self.error = None;
        self.auto_poll = false;
        self.progress = ProgressLog::default();
        cx.notify();
    }

    fn browse_output(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked(cx) {
            return;
        }
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Use directory".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = selection.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            _ = this.update_in(cx, |view, window, cx| {
                if view.locked(cx) {
                    return;
                }
                match path.to_str() {
                    Some(path) => view
                        .fields
                        .output
                        .update(cx, |input, cx| input.set_value(path.to_owned(), window, cx)),
                    None => {
                        view.error = Some("The output directory must be a valid UTF-8 path; the selected path was not changed or approximated".into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs `action`. A mutation first takes the shared operation slot; when
    /// it is busy nothing changes and this returns `false`. `next` is the
    /// session to move to once the action has really started.
    fn run(
        &mut self,
        action: MaintenanceAction,
        next: Option<BootstrapSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.work.is_some() {
            return false;
        }
        let mutation = is_mutation(&action);
        let read_label = match &action {
            MaintenanceAction::CollectInsecure { .. } => {
                "Reading the node over the insecure Talos API…"
            }
            _ => "Polling the secure Talos, etcd and Kubernetes APIs…",
        };
        let operations = Operations::global(cx);
        let ticket = if mutation {
            let label = match &action {
                MaintenanceAction::GenerateConfiguration(_) => {
                    "Maintenance: generating configuration"
                }
                MaintenanceAction::ApplyConfiguration(_) => "Maintenance: applying configuration",
                _ => "Maintenance: bootstrapping etcd",
            };
            match operations.update(cx, |operations, cx| operations.begin(label, cx)) {
                Ok(ticket) => Some(ticket),
                Err(running) => {
                    self.error = Some(format!(
                        "{} is still running; try again when it has finished.",
                        running.label
                    ));
                    cx.notify();
                    return false;
                }
            }
        } else {
            None
        };
        if let Some(next) = next {
            self.progress.record(next.progress_line());
            self.session = Some(next);
        }
        self.error = None;
        self.generation += 1;
        let generation = self.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let slot = ticket.as_ref().map(|ticket| ticket.cancellation());
        let cancelled: Cancelled = Arc::new(move || {
            flag.load(Ordering::SeqCst) || slot.as_ref().is_some_and(|cancelled| cancelled())
        });
        let future = (self.runner)(action, cancelled);
        let handle = self.runtime.spawn(future);
        self.work = Some(Work {
            mutation,
            label: if mutation {
                "Changing the node".into()
            } else {
                read_label.into()
            },
            cancel,
            // A submitted mutation is never abandoned.
            abort: (!mutation).then(|| handle.abort_handle()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = handle.await;
            // Release the slot first, whatever happened to the view.
            if let Some(ticket) = ticket {
                _ = cx.update(|_, cx| {
                    operations.update(cx, |operations, cx| operations.finish(ticket, cx))
                });
            }
            _ = this.update(cx, |view, cx| view.finish(generation, result.ok(), cx));
        })
        .detach();
        cx.notify();
        true
    }

    /// Applies a finished run's result. `None` means the worker ended without
    /// one: abandoned (cancelled) or stopped.
    fn finish(&mut self, generation: u64, event: Option<MaintenanceEvent>, cx: &mut Context<Self>) {
        if self.generation != generation {
            return;
        }
        let work = self.work.take();
        match event {
            Some(event) => self.reduce(event),
            None => {
                let cancelled = work
                    .as_ref()
                    .is_some_and(|work| work.cancel.load(Ordering::SeqCst));
                if cancelled {
                    self.reduce(MaintenanceEvent::Cancelled);
                } else {
                    let message =
                        "The maintenance worker stopped; the outcome is unknown".to_owned();
                    self.reduce(MaintenanceEvent::OperationFailed(
                        MaintenanceError::ConfigurationApplicationFailed {
                            message: message.clone(),
                        },
                    ));
                    self.error = Some(message);
                }
            }
        }
        cx.notify();
    }
}
