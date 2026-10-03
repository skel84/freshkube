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

/// Display width of one review row. Longer lines wrap into further rows; the
/// reviewed text itself is never altered.
const REVIEW_ROW_CHARS: usize = 96;
const REVIEW_ROW_HEIGHT: f32 = 18.;
const MAX_DISKS: usize = 256;

/// The text frozen for review, split into display rows.
struct Review {
    text: String,
    rows: Vec<Range<usize>>,
    scroll: UniformListScrollHandle,
}

impl Review {
    fn new(text: String) -> Self {
        Self {
            rows: review_rows(&text),
            text,
            scroll: UniformListScrollHandle::new(),
        }
    }
}

/// Splits `text` into byte ranges of at most [`REVIEW_ROW_CHARS`] characters,
/// one or more per line. Together the ranges cover every character except the
/// line breaks, so nothing the reviewer approves is hidden.
fn review_rows(text: &str) -> Vec<Range<usize>> {
    let mut rows = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        if body.is_empty() {
            rows.push(offset..offset);
        } else {
            let mut start = 0;
            let mut count = 0;
            for (ix, _) in body.char_indices() {
                if count == REVIEW_ROW_CHARS {
                    rows.push(offset + start..offset + ix);
                    start = ix;
                    count = 0;
                }
                count += 1;
            }
            rows.push(offset + start..offset + body.len());
        }
        offset += line.len();
    }
    rows
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

// ---- presentation -----------------------------------------------------------

fn phase_label(phase: &BootstrapPhase) -> (&'static str, Tone) {
    match phase {
        BootstrapPhase::CollectingInsecureData => ("Collecting node data", Tone::Accent),
        BootstrapPhase::SelectingInstallTarget => ("Choose an install disk", Tone::Accent),
        BootstrapPhase::Configuring => ("Ready to generate configuration", Tone::Accent),
        BootstrapPhase::GeneratingConfiguration => ("Generating configuration", Tone::Accent),
        BootstrapPhase::ConfigurationReady => ("Configuration ready for review", Tone::Accent),
        BootstrapPhase::AwaitingApplyConfirmation => ("Awaiting install confirmation", Tone::Warn),
        BootstrapPhase::ApplyingConfiguration => ("Applying configuration", Tone::Warn),
        BootstrapPhase::WaitingForTalos => ("Waiting for the secure Talos API", Tone::Unknown),
        BootstrapPhase::ReadyToBootstrap => ("Ready to bootstrap", Tone::Accent),
        BootstrapPhase::Bootstrapping => ("Bootstrapping", Tone::Warn),
        BootstrapPhase::WaitingForKubernetes => ("Waiting for etcd and Kubernetes", Tone::Unknown),
        BootstrapPhase::Complete => ("Complete", Tone::Good),
        BootstrapPhase::Cancelled => ("Cancelled", Tone::Outline),
        BootstrapPhase::Failed(_) => ("Failed", Tone::Crit),
    }
}

/// Readiness evidence as labelled rows. A source that didn't answer is
/// unknown, never failed and never healthy.
fn readiness_rows(evidence: &BootstrapReadinessEvidence) -> Vec<(&'static str, Tone, String)> {
    let mut rows = Vec::new();
    match &evidence.talos {
        TalosReadinessEvidence::Available { versions, etcd } => {
            let versions = versions
                .iter()
                .map(|version| format!("{} {}", version.node, version.version))
                .collect::<Vec<_>>()
                .join(", ");
            rows.push((
                "Talos API",
                Tone::Good,
                if versions.is_empty() {
                    "Answering".to_owned()
                } else {
                    format!("Answering · {versions}")
                },
            ));
            rows.push(match etcd {
                EtcdReadinessEvidence::Ready { responding_members } => (
                    "etcd",
                    Tone::Good,
                    format!("{responding_members} responding member(s), leader reported"),
                ),
                EtcdReadinessEvidence::NotReady {
                    responding_members,
                    errors,
                } => (
                    "etcd",
                    Tone::Warn,
                    if errors.is_empty() {
                        format!("Not ready · {responding_members} responding member(s)")
                    } else {
                        format!(
                            "Not ready · {responding_members} responding member(s) · {}",
                            errors.join("; ")
                        )
                    },
                ),
                EtcdReadinessEvidence::Unavailable { reason } => {
                    ("etcd", Tone::Unknown, format!("Unknown · {reason}"))
                }
            });
        }
        TalosReadinessEvidence::Unavailable { reason } => {
            rows.push(("Talos API", Tone::Unknown, format!("Unknown · {reason}")));
            rows.push(("etcd", Tone::Unknown, "Not checked".to_owned()));
        }
    }
    rows.push(match &evidence.kubernetes {
        KubernetesReadinessEvidence::Ready { nodes, .. } => (
            "Kubernetes",
            Tone::Good,
            format!("{} node(s) reported, required node Ready", nodes.len()),
        ),
        KubernetesReadinessEvidence::NotReady { nodes, .. } => (
            "Kubernetes",
            Tone::Warn,
            format!(
                "API answered, required node not Ready · {} node(s)",
                nodes.len()
            ),
        ),
        KubernetesReadinessEvidence::Unavailable { reason, .. } => {
            ("Kubernetes", Tone::Unknown, format!("Unknown · {reason}"))
        }
        KubernetesReadinessEvidence::NotChecked => {
            ("Kubernetes", Tone::Unknown, "Not checked yet".to_owned())
        }
    });
    rows
}

fn heading(text: &'static str) -> Div {
    div()
        .font_weight(ui::HEADING_WEIGHT)
        .text_size(dp(16.))
        .child(text)
}

impl MaintenanceView {
    fn labelled(&self, label: &'static str, input: impl IntoElement, cx: &App) -> Div {
        v_flex().gap_1().child(ui::caption(label, cx)).child(input)
    }

    fn input(
        &self,
        label: &'static str,
        id: &'static str,
        state: &Entity<InputState>,
        locked: bool,
        cx: &App,
    ) -> Div {
        self.labelled(
            label,
            Input::new(state)
                .id(id)
                .aria_label(label)
                .small()
                .disabled(locked),
            cx,
        )
    }

