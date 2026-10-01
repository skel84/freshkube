use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use dioxus::prelude::*;
use talos_pilot_core::maintenance::{
    BOOTSTRAP_POLL_INTERVAL, BootstrapPhase, BootstrapPlan, BootstrapSession, MachineRole,
    MaintenanceAction, MaintenanceDraft, MaintenanceError, MaintenanceEvent, ProgressLog,
    READ_TIMEOUT, confirmation_matches, execute_maintenance_action_bounded, frozen_review,
    is_mutation,
};
use tokio::{
    runtime::Handle,
    sync::mpsc,
    task::{AbortHandle, JoinHandle},
};

use crate::{
    DioxusOptions,
    operations::{operation_busy, try_begin_operation},
};

#[derive(Props, Clone)]
pub(crate) struct MaintenancePanelProps {
    options: DioxusOptions,
    runtime: Handle,
}

impl PartialEq for MaintenancePanelProps {
    fn eq(&self, other: &Self) -> bool {
        self.options.config_path == other.options.config_path
            && self.options.context == other.options.context
            && self.options.tail == other.options.tail
            && self.options.maintenance_endpoint == other.options.maintenance_endpoint
    }
}

#[derive(Clone, Default)]
struct ViewState {
    session: Option<BootstrapSession>,
    busy: bool,
    auto_poll: bool,
    review: Option<String>,
    error: Option<String>,
    progress: ProgressLog,
}

impl ViewState {
    fn record(&mut self, message: impl Into<String>) {
        self.progress.record(message);
    }

    fn reduce(&mut self, event: MaintenanceEvent) {
        let Some(session) = &self.session else {
            return;
        };
        match session.reduce(event) {
            Ok(next) => {
                self.record(next.progress_line());
                self.session = Some(next);
            }
            Err(error) => {
                self.error = Some(error.to_string());
            }
        }
    }

    fn apply_phrase(&self) -> Option<String> {
        self.session.as_ref()?.apply_confirmation_phrase()
    }

    fn bootstrap_phrase(&self) -> Option<String> {
        self.session.as_ref()?.bootstrap_confirmation_phrase()
    }
}

#[derive(Default)]
struct WorkControl {
    cancellation: Option<Arc<AtomicBool>>,
    read_abort: Option<AbortHandle>,
}

impl WorkControl {
    fn cancel(&mut self) {
        if let Some(cancel) = &self.cancellation {
            cancel.store(true, Ordering::Release);
        }
        if let Some(abort) = self.read_abort.take() {
            abort.abort();
        }
    }
}

struct AbortOnDrop<T>(JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone)]
enum Command {
    Start(BootstrapPlan),
    SelectDisk(String),
    Generate,
    Review,
    PrepareApply,
    Apply(String),
    Bootstrap(String),
    Poll,
    PickDirectory,
}

/// One focus stop per scrollable evidence pane, rather than per line of text.
pub(crate) fn text_review_region(label: &str, style: &str, content: Element) -> Element {
    rsx! {
        div {
            class: "feature-scroll text-review-region",
            tabindex: "0",
            role: "region",
            aria_label: label,
            style: style,
            {content}
        }
    }
}

fn exact_yaml_review(review: &str) -> Element {
    text_review_region(
        "Exact generated YAML review (contains secrets)",
        "max-height: 28rem; overflow: auto; overflow-wrap: anywhere;",
        rsx! { pre { style: "white-space: pre-wrap;", "{review}" } },
    )
}

async fn read_on_runtime<T: Send + 'static>(
    runtime: &Handle,
    future: impl Future<Output = T> + Send + 'static,
) -> Result<T, String> {
    let mut worker = AbortOnDrop(runtime.spawn(future));
    (&mut worker.0)
        .await
        .map_err(|error| format!("Read worker stopped: {error}"))
}

