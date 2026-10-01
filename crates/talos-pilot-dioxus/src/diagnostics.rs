use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use dioxus::prelude::*;
use talos_pilot_core::{
    diagnostic_runner::{
        DiagnosticCheck, DiagnosticCollector, DiagnosticFixAction, DiagnosticFixRequest,
        DiagnosticSnapshot, DiagnosticTarget, FixExecution, KubernetesAccess, SourceUnavailable,
        execute_confirmed_fix,
    },
    diagnostics::{CheckCategory, CheckStatus},
};
use tokio::sync::Notify;

use crate::{
    feature::{FeatureContext, LogRequest, copy_text},
    maintenance::text_review_region,
    operations::{operation_busy, try_acquire_mutation},
};

const PAGE_SIZE: usize = 48;
const MAX_TEXT: usize = 32 * 1024;
const CATEGORIES: [CheckCategory; 5] = [
    CheckCategory::System,
    CheckCategory::Kubernetes,
    CheckCategory::Services,
    CheckCategory::Cni,
    CheckCategory::Addons,
];

#[derive(Clone)]
struct PendingFix {
    key: String,
    request: DiagnosticFixRequest,
}

#[derive(Clone, Default)]
struct DiagnosticView {
    key: String,
    sequence: u64,
    loading: bool,
    snapshot: Option<DiagnosticSnapshot>,
    refreshed: Option<Instant>,
    error: Option<String>,
    category: Option<CheckCategory>,
    status: Option<CheckStatus>,
    query: String,
    selected: Option<String>,
    page: usize,
    pending: Option<PendingFix>,
    executing: bool,
    result: Option<String>,
}

impl DiagnosticView {
    fn begin(&mut self) -> (String, u64) {
        self.sequence = self.sequence.wrapping_add(1);
        self.loading = true;
        (self.key.clone(), self.sequence)
    }

    fn accept(
        &mut self,
        ticket: &(String, u64),
        result: Result<DiagnosticSnapshot, String>,
    ) -> bool {
        if self.key != ticket.0 || self.sequence != ticket.1 {
            return false;
        }
        self.loading = false;
        match result {
            Ok(snapshot) => {
                self.snapshot = Some(snapshot);
                self.refreshed = Some(Instant::now());
                self.error = None;
                self.reconcile_selection();
            }
            Err(error) => self.error = Some(error),
        }
        true
    }

    fn visible(&self) -> Vec<&DiagnosticCheck> {
        let query = self.query.to_lowercase();
        self.snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .checks
                    .iter()
                    .filter(|check| {
                        self.category
                            .is_none_or(|category| check.category == category)
                            && self
                                .status
                                .as_ref()
                                .is_none_or(|status| check.status == *status)
                            && (query.is_empty()
                                || format!(
                                    "{} {} {} {}",
                                    check.id,
                                    check.name,
                                    check.message,
                                    check.details.as_deref().unwrap_or("")
                                )
                                .to_lowercase()
                                .contains(&query))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn reconcile_selection(&mut self) {
        let visible = self.visible();
        let pages = visible.len().div_ceil(PAGE_SIZE).max(1);
        let selected = self
            .selected
            .as_ref()
            .filter(|id| visible.iter().any(|check| &check.id == *id))
            .cloned()
            .or_else(|| visible.first().map(|check| check.id.clone()));
        self.selected = selected;
        self.page = self.page.min(pages - 1);
    }

    fn navigate(&mut self, forward: bool) {
        let visible = self.visible();
        if visible.is_empty() {
            return;
        }
        let index = self
            .selected
            .as_ref()
            .and_then(|id| visible.iter().position(|check| &check.id == id))
            .unwrap_or(0);
        let next = if forward {
            (index + 1).min(visible.len() - 1)
        } else {
            index.saturating_sub(1)
        };
        let id = visible[next].id.clone();
        self.selected = Some(id);
        self.page = next / PAGE_SIZE;
    }

    fn selected_check(&self) -> Option<&DiagnosticCheck> {
        let id = self.selected.as_ref()?;
        self.visible().into_iter().find(|check| &check.id == id)
    }

    fn valid_pending(&self, pending: &PendingFix) -> bool {
        !self.loading
            && self.error.is_none()
            && self.key == pending.key
            && self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.context.target == pending.request.target
                    && snapshot.checks.iter().any(|check| {
                        check.id == pending.request.check_id
                            && check
                                .fix
                                .as_ref()
                                .is_some_and(|fix| fix.action == pending.request.fix.action)
                    })
            })
    }
}

fn bounded(text: &str) -> String {
    if text.len() <= MAX_TEXT {
        return text.to_string();
    }
    let mut end = MAX_TEXT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[Display truncated; copy retains the complete value]",
        &text[..end]
    )
}