    fn title_bar(&self, cx: &App) -> AnyElement {
        let p = palette(cx);
        TitleBar::new()
            // The app header's height, where the window puts its traffic lights.
            .h(dp(52.))
            .when(cfg!(target_os = "macos"), |bar| bar.pl(dp(84.)))
            .child(
                h_flex()
                    .id("maint-title")
                    .test_support()
                    .role(Role::Status)
                    .aria_label("Maintenance mode: insecure Talos access")
                    .gap_2()
                    .min_w_0()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(dp(12.5))
                            .text_color(p.muted)
                            .child(self.draft.endpoint.clone()),
                    )
                    .child(div().text_color(p.faint).child("/"))
                    .child(
                        div()
                            .text_size(dp(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Maintenance"),
                    )
                    .child(ui::tag(Tone::Warn, None, "Insecure", cx)),
            )
            .into_any_element()
    }

    fn form(&self, cx: &mut Context<Self>) -> Div {
        let locked = self.locked(cx);
        let role = self.draft.role;
        let role_button = |id: &'static str, label: &'static str, value: MachineRole| {
            Button::new(id)
                .outline()
                .small()
                .label(label)
                .selected(role == value)
                .disabled(locked)
                .on_click(cx.listener(move |view, _, _, cx| {
                    if !view.locked(cx) {
                        view.draft.role = value;
                        cx.notify();
                    }
                }))
        };
        panel(cx)
            .p_4()
            .gap_3()
            .child(heading("Node and cluster"))
            .child(self.input(
                "Insecure Talos endpoint",
                "maint-endpoint",
                &self.fields.endpoint,
                locked,
                cx,
            ))
            .child(self.input(
                "Cluster name",
                "maint-cluster",
                &self.fields.cluster,
                locked,
                cx,
            ))
            .child(self.input(
                "Kubernetes API HTTPS endpoint",
                "maint-kubernetes-api",
                &self.fields.kubernetes_api,
                locked,
                cx,
            ))
            .child(self.input(
                "Configuration output directory (generation overwrites files here)",
                "maint-output",
                &self.fields.output,
                locked,
                cx,
            ))
            .child(
                Button::new("maint-browse-output")
                    .outline()
                    .small()
                    .label("Choose output directory…")
                    .disabled(locked)
                    .on_click(cx.listener(|view, _, window, cx| view.browse_output(window, cx))),
            )
            .child(self.labelled(
                "Machine role",
                h_flex()
                    .gap_2()
                    .child(role_button(
                        "maint-role-controlplane",
                        "Control plane",
                        MachineRole::ControlPlane,
                    ))
                    .child(role_button("maint-role-worker", "Worker", MachineRole::Worker)),
                cx,
            ))
            .child(self.input(
                "Authenticated Talos context",
                "maint-context",
                &self.fields.context,
                locked,
                cx,
            ))
            .child(
                div()
                    .id("maint-talosconfig")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(format!(
                        "Authenticated talosconfig: {}/talosconfig",
                        self.draft.output
                    ))
                    .text_size(dp(12.))
                    .text_color(palette(cx).muted)
                    .child(format!(
                        "Authenticated talosconfig: {}/talosconfig",
                        self.draft.output
                    )),
            )
            .child(self.input(
                "Exact Talos node to wait for / bootstrap (must match the endpoint)",
                "maint-bootstrap-node",
                &self.fields.bootstrap_node,
                locked,
                cx,
            ))
            .child(self.input(
                "Expected Kubernetes node name (optional; no IP-to-name inference)",
                "maint-kubernetes-node",
                &self.fields.kubernetes_node,
                locked,
                cx,
            ))
            .child(self.input(
                "Selected kubeconfig path (optional; empty uses the authenticated Talos API)",
                "maint-kubeconfig",
                &self.fields.kubeconfig,
                locked,
                cx,
            ))
            .child(
                div()
                    .text_size(dp(12.))
                    .text_color(palette(cx).muted)
                    .child("A selected kubeconfig must match the CA/TLS identity of this authenticated Talos node before any credentials or plugins are used. Rejection means unavailable, never an ambient fallback."),
            )
            .child(
                Button::new("maint-start")
                    .primary()
                    .icon(IconName::Server)
                    .label("Connect insecurely and inspect disks")
                    .disabled(locked)
                    .on_click(cx.listener(|view, _, window, cx| view.start(window, cx))),
            )
    }

    fn snapshot_panel(
        &self,
        snapshot: &InsecureMaintenanceSnapshot,
        session: &BootstrapSession,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = palette(cx);
        let version = match &snapshot.version {
            SourceAvailability::Available(version) => (
                format!(
                    "{} · {}",
                    version.tag,
                    if version.maintenance_mode {
                        "maintenance mode"
                    } else {
                        "not reporting maintenance mode"
                    }
                ),
                if version.maintenance_mode {
                    Tone::Good
                } else {
                    Tone::Warn
                },
            ),
            SourceAvailability::Unavailable { reason } => {
                (format!("Unknown · {reason}"), Tone::Unknown)
            }
        };
        let volumes = match &snapshot.volumes {
            SourceAvailability::Available(volumes) => (
                if volumes.is_empty() {
                    "No volumes reported".to_owned()
                } else {
                    volumes
                        .iter()
                        .take(64)
                        .map(|volume| format!("{} ({}, {})", volume.id, volume.phase, volume.size))
                        .collect::<Vec<_>>()
                        .join(" · ")
                },
                Tone::Outline,
            ),
            SourceAvailability::Unavailable { reason } => {
                (format!("Unknown · {reason}"), Tone::Unknown)
            }
        };
        let selecting = session.phase == BootstrapPhase::SelectingInstallTarget;
        let selected = session
            .install_target
            .as_ref()
            .map(|target| target.device_path().to_owned());
        let fact = |id: &'static str, label: &'static str, (text, tone): (String, Tone)| {
            div()
                .id(id)
                .test_support()
                .role(Role::Status)
                .aria_label(format!("{label}: {text}"))
                .child(field(
                    label,
                    h_flex()
                        .gap_2()
                        .items_start()
                        .child(ui::tag(
                            tone,
                            None,
                            match tone {
                                Tone::Unknown => "Unknown",
                                Tone::Warn => "Check",
                                _ => "Reported",
                            },
                            cx,
                        ))
                        .child(div().min_w_0().flex_1().child(text)),
                    cx,
                ))
        };
        let disks = snapshot
            .disks
            .iter()
            .take(MAX_DISKS)
            .enumerate()
            .map(|(ix, disk)| {
                let usable = !disk.readonly && !disk.cdrom;
                let chosen = selected.as_deref() == Some(disk.dev_path.as_str());
                let path = disk.dev_path.clone();
                let detail = format!(
                    "{} · {} · {}",
                    disk.size_pretty,
                    disk.model.as_deref().unwrap_or("model unknown"),
                    disk.serial.as_deref().unwrap_or("serial unknown"),
                );
                h_flex()
                    .id(("maint-disk", ix))
                    .test_support()
                    .role(Role::Group)
                    .aria_label(format!("Disk {}, {detail}", disk.dev_path))
                    .gap_3()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(p.line)
                    .items_center()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(mono(format!("{} / {}", disk.dev_path, disk.id)))
                            .child(div().text_size(dp(12.)).text_color(p.muted).child(detail)),
                    )
                    .when(disk.readonly, |row| {
                        row.child(ui::tag(Tone::Warn, None, "Read-only", cx))
                    })
                    .when(disk.cdrom, |row| {
                        row.child(ui::tag(Tone::Warn, None, "Optical", cx))
                    })
                    .when(chosen, |row| {
                        row.child(ui::tag(
                            Tone::Accent,
                            Some(IconName::CircleCheck),
                            "Selected",
                            cx,
                        ))
                    })
                    .child(
                        Button::new(("maint-disk-select", ix))
                            .outline()
                            .small()
                            .label("Select install disk")
                            .disabled(busy || !selecting || !usable)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.select_disk(path.clone(), cx)
                            })),
                    )
            });
        panel(cx)
            .id("maint-snapshot")
            .test_support()
            .role(Role::Group)
            .aria_label("Talos hardware and install disks")
            .p_4()
            .gap_3()
            .child(heading("Talos hardware and install disks"))
            .child(fact("maint-version", "Version", version))
            .child(fact("maint-volumes", "Volumes", volumes))
            .child(if snapshot.disks.is_empty() {
                div()
                    .text_size(dp(12.5))
                    .text_color(p.muted)
                    .child("Talos reported no disks.")
            } else {
                div()
            })
            .child(
                v_flex()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(p.line)
                    .children(disks),
            )
    }

    fn review_panel(&self, cx: &mut Context<Self>) -> Option<Div> {
        let review = self.review.as_ref()?;
        let p = palette(cx);
        Some(
            panel(cx)
                .p_4()
                .gap_2()
                .child(heading("Exact generated YAML (contains secrets; do not share)"))
                .child(
                    div()
                        .text_size(dp(12.))
                        .text_color(p.muted)
                        .child(format!(
                            "Long lines wrap every {REVIEW_ROW_CHARS} characters for display only; the reviewed bytes are unchanged."
                        )),
                )
                .child(
                    div()
                        .id("maint-review")
                        .test_support()
                        .role(Role::Group)
                        .aria_label("Exact generated YAML review (contains secrets)")
                        .h(dp(340.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(p.line)
                        .bg(p.surface_2)
                        .child(
                            uniform_list(
                                "maint-review-lines",
                                review.rows.len(),
                                cx.processor(|view, range: Range<usize>, _, _| {
                                    let Some(review) = &view.review else {
                                        return Vec::new();
                                    };
                                    range
                                        .filter_map(|ix| {
                                            review.rows.get(ix).map(|row| {
                                                div()
                                                    .h(dp(REVIEW_ROW_HEIGHT))
                                                    .px_3()
                                                    .whitespace_nowrap()
                                                    .font_family(MONO_FONT)
                                                    .text_size(dp(12.))
                                                    .child(review.text[row.clone()].to_owned())
                                            })
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .track_scroll(&review.scroll)
                            .size_full(),
                        ),
                ),
        )
    }

    fn readiness_panel(&self, evidence: &BootstrapReadinessEvidence, cx: &App) -> impl IntoElement {
        let rows = readiness_rows(evidence);
        let summary = rows
            .iter()
            .map(|(label, _, text)| format!("{label}: {text}"))
            .collect::<Vec<_>>()
            .join("; ");
        panel(cx)
            .id("maint-readiness")
            .test_support()
            .role(Role::Group)
            .aria_label(format!("API-backed readiness evidence. {summary}"))
            .p_4()
            .gap_2()
            .child(heading("API-backed readiness (unknown is not healthy)"))
            .children(rows.into_iter().map(|(label, tone, text)| {
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(
                        div()
                            .w(dp(100.))
                            .flex_none()
                            .text_size(dp(12.))
                            .text_color(palette(cx).muted)
                            .child(label),
                    )
                    .child(ui::tag(
                        tone,
                        None,
                        match tone {
                            Tone::Good => "Ready",
                            Tone::Warn => "Not ready",
                            _ => "Unknown",
                        },
                        cx,
                    ))
                    .child(div().min_w_0().flex_1().text_size(dp(13.)).child(text))
            }))
    }

    fn workflow(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let busy = self.work.is_some() || Self::slot_busy(cx).is_some();
        let Some(session) = &self.session else {
            return panel(cx).p_4().gap_2().child(heading("Workflow")).child(
                div()
                    .text_size(dp(13.))
                    .text_color(p.muted)
                    .child("Choose cluster settings, then collect hardware. An install disk must be selected explicitly before generation. Nothing changes the node automatically."),
            );
        };
        let phase = &session.phase;
        let (label, tone) = phase_label(phase);
        let mut column = v_flex().gap_4().min_w_0();
        column = column.child(
            panel(cx)
                .p_4()
                .gap_2()
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .flex_wrap()
                        .child(
                            div()
                                .id("maint-phase")
                                .test_support()
                                .role(Role::Status)
                                .aria_label(format!("Phase: {label}"))
                                .child(ui::tag(tone, None, label, cx)),
                        )
                        .child(mono(format!(
                            "{} · context {} · poll attempts {}",
                            session.plan.endpoint,
                            session.plan.authenticated_target.talos_context,
                            session.poll_attempts
                        ))),
                )
                .when_some(session.last_message.clone(), |this, message| {
                    this.child(
                        div()
                            .id("maint-message")
                            .test_support()
                            .role(Role::Status)
                            .aria_label(message.clone())
                            .text_size(dp(13.))
                            .child(message),
                    )
                }),
        );
        if let Some(snapshot) = &session.insecure_snapshot {
            column = column.child(self.snapshot_panel(snapshot, session, busy, cx));
        }
        if let Some(target) = &session.install_target {
            column = column.child(
                div()
                    .id("maint-install-target")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(format!(
                        "Selected install disk {} (Talos disk ID {})",
                        target.device_path(),
                        target.disk_id()
                    ))
                    .child(ui::warning_banner(
                        Some("Selected install disk".into()),
                        format!(
                            "{} · Talos disk ID {}",
                            target.device_path(),
                            target.disk_id()
                        ),
                        None,
                        cx,
                    )),
            );
        }
        let mut actions = h_flex().gap_2().flex_wrap();
        if *phase == BootstrapPhase::Configuring {
            actions = actions.child(
                Button::new("maint-generate")
                    .primary()
                    .label("Generate configuration with the selected disk")
                    .disabled(busy)
                    .on_click(cx.listener(|view, _, window, cx| view.generate(window, cx))),
            );
        }
        if session.generated_configuration.is_some() {
            actions = actions.child(
                Button::new("maint-review-read")
                    .outline()
                    .label("Read configuration for review")
                    .disabled(busy || *phase != BootstrapPhase::ConfigurationReady)
                    .on_click(cx.listener(|view, _, window, cx| view.read_review(window, cx))),
            );
        }
        if self.review.is_some() {
            actions = actions.child(
                Button::new("maint-review-request")
                    .primary()
                    .label("I reviewed the YAML: request apply confirmation")
                    .disabled(busy || *phase != BootstrapPhase::ConfigurationReady)
                    .on_click(cx.listener(|view, _, _, cx| view.prepare_apply(cx))),
            );
        }
        if *phase == BootstrapPhase::AwaitingApplyConfirmation {
            actions = actions.child(
                Button::new("maint-apply")
                    .danger()
                    .label("Install / apply…")
                    .disabled(busy)
                    .on_click(cx.listener(|view, _, window, cx| view.confirm_apply(window, cx))),
            );
        }
        if let Some(configuration) = &session.generated_configuration {
            column = column.child(
                panel(cx)
                    .p_4()
                    .gap_2()
                    .child(heading("Generated files"))
                    .child(field(
                        "Machine config",
                        mono(configuration.machine_config_path.display().to_string()),
                        cx,
                    ))
                    .child(field(
                        "Talosconfig",
                        mono(configuration.talosconfig_path.display().to_string()),
                        cx,
                    )),
            );
        }
        if let Some(review) = self.review_panel(cx) {
            column = column.child(review);
        }
        let configuring = [
            BootstrapPhase::Configuring,
            BootstrapPhase::ConfigurationReady,
            BootstrapPhase::AwaitingApplyConfirmation,
        ]
        .contains(phase)
            || session.generated_configuration.is_some();
        if configuring {
            column = column.child(actions);
        }
        if *phase == BootstrapPhase::ReadyToBootstrap {
            column = match session.plan.role {
                MachineRole::ControlPlane => column.child(
                    panel(cx)
                        .p_4()
                        .gap_2()
                        .child(heading("Separate cluster bootstrap confirmation"))
                        .child(div().text_size(dp(13.)).child("The secure Talos API answered from the explicit node. Bootstrap initializes etcd and must run on the intended first control plane only."))
                        .child(
                            div().child(
                                Button::new("maint-bootstrap")
                                    .danger()
                                    .label("Bootstrap this node…")
                                    .disabled(busy)
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.confirm_bootstrap(window, cx)
                                    })),
                            ),
                        ),
                ),
                MachineRole::Worker => column.child(
                    div()
                        .id("maint-worker-note")
                        .test_support()
                        .role(Role::Status)
                        .aria_label("Worker installation reached its secure Talos API; do not bootstrap etcd on a worker")
                        .child(ui::warning_banner(
                            None,
                            "Worker installation reached its secure Talos API. Do not bootstrap etcd on a worker; cluster bootstrap is performed separately on the first control plane.",
                            None,
                            cx,
                        )),
                ),
            };
        }
        if let Some(evidence) = &session.latest_readiness {
            column = column.child(self.readiness_panel(evidence, cx));
        }
        let finished = matches!(
            phase,
            BootstrapPhase::Complete | BootstrapPhase::Cancelled | BootstrapPhase::Failed(_)
        );
        column.child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(
                    Button::new("maint-poll")
                        .outline()
                        .small()
                        .label("Poll secure Talos / etcd / Kubernetes now")
                        .disabled(busy || session.polling_action().is_none())
                        .on_click(cx.listener(|view, _, window, cx| view.poll_now(window, cx))),
                )
                .child(
                    Button::new("maint-cancel")
                        .outline()
                        .small()
                        .label("Cancel / stop polling")
                        .disabled(finished)
                        .tooltip("Stops before the next step; a request already sent to the node still completes")
                        .on_click(cx.listener(|view, _, _, cx| view.cancel(cx))),
                )
                .child(
                    Button::new("maint-reset")
                        .outline()
                        .small()
                        .label("Reset workflow (keeps generated files)")
                        .disabled(busy)
                        .on_click(cx.listener(|view, _, _, cx| view.reset(cx))),
                ),
        )
    }

    fn progress_panel(&self, cx: &App) -> impl IntoElement {
        let p = palette(cx);
        let lines: Vec<String> = self.progress.iter().cloned().collect();
        panel(cx)
            .id("maint-progress")
            .test_support()
            .role(Role::Log)
            .aria_label(if lines.is_empty() {
                "Maintenance progress: nothing recorded yet".to_owned()
            } else {
                format!(
                    "Maintenance progress: {} entries, latest: {}",
                    lines.len(),
                    lines[lines.len() - 1]
                )
            })
            .p_4()
            .gap_1p5()
            .child(heading("Progress and results"))
            .children(if lines.is_empty() {
                vec![
                    div()
                        .text_size(dp(12.5))
                        .text_color(p.muted)
                        .child("No progress recorded yet."),
                ]
            } else {
                lines
                    .into_iter()
                    .map(|line| div().text_size(dp(12.5)).child(line))
                    .collect()
            })
    }

    fn status_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let line = match Operations::current(cx) {
            Some(running) if running.cancel_requested() => {
                format!("{} · stopping after the current step…", running.label)
            }
            Some(running) => running.label.to_string(),
            None if self.work.is_some() => self
                .work
                .as_ref()
                .map(|work| work.label.to_string())
                .unwrap_or_default(),
            None => "Idle".to_owned(),
        };
        h_flex()
            .id("maint-status")
            .test_support()
            .role(Role::Status)
            .aria_label(line.clone())
            .px_4()
            .py_1p5()
            .gap_2()
            .border_t_1()
            .border_color(p.line)
            .text_size(dp(12.))
            .text_color(p.muted)
            .child(line)
            .into_any_element()
    }
}

