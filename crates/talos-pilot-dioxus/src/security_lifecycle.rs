//! Identity-selected, read-only PKI and lifecycle inspection.

use std::time::{Duration, Instant};

use dioxus::prelude::*;
use talos_pilot_core::security_lifecycle::{
    CertificateAudit, CertificateExpiryStatus, ClusterIdentity, EtcdPreOperationAudit,
    LifecycleCollector, LifecycleSnapshot, SecurityAuditCollector, SecurityAuditSnapshot,
    SourceSnapshot,
};

use crate::{
    feature::{FeatureContext, LogRequest},
    maintenance::text_review_region,
};

const MAX_ROWS: usize = 256;
const PANE_STYLE: &str = "max-height:65vh;overflow:auto;overflow-wrap:anywhere;min-width:0";

#[derive(Clone)]
struct PanelState<T> {
    data: Option<T>,
    loading: bool,
    error: Option<String>,
    updated: Option<Instant>,
}

impl<T> Default for PanelState<T> {
    fn default() -> Self {
        Self {
            data: None,
            loading: false,
            error: None,
            updated: None,
        }
    }
}

impl<T> PanelState<T> {
    fn finish(&mut self, result: Result<T, String>) {
        self.loading = false;
        match result {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
                self.updated = Some(Instant::now());
            }
            Err(error) => self.error = Some(error),
        }
    }
}

/// Resource cancellation drops ctx.run's abort-owned worker. Polling never
/// supersedes an in-flight refresh, and failed refreshes retain the last data.
fn use_inspection<T, F, Fut>(
    ctx: FeatureContext,
    collect: F,
) -> (Signal<PanelState<T>>, Signal<u64>)
where
    T: Clone + Send + 'static,
    F: Fn(FeatureContext) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Result<T, String>> + Send + 'static,
{
    let mut state = use_signal(PanelState::<T>::default);
    let mut refresh = use_signal(|| 0_u64);
    let timer = ctx.clone();
    use_future(move || {
        let timer = timer.clone();
        async move {
            loop {
                if timer
                    .run(async { tokio::time::sleep(Duration::from_secs(10)).await })
                    .await
                    .is_err()
                {
                    break;
                }
                if !state.peek().loading {
                    refresh += 1;
                }
            }
        }
    });
    use_resource(move || {
        let _revision = *refresh.read();
        let runner = ctx.clone();
        let target = ctx.clone();
        let collect = collect.clone();
        async move {
            state.write().loading = true;
            let result = runner
                .run(async move {
                    tokio::time::timeout(Duration::from_secs(75), collect(target))
                        .await
                        .map_err(|_| {
                            "Inspection collection timed out; retained snapshot is stale"
                                .to_string()
                        })?
                })
                .await
                .and_then(|result| result);
            state.write().finish(result);
        }
    });
    (state, refresh)
}

#[derive(Clone)]
struct SecurityData {
    snapshot: SecurityAuditSnapshot,
    certificate_source: String,
}

async fn collect_security(ctx: FeatureContext) -> Result<SecurityData, String> {
    let collector = SecurityAuditCollector::new(ctx.config_path.clone());
    // A worker node cannot serve Kubernetes credentials. Pin this inventory to
    // the actual selected cluster control plane; volume target remains selected.
    let pinned = ctx.cluster.control_plane_ip();
    let client = pinned.as_deref().map_or_else(
        || ctx.client.clone(),
        |address| {
            ctx.cluster
                .client
                .as_ref()
                .unwrap_or(&ctx.client)
                .with_node(address)
        },
    );
    let mut snapshot = collector
        .collect_for_context(&client, &ctx.context, &ctx.address)
        .await;
    let certificate_source = match pinned {
        Some(address) => format!(
            "Pinned Talos control plane {address}; this certificate inventory is not the selected Kubernetes file"
        ),
        None => {
            snapshot.talos_kubeconfig_certificates = SourceSnapshot::Unavailable {
                reason: "No pinned Talos control plane is known; no ambient kubeconfig was audited"
                    .into(),
            };
            "Unavailable: no pinned Talos control plane".into()
        }
    };
    Ok(SecurityData {
        snapshot,
        certificate_source,
    })
}