fn target(ctx: &FeatureContext) -> DiagnosticTarget {
    let role = if ctx.cluster.node_is_controlplane(&ctx.node)
        || ctx.cluster.node_is_controlplane(&ctx.address)
    {
        "controlplane"
    } else {
        "worker"
    };
    let target = DiagnosticTarget::new(&ctx.node, &ctx.address, role, &ctx.context);
    match ctx.cluster.control_plane_ip() {
        Some(address) => target.with_control_plane_address(address),
        None => target,
    }
}

fn log_services(check: &DiagnosticCheck) -> Vec<String> {
    if let Some(service) = check.id.strip_prefix("service_") {
        vec![service.to_string()]
    } else if check.id.starts_with("etcd") {
        vec!["etcd".to_string()]
    } else if matches!(
        check.category,
        CheckCategory::Kubernetes | CheckCategory::Cni
    ) {
        vec!["kubelet".to_string()]
    } else {
        Vec::new()
    }
}

fn execution_text(result: Result<FixExecution, String>) -> String {
    match result {
        Err(error) => format!("Remediation failed: {error}"),
        Ok(FixExecution::Cancelled { phase, .. }) => format!(
            "Cancelled at {phase:?}; no subsequent step was executed. An RPC already started was allowed to finish."
        ),
        Ok(FixExecution::CopyOnly { .. }) => {
            "Copy-only guidance: no command was executed.".to_string()
        }
        Ok(FixExecution::Applied {
            results,
            requires_reboot,
        }) => {
            let details = results
                .into_iter()
                .map(|result| {
                    format!(
                        "{}: {}{}",
                        result.node,
                        result.message,
                        if result.warnings.is_empty() {
                            String::new()
                        } else {
                            format!("\nWarnings: {}", result.warnings.join("; "))
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "Talos accepted the remediation. {}\n{details}",
                if requires_reboot {
                    "A reboot was requested; this is not evidence that reboot or recovery completed."
                } else {
                    "Refresh diagnostics to verify the resulting state."
                }
            )
        }
    }
}

// A mutation cannot be aborted mid-RPC. Dropping its UI waiter only requests
// cancellation before the next core step; the runtime owns the global guard.
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[component]
pub(crate) fn DiagnosticsPanel(ctx: FeatureContext, on_logs: EventHandler<LogRequest>) -> Element {
    let initial_key = ctx.key();
    let mut view = use_signal(move || DiagnosticView {
        key: initial_key,
        ..Default::default()
    });
    let mut cancellation = use_signal(|| None::<Arc<AtomicBool>>);
    let wake = use_hook(|| Arc::new(Notify::new()));
    let collect_ctx = ctx.clone();
    let collect_wake = wake.clone();
    use_future(move || {
        let ctx = collect_ctx.clone();
        let wake = collect_wake.clone();
        async move {
            loop {
                if !operation_busy() {
                    let ticket = view.write().begin();
                    let work_ctx = ctx.clone();
                    let result = ctx
                        .run(async move {
                            tokio::time::timeout(Duration::from_secs(90), async move {
                                let target = target(&work_ctx);
                                let access = match work_ctx.kubernetes_with_source().await {
                                    Ok((client, source, warning)) => Ok((
                                        client,
                                        KubernetesAccess {
                                            control_plane_address: target
                                                .control_plane_address
                                                .clone()
                                                .unwrap_or_default(),
                                            config_identity: target.config_identity.clone(),
                                            source: Some(source),
                                            warning,
                                        },
                                    )),
                                    Err(reason) => Err(SourceUnavailable {
                                        source: "Selected Kubernetes access".to_string(),
                                        reason,
                                    }),
                                };
                                DiagnosticCollector::new(target)
                                    .collect_with_kubernetes(&work_ctx.client, access)
                                    .await
                            })
                            .await
                            .map_err(|_| {
                                "Diagnostics refresh timed out; prior results are stale."
                                    .to_string()
                            })
                        })
                        .await
                        .and_then(|result| result);
                    view.write().accept(&ticket, result);
                }
                let wake_wait = wake.clone();
                let _ = ctx
                    .run(async move {
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_secs(10)) => {},
                            _ = wake_wait.notified() => {},
                        }
                    })
                    .await;
            }
        }
    });

    let state = view.read().clone();
    let visible = state.visible();
    let total = visible.len();
    let rows = visible
        .into_iter()
        .skip(state.page * PAGE_SIZE)
        .take(PAGE_SIZE)
        .cloned()
        .collect::<Vec<_>>();
    let selected = state.selected_check().cloned();
    let pending = state.pending.clone();
    let busy = state.executing || operation_busy();
    let refresh_wake = wake.clone();
    let confirm_ctx = ctx.clone();
    let copy_ctx = ctx.clone();
    let counts = state
        .snapshot
        .as_ref()
        .map(|snapshot| {
            [
                CheckStatus::Pass,
                CheckStatus::Warn,
                CheckStatus::Fail,
                CheckStatus::Unknown,
            ]
            .iter()
            .map(|status| {
                format!(
                    "{status:?}: {}",
                    snapshot
                        .checks
                        .iter()
                        .filter(|check| check.status == *status)
                        .count()
                )
            })
            .collect::<Vec<_>>()
            .join(" · ")
        })
        .unwrap_or_else(|| "No diagnostic snapshot yet".to_string());
    let age = state
        .refreshed
        .map(|at| format!("Last completed refresh {}s ago", at.elapsed().as_secs()))
        .unwrap_or_default();
    let evidence = state.snapshot.as_ref().map(|snapshot| format!(
        "Platform: {:?}\nCPU count: {:?}\nKubernetes access: {:?}\n\nSystem:\n{:#?}\n\nServices:\n{:#?}\n\nEtcd:\n{:#?}\n\nKubernetes:\n{:#?}\n\nCNI:\n{:#?}\n\nAddons:\n{:#?}",
        snapshot.context.platform, snapshot.context.cpu_count, snapshot.context.kubernetes_access,
        snapshot.system, snapshot.services, snapshot.etcd, snapshot.kubernetes, snapshot.cni, snapshot.addons,
    ));
    let addon_rows = state
        .snapshot
        .as_ref()
        .map(|snapshot| snapshot.addons.addons.clone())
        .unwrap_or_default();
    let kubernetes_source = ctx
        .cluster
        .kubeconfig_source
        .clone()
        .unwrap_or_else(|| "selected source not yet available".to_string());
    rsx! {
        section { class: "panel diagnostics",
            h2 { "Diagnostics" }
            p { "Context: {ctx.context} · Node: {ctx.node} · Address: {ctx.address}" }
            p { "{counts}" }
            p { "{age}" }
            p { "Kubernetes source: {kubernetes_source}" }
            button { disabled: state.loading || busy, onclick: move |_| refresh_wake.notify_one(), "Refresh" }
            if state.loading { p { role: "status", "Checking current system state…" } }
            if let Some(error) = &state.error { p { class: "feature-status error", "{error} Previous results, if shown, are stale." } }
            if let Some(evidence) = evidence {
                details { class: "feature-detail",
                    summary { "Collected source evidence and addon discovery" }
                    table {
                        thead { tr { th { "Supported addon" } th { "Current discovery status" } } }
                        tbody { for addon in addon_rows { tr { key: "{addon.id}", td { "{addon.name}" } td { "{addon.presence:?}" } } } }
                    }
                    p { "NotDetected means direct sources were queried; Unknown means access or evidence was unavailable." }
                    {text_review_region(
                        "Collected diagnostic source evidence",
                        "max-height: 340px; overflow: auto; overflow-wrap: anywhere;",
                        rsx! { pre { style: "white-space: pre-wrap;", "{bounded(&evidence)}" } },
                    )}
                    button { onclick: { let ctx = copy_ctx.clone(); let text = evidence.clone(); move |_| {
                        let ctx = ctx.clone(); let text = text.clone();
                        spawn(async move { let result = copy_text(ctx, text).await;
                            view.write().result = Some(match result { Ok(()) => "Complete source evidence copied.".to_string(), Err(error) => format!("Copy failed: {error}") });
                        });
                    } }, "Copy complete source evidence" }
                }
            }
            div { class: "toolbar",
                button { aria_pressed: state.category.is_none(), onclick: move |_| { let mut model = view.write(); model.category = None; model.page = 0; model.reconcile_selection(); }, "All categories" }
                for category in CATEGORIES {
                    button { aria_pressed: state.category == Some(category), onclick: move |_| { let mut model = view.write(); model.category = Some(category); model.page = 0; model.reconcile_selection(); }, "{category.title()}" }
                }
                label { "Search checks and evidence "
                    input { value: "{state.query}", oninput: move |event| { let mut model = view.write(); model.query = event.value(); model.page = 0; model.reconcile_selection(); } }
                }
                select { aria_label: "Diagnostic status filter", value: state.status.as_ref().map(|status| format!("{status:?}")).unwrap_or_default(),
                    onchange: move |event| {
                        let mut model = view.write();
                        model.status = match event.value().as_str() { "Pass" => Some(CheckStatus::Pass), "Warn" => Some(CheckStatus::Warn), "Fail" => Some(CheckStatus::Fail), "Unknown" => Some(CheckStatus::Unknown), _ => None };
                        model.page = 0; model.reconcile_selection();
                    },
                    option { value: "", "All statuses" }
                    option { value: "Pass", "Pass" }
                    option { value: "Warn", "Warning" }
                    option { value: "Fail", "Failure" }
                    option { value: "Unknown", "Unknown / unavailable" }
                }
            }
            div { class: "feature-scroll diagnostic-list", style: "max-height: 420px; overflow: auto;",
                table {
                    thead { tr { th { "Category" } th { "Check" } th { "Status" } th { "Summary" } } }
                    tbody {
                        for check in rows {
                            tr { key: "{check.id}", class: if state.selected.as_deref() == Some(check.id.as_str()) { "selected-row" } else { "" },
                                td { "{check.category.title()}" }
                                td { button { aria_pressed: state.selected.as_deref() == Some(check.id.as_str()), onclick: { let id = check.id.clone(); move |_| view.write().selected = Some(id.clone()) }, "{check.name}" } }
                                td { "{check.status:?}" }
                                td { style: "white-space: pre-wrap; overflow-wrap: anywhere;", "{bounded(&check.message)}" }
                            }
                        }
                    }
                }
            }
            p { "{total} matching checks · Page {state.page + 1} of {total.div_ceil(PAGE_SIZE).max(1)}" }
            button { disabled: state.page == 0, onclick: move |_| { let mut model = view.write(); model.page = model.page.saturating_sub(1); }, "Previous page" }
            button { disabled: (state.page + 1) * PAGE_SIZE >= total, onclick: move |_| view.write().page += 1, "Next page" }
            button { onclick: move |_| view.write().navigate(false), "Previous check" }
            button { onclick: move |_| view.write().navigate(true), "Next check" }
            if let Some(check) = selected {
                article { class: "feature-detail diagnostic-details",
                    {text_review_region(
                        &format!("Diagnostic details: {}", check.name),
                        "max-height: 420px; overflow: auto; overflow-wrap: anywhere;",
                        rsx! {
                            h3 { "{check.name} · {check.status:?}" }
                            p { "{bounded(&check.message)}" }
                            pre { style: "white-space: pre-wrap;", {bounded(check.details.as_deref().unwrap_or("No additional evidence"))} }
                        },
                    )}
                    button { onclick: { let ctx = copy_ctx.clone(); let text = format!("{}\n{}\n{}", check.name, check.message, check.details.as_deref().unwrap_or("No additional evidence")); move |_| {
                        let ctx = ctx.clone(); let text = text.clone();
                        spawn(async move { let result = copy_text(ctx, text).await;
                            view.write().result = Some(match result { Ok(()) => "Check details copied.".to_string(), Err(error) => format!("Copy failed: {error}") });
                        });
                    } }, "Copy complete check details" }
                    if !log_services(&check).is_empty() {
                        button { onclick: { let services = log_services(&check); move |_| on_logs.call(LogRequest { node: None, address: None, services: services.clone() }) }, "Open related Talos logs" }
                    }
                    if let Some(fix) = check.fix {
                        button { disabled: busy, onclick: { let check_id = check.id.clone(); let fix = fix.clone(); move |_| {
                            let mut model = view.write();
                            if let Some(snapshot) = &model.snapshot {
                                model.pending = Some(PendingFix { key: model.key.clone(), request: DiagnosticFixRequest::new(check_id.clone(), snapshot.context.target.clone(), fix.clone()) });
                                model.result = None;
                            }
                        } }, "Preview remediation" }
                    }
                }
            }
            if let Some(pending) = pending {
                div { class: "confirmation", role: "dialog", aria_label: "Diagnostic remediation preview",
                    h3 { "Remediation preview" }
                    p { "{pending.request.fix.description}" }
                    p { "Talos context: {pending.request.target.config_identity} · Node: {pending.request.target.node_name} · Address: {pending.request.target.node_address}" }
                    {text_review_region(
                        "Diagnostic remediation content for review",
                        "max-height: 260px; overflow: auto; overflow-wrap: anywhere;",
                        rsx! { pre { style: "white-space: pre-wrap;", {bounded(pending.request.preview().content.as_deref().unwrap_or(""))} } },
                    )}
                    if pending.request.fix.action.requires_reboot() { p { class: "warning", "This patch requests an immediate reboot of the displayed node." } }
                    if let DiagnosticFixAction::CopyGuidance { text, .. } = &pending.request.fix.action {
                        p { "Host/operator guidance is copy-only. Talos Pilot will never run this command." }
                        button { onclick: { let ctx = copy_ctx.clone(); let text = text.clone(); move |_| {
                            let ctx = ctx.clone(); let text = text.clone();
                            spawn(async move { let result = copy_text(ctx, text).await;
                                view.write().result = Some(match result { Ok(()) => "Guidance copied; no command was executed.".to_string(), Err(error) => format!("Copy failed: {error}") });
                            });
                        } }, "Copy guidance" }
                    } else {
                        if !state.valid_pending(&pending) { p { class: "warning", "The target/check changed. Close this preview and select the current remediation again." } }
                        button { disabled: busy || !state.valid_pending(&pending), onclick: { let pending = pending.clone(); let ctx = confirm_ctx.clone(); let wake = wake.clone(); move |_| {
                            if view.peek().executing || !view.peek().valid_pending(&pending) { return; }
                            let guard = match try_acquire_mutation() { Ok(guard) => guard, Err(error) => { view.write().result = Some(error); return; } };
                            let cancel = guard.cancellation();
                            cancellation.set(Some(cancel.clone()));
                            view.write().executing = true;
                            view.write().result = Some("Remediation running. Cancellation is checked before each network step.".to_string());
                            let confirmed = pending.request.clone().confirm();
                            let client = ctx.client.clone();
                            let (sender, receiver) = tokio::sync::oneshot::channel();
                            ctx.runtime.spawn(async move {
                                let mut is_cancelled = || guard.cancelled();
                                let result = execute_confirmed_fix(&client, confirmed, &mut is_cancelled).await.map_err(|error| error.to_string());
                                let _ = sender.send(result);
                                drop(guard);
                            });
                            let wake = wake.clone();
                            let result_key = pending.key.clone();
                            let cancel_on_drop = CancelOnDrop(cancel);
                            spawn(async move {
                                let _cancel_on_drop = cancel_on_drop;
                                let result = receiver.await.unwrap_or_else(|_| Err("Mutation worker ended without a result; verify node state before retrying.".to_string()));
                                let mut model = view.write();
                                if model.key == result_key { model.executing = false; model.pending = None; model.result = Some(execution_text(result)); }
                                cancellation.set(None);
                                wake.notify_one();
                            });
                        } }, "Confirm and execute on this node" }
                    }
                    button { disabled: state.executing, onclick: move |_| view.write().pending = None, "Close preview" }
                }
            }
            if state.executing {
                button { onclick: move |_| { if let Some(cancel) = cancellation.peek().as_ref() { cancel.store(true, Ordering::Release); view.write().result = Some("Cancellation requested; waiting for the active RPC to finish safely.".to_string()); } }, "Cancel before next step" }
            }
            if let Some(result) = &state.result {
                {text_review_region(
                    "Diagnostic remediation result",
                    "max-height: 260px; overflow: auto; overflow-wrap: anywhere;",
                    rsx! { pre { role: "status", style: "white-space: pre-wrap;", "{bounded(result)}" } },
                )}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talos_pilot_core::diagnostic_runner::{
        AddonSnapshot, CniFileEvidence, CniSnapshot, DiagnosticContext, EtcdSnapshot,
        KubernetesSnapshot, ServicesSnapshot, SourceState, SystemSnapshot,
    };

    fn snapshot() -> DiagnosticSnapshot {
        DiagnosticSnapshot {
            context: DiagnosticContext {
                target: DiagnosticTarget::new("worker-a", "10.0.0.11", "worker", "lab"),
                platform: SourceState::Available("metal".to_string()),
                cpu_count: SourceState::Available(2),
                kubernetes_access: SourceState::unavailable(
                    "Selected Kubernetes access",
                    "CA mismatch",
                ),
            },
            system: SystemSnapshot {
                memory: SourceState::unavailable("memory", "fixture"),
                load_average: SourceState::unavailable("load", "fixture"),
            },
            services: ServicesSnapshot {
                services: SourceState::unavailable("services", "fixture"),
            },
            etcd: EtcdSnapshot::NotApplicable,
            kubernetes: KubernetesSnapshot {
                pod_health: SourceState::unavailable("pods", "CA mismatch"),
            },
            cni: CniSnapshot {
                cni_type: SourceState::unavailable("CNI", "fixture"),
                pods: SourceState::unavailable("CNI pods", "fixture"),
                files: CniFileEvidence::default(),
            },
            addons: AddonSnapshot {
                crd_names: SourceState::unavailable("CRDs", "fixture"),
                pod_sources: vec![],
                addons: vec![],
            },
            checks: vec![
                DiagnosticCheck::pass(
                    "memory",
                    CheckCategory::System,
                    "Memory",
                    "Current memory healthy",
                ),
                DiagnosticCheck::fail(
                    "service_kubelet",
                    CheckCategory::Services,
                    "kubelet",
                    "Unhealthy",
                )
                .with_details("direct Talos ServiceList health evidence")
                .with_fix(talos_pilot_core::diagnostic_runner::DiagnosticFix {
                    description: "Restart kubelet".to_string(),
                    action: DiagnosticFixAction::RestartService {
                        service_id: "kubelet".to_string(),
                    },
                }),
            ],
        }
    }

    #[test]
    fn filters_include_details_and_navigation_preserves_real_check_identity() {
        let mut model = DiagnosticView {
            key: "lab/worker-a".to_string(),
            snapshot: Some(snapshot()),
            ..Default::default()
        };
        model.reconcile_selection();
        assert_eq!(model.selected.as_deref(), Some("memory"));
        model.navigate(true);
        assert_eq!(model.selected.as_deref(), Some("service_kubelet"));
        assert_eq!(
            log_services(model.selected_check().unwrap()),
            vec!["kubelet"]
        );
        model.query = "ServiceList".to_string();
        assert_eq!(model.visible().len(), 1);
        model.category = Some(CheckCategory::System);
        model.reconcile_selection();
        assert!(model.selected.is_none());
        model.category = None;
        model.status = Some(CheckStatus::Fail);
        model.reconcile_selection();
        assert_eq!(model.selected.as_deref(), Some("service_kubelet"));
    }

    #[test]
    fn failed_refresh_retains_data_but_blocks_stale_mutation_confirmation() {
        let mut model = DiagnosticView {
            key: "one".to_string(),
            snapshot: Some(snapshot()),
            ..Default::default()
        };
        let snapshot = model.snapshot.as_ref().unwrap();
        let pending = PendingFix {
            key: model.key.clone(),
            request: DiagnosticFixRequest::new(
                "service_kubelet",
                snapshot.context.target.clone(),
                snapshot.checks[1].fix.clone().unwrap(),
            ),
        };
        let ticket = model.begin();
        assert!(!model.valid_pending(&pending));
        model.accept(&ticket, Err("selected-source failure".to_string()));
        assert_eq!(model.snapshot.as_ref().unwrap().checks.len(), 2);
        assert!(!model.valid_pending(&pending));
        assert!(model.error.is_some());
    }

    #[test]
    fn pagination_navigation_is_bounded_and_cancellation_is_cooperative() {
        let mut snapshot = snapshot();
        snapshot.checks = (0..(PAGE_SIZE + 2))
            .map(|index| {
                DiagnosticCheck::pass(
                    format!("check-{index}"),
                    CheckCategory::System,
                    format!("Check {index}"),
                    "Fixture",
                )
            })
            .collect();
        let mut model = DiagnosticView {
            snapshot: Some(snapshot),
            ..Default::default()
        };
        model.reconcile_selection();
        for _ in 0..PAGE_SIZE {
            model.navigate(true);
        }
        assert_eq!(model.page, 1);
        assert_eq!(
            model.selected.as_deref(),
            Some(format!("check-{PAGE_SIZE}").as_str())
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let owner = CancelOnDrop(cancel.clone());
        assert!(!cancel.load(Ordering::Acquire));
        drop(owner);
        assert!(cancel.load(Ordering::Acquire));
    }

    #[test]
    fn stale_refresh_and_confirmation_cannot_cross_target_or_changed_action() {
        let mut model = DiagnosticView {
            key: "one".to_string(),
            snapshot: Some(snapshot()),
            ..Default::default()
        };
        let ticket = model.begin();
        let newer = model.begin();
        assert!(!model.accept(&ticket, Err("stale".to_string())));
        assert!(model.accept(&newer, Ok(snapshot())));
        let snapshot = model.snapshot.as_ref().unwrap();
        let pending = PendingFix {
            key: model.key.clone(),
            request: DiagnosticFixRequest::new(
                "service_kubelet",
                snapshot.context.target.clone(),
                snapshot.checks[1].fix.clone().unwrap(),
            ),
        };
        assert!(pending.request.preview().confirmation_required);
        assert!(model.valid_pending(&pending));
        model.snapshot.as_mut().unwrap().checks[1].fix = None;
        assert!(!model.valid_pending(&pending));
        model.key = "two".to_string();
        assert!(!model.valid_pending(&pending));
        assert!(!model.accept(&newer, Ok(super::tests::snapshot())));
    }

    #[test]
    fn host_guidance_is_copy_only_and_display_is_utf8_bounded() {
        let action = DiagnosticFixAction::CopyGuidance {
            title: "Host module".to_string(),
            text: "sudo modprobe br_netfilter; touch /tmp/must-never-execute".to_string(),
        };
        assert!(!action.is_executable());
        let request = DiagnosticFixRequest::new(
            "host",
            DiagnosticTarget::new("node", "10.0.0.1", "worker", "lab"),
            talos_pilot_core::diagnostic_runner::DiagnosticFix {
                description: "Host command".to_string(),
                action,
            },
        );
        assert!(!request.preview().confirmation_required);
        assert!(
            request
                .preview()
                .content
                .unwrap()
                .contains("must-never-execute")
        );
        let text = "é".repeat(MAX_TEXT);
        let display = bounded(&text);
        assert!(display.len() < MAX_TEXT + 100);
        assert!(display.contains("Display truncated"));
    }

    #[test]
    fn component_wires_selected_access_and_has_no_host_command_executor() {
        let implementation = include_str!("diagnostics.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(implementation.contains("work_ctx.kubernetes_with_source().await"));
        assert!(implementation.contains("collect_with_kubernetes(&work_ctx.client, access)"));
        assert!(implementation.contains("try_acquire_mutation()"));
        assert!(
            implementation.contains("execute_confirmed_fix(&client, confirmed, &mut is_cancelled)")
        );
        assert!(!implementation.contains("process::Command"));
        assert!(!implementation.contains("from_default_config"));
        assert!(!implementation.contains("create_pinned_k8s_client"));
    }
}