impl Render for MaintenanceView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let error = self.error.clone();
        let form = self.form(cx);
        let workflow = self.workflow(cx);
        let progress = self.progress_panel(cx);
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .track_focus(&self.focus)
            .child(self.title_bar(cx))
            .child(
                div().flex_1().min_h_0().child(
                    page_scroll("maint-page").child(
                        page_body()
                            .child(
                                div()
                                    .id("maint-warning")
                                    .test_support()
                                    .role(Role::Status)
                                    .aria_label("Insecure Talos access: verify the exact physical node. Applying configuration can erase the selected disk. No ambient Talos or Kubernetes cluster is used.")
                                    .child(ui::warning_banner(
                                        Some("Insecure Talos access".into()),
                                        "Verify the exact physical node. Applying configuration can erase the selected disk. No ambient Talos or Kubernetes cluster is used.",
                                        None,
                                        cx,
                                    )),
                            )
                            .when_some(error, |this, error| {
                                this.child(
                                    div()
                                        .id("maint-error")
                                        .test_support()
                                        .role(Role::Alert)
                                        .aria_label(error.clone())
                                        .px_3()
                                        .py_2p5()
                                        .rounded(px(8.))
                                        .bg(p.crit_soft)
                                        .text_color(p.crit_ink)
                                        .text_size(dp(13.))
                                        .child(error),
                                )
                            })
                            .child(
                                h_flex()
                                    .items_start()
                                    .gap_5()
                                    .flex_wrap()
                                    .child(div().w(dp(440.)).flex_none().child(form))
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w(dp(420.))
                                            .gap_4()
                                            .child(workflow)
                                            .child(progress),
                                    ),
                            ),
                    ),
                ),
            )
            .child(self.status_bar(cx))
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: it brings gpui_kit's `test` macro over the built-in one.
    use super::{phase_label, readiness_rows, review_rows};
    use crate::ui::Tone;
    use freshkube_core::maintenance::{
        AuthenticatedTarget, BootstrapPhase, BootstrapReadinessEvidence, EtcdReadinessEvidence,
        KubernetesReadinessEvidence, MaintenanceEndpoint, MaintenanceError, TalosReadinessEvidence,
    };

    #[test]
    fn review_rows_cover_every_reviewed_character() {
        let text = format!(
            "machine:\n  token: abc\n\n{}\nlast line without a break é{}",
            "x".repeat(300),
            "ü".repeat(200)
        );
        let rows = review_rows(&text);
        let shown: String = rows.iter().map(|row| &text[row.clone()]).collect();
        let expected: String = text.chars().filter(|c| *c != '\n').collect();
        // Not assert_eq!: the text stands in for secrets and must not print.
        assert!(shown == expected);
        assert!(
            rows.iter()
                .all(|row| text[row.clone()].chars().count() <= super::REVIEW_ROW_CHARS)
        );
        assert!(review_rows("").is_empty());
        // A blank line is still a row, so line structure survives.
        assert_eq!(review_rows("a\n\nb").len(), 3);
    }

    #[test]
    fn readiness_never_shows_missing_evidence_as_healthy_or_failed() {
        let node = MaintenanceEndpoint::parse("192.0.2.10").unwrap();
        let target = AuthenticatedTarget::new("lab", node, None).unwrap();
        let unknown = BootstrapReadinessEvidence {
            target: target.clone(),
            talos: TalosReadinessEvidence::Unavailable {
                reason: "connection refused".into(),
            },
            kubernetes: KubernetesReadinessEvidence::NotChecked,
        };
        let rows = readiness_rows(&unknown);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|(_, tone, _)| *tone == Tone::Unknown));
        assert!(rows[0].2.contains("connection refused"));

        let partial = BootstrapReadinessEvidence {
            target,
            talos: TalosReadinessEvidence::Available {
                versions: Vec::new(),
                etcd: EtcdReadinessEvidence::Unavailable {
                    reason: "etcd not up".into(),
                },
            },
            kubernetes: KubernetesReadinessEvidence::NotChecked,
        };
        let rows = readiness_rows(&partial);
        assert_eq!(rows[0].1, Tone::Good);
        assert_eq!(rows[1].1, Tone::Unknown);
        assert!(rows.iter().all(|(_, tone, _)| *tone != Tone::Crit));
    }

    #[test]
    fn only_a_real_failure_is_critical() {
        assert_eq!(
            phase_label(&BootstrapPhase::Failed(MaintenanceError::Cancelled)).1,
            Tone::Crit
        );
        for phase in [
            BootstrapPhase::CollectingInsecureData,
            BootstrapPhase::WaitingForTalos,
            BootstrapPhase::WaitingForKubernetes,
            BootstrapPhase::Cancelled,
        ] {
            assert_ne!(phase_label(&phase).1, Tone::Crit);
        }
    }
}