#[component]
pub(crate) fn SecurityPanel(ctx: FeatureContext, on_logs: EventHandler<LogRequest>) -> Element {
    let (state, mut refresh) = use_inspection(ctx.clone(), collect_security);
    let mut filter = use_signal(String::new);
    let state = state.read();
    let query = filter.read().to_lowercase();
    let config_path = ctx.config_path.as_ref().map_or_else(
        || "Applied default Talosconfig".to_string(),
        |path| path.display().to_string(),
    );
    rsx! {
        section { class: "panel feature-panel security-panel", style: PANE_STYLE,
            h2 { "Security / PKI" }
            p { "Context: {ctx.context} · target: {ctx.node} ({ctx.address})" }
            p { class: "muted", "Talosconfig: {config_path}" }
            div { class: "toolbar",
                button { disabled: state.loading, onclick: move |_| refresh += 1, "Refresh" }
                input { placeholder: "Filter certificates by name, subject, issuer", value: "{filter}",
                    aria_label: "Filter certificates by name, subject or issuer",
                    oninput: move |event| filter.set(event.value().chars().take(256).collect()) }
                button { onclick: move |_| on_logs.call(LogRequest { node: None, address: None, services: vec!["apid".into(), "trustd".into()] }), "API / trust logs" }
            }
            {inspection_status(&state)}
            if let Some(data) = &state.data {
                {identity_view(&data.snapshot.identity)}
                h3 { "Talosconfig certificates" }
                {source_status(&data.snapshot.talosconfig_certificates)}
                {certificate_inventory(&data.snapshot.talosconfig_certificates, &query, "Talosconfig certificate inventory")}
                h3 { "Kubernetes certificate inventory" }
                p { class: "muted", "{data.certificate_source}" }
                {source_status(&data.snapshot.talos_kubeconfig_certificates)}
                {certificate_inventory(&data.snapshot.talos_kubeconfig_certificates, &query, "Pinned Kubernetes certificate inventory")}
                h3 { "Talos RBAC identity" }
                {source_status(&data.snapshot.rbac)}
                if let Some(rbac) = data.snapshot.rbac.value() {
                    p { "Role: {bounded(&rbac.role)}" }
                    p { "Certificate subject: {bounded(&rbac.certificate_subject)}" }
                    p { class: "muted", "Role is inferred from the certificate subject, not a live authorization probe." }
                }
                h3 { "Selected-node volume encryption" }
                {source_status(&data.snapshot.volume_encryption)}
                if let Some(volumes) = data.snapshot.volume_encryption.value() {
                    for volume in volumes.iter().take(MAX_ROWS) {
                        p { "{bounded(&volume.volume_name)}: {volume.provider:?} ({volume.provider.health():?})" }
                    }
                    p { class: "muted", "STATE and EPHEMERAL volume status only; unavailable data is not proof of encryption." }
                }
            }
        }
    }
}

#[derive(Clone)]
struct LifecycleData {
    snapshot: LifecycleSnapshot,
    kubernetes_source: String,
    source_warning: Option<String>,
}

async fn collect_lifecycle(ctx: FeatureContext) -> Result<LifecycleData, String> {
    let (kubernetes, source, warning) = match ctx.kubernetes_with_source().await {
        Ok((client, source, warning)) => (Ok(client), source, warning),
        Err(reason) => (Err(reason.clone()), format!("Unavailable: {reason}"), None),
    };
    let collector = LifecycleCollector::new(ctx.config_path.clone());
    let client = ctx.cluster.client.as_ref().unwrap_or(&ctx.client);
    let snapshot = collector
        .collect_with_kubernetes(client, &ctx.context, kubernetes)
        .await;
    Ok(LifecycleData {
        snapshot,
        kubernetes_source: source,
        source_warning: warning,
    })
}