async fn run_action(
    runtime: &Handle,
    action: MaintenanceAction,
    control: Arc<Mutex<WorkControl>>,
) -> Result<MaintenanceEvent, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let mutation = is_mutation(&action);
    let guard = if mutation {
        Some(try_begin_operation("Maintenance workflow".into())?)
    } else {
        None
    };
    {
        let mut control = control.lock().expect("maintenance control");
        control.cancellation = Some(cancel.clone());
        control.read_abort = None;
    }
    let task = runtime.spawn(async move {
        let cancelled = || {
            cancel.load(Ordering::Acquire) || guard.as_ref().is_some_and(|guard| guard.cancelled())
        };
        // A mutation task retains the shared guard independently of the
        // component/JoinHandle. Dropping UI work never aborts submitted I/O.
        let result = execute_maintenance_action_bounded(action, cancelled, READ_TIMEOUT).await;
        drop(guard);
        result
    });
    let result = if mutation {
        task.await
            .map_err(|error| format!("Mutation worker failed; outcome may be unknown: {error}"))
    } else {
        control.lock().expect("maintenance control").read_abort = Some(task.abort_handle());
        let mut task = AbortOnDrop(task);
        match (&mut task.0).await {
            Ok(event) => Ok(event),
            Err(error) if error.is_cancelled() => Ok(MaintenanceEvent::Cancelled),
            Err(error) => Err(error.to_string()),
        }
    };
    let mut control = control.lock().expect("maintenance control");
    control.cancellation = None;
    control.read_abort = None;
    result
}