/// Offline UI tests. Every node interaction goes through the injected
/// [`Runner`], so no test touches a network or a node. The "generated"
/// configuration is a harmless fake written to a throwaway directory.
#[cfg(test)]
mod ui_tests {
    // Not `super::*`: it brings gpui_kit's `test` macro over the built-in one.
    use super::{Cancelled, MaintenanceView, Runner};
    use crate::mutation::{self, Operations};
    use freshkube_core::maintenance::{
        BootstrapCommandOutcome, BootstrapPhase, BootstrapReadinessEvidence,
        ConfigurationApplicationOutcome, EtcdReadinessEvidence, GeneratedConfiguration,
        InsecureMaintenanceSnapshot, InstallTarget, KubernetesReadinessEvidence, MachineRole,
        MaintenanceAction, MaintenanceError, MaintenanceEvent, SourceAvailability,
        TalosReadinessEvidence, TalosVersionEvidence,
    };
    use gpui_kit::test::{TestAppContextExt, TestWindowExt};
    use gpui_kit::{
        AnyWindowHandle, AppContext, Entity, TestAppContext,
        component::{Root, Theme, ThemeMode},
        px, size,
    };
    use std::{
        path::PathBuf,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };
    use talos_rs::{DiskInfo, InsecureVersionInfo, VolumeStatus};