#[component]
pub(crate) fn LifecyclePanel(ctx: FeatureContext, on_logs: EventHandler<LogRequest>) -> Element {
    let (state, mut refresh) = use_inspection(ctx.clone(), collect_lifecycle);
    let mut filter = use_signal(String::new);
    let state = state.read();
    let query = filter.read().to_lowercase();
    let config_path = ctx.config_path.as_ref().map_or_else(
        || "Applied default Talosconfig".to_string(),
        |path| path.display().to_string(),
    );
    rsx! {
        section { class: "panel feature-panel lifecycle-panel", style: PANE_STYLE,
            h2 { "Lifecycle / versions / configuration" }
            p { "Context: {ctx.context} · selected target: {ctx.node} ({ctx.address})" }
            p { class: "muted", "Talosconfig: {config_path}" }
            div { class: "toolbar",
                button { disabled: state.loading, onclick: move |_| refresh += 1, "Refresh" }
                input { aria_label: "Filter lifecycle nodes", placeholder: "Filter nodes", value: "{filter}", oninput: move |event| filter.set(event.value().chars().take(256).collect()) }
                button { onclick: move |_| on_logs.call(LogRequest { node: None, address: None, services: vec!["etcd".into(), "machined".into()] }), "Lifecycle logs" }
            }
            {inspection_status(&state)}
            if let Some(data) = &state.data {
                {identity_view(&data.snapshot.identity)}
                h3 { "Source availability" }
                p { "Kubernetes: {bounded(&data.kubernetes_source)}" }
                if let Some(warning) = &data.source_warning { p { class: "warning", "{bounded(warning)}" } }
                p { "Talos discovery:" }
                {source_status(&data.snapshot.talos_discovery)}
                p { "Kubernetes roster:" }
                {source_status(&data.snapshot.kubernetes_roster)}
                h3 { "etcd pre-operation safety" }
                {source_status(&data.snapshot.etcd_pre_operation)}
                p { class: "warning", "{safety_label(&data.snapshot.etcd_pre_operation)}" }
                if let Some(etcd) = data.snapshot.etcd_pre_operation.value() {
                    p { "Responding {etcd.responding_members}/{etcd.total_members} · quorum required {etcd.quorum_required} · can lose {etcd.can_lose} · {etcd.quorum:?}" }
                }
                p { class: "muted", "Read-only snapshot, not permission to execute. Operations must repeat safety checks immediately before mutation." }
                h3 { "Alerts" }
                if data.snapshot.alerts.is_empty() { p { "No observed lifecycle alerts. Unavailable sources below remain unknown." } }
                for alert in data.snapshot.alerts.iter().take(MAX_ROWS) {
                    p { "{alert.health:?}: {bounded(&alert.message)}" }
                }
                h3 { "Node versions, clock and configuration drift" }
                p { class: "muted", "Configuration hash is the Talos machineconfig resource version. Differences are a drift indicator, not a semantic diff; node-specific configuration may legitimately differ." }
                if data.snapshot.nodes.is_empty() { p { "No node roster available; version, time and configuration status are unknown." } }
                for node in data.snapshot.nodes.iter().filter(|node| node_matches(node, &query)).take(MAX_ROWS) {
                    {
                        let request = log_target(&ctx, &node.name);
                        let selected = node.address.as_deref() == Some(ctx.address.as_str());
                        let address = node.address.as_deref().unwrap_or("address unavailable");
                        let role = node.machine_type.as_deref().unwrap_or("unknown");
                        let selection = if selected { "selected" } else { "" };
                        rsx! {
                            details { class: "node-lifecycle feature-detail", open: selected,
                                summary { "{bounded(&node.name)} · {address} · {selection}" }
                                {text_review_region(
                                    &format!("Lifecycle evidence for {}", node.name),
                                    "max-height:32rem;overflow:auto;overflow-wrap:anywhere",
                                    rsx! {
                                        p { "Role: {role}" }
                                        p { "Talos: {string_value(&node.version)} · platform: {string_value(&node.platform)}" }
                                        {source_status(&node.version)}
                                        {source_status(&node.platform)}
                                        p { "Time synchronization:" }
                                        {source_status(&node.time_synchronization)}
                                        if let Some(time) = node.time_synchronization.value() {
                                            p { "Synchronized: {time.synced} · NTP server: {bounded(&time.server)} · offset: {time.offset_seconds:.6} seconds" }
                                        }
                                        p { "Configuration hash/resource version: {string_value(&node.config_hash)}" }
                                        {source_status(&node.config_hash)}
                                    },
                                )}
                                button { disabled: request.is_none(), onclick: move |_| {
                                    if let Some(request) = request.clone() { on_logs.call(request); }
                                }, "Node lifecycle logs" }
                            }
                        }
                    }
                }
                if data.snapshot.nodes.len() >= MAX_ROWS { p { class: "warning", "Node inventory is bounded to 256 records; larger rosters may be partial." } }
            }
        }
    }
}