#[allow(non_snake_case)]
pub(crate) fn MaintenancePanel(props: MaintenancePanelProps) -> Element {
    let mut draft = use_signal(|| {
        MaintenanceDraft::new(
            props
                .options
                .maintenance_endpoint
                .as_deref()
                .unwrap_or_default(),
        )
    });
    let mut state = use_signal(ViewState::default);
    let mut apply_text = use_signal(String::new);
    let mut bootstrap_text = use_signal(String::new);
    let control = use_hook(|| Arc::new(Mutex::new(WorkControl::default())));
    let (sender, receiver) = use_hook(|| {
        let (sender, receiver) = mpsc::channel::<Command>(8);
        (sender, Arc::new(Mutex::new(Some(receiver))))
    });
    let drop_control = control.clone();
    use_drop(move || drop_control.lock().expect("maintenance control").cancel());

    let work_runtime = props.runtime.clone();
    let work_control = control.clone();
    use_future(move || {
        let mut receiver = receiver
            .lock()
            .expect("maintenance receiver")
            .take()
            .expect("single maintenance worker");
        let runtime = work_runtime.clone();
        let control = work_control.clone();
        async move {
            while let Some(command) = receiver.recv().await {
                if state.peek().busy {
                    continue;
                }
                if matches!(command, Command::PickDirectory) {
                    if state.peek().session.is_some() || operation_busy() {
                        continue;
                    }
                    state.write().busy = true;
                    let picked = read_on_runtime(&runtime, async {
                        tokio::task::spawn_blocking(|| {
                            rfd::FileDialog::new()
                                .set_title("Configuration output directory")
                                .pick_folder()
                        })
                        .await
                        .map_err(|error| error.to_string())
                    })
                    .await;
                    state.write().busy = false;
                    match picked {
                        Ok(Ok(Some(path))) => match path.to_str() {
                            Some(path) => draft.write().output = path.to_string(),
                            None => state.write().error = Some("Output directory must be a valid UTF-8 path; the selected path was not changed or approximated".into()),
                        },
                        Ok(Ok(None)) => {},
                        Ok(Err(error)) | Err(error) => state.write().error = Some(error),
                    }
                    continue;
                }
                let current = state.peek().clone();
                let action = match command {
                    Command::Start(plan) => {
                        if current.session.is_some() || operation_busy() {
                            continue;
                        }
                        let session = BootstrapSession::new(plan);
                        let action = session.initial_collection_action();
                        let mut view = state.write();
                        *view = ViewState {
                            session: Some(session),
                            auto_poll: true,
                            ..Default::default()
                        };
                        view.record("Collecting insecure Talos version, hardware/disks and volumes; no credentials used.");
                        action
                    }
                    Command::SelectDisk(path) => {
                        if let Some(session) = &current.session {
                            match session.select_install_target(path) {
                                Ok(event) => state.write().reduce(event),
                                Err(error) => state.write().error = Some(error.to_string()),
                            }
                        }
                        None
                    }
                    Command::Generate => {
                        let Some(session) = &current.session else {
                            continue;
                        };
                        match session.begin_configuration_generation() {
                            Ok((next, action)) => {
                                state.write().session = Some(next);
                                Some(action)
                            }
                            Err(error) => {
                                state.write().error = Some(error.to_string());
                                None
                            }
                        }
                    }
                    Command::Review => {
                        let Some(configuration) = current
                            .session
                            .as_ref()
                            .and_then(|session| session.generated_configuration.as_ref())
                        else {
                            continue;
                        };
                        let path = configuration.machine_config_path.clone();
                        state.write().busy = true;
                        let review = read_on_runtime(&runtime, async move {
                            tokio::task::spawn_blocking(move || frozen_review(&path))
                                .await
                                .map_err(|error| error.to_string())?
                        })
                        .await;
                        let mut view = state.write();
                        view.busy = false;
                        match review {
                            Ok(Ok(content)) => {
                                view.review = Some(content);
                                view.error = None;
                            }
                            Ok(Err(error)) | Err(error) => {
                                view.review = None;
                                view.error = Some(error);
                            }
                        }
                        None
                    }
                    Command::PrepareApply => {
                        let Some(session) = &current.session else {
                            continue;
                        };
                        let Some(content) = current.review else {
                            state.write().error = Some(
                                "Read and review configuration before requesting apply".into(),
                            );
                            continue;
                        };
                        match session.request_reviewed_configuration_application(content) {
                            Ok((next, _)) => {
                                state.write().session = Some(next);
                                apply_text.set(String::new());
                            }
                            Err(error) => state.write().error = Some(error.to_string()),
                        }
                        None
                    }
                    Command::Apply(typed) => {
                        if !confirmation_matches(current.apply_phrase(), &typed) {
                            state.write().error =
                                Some("Exact apply confirmation does not match".into());
                            continue;
                        }
                        let Some(session) = &current.session else {
                            continue;
                        };
                        let Some(confirmation) = session.pending_confirmation.clone() else {
                            continue;
                        };
                        match session.confirm_configuration_application(confirmation) {
                            Ok((next, action)) => {
                                state.write().session = Some(next);
                                Some(action)
                            }
                            Err(error) => {
                                state.write().error = Some(error.to_string());
                                None
                            }
                        }
                    }
                    Command::Bootstrap(typed) => {
                        if !confirmation_matches(current.bootstrap_phrase(), &typed) {
                            state.write().error =
                                Some("Exact bootstrap confirmation does not match".into());
                            continue;
                        }
                        let Some(session) = &current.session else {
                            continue;
                        };
                        if session.plan.role != MachineRole::ControlPlane {
                            state.write().error =
                                Some("Bootstrap is only valid for a control-plane node".into());
                            continue;
                        }
                        match session.begin_bootstrap() {
                            Ok((next, action)) => {
                                state.write().session = Some(next);
                                Some(action)
                            }
                            Err(error) => {
                                state.write().error = Some(error.to_string());
                                None
                            }
                        }
                    }
                    Command::Poll => current
                        .session
                        .as_ref()
                        .and_then(BootstrapSession::polling_action),
                    Command::PickDirectory => None,
                };
                let Some(action) = action else {
                    continue;
                };
                state.write().busy = true;
                state.write().error = None;
                let event = run_action(&runtime, action, control.clone()).await;
                let mut view = state.write();
                view.busy = false;
                match event {
                    Ok(event) => view.reduce(event),
                    Err(error) => {
                        view.reduce(MaintenanceEvent::OperationFailed(
                            MaintenanceError::ConfigurationApplicationFailed {
                                message: error.clone(),
                            },
                        ));
                        view.error = Some(error.clone());
                        view.record(error);
                    }
                }
            }
        }
    });

    let timer_runtime = props.runtime.clone();
    let timer_sender = sender.clone();
    use_future(move || {
        let runtime = timer_runtime.clone();
        let sender = timer_sender.clone();
        async move {
            loop {
                if read_on_runtime(&runtime, async {
                    tokio::time::sleep(BOOTSTRAP_POLL_INTERVAL).await
                })
                .await
                .is_err()
                {
                    return;
                }
                let view = state.peek();
                if view.auto_poll
                    && !view.busy
                    && view
                        .session
                        .as_ref()
                        .and_then(BootstrapSession::polling_action)
                        .is_some()
                {
                    let _ = sender.try_send(Command::Poll);
                }
            }
        }
    });

    let view = state.read().clone();
    let form = draft.read().clone();
    let locked = view.busy || view.session.is_some() || operation_busy();
    let busy = view.busy || operation_busy();
    let phase = view.session.as_ref().map(|session| session.phase.clone());
    let session = view.session.clone();
    let apply_phrase = view.apply_phrase().unwrap_or_default();
    let bootstrap_phrase = view.bootstrap_phrase().unwrap_or_default();
    let start_sender = sender.clone();
    let pick_sender = sender.clone();
    let generate_sender = sender.clone();
    let review_sender = sender.clone();
    let prepare_sender = sender.clone();
    let apply_sender = sender.clone();
    let bootstrap_sender = sender.clone();
    let poll_sender = sender.clone();
    let select_sender = sender.clone();
    let cancel_control = control.clone();
    rsx! {
        section { class: "panel maintenance-panel",
            header { class: "feature-heading", h2 { "Maintenance · installation and bootstrap" } }
            p { class: "feature-status warning", "Insecure Talos access: verify the exact physical node. Applying configuration can erase the selected disk. No ambient Talos/Kubernetes cluster is used." }
            div { class: "feature-split",
                div { class: "feature-detail",
                    label { "Insecure Talos endpoint"
                        input { value: "{form.endpoint}", disabled: locked, oninput: move |event| draft.write().endpoint = event.value() }
                    }
                    label { "Cluster name"
                        input { value: "{form.cluster}", disabled: locked, oninput: move |event| {
                            draft.write().set_cluster_name(event.value());
                        } }
                    }
                    label { "Kubernetes API HTTPS endpoint"
                        input { value: "{form.kubernetes_api}", disabled: locked, oninput: move |event| draft.write().kubernetes_api = event.value() }
                    }
                    label { "Configuration output directory (generation overwrites files in this directory)"
                        input { value: "{form.output}", disabled: locked, oninput: move |event| draft.write().output = event.value() }
                    }
                    button { disabled: locked, onclick: move |_| { let _ = pick_sender.try_send(Command::PickDirectory); }, "Choose output directory…" }
                    label { "Machine role"
                        select { value: if form.role == MachineRole::ControlPlane { "controlplane" } else { "worker" }, disabled: locked,
                            onchange: move |event| draft.write().role = if event.value() == "worker" { MachineRole::Worker } else { MachineRole::ControlPlane },
                            option { value: "controlplane", "Control plane" }
                            option { value: "worker", "Worker (never bootstrap etcd on this node)" }
                        }
                    }
                    label { "Authenticated Talos context (generated context normally equals cluster name)"
                        input { value: "{form.context}", disabled: locked, oninput: move |event| draft.write().context = event.value() }
                    }
                    p { "Authenticated Talosconfig: {form.output}/talosconfig" }
                    label { "Exact Talos node to wait for / bootstrap (must match maintenance endpoint)"
                        input { value: "{form.bootstrap_node}", disabled: locked, oninput: move |event| draft.write().bootstrap_node = event.value() }
                    }
                    label { "Expected Kubernetes node name (optional; no IP-to-name inference)"
                        input { value: "{form.kubernetes_node}", disabled: locked, oninput: move |event| draft.write().kubernetes_node = event.value() }
                    }
                    label { "Selected kubeconfig path (optional; empty uses authenticated Talos API)"
                        input { value: "{form.kubeconfig}", disabled: locked, oninput: move |event| draft.write().kubeconfig = event.value() }
                    }
                    p { "A selected kubeconfig must match the CA/TLS identity from this authenticated Talos node before any credentials or plugins are used. Rejection is unavailable, never an ambient fallback." }
                    button { disabled: locked, onclick: move |_| match draft.peek().plan() {
                        Ok(plan) => { let _ = start_sender.try_send(Command::Start(plan)); }
                        Err(error) => state.write().error = Some(error.to_string()),
                    }, "Connect insecurely and inspect disks" }
                }
                div { class: "feature-detail",
                    if let Some(error) = &view.error { p { class: "feature-status error", "{error}" } }
                    if let Some(session) = &session {
                        h3 { "Phase: {session.phase:?}" }
                        p { "Target: {session.plan.endpoint} · authenticated context: {session.plan.authenticated_target.talos_context} · poll attempts: {session.poll_attempts}" }
                        if let Some(message) = &session.last_message { p { class: "feature-status", "{message}" } }
                        if let Some(snapshot) = &session.insecure_snapshot {
                            h3 { "Talos hardware / install disks" }
                            {text_review_region(
                                "Insecure Talos version and volume evidence",
                                "max-height: 18rem; overflow: auto; overflow-wrap: anywhere;",
                                rsx! {
                                    p { "Version evidence: {snapshot.version:?}" }
                                    pre { style: "white-space: pre-wrap;", "Volumes (bounded display): " {format!("{:?}", snapshot.volumes).chars().take(16384).collect::<String>()} }
                                },
                            )}
                            div { class: "table-scroll", table {
                                thead { tr { th { "Device / ID" } th { "Size / model / serial" } th { "Safety / selection" } } }
                                tbody { for disk in snapshot.disks.iter().take(256) {
                                    tr { key: "{disk.id}",
                                        td { "{disk.dev_path} / {disk.id}" }
                                        td { "{disk.size_pretty} · {disk.model:?} · {disk.serial:?}" }
                                        td { "Read-only: {disk.readonly}; optical: {disk.cdrom}"
                                            button { disabled: busy || phase != Some(BootstrapPhase::SelectingInstallTarget) || disk.readonly || disk.cdrom,
                                                onclick: { let sender = select_sender.clone(); let path = disk.dev_path.clone(); move |_| { let _ = sender.try_send(Command::SelectDisk(path.clone())); } },
                                                "Select install disk"
                                            }
                                        }
                                    }
                                } }
                            } }
                        }
                        if let Some(target) = &session.install_target { p { class: "feature-status warning", "Selected install disk: {target.device_path()} · Talos disk ID: {target.disk_id()}" } }
                        if phase == Some(BootstrapPhase::Configuring) {
                            button { disabled: busy, onclick: move |_| { let _ = generate_sender.try_send(Command::Generate); }, "Generate configuration with selected install disk" }
                        }
                        if let Some(configuration) = &session.generated_configuration {
                            p { "Generated machine configuration: {configuration.machine_config_path.display()}" }
                            p { "Generated credentials: {configuration.talosconfig_path.display()}" }
                            button { disabled: busy || phase != Some(BootstrapPhase::ConfigurationReady), onclick: move |_| { let _ = review_sender.try_send(Command::Review); }, "Read configuration for review" }
                        }
                        if let Some(review) = &view.review {
                            details { open: true, summary { "Review exact generated YAML (contains secrets; do not share)" }
                                {exact_yaml_review(review)}
                            }
                            button { disabled: busy || phase != Some(BootstrapPhase::ConfigurationReady), onclick: move |_| { let _ = prepare_sender.try_send(Command::PrepareApply); }, "I reviewed the YAML — request apply confirmation" }
                        }
                        if phase == Some(BootstrapPhase::AwaitingApplyConfirmation) {
                            div { class: "confirmation",
                                h3 { "Destructive install confirmation" }
                                p { "Apply the frozen reviewed YAML to {session.plan.endpoint}. This may erase the selected install disk and will install/reboot the node. Target, role, disk and credentials are locked." }
                                p { "Type exactly: {apply_phrase}" }
                                input { aria_label: "Exact apply confirmation", value: "{apply_text}", disabled: busy, oninput: move |event| apply_text.set(event.value()) }
                                button { class: "danger", disabled: busy || !confirmation_matches(view.apply_phrase(), &apply_text.read()),
                                    onclick: move |_| { let _ = apply_sender.try_send(Command::Apply(apply_text.peek().clone())); }, "Confirm install / apply"
                                }
                            }
                        }
                        if phase == Some(BootstrapPhase::ReadyToBootstrap) && session.plan.role == MachineRole::ControlPlane {
                            div { class: "confirmation",
                                h3 { "Separate cluster bootstrap confirmation" }
                                p { "Secure Talos API responded from the explicit node. Bootstrap initializes etcd and must run on the intended first control plane only." }
                                p { "Type exactly: {bootstrap_phrase}" }
                                input { aria_label: "Exact bootstrap confirmation", value: "{bootstrap_text}", disabled: busy, oninput: move |event| bootstrap_text.set(event.value()) }
                                button { class: "danger", disabled: busy || !confirmation_matches(view.bootstrap_phrase(), &bootstrap_text.read()),
                                    onclick: move |_| { let _ = bootstrap_sender.try_send(Command::Bootstrap(bootstrap_text.peek().clone())); }, "Confirm bootstrap of exact node"
                                }
                            }
                        }
                        if phase == Some(BootstrapPhase::ReadyToBootstrap) && session.plan.role == MachineRole::Worker {
                            p { class: "feature-status", "Worker installation reached its secure Talos API. Do not bootstrap etcd on a worker. Cluster bootstrap must be performed separately on the first control plane." }
                        }
                        if let Some(evidence) = &session.latest_readiness {
                            h3 { "API-backed readiness evidence (unknown is not healthy)" }
                            {text_review_region(
                                "API-backed bootstrap readiness evidence",
                                "max-height: 22rem; overflow: auto; overflow-wrap: anywhere;",
                                rsx! { pre { style: "white-space: pre-wrap;", {format!("{evidence:#?}").chars().take(65536).collect::<String>()} } },
                            )}
                        }
                        div { class: "feature-toolbar",
                            button { disabled: busy || session.polling_action().is_none(), onclick: move |_| { state.write().auto_poll = true; let _ = poll_sender.try_send(Command::Poll); }, "Poll secure Talos / etcd / Kubernetes now" }
                            button { disabled: matches!(phase, Some(BootstrapPhase::Complete | BootstrapPhase::Cancelled | BootstrapPhase::Failed(_))), onclick: move |_| {
                                cancel_control.lock().expect("maintenance control").cancel();
                                let mut view = state.write(); view.auto_poll = false;
                                if !view.busy { view.reduce(MaintenanceEvent::Cancelled); }
                                view.record("Cancellation requested. Submitted mutations are not aborted; their actual result is retained. Automatic polling stopped.");
                            }, "Cancel / stop polling cooperatively" }
                            button { disabled: busy, onclick: move |_| {
                                *state.write() = ViewState::default(); apply_text.set(String::new()); bootstrap_text.set(String::new());
                            }, "Reset workflow (keeps generated files)" }
                        }
                    } else { p { "Choose cluster settings, then collect hardware. An install disk must be explicitly selected before generation. No node mutation is automatic." } }
                    h3 { "Progress / results" }
                    if view.progress.is_empty() {
                        p { class: "muted", "No progress recorded yet." }
                    } else {
                        {text_review_region(
                            "Maintenance progress and results",
                            "max-height: 18rem; overflow: auto; overflow-wrap: anywhere;",
                            rsx! { for (index, message) in view.progress.iter().enumerate() { p { key: "{index}", "{message}" } } },
                        )}
                    }
                    if busy { p { class: "feature-status", "Worker running. Identity changes and navigation are disabled during submitted mutations." } }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::core::{AttributeValue, DynamicNode, TemplateAttribute, TemplateNode};
    use std::path::PathBuf;

    fn root_attribute(node: &VNode, name: &str) -> Option<String> {
        let TemplateNode::Element { attrs, .. } = &node.template.roots[0] else {
            return node.dynamic_nodes.iter().find_map(|dynamic| match dynamic {
                DynamicNode::Fragment(nodes) => {
                    nodes.iter().find_map(|node| root_attribute(node, name))
                }
                _ => None,
            });
        };
        for attribute in *attrs {
            match attribute {
                TemplateAttribute::Static {
                    name: key, value, ..
                } if *key == name => {
                    return Some((*value).to_string());
                }
                TemplateAttribute::Dynamic { id } => {
                    for attribute in node.dynamic_attrs[*id].iter() {
                        if attribute.name == name {
                            return match &attribute.value {
                                AttributeValue::Text(value) => Some(value.clone()),
                                AttributeValue::Int(value) => Some(value.to_string()),
                                _ => None,
                            };
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn rendered_review_text(node: &VNode) -> String {
        fn template_text(nodes: &[TemplateNode], output: &mut String) {
            for node in nodes {
                match node {
                    TemplateNode::Text { text } => output.push_str(text),
                    TemplateNode::Element { children, .. } => template_text(children, output),
                    TemplateNode::Dynamic { .. } => {}
                }
            }
        }
        let mut output = String::new();
        template_text(node.template.roots, &mut output);
        for dynamic in node.dynamic_nodes.iter() {
            match dynamic {
                DynamicNode::Text(text) => output.push_str(&text.value),
                DynamicNode::Fragment(nodes) => {
                    for node in nodes {
                        output.push_str(&rendered_review_text(node));
                    }
                }
                DynamicNode::Component(_) => panic!("Text-only review fixture must not need hooks"),
                DynamicNode::Placeholder(_) => {}
            }
        }
        output
    }

    #[test]
    fn text_review_helper_emits_a_labeled_keyboard_scroll_region() {
        let node = text_review_region(
            "Raw evidence details",
            "max-height: 18rem; overflow: auto;",
            rsx! { pre { "line one\nline two" } },
        )
        .unwrap();
        assert_eq!(root_attribute(&node, "role").as_deref(), Some("region"));
        assert_eq!(root_attribute(&node, "tabindex").as_deref(), Some("0"));
        assert_eq!(
            root_attribute(&node, "aria-label").as_deref(),
            Some("Raw evidence details")
        );
        assert!(
            root_attribute(&node, "class")
                .unwrap()
                .split_whitespace()
                .any(|class| class == "text-review-region")
        );
        assert!(
            root_attribute(&node, "style")
                .unwrap()
                .contains("overflow: auto")
        );
        assert_eq!(rendered_review_text(&node), "line one\nline two");
    }

    #[test]
    fn headless_exact_yaml_review_is_focusable_and_retains_every_reviewed_byte() {
        fn fixture() -> Element {
            let yaml = format!(
                "machine:\n  token: '<secret&value>'\n{}# final reviewed line\n",
                "# review evidence\n".repeat(6000)
            );
            exact_yaml_review(&yaml)
        }
        let mut dom = VirtualDom::new(fixture);
        dom.rebuild_in_place();
        // The base scope is an artificial component wrapper. Render the mounted
        // component tree so these assertions inspect the actual review DOM.
        let rendered = dioxus_ssr::render(&dom);
        if let Some(directory) = std::env::var_os("TALOS_PILOT_UI_SMOKE_DIR") {
            let html = format!(
                "<!doctype html><html><head><style>{}</style></head><body>{rendered}</body></html>",
                include_str!("style.css")
            );
            std::fs::write(
                PathBuf::from(directory).join("maintenance-yaml-review.html"),
                html,
            )
            .unwrap();
        }
        let (region, content) = rendered.split_once('>').unwrap();
        assert!(region.starts_with("<div "));
        assert!(region.contains(" role=\"region\""));
        assert!(region.contains(" tabindex=\"0\""));
        assert!(region.contains(" aria-label=\"Exact generated YAML review (contains secrets)\""));
        let style = region
            .split_once(" style=\"")
            .unwrap()
            .1
            .split_once('"')
            .unwrap()
            .0;
        assert!(style.contains("max-height: 28rem"));
        assert!(style.contains("overflow: auto"));
        let (pre, content) = content.split_once('>').unwrap();
        assert!(pre.starts_with("<pre "));
        assert!(pre.contains("white-space: pre-wrap;"));
        let reviewed_html = content.strip_suffix("</pre></div>").unwrap();
        let expected = format!(
            "machine:\n  token: '<secret&value>'\n{}# final reviewed line\n",
            "# review evidence\n".repeat(6000)
        );
        // Compare every reviewed byte after only the renderer's HTML escaping;
        // no line sampling, truncation, or whitespace normalization is allowed.
        let escaped_expected = expected
            .replace('&', "&#38;")
            .replace('<', "&#60;")
            .replace('>', "&#62;")
            .replace('\'', "&#39;")
            .replace('"', "&#34;");
        assert_eq!(reviewed_html, escaped_expected);
    }

    #[test]
    fn cancellation_is_cooperative() {
        let flag = Arc::new(AtomicBool::new(false));
        let mut control = WorkControl {
            cancellation: Some(flag.clone()),
            read_abort: None,
        };
        control.cancel();
        assert!(flag.load(Ordering::Acquire));
    }

    #[test]
    fn component_contains_all_explicit_workflow_controls() {
        let source = include_str!("maintenance.rs");
        for label in [
            "Select install disk",
            "Read configuration for review",
            "Confirm install / apply",
            "Confirm bootstrap of exact node",
            "Poll secure Talos / etcd / Kubernetes now",
            "Choose output directory",
        ] {
            assert!(source.contains(label));
        }
        assert!(source.contains("try_begin_operation"));
        assert!(source.contains("request_reviewed_configuration_application"));
    }
}