    const NODE: &str = "192.0.2.10";
    const WAIT: Duration = Duration::from_secs(10);

    /// A scripted stand-in for the node. It records which actions reached it.
    #[derive(Clone)]
    struct World {
        calls: Arc<Mutex<Vec<&'static str>>>,
        directory: PathBuf,
        version_unknown: bool,
        volumes_unknown: bool,
        fail_collection: bool,
        /// Bootstrap waits until released or cancelled.
        hold_bootstrap: bool,
        release_bootstrap: Arc<AtomicBool>,
    }

    impl World {
        fn new(name: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "freshkube-desktop-maintenance-{}-{name}",
                std::process::id()
            ));
            _ = std::fs::remove_dir_all(&directory);
            std::fs::create_dir_all(&directory).unwrap();
            Self {
                calls: Arc::default(),
                directory,
                version_unknown: false,
                volumes_unknown: false,
                fail_collection: false,
                hold_bootstrap: false,
                release_bootstrap: Arc::default(),
            }
        }

        fn count(&self, name: &str) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| **call == name)
                .count()
        }

        fn log(&self, name: &'static str) {
            self.calls.lock().unwrap().push(name);
        }

        fn disks() -> Vec<DiskInfo> {
            let disk = |id: &str, readonly: bool, cdrom: bool| DiskInfo {
                id: id.into(),
                dev_path: format!("/dev/{id}"),
                size: 100_000_000_000,
                size_pretty: "100 GB".into(),
                model: Some("Example Disk".into()),
                serial: None,
                transport: None,
                rotational: false,
                readonly,
                cdrom,
                wwid: None,
                bus_path: None,
            };
            vec![
                disk("sda", false, false),
                disk("sr0", false, true),
                disk("sdb", true, false),
            ]
        }

        /// A harmless stand-in for a generated machine configuration.
        fn yaml(role: MachineRole, disk: &str) -> String {
            let role = match role {
                MachineRole::ControlPlane => "controlplane",
                MachineRole::Worker => "worker",
            };
            format!(
                "machine:\n  type: {role}\n  install:\n    disk: {disk}\n  token: fake-test-token\n"
            )
        }

        async fn execute(
            &self,
            action: MaintenanceAction,
            cancelled: Cancelled,
        ) -> MaintenanceEvent {
            match action {
                MaintenanceAction::CollectInsecure { endpoint } => {
                    self.log("collect");
                    if self.fail_collection {
                        return MaintenanceEvent::OperationFailed(
                            MaintenanceError::InsecureDiskCollectionFailed {
                                message: "connection refused".into(),
                            },
                        );
                    }
                    MaintenanceEvent::InsecureDataCollected(InsecureMaintenanceSnapshot {
                        endpoint,
                        version: if self.version_unknown {
                            SourceAvailability::Unavailable {
                                reason: "version API unavailable".into(),
                            }
                        } else {
                            SourceAvailability::Available(InsecureVersionInfo {
                                tag: "v1.12.1".into(),
                                maintenance_mode: true,
                            })
                        },
                        disks: Self::disks(),
                        volumes: if self.volumes_unknown {
                            SourceAvailability::Unavailable {
                                reason: "volume API unavailable".into(),
                            }
                        } else {
                            SourceAvailability::Available(vec![VolumeStatus {
                                id: "STATE".into(),
                                encryption_provider: None,
                                phase: "waiting".into(),
                                size: "0 B".into(),
                                filesystem: None,
                                mount_location: None,
                            }])
                        },
                    })
                }
                MaintenanceAction::GenerateConfiguration(request) => {
                    self.log("generate");
                    let path = request
                        .plan
                        .output_dir
                        .join(request.plan.role.configuration_filename());
                    assert!(request.plan.output_dir.starts_with(&self.directory));
                    std::fs::create_dir_all(&request.plan.output_dir).unwrap();
                    std::fs::write(
                        &path,
                        Self::yaml(request.plan.role, request.install_target.device_path()),
                    )
                    .unwrap();
                    MaintenanceEvent::ConfigurationGenerationFinished(GeneratedConfiguration {
                        machine_config_path: path,
                        talosconfig_path: request.plan.paths.talosconfig_path.clone(),
                        role: request.plan.role,
                        install_target: request.install_target,
                    })
                }
                MaintenanceAction::ApplyConfiguration(_) => {
                    self.log("apply");
                    MaintenanceEvent::ConfigurationApplicationFinished(
                        ConfigurationApplicationOutcome {
                            applied: true,
                            message: "Configuration accepted (test double)".into(),
                        },
                    )
                }
                MaintenanceAction::Bootstrap(_) => {
                    self.log("bootstrap");
                    while self.hold_bootstrap && !self.release_bootstrap.load(Ordering::SeqCst) {
                        if cancelled() {
                            return MaintenanceEvent::Cancelled;
                        }
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                    MaintenanceEvent::BootstrapFinished(BootstrapCommandOutcome {
                        started: true,
                        message: "Bootstrap accepted (test double)".into(),
                    })
                }
                MaintenanceAction::PollReadiness(request) => {
                    self.log("poll");
                    MaintenanceEvent::ReadinessPolled(BootstrapReadinessEvidence {
                        target: request.target,
                        // The Talos API answers; etcd and Kubernetes say nothing.
                        talos: TalosReadinessEvidence::Available {
                            versions: vec![TalosVersionEvidence {
                                node: NODE.into(),
                                version: "v1.12.1".into(),
                            }],
                            etcd: EtcdReadinessEvidence::Unavailable {
                                reason: "etcd API not answering yet".into(),
                            },
                        },
                        kubernetes: KubernetesReadinessEvidence::NotChecked,
                    })
                }
            }
        }

        fn runner(&self) -> Runner {
            let world = self.clone();
            Arc::new(move |action, cancelled| {
                let world = world.clone();
                Box::pin(async move { world.execute(action, cancelled).await })
            })
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            // Clones share the directory; the last test thread cleans up.
            if Arc::strong_count(&self.calls) == 1 {
                _ = std::fs::remove_dir_all(&self.directory);
            }
        }
    }

    type Mounted = (
        tokio::runtime::Runtime,
        AnyWindowHandle,
        Entity<MaintenanceView>,
    );

    fn mount(cx: &mut TestAppContext, world: &World, endpoint: &str) -> Mounted {
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            Theme::change(ThemeMode::Light, None, cx);
        });
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut view = None;
        let window = cx.open_window(size(px(1400.), px(2600.)), |window, cx| {
            let created = cx.new(|cx| {
                MaintenanceView::with_runner(
                    endpoint.to_owned(),
                    runtime.handle().clone(),
                    world.runner(),
                    window,
                    cx,
                )
            });
            view = Some(created.clone());
            Root::new(created, window, cx)
        });
        cx.run_until_parked();
        (runtime, window.into(), view.unwrap())
    }

    /// Types like a user: focus the field, replace its text, let events settle.
    fn set_input(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str, value: &str) {
        let select_all = if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        };
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
            window.press(select_all, cx);
            if value.is_empty() {
                window.press("backspace", cx);
            } else {
                window.input(value, cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
            window.render_frame(cx);
        })
        .unwrap();
    }

    fn phase(cx: &mut TestAppContext, view: &Entity<MaintenanceView>) -> Option<BootstrapPhase> {
        cx.update(|cx| view.read(cx).session.as_ref().map(|s| s.phase.clone()))
    }

    async fn wait_phase(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        view: &Entity<MaintenanceView>,
        expected: BootstrapPhase,
    ) {
        let view = view.clone();
        cx.wait_for(handle, WAIT, move |_, cx| {
            view.read(cx).session.as_ref().map(|s| &s.phase) == Some(&expected)
        })
        .await;
    }

    fn label(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) -> String {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .find(id)
                .label()
                .map(str::to_owned)
                .unwrap_or_default()
        })
        .unwrap()
    }

    fn present(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) -> bool {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    fn use_output_directory(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        view: &Entity<MaintenanceView>,
        world: &World,
    ) {
        set_input(
            cx,
            handle,
            "maint-output",
            world.directory.to_str().unwrap(),
        );
        // Guard: a silent no-op here would let the fake write under the cwd.
        cx.update(|cx| {
            assert!(view.read(cx).draft.output.as_str() == world.directory.to_str().unwrap());
        });
    }

    /// Connects and inspects, leaving the session choosing an install disk.
    async fn inspect(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        view: &Entity<MaintenanceView>,
        world: &World,
    ) {
        use_output_directory(cx, handle, view, world);
        click(cx, handle, "maint-start");
        wait_phase(cx, handle, view, BootstrapPhase::SelectingInstallTarget).await;
    }

    /// Selects `/dev/sda`, generates and reviews: ends awaiting confirmation.
    async fn reach_apply_confirmation(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        view: &Entity<MaintenanceView>,
        world: &World,
    ) {
        inspect(cx, handle, view, world).await;
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(("maint-disk-select", 0usize), cx);
            window.render_frame(cx);
        })
        .unwrap();
        assert_eq!(phase(cx, view), Some(BootstrapPhase::Configuring));
        click(cx, handle, "maint-generate");
        wait_phase(cx, handle, view, BootstrapPhase::ConfigurationReady).await;
        click(cx, handle, "maint-review-read");
        let watched = view.clone();
        cx.wait_for(handle, WAIT, move |_, cx| watched.read(cx).review.is_some())
            .await;
        click(cx, handle, "maint-review-request");
        assert_eq!(
            phase(cx, view),
            Some(BootstrapPhase::AwaitingApplyConfirmation)
        );
    }

    /// Opens a confirmation dialog from `button` and waits for its animation.
    fn open_confirmation(cx: &mut TestAppContext, handle: AnyWindowHandle, button: &'static str) {
        click(cx, handle, button);
        crate::mutation::settle_confirmation(cx, handle);
    }

    /// Applies and waits for the node to be waiting for its secure API.
    async fn apply(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        view: &Entity<MaintenanceView>,
    ) {
        open_confirmation(cx, handle, "maint-apply");
        click(cx, handle, "confirm-ok");
        wait_phase(cx, handle, view, BootstrapPhase::WaitingForTalos).await;
    }

    fn slot_free(cx: &mut TestAppContext) -> bool {
        cx.update(|cx| Operations::current(cx).is_none())
    }

    #[gpui_kit::test]
    async fn the_endpoint_is_validated_before_any_node_is_contacted(cx: &mut TestAppContext) {
        let world = World::new("endpoint");
        let (_runtime, handle, view) = mount(cx, &world, NODE);
        // The endpoint from the command line fills the form.
        cx.update(|cx| {
            assert_eq!(view.read(cx).draft.endpoint, NODE);
            assert_eq!(view.read(cx).draft.bootstrap_node, NODE);
            assert_eq!(
                view.read(cx).draft.kubernetes_api,
                "https://192.0.2.10:6443"
            );
        });
        for bad in [
            "",
            "node.example:0",
            "https://node.example/path",
            "not a valid endpoint",
        ] {
            set_input(cx, handle, "maint-endpoint", bad);
            click(cx, handle, "maint-start");
            let seen = cx.update(|cx| view.read(cx).draft.endpoint.clone());
            assert!(present(cx, handle, "maint-error"), "{bad:?} draft={seen:?}");
            assert!(phase(cx, &view).is_none(), "{bad:?}");
        }
        // A different node than the one to bootstrap is also refused up front.
        set_input(cx, handle, "maint-endpoint", "192.0.2.11");
        click(cx, handle, "maint-start");
        assert!(label(cx, handle, "maint-error").contains("does not match"));
        assert_eq!(world.count("collect"), 0);
        // The same node with a port is the same node.
        set_input(cx, handle, "maint-endpoint", "192.0.2.10:50000");
        use_output_directory(cx, handle, &view, &world);
        click(cx, handle, "maint-start");
        wait_phase(cx, handle, &view, BootstrapPhase::SelectingInstallTarget).await;
        assert_eq!(world.count("collect"), 1);
        assert!(!present(cx, handle, "maint-error"));
        // Once connected the form is frozen: connecting again does nothing.
        click(cx, handle, "maint-start");
        assert_eq!(world.count("collect"), 1);
    }

    #[gpui_kit::test]
    async fn an_unanswered_source_shows_as_unknown_not_failed(cx: &mut TestAppContext) {
        let mut world = World::new("unknown");
        world.version_unknown = true;
        world.volumes_unknown = true;
        let (_runtime, handle, view) = mount(cx, &world, NODE);
        inspect(cx, handle, &view, &world).await;
        let version = label(cx, handle, "maint-version");
        let volumes = label(cx, handle, "maint-volumes");
        assert!(
            version.contains("Unknown") && version.contains("version API unavailable"),
            "{version}"
        );
        assert!(
            volumes.contains("Unknown") && volumes.contains("volume API unavailable"),
            "{volumes}"
        );
        // Nothing is reported as an error, and disks stay selectable.
        assert!(!present(cx, handle, "maint-error"));
        assert!(label(cx, handle, "maint-phase").contains("Choose an install disk"));
        // Optical and read-only disks can't be chosen.
        for ix in [1usize, 2] {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click(("maint-disk-select", ix), cx);
            })
            .unwrap();
        }
        assert_eq!(
            phase(cx, &view),
            Some(BootstrapPhase::SelectingInstallTarget)
        );
        assert_eq!(world.count("generate"), 0);
    }

    #[gpui_kit::test]
    async fn a_node_that_cannot_be_inspected_is_reported_and_nothing_proceeds(
        cx: &mut TestAppContext,
    ) {
        let mut world = World::new("unreachable");
        world.fail_collection = true;
        let (_runtime, handle, view) = mount(cx, &world, NODE);
        use_output_directory(cx, handle, &view, &world);
        click(cx, handle, "maint-start");
        wait_phase(
            cx,
            handle,
            &view,
            BootstrapPhase::Failed(MaintenanceError::InsecureDiskCollectionFailed {
                message: "connection refused".into(),
            }),
        )
        .await;
        assert!(label(cx, handle, "maint-phase").contains("Failed"));
        assert!(label(cx, handle, "maint-message").contains("connection refused"));
        assert!(!present(cx, handle, "maint-snapshot"));
        assert!(!present(cx, handle, "maint-generate"));
        assert!(slot_free(cx));
        // Reset returns to the form, which can try again.
        click(cx, handle, "maint-reset");
        assert!(phase(cx, &view).is_none());
    }

    #[gpui_kit::test]
    async fn apply_needs_an_explicit_confirmation_of_the_reviewed_yaml(cx: &mut TestAppContext) {
        let world = World::new("apply");
        let (_runtime, handle, view) = mount(cx, &world, NODE);
        reach_apply_confirmation(cx, handle, &view, &world).await;
        // Reaching confirmation changed nothing on the node.
        assert_eq!(world.count("apply"), 0);
        // What was reviewed is exactly what the node's config holds.
        let expected = World::yaml(MachineRole::ControlPlane, "/dev/sda");
        cx.update(|cx| {
            let review = view.read(cx).review.as_ref().unwrap();
            // Not assert_eq!: this stands in for secrets and must not print.
            assert!(review.text == expected);
        });
        assert!(present(cx, handle, "maint-review"));

        open_confirmation(cx, handle, "maint-apply");
        let title = label(cx, handle, "confirm-dialog");
        assert!(title.contains("Install Talos on 192.0.2.10"), "{title}");
        assert_eq!(world.count("apply"), 0);
        click(cx, handle, "confirm-cancel");
        cx.run_until_parked();
        assert_eq!(world.count("apply"), 0);
        assert_eq!(
            phase(cx, &view),
            Some(BootstrapPhase::AwaitingApplyConfirmation)
        );
        assert!(slot_free(cx));

        open_confirmation(cx, handle, "maint-apply");
        click(cx, handle, "confirm-ok");
        wait_phase(cx, handle, &view, BootstrapPhase::WaitingForTalos).await;
        assert_eq!(world.count("apply"), 1);
        assert!(slot_free(cx));
        // Reviewed secrets are dropped once they have been sent.
        cx.update(|cx| assert!(view.read(cx).review.is_none()));
        assert!(!present(cx, handle, "maint-review"));
        assert!(label(cx, handle, "maint-progress").contains("WaitingForTalos"));
    }

    #[gpui_kit::test]
    async fn a_stale_apply_preview_is_refused_and_sends_nothing(cx: &mut TestAppContext) {
        let world = World::new("stale");
        let (_runtime, handle, view) = mount(cx, &world, NODE);
        reach_apply_confirmation(cx, handle, &view, &world).await;
        open_confirmation(cx, handle, "maint-apply");
        // The workflow moved on while the dialog was open.
        view.update(cx, |view, cx| view.reset(cx));
        cx.update_window(handle, |_, window, cx| {
            window.click("confirm-ok", cx);
            window.render_frame(cx);
            assert!(window.find("confirm-blocked").visible());
            assert!(window.find("confirm-dialog").visible());
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(world.count("apply"), 0);
        assert!(slot_free(cx));
    }

    #[gpui_kit::test]
    async fn a_busy_operation_slot_refuses_apply_until_it_is_free(cx: &mut TestAppContext) {
        let world = World::new("busy");
        let (_runtime, handle, view) = mount(cx, &world, NODE);
        reach_apply_confirmation(cx, handle, &view, &world).await;
        open_confirmation(cx, handle, "maint-apply");
        let operations = cx.update(Operations::global);
        let ticket = operations
            .update(cx, |operations, cx| {
                operations.begin("Reboot another-node", cx)
            })
            .ok()
            .unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.click("confirm-ok", cx);
            window.render_frame(cx);
            assert!(window.find("confirm-blocked").visible());
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(world.count("apply"), 0);
        // The holder of the slot is untouched.
        cx.update(|cx| {
            assert_eq!(
                Operations::current(cx)
                    .map(|running| running.label.to_string())
                    .as_deref(),
                Some("Reboot another-node")
            );
        });
        operations.update(cx, |operations, cx| operations.finish(ticket, cx));
        click(cx, handle, "confirm-ok");
        wait_phase(cx, handle, &view, BootstrapPhase::WaitingForTalos).await;
        assert_eq!(world.count("apply"), 1);
        assert!(slot_free(cx));
    }

    #[gpui_kit::test]
    async fn connecting_is_refused_while_another_operation_runs(cx: &mut TestAppContext) {
        let world = World::new("start-busy");
        let (_runtime, handle, view) = mount(cx, &world, NODE);
        use_output_directory(cx, handle, &view, &world);
        let operations = cx.update(Operations::global);
        let ticket = operations
            .update(cx, |operations, cx| operations.begin("Restart etcd", cx))
            .ok()
            .unwrap();
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.start(window, cx));
        })
        .unwrap();
        assert_eq!(world.count("collect"), 0);
        assert!(label(cx, handle, "maint-error").contains("Restart etcd is still running"));
        operations.update(cx, |operations, cx| operations.finish(ticket, cx));
        click(cx, handle, "maint-start");
        wait_phase(cx, handle, &view, BootstrapPhase::SelectingInstallTarget).await;
    }

    #[gpui_kit::test]
    async fn bootstrap_shows_progress_blocks_closing_and_cancel_frees_the_slot(
        cx: &mut TestAppContext,
    ) {
        let mut world = World::new("bootstrap");
        world.hold_bootstrap = true;
        let (_runtime, handle, view) = mount(cx, &world, NODE);
        reach_apply_confirmation(cx, handle, &view, &world).await;
        apply(cx, handle, &view).await;
        // Bootstrap isn't offered until the secure Talos API has answered.
        assert!(!present(cx, handle, "maint-bootstrap"));
        click(cx, handle, "maint-poll");
        wait_phase(cx, handle, &view, BootstrapPhase::ReadyToBootstrap).await;
        assert_eq!(world.count("bootstrap"), 0);

        open_confirmation(cx, handle, "maint-bootstrap");
        assert!(label(cx, handle, "confirm-dialog").contains("Bootstrap etcd on 192.0.2.10"));
        click(cx, handle, "confirm-ok");
        assert_eq!(phase(cx, &view), Some(BootstrapPhase::Bootstrapping));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let running = Operations::current(cx).expect("bootstrap holds the slot");
            assert!(running.label.contains("bootstrapping"));
            assert!(
                window
                    .find("maint-status")
                    .label()
                    .unwrap()
                    .contains("bootstrapping")
            );
            // The window refuses to close under a half-done change.
            assert!(!mutation::may_close(window, cx));
            // Nothing else can start meanwhile.
            assert!(
                window
                    .find("maint-progress")
                    .label()
                    .unwrap()
                    .contains("Bootstrapping")
            );
        })
        .unwrap();
        click(cx, handle, "maint-reset");
        click(cx, handle, "maint-poll");
        assert_eq!(phase(cx, &view), Some(BootstrapPhase::Bootstrapping));
        assert_eq!(world.count("poll"), 1);
        // The prompt that explains the refusal can be dismissed.
        cx.simulate_prompt_answer("Keep running");

        click(cx, handle, "maint-cancel");
        wait_phase(cx, handle, &view, BootstrapPhase::Cancelled).await;
        cx.wait_for(handle, WAIT, |_, cx| Operations::current(cx).is_none())
            .await;
        assert!(slot_free(cx));
        assert!(label(cx, handle, "maint-phase").contains("Cancelled"));
        cx.update_window(handle, |_, window, cx| {
            assert!(mutation::may_close(window, cx));
        })
        .unwrap();
        cx.update(|cx| assert!(!view.read(cx).auto_poll));
    }

    #[gpui_kit::test]
    async fn readiness_that_is_not_proven_is_unknown_and_never_complete(cx: &mut TestAppContext) {
        let world = World::new("readiness");
        let (_runtime, handle, view) = mount(cx, &world, NODE);
        reach_apply_confirmation(cx, handle, &view, &world).await;
        apply(cx, handle, &view).await;
        click(cx, handle, "maint-poll");
        wait_phase(cx, handle, &view, BootstrapPhase::ReadyToBootstrap).await;
        open_confirmation(cx, handle, "maint-bootstrap");
        click(cx, handle, "confirm-ok");
        wait_phase(cx, handle, &view, BootstrapPhase::WaitingForKubernetes).await;
        assert_eq!(world.count("bootstrap"), 1);
        click(cx, handle, "maint-poll");
        let watched = view.clone();
        cx.wait_for(handle, WAIT, move |_, cx| {
            watched
                .read(cx)
                .session
                .as_ref()
                .is_some_and(|s| s.poll_attempts >= 2)
        })
        .await;
        // etcd and Kubernetes gave no evidence: unknown, so not complete.
        assert_eq!(phase(cx, &view), Some(BootstrapPhase::WaitingForKubernetes));
        let evidence = label(cx, handle, "maint-readiness");
        assert!(evidence.contains("etcd: Unknown"), "{evidence}");
        assert!(
            evidence.contains("Kubernetes: Not checked yet"),
            "{evidence}"
        );
        assert!(!present(cx, handle, "maint-error"));
        assert!(slot_free(cx));
    }

    #[gpui_kit::test]
    async fn waiting_for_the_secure_api_polls_on_a_timer(cx: &mut TestAppContext) {
        let world = World::new("timer");
        let (_runtime, handle, view) = mount(cx, &world, NODE);
        reach_apply_confirmation(cx, handle, &view, &world).await;
        apply(cx, handle, &view).await;
        assert_eq!(world.count("poll"), 0);
        cx.executor()
            .advance_clock(super::BOOTSTRAP_POLL_INTERVAL + Duration::from_secs(1));
        wait_phase(cx, handle, &view, BootstrapPhase::ReadyToBootstrap).await;
        assert_eq!(world.count("poll"), 1);
        // Polling stops by itself once there is nothing to wait for.
        cx.executor()
            .advance_clock(super::BOOTSTRAP_POLL_INTERVAL * 2);
        cx.run_until_parked();
        assert_eq!(world.count("poll"), 1);
    }

    #[gpui_kit::test]
    async fn a_worker_is_never_offered_bootstrap(cx: &mut TestAppContext) {
        let world = World::new("worker");
        let (_runtime, handle, view) = mount(cx, &world, NODE);
        click(cx, handle, "maint-role-worker");
        reach_apply_confirmation(cx, handle, &view, &world).await;
        apply(cx, handle, &view).await;
        click(cx, handle, "maint-poll");
        wait_phase(cx, handle, &view, BootstrapPhase::ReadyToBootstrap).await;
        assert!(!present(cx, handle, "maint-bootstrap"));
        assert!(present(cx, handle, "maint-worker-note"));
        assert_eq!(world.count("bootstrap"), 0);
        cx.update(|cx| {
            assert!(view.read(cx).bootstrap_fingerprint().is_none());
        });
    }

    #[gpui_kit::test]
    async fn changing_the_form_after_connecting_is_not_possible(cx: &mut TestAppContext) {
        let world = World::new("locked");
        let (_runtime, handle, view) = mount(cx, &world, NODE);
        inspect(cx, handle, &view, &world).await;
        click(cx, handle, "maint-start");
        assert_eq!(world.count("collect"), 1);
        click(cx, handle, "maint-role-worker");
        cx.update(|cx| {
            assert_eq!(view.read(cx).draft.role, MachineRole::ControlPlane);
        });
        // The chosen disk is mandatory before anything can be generated.
        assert!(!present(cx, handle, "maint-generate"));
        let target = cx.update(|cx| {
            view.read(cx)
                .session
                .as_ref()
                .unwrap()
                .install_target
                .clone()
        });
        assert!(target.is_none());
        let _ = InstallTarget::select(&World::disks(), "/dev/sda").unwrap();
    }
}