fn inspection_status<T>(state: &PanelState<T>) -> Element {
    let age = state.updated.map(|updated| updated.elapsed().as_secs());
    rsx! {
        if state.loading {
            p { class: "muted", if state.data.is_some() { "Refreshing — previous snapshot retained (stale until refresh completes)" } else { "Loading inspection…" } }
        }
        if let Some(error) = &state.error {
            p { class: "error", "{bounded(error)}" }
            if state.data.is_some() { p { class: "warning", "Showing stale last successful snapshot" } }
        }
        if let Some(age) = age { p { class: "muted", "Last successful refresh {age}s ago · automatic refresh every 10s while visible" } }
    }
}

fn source_status<T>(source: &SourceSnapshot<T>) -> Element {
    match source {
        SourceSnapshot::Available(_) => rsx! { p { class: "muted", "Available" } },
        SourceSnapshot::Partial { warnings, .. } => rsx! {
            p { class: "warning", "Partial source" }
            for warning in warnings.iter().take(16) { p { class: "warning", "{bounded(warning)}" } }
        },
        SourceSnapshot::Unavailable { reason } => {
            rsx! { p { class: "warning", "Unknown / unavailable: {bounded(reason)}" } }
        }
    }
}

fn identity_view(source: &SourceSnapshot<ClusterIdentity>) -> Element {
    rsx! {
        {source_status(source)}
        if let Some(identity) = source.value() {
            p { "Audited Talos identity: {bounded(&identity.context_name)}" }
            p { class: "muted",
                "Endpoints: "
                {bounded(&identity.endpoint_addresses.join(", "))}
                " · targets: "
                {bounded(&identity.target_addresses.join(", "))}
            }
        }
    }
}

fn certificate_inventory(
    source: &SourceSnapshot<Vec<CertificateAudit>>,
    query: &str,
    label: &str,
) -> Element {
    let certificates = source.value().map(Vec::as_slice).unwrap_or(&[]);
    let expired = certificates
        .iter()
        .filter(|certificate| certificate.expiry == CertificateExpiryStatus::Expired)
        .count();
    let expiring = certificates
        .iter()
        .filter(|certificate| {
            matches!(
                certificate.expiry,
                CertificateExpiryStatus::ExpiringSoon | CertificateExpiryStatus::ExpiringVerySoon
            )
        })
        .count();
    if !certificates
        .iter()
        .any(|certificate| certificate_matches(certificate, query))
    {
        return rsx! { p { "{certificates.len()} certificates · {expired} expired · {expiring} expiring" } };
    }
    text_review_region(
        label,
        "max-height:32vh;overflow:auto;overflow-wrap:anywhere;min-width:0",
        rsx! {
            p { "{certificates.len()} certificates · {expired} expired · {expiring} expiring" }
            for certificate in certificates.iter().filter(|certificate| certificate_matches(certificate, query)).take(MAX_ROWS) {
                details { class: "certificate-detail",
                    summary { "{bounded(&certificate.name)} · {certificate.expiry:?} · {certificate.days_remaining} days remaining" }
                    p { "Source: {certificate.source:?} · CA: {certificate.is_ca}" }
                    p { "Subject: {bounded(&certificate.subject)}" }
                    p { "Issuer: {bounded(&certificate.issuer)}" }
                    p { "Valid from: {certificate.not_before} · expires: {certificate.not_after}" }
                }
            }
            if certificates.len() > MAX_ROWS { p { class: "warning", "Rendering limited to 256 certificates; refine the filter." } }
        },
    )
}

fn certificate_matches(certificate: &CertificateAudit, query: &str) -> bool {
    query.is_empty()
        || [&certificate.name, &certificate.subject, &certificate.issuer]
            .iter()
            .any(|value| value.to_lowercase().contains(query))
}

fn node_matches(
    node: &talos_pilot_core::security_lifecycle::NodeLifecycleSnapshot,
    query: &str,
) -> bool {
    query.is_empty()
        || node.name.to_lowercase().contains(query)
        || node
            .address
            .as_deref()
            .is_some_and(|address| address.to_lowercase().contains(query))
}

fn string_value(source: &SourceSnapshot<String>) -> String {
    source
        .value()
        .map_or_else(|| "unknown".into(), |value| bounded(value))
}

fn safety_label(source: &SourceSnapshot<EtcdPreOperationAudit>) -> &'static str {
    match source {
        SourceSnapshot::Available(value) if value.safe_for_single_member_operation() => {
            "Observed quorum and failure tolerance: safe for one member at snapshot time"
        }
        SourceSnapshot::Available(_) => "Not safe for a single-member disruptive operation",
        SourceSnapshot::Partial { .. } | SourceSnapshot::Unavailable { .. } => {
            "Unknown safety — disruptive operations must not assume a green light"
        }
    }
}

fn log_target(ctx: &FeatureContext, name: &str) -> Option<LogRequest> {
    ctx.cluster.node_ips.get(name).map(|address| LogRequest {
        node: Some(name.to_string()),
        address: Some(address.clone()),
        services: vec!["machined".into(), "etcd".into()],
    })
}

fn bounded(value: &str) -> String {
    let mut chars = value.chars();
    let mut result: String = chars.by_ref().take(4096).collect();
    if chars.next().is_some() {
        result.push_str(" [truncated]");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificate_filter_covers_subject_issuer_and_name() {
        let certificate = CertificateAudit {
            name: "Talos CA".into(),
            source: talos_pilot_core::security_lifecycle::CertificateSource::TalosConfigCa,
            subject: "CN=cluster-root".into(),
            issuer: "CN=issuer".into(),
            not_before: "2026-01-01T00:00:00Z".parse().unwrap(),
            not_after: "2027-01-01T00:00:00Z".parse().unwrap(),
            days_remaining: 90,
            expiry: CertificateExpiryStatus::Valid,
            is_ca: true,
        };
        assert!(certificate_matches(&certificate, "talos"));
        assert!(certificate_matches(&certificate, "cluster-root"));
        assert!(certificate_matches(&certificate, "issuer"));
        assert!(!certificate_matches(&certificate, "unrelated"));
        let source = SourceSnapshot::Available(vec![certificate]);
        let matching = certificate_inventory(&source, "talos", "Talos certificates").unwrap();
        assert!(matches!(
            matching.template.roots[0],
            dioxus::core::TemplateNode::Element { tag: "div", .. }
        ));
        let empty = certificate_inventory(&source, "unrelated", "Talos certificates").unwrap();
        assert!(matches!(
            empty.template.roots[0],
            dioxus::core::TemplateNode::Element { tag: "p", .. }
        ));
    }

    #[test]
    fn failed_refresh_preserves_last_snapshot_and_marks_it_stale() {
        let mut state = PanelState::<u32>::default();
        state.finish(Ok(7));
        let updated = state.updated;
        state.loading = true;
        state.finish(Err("disconnected".into()));
        assert_eq!(state.data, Some(7));
        assert_eq!(state.updated, updated);
        assert_eq!(state.error.as_deref(), Some("disconnected"));
        assert!(!state.loading);
        state.finish(Ok(8));
        assert_eq!(state.data, Some(8));
        assert!(state.error.is_none());
    }

    #[test]
    fn partial_and_unavailable_safety_never_receive_green_light() {
        let unknown = SourceSnapshot::<EtcdPreOperationAudit>::Unavailable {
            reason: "offline".into(),
        };
        assert!(safety_label(&unknown).starts_with("Unknown"));
        let value = EtcdPreOperationAudit {
            total_members: 3,
            responding_members: 3,
            quorum_required: 2,
            can_lose: 1,
            quorum: talos_pilot_core::QuorumState::Healthy,
        };
        assert!(safety_label(&SourceSnapshot::Available(value.clone())).starts_with("Observed"));
        assert!(
            safety_label(&SourceSnapshot::Partial {
                value,
                warnings: vec!["incomplete".into()]
            })
            .starts_with("Unknown")
        );
    }

    #[test]
    fn display_bounds_preserve_utf8() {
        assert_eq!(bounded("short"), "short");
        let result = bounded(&"é".repeat(5000));
        assert_eq!(
            result.chars().filter(|character| *character == 'é').count(),
            4096
        );
        assert!(result.ends_with("[truncated]"));
    }
}
