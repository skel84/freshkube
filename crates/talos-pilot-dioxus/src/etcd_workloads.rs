use std::{
    collections::HashMap,
    ops::Range,
    time::{Duration, Instant},
};

use dioxus::prelude::*;
use talos_pilot_core::{
    cluster_overview::ClusterOverview,
    formatting::format_bytes_signed,
    inspection::{
        EtcdHealthSnapshot, EtcdInspectionRequest, EtcdMemberSnapshot, InspectionSource,
        collect_etcd_health,
    },
    workloads::{
        HealthState, NamespaceSummary, PodInfo, WorkloadCollectionOutcome, WorkloadInfo,
        WorkloadSnapshot, collect_workloads,
    },
};
use talos_rs::EtcdMemberInfo;

use crate::feature::{FeatureContext, LogRequest};

const PAGE_SIZE: usize = 25;
const MAX_NAMESPACES: usize = 1_000;
const MAX_ROWS: usize = 10_000;
const MAX_DETAIL_ITEMS: usize = 32;
const MAX_TEXT: usize = 4_096;
const CONTROL_PLANE_SERVICES: [&str; 4] = [
    "etcd",
    "kube-apiserver",
    "kube-controller-manager",
    "kube-scheduler",
];
const SCROLL_STYLE: &str = "overflow:auto;max-height:32rem;min-width:0;";
const WRAP_STYLE: &str =
    "white-space:pre-wrap;overflow-wrap:anywhere;max-height:24rem;overflow:auto;";

#[derive(Clone)]
struct SnapshotState<T> {
    data: Option<T>,
    error: Option<String>,
    loading: bool,
    updated: Option<Instant>,
}

impl<T> Default for SnapshotState<T> {
    fn default() -> Self {
        Self {
            data: None,
            error: None,
            loading: false,
            updated: None,
        }
    }
}

impl<T> SnapshotState<T> {
    fn finish(&mut self, result: Result<T, String>) {
        self.loading = false;
        match result {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
                self.updated = Some(Instant::now());
            }
            Err(error) => self.error = Some(bounded_text(&error)),
        }
    }

    fn freshness(&self) -> String {
        match self.updated {
            Some(updated) => format!(
                "Last successful refresh {}s ago{}",
                updated.elapsed().as_secs(),
                if self.loading || self.error.is_some() {
                    " · retained snapshot may be stale"
                } else {
                    ""
                }
            ),
            None => "No successful snapshot yet; unavailable data is unknown, not healthy.".into(),
        }
    }
}

fn use_poll<T: 'static>(
    ctx: FeatureContext,
    mut tick: Signal<u64>,
    state: Signal<SnapshotState<T>>,
) {
    use_future(move || {
        let ctx = ctx.clone();
        async move {
            loop {
                // The runtime owns the timer, just as it owns all network work.
                if ctx
                    .run(async { tokio::time::sleep(Duration::from_secs(10)).await })
                    .await
                    .is_err()
                {
                    return;
                }
                // Do not cancel a slow but bounded request on every polling tick.
                if !state.read().loading {
                    tick += 1;
                }
            }
        }
    });
}

fn bounded_text(text: &str) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(MAX_TEXT).collect();
    if chars.next().is_some() {
        result.push_str("… [text truncated]");
    }
    result
}

fn page_range(total: usize, requested: usize) -> (usize, usize, Range<usize>) {
    let pages = total.div_ceil(PAGE_SIZE).max(1);
    let page = requested.min(pages - 1);
    let start = page * PAGE_SIZE;
    (page, pages, start..(start + PAGE_SIZE).min(total))
}

fn member_selection(members: &[EtcdMemberInfo], selected: Option<u64>) -> Option<u64> {
    match selected {
        Some(id) => members.iter().any(|member| member.id == id).then_some(id),
        None => members.first().map(|member| member.id),
    }
}

fn initial_member_selection(members: &[EtcdMemberSnapshot], selected: Option<u64>) -> Option<u64> {
    selected.or_else(|| members.first().map(|member| member.info.id))
}

// Parse the authoritative peer URL without guessing that an IP is a node name.
// Unlike EtcdMemberInfo::ip_address this also preserves IPv6 literals.
fn peer_address(member: &EtcdMemberInfo) -> Option<String> {
    member.peer_urls.iter().find_map(|url| {
        let authority = url.split_once("://")?.1.split('/').next()?;
        if authority.contains('@') {
            return None;
        }
        let host = if let Some(rest) = authority.strip_prefix('[') {
            rest.split_once(']')?.0
        } else {
            authority.split(':').next()?
        };
        (!host.is_empty()).then(|| host.to_owned())
    })
}

fn member_log_request(
    member: &EtcdMemberInfo,
    addresses: &HashMap<String, String>,
    service: &str,
) -> Option<LogRequest> {
    if member.hostname.is_empty() || !CONTROL_PLANE_SERVICES.contains(&service) {
        return None;
    }
    let address = addresses
        .get(&member.hostname)
        .filter(|address| !address.is_empty())
        .cloned()
        .or_else(|| peer_address(member))?;
    Some(LogRequest {
        node: Some(member.hostname.clone()),
        address: Some(address),
        services: vec![service.into()],
    })
}

fn pod_log_request(pod: &PodInfo, cluster: &ClusterOverview) -> Option<LogRequest> {
    let node = pod.node.as_ref()?;
    let address = cluster
        .node_ips
        .get(node)
        .filter(|address| !address.is_empty())
        .cloned()
        .or_else(|| {
            cluster
                .discovery_members
                .iter()
                .find(|member| &member.hostname == node)
                .and_then(|member| member.addresses.first().cloned())
        })
        .or_else(|| {
            cluster
                .etcd_members
                .iter()
                .find(|member| &member.hostname == node)
                .and_then(peer_address)
        })?;
    Some(LogRequest {
        node: Some(node.clone()),
        address: Some(address),
        services: vec!["kubelet".into()],
    })
}

#[component]
fn Pager(page: Signal<usize>, total: usize, label: String) -> Element {
    let mut page = page;
    let (current, pages, range) = page_range(total, page());
    rsx! {
        div { class: "toolbar",
            button { disabled: current == 0, onclick: move |_| page.set(current.saturating_sub(1)), "Previous {label}" }
            span { class: "muted", "Page {current + 1}/{pages} · {range.len()} of {total}" }
            button { disabled: current + 1 >= pages, onclick: move |_| page.set(current + 1), "Next {label}" }
        }
    }
}

#[component]
fn MemberLogs(member: EtcdLogTarget, on_logs: EventHandler<LogRequest>) -> Element {
    rsx! {
        div { class: "toolbar",
            for (service, request) in member.requests {
                button { disabled: request.is_none(), onclick: move |_| { if let Some(request) = request.clone() { on_logs.call(request); } }, "{service} logs" }
            }
        }
        if !member.available {
            p { class: "muted", "Logs unavailable: member has no authoritative name/address mapping in the roster." }
        }
    }
}

#[derive(Clone, PartialEq)]
struct EtcdLogTarget {
    requests: Vec<(String, Option<LogRequest>)>,
    available: bool,
}

fn log_target(member: &EtcdMemberInfo, cluster: &ClusterOverview) -> EtcdLogTarget {
    let requests: Vec<_> = CONTROL_PLANE_SERVICES
        .iter()
        .map(|service| {
            (
                (*service).to_owned(),
                member_log_request(member, &cluster.node_ips, service),
            )
        })
        .collect();
    let available = requests.iter().any(|(_, request)| request.is_some());
    EtcdLogTarget {
        requests,
        available,
    }
}

#[derive(Clone)]
struct EtcdData {
    snapshot: EtcdHealthSnapshot,
    truncated: bool,
}

fn cap_list<T>(items: &mut Vec<T>, limit: usize) -> bool {
    let truncated = items.len() > limit;
    items.truncate(limit);
    truncated
}

fn bound_etcd(mut snapshot: EtcdHealthSnapshot) -> EtcdData {
    let mut truncated = cap_list(&mut snapshot.members, MAX_NAMESPACES);
    truncated |= cap_list(&mut snapshot.unmatched_statuses, MAX_NAMESPACES);
    truncated |= cap_list(&mut snapshot.unmatched_alarms, MAX_NAMESPACES);
    for member in &mut snapshot.members {
        truncated |= cap_list(&mut member.info.peer_urls, MAX_DETAIL_ITEMS);
        truncated |= cap_list(&mut member.info.client_urls, MAX_DETAIL_ITEMS);
        truncated |= cap_list(&mut member.alarms, MAX_DETAIL_ITEMS);
        for url in member
            .info
            .peer_urls
            .iter_mut()
            .chain(&mut member.info.client_urls)
        {
            truncated |= url.chars().count() > MAX_TEXT;
            *url = bounded_text(url);
        }
        if let Some(status) = &mut member.status {
            truncated |= cap_list(&mut status.errors, MAX_DETAIL_ITEMS);
            for error in &mut status.errors {
                truncated |= error.chars().count() > MAX_TEXT;
                *error = bounded_text(error);
            }
        }
    }
    for status in &mut snapshot.unmatched_statuses {
        truncated |= cap_list(&mut status.errors, MAX_DETAIL_ITEMS);
        for error in &mut status.errors {
            truncated |= error.chars().count() > MAX_TEXT;
            *error = bounded_text(error);
        }
    }
    EtcdData {
        snapshot,
        truncated,
    }
}

#[component]
fn ControlPlaneLogs(ctx: FeatureContext, on_logs: EventHandler<LogRequest>) -> Element {
    let initial_member = ctx.cluster.etcd_members.first().map(|member| member.id);
    let mut selected = use_signal(move || initial_member);
    let roster = &ctx.cluster.etcd_members;
    let current = member_selection(roster, selected());
    let target = roster
        .iter()
        .find(|member| Some(member.id) == current)
        .map(|member| log_target(member, &ctx.cluster));
    rsx! {
        section { class: "panel",
            h3 { "Control-plane logs" }
            p { class: "muted", "These are Talos control-plane service logs, not Kubernetes container logs. Targets come only from the authoritative etcd roster." }
            select {
                value: current.map(|id| id.to_string()).unwrap_or_default(),
                aria_label: "Control-plane log target",
                onchange: move |event| selected.set(event.value().parse().ok()),
                option { value: "", disabled: true, "Choose a current control-plane member" }
                for member in roster.iter().take(MAX_NAMESPACES) {
                    option { value: "{member.id}", "{member.hostname}" }
                }
            }
            if let Some(target) = target { MemberLogs { member: target, on_logs } }
            else { p { class: "muted", "Control-plane roster or selected member unavailable; explicitly choose a current member before opening logs." } }
        }
    }
}

#[component]
pub(crate) fn EtcdPanel(ctx: FeatureContext, on_logs: EventHandler<LogRequest>) -> Element {
    let mut state = use_signal(SnapshotState::<EtcdData>::default);
    let mut tick = use_signal(|| 0_u64);
    let mut selected = use_signal(|| None::<u64>);
    let mut page = use_signal(|| 0_usize);
    let mut search = use_signal(String::new);
    use_poll(ctx.clone(), tick, state);
    let worker_ctx = ctx.clone();
    let _collection = use_resource(move || {
        let _ = tick();
        let ctx = worker_ctx.clone();
        async move {
            state.write().loading = true;
            let target = ctx.inspection_target();
            let client = ctx
                .cluster
                .client
                .clone()
                .unwrap_or_else(|| ctx.client.clone());
            let result = ctx
                .run(async move {
                    tokio::time::timeout(
                        Duration::from_secs(25),
                        collect_etcd_health(client, EtcdInspectionRequest::new(target)),
                    )
                    .await
                    .map_err(|_| "Etcd inspection timed out; previous data is stale.".to_owned())?
                    .map(bound_etcd)
                    .map_err(|error| error.to_string())
                })
                .await
                .and_then(|result| result);
            if let Ok(data) = &result {
                let current = *selected.peek();
                selected.set(initial_member_selection(&data.snapshot.members, current));
            }
            state.write().finish(result);
        }
    });
    let snapshot = state.read();
    let loading = snapshot.loading;
    let freshness = snapshot.freshness();
    let error = snapshot.error.clone();
    let limited = snapshot.data.as_ref().is_some_and(|data| data.truncated);
    let data = snapshot.data.as_ref().map(|data| &data.snapshot);
    let query = search().trim().to_lowercase();
    let members: Vec<_> = data
        .into_iter()
        .flat_map(|data| data.members.iter())
        .filter(|member| {
            query.is_empty()
                || format!("{} {:x}", member.info.hostname, member.info.id)
                    .to_lowercase()
                    .contains(&query)
        })
        .collect();
    let (_, _, range) = page_range(members.len(), page());
    let rows: Vec<_> = members[range]
        .iter()
        .map(|member| (**member).clone())
        .collect();
    let chosen = data
        .and_then(|data| {
            data.members
                .iter()
                .find(|member| Some(member.info.id) == selected())
        })
        .cloned();
    let chosen_id = chosen.as_ref().map(|member| member.info.id);
    let missing_member = selected().is_some() && chosen.is_none();
    let details = chosen.as_ref().map(member_details);
    let target = chosen
        .as_ref()
        .map(|member| log_target(&member.info, &ctx.cluster));
    let quorum = data.map(quorum_display).unwrap_or("UNKNOWN");
    let count = data
        .map(|data| {
            if etcd_status_unavailable(data) {
                format!("unknown / {}", data.voting_members)
            } else {
                format!("{}/{}", data.responding_voting_members, data.voting_members)
            }
        })
        .unwrap_or_else(|| "unknown".into());
    let leader = data.map(leader_display).unwrap_or_else(|| "unknown".into());
    let revision = data
        .filter(|data| data.members.iter().any(|member| member.status.is_some()))
        .map(|data| data.revision.to_string())
        .unwrap_or_else(|| "unknown".into());
    let database = data
        .filter(|data| data.members.iter().any(|member| member.status.is_some()))
        .map(|data| data.largest_database_size_display())
        .unwrap_or_else(|| "unknown".into());
    let unavailable: Vec<_> = data
        .into_iter()
        .flat_map(|data| data.unavailable.iter())
        .map(|source| bounded_text(&format!("{}: {}", source.source, source.message)))
        .collect();
    let status_unavailable =
        data.is_some_and(|data| data.members.iter().any(|member| member.status.is_none()));
    let unmatched = data.map(|data| {
        format!(
            "{} status records and {} alarms could not be correlated to the authoritative roster.",
            data.unmatched_statuses.len(),
            data.unmatched_alarms.len()
        )
    });
    let unmatched_details = data.map(unmatched_details);
    let total = members.len();
    drop(snapshot);
    rsx! {
        section { class: "panel",
            div { class: "feature-heading", h2 { "Etcd health" }
                button { disabled: loading, onclick: move |_| tick += 1, "Refresh" }
            }
            p { class: "muted", "Context: {ctx.context} · endpoint {ctx.node} ({ctx.address}) · Automatic refresh every 10s while visible" }
            p { class: "muted freshness", "{freshness}" }
            if loading { div { class: "banner", role: "status", "Loading correlated membership, status and alarms…" } }
            if let Some(error) = error { div { class: "banner warning", role: "alert", "Etcd unavailable: {error}" } }
            if limited { div { class: "banner warning", "Retention limit reached: member and uncorrelated record lists are capped at 1,000; per-member detail lists at 32. Aggregate quorum / raft / DB summarize the full collected sample, but drilldown is incomplete." } }
            if missing_member { div { class: "banner warning", "The selected member is no longer in this snapshot. Select a current member; its log target was not silently replaced." } }
            if status_unavailable || !unavailable.is_empty() {
                div { class: "banner warning", "Partial snapshot: nonresponding members have unknown status. Quorum below describes observed responders, not a definitive network-wide failure verdict." }
            }
            for reason in unavailable { div { class: "banner warning text-review-region", style: WRAP_STYLE, tabindex: "0", role: "region", aria_label: "Etcd unavailable source evidence", "{reason}" } }
            div { class: "metric-grid",
                div { class: "metric", small { "Observed quorum" } strong { "{quorum}" } span { "Voting responders {count}" } }
                div { class: "metric", small { "Reported leader" } strong { "{leader}" } }
                div { class: "metric", small { "Revision (maximum raft index)" } strong { "{revision}" } }
                div { class: "metric", small { "Largest database" } strong { "{database}" } }
            }
            input { r#type: "search", aria_label: "Filter etcd members", placeholder: "Filter member hostname or hex ID", value: search(), oninput: move |event| { search.set(event.value()); page.set(0); } }
            div { style: SCROLL_STYLE,
                table { class: "data-table", thead { tr { th { "Member" } th { "Role" } th { "Status" } th { "DB" } th { "Raft index / applied / term" } th { "Alarms" } } }
                    tbody {
                        for member in rows {
                            tr { key: "{member.info.id}", class: if chosen_id == Some(member.info.id) { "selected-row" } else { "" },
                                td { button { aria_pressed: chosen_id == Some(member.info.id), aria_current: (chosen_id == Some(member.info.id)).then_some("true"), onclick: move |_| selected.set(Some(member.info.id)), "{member.info.hostname}" } small { "{member.info.id:x}" } }
                                td { if member.info.is_learner { "Learner" } else if member.is_leader() { "Leader" } else { "Voter" } }
                                td { "{member_status(&member)}" }
                                td { {member.status.as_ref().map(|status| format_bytes_signed(status.db_size)).unwrap_or_else(|| "unknown".into())} }
                                td { {member.status.as_ref().map(|status| format!("{} / {} / {}", status.raft_index, status.raft_applied_index, status.raft_term)).unwrap_or_else(|| "unknown".into())} }
                                td { "{member.alarms.len()}" }
                            }
                        }
                    }
                }
            }
            if total == 0 && !loading { p { class: "muted", "No members match, or membership has not been collected." } }
            Pager { page, total, label: "members".to_owned() }
            if let Some(details) = details {
                h3 { "Selected member details" }
                pre { class: "text-review-region", style: WRAP_STYLE, tabindex: "0", role: "region", aria_label: "Selected etcd member details", "{details}" }
                if let Some(target) = target { MemberLogs { member: target, on_logs } }
            }
            if let Some(unmatched) = unmatched { p { class: "muted", "{unmatched}" } }
            if let Some(details) = unmatched_details { if !details.is_empty() { details { summary { "Uncorrelated records" } pre { class: "text-review-region", style: WRAP_STYLE, tabindex: "0", role: "region", aria_label: "Uncorrelated etcd records", "{details}" } } } }
        }
    }
}

fn member_status(member: &EtcdMemberSnapshot) -> &'static str {
    if member.status.is_none() {
        "Unknown / nonresponding"
    } else if member.has_problems() {
        "Issues reported"
    } else {
        "Responding"
    }
}

fn etcd_status_unavailable(data: &EtcdHealthSnapshot) -> bool {
    data.unavailable
        .iter()
        .any(|source| source.source == InspectionSource::EtcdStatus)
}

fn quorum_display(data: &EtcdHealthSnapshot) -> &'static str {
    if etcd_status_unavailable(data) {
        "UNKNOWN (status source unavailable)"
    } else {
        data.quorum.display().0
    }
}

fn leader_display(data: &EtcdHealthSnapshot) -> String {
    match data.reported_leader_ids.as_slice() {
        [] => "unknown".into(),
        [id] => data
            .members
            .iter()
            .find(|member| member.info.id == *id)
            .map(|member| format!("{} ({id:x})", member.info.hostname))
            .unwrap_or_else(|| format!("{id:x} (outside roster)")),
        ids => format!(
            "Conflicting leader IDs: {}",
            ids.iter()
                .take(MAX_DETAIL_ITEMS)
                .map(|id| format!("{id:x}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn member_details(member: &EtcdMemberSnapshot) -> String {
    let mut lines = vec![
        format!(
            "{} · ID {:x} · {}",
            member.info.hostname,
            member.info.id,
            member_status(member)
        ),
        format!(
            "Membership: {}",
            if member.info.is_learner {
                "learner (not counted toward quorum)"
            } else {
                "voter"
            }
        ),
        format!(
            "Peer URLs: {}",
            member
                .info
                .peer_urls
                .iter()
                .take(MAX_DETAIL_ITEMS)
                .map(|url| bounded_text(url))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        format!(
            "Client URLs: {}",
            member
                .info
                .client_urls
                .iter()
                .take(MAX_DETAIL_ITEMS)
                .map(|url| bounded_text(url))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ];
    if let Some(status) = &member.status {
        lines.extend([
            format!(
                "Reported by: {} · protocol {} · learner {}",
                status.node, status.protocol_version, status.is_learner
            ),
            format!(
                "Leader ID: {:x} · is leader {}",
                status.leader_id,
                member.is_leader()
            ),
            format!(
                "Database: {} total · {} in use · {:.1}% usage",
                format_bytes_signed(status.db_size),
                format_bytes_signed(status.db_size_in_use),
                status.db_usage_percent()
            ),
            format!(
                "Raft: index {} · applied {} · term {} · unapplied {}",
                status.raft_index,
                status.raft_applied_index,
                status.raft_term,
                status.raft_index.saturating_sub(status.raft_applied_index)
            ),
        ]);
        if status.errors.is_empty() {
            lines.push("Status errors: none reported".into());
        } else {
            for error in status.errors.iter().take(MAX_DETAIL_ITEMS) {
                lines.push(format!("Status error: {}", bounded_text(error)));
            }
        }
    } else {
        lines.push("Status unavailable; DB, raft, leader and errors are unknown.".into());
    }
    for alarm in member.alarms.iter().take(MAX_DETAIL_ITEMS) {
        lines.push(format!(
            "Alarm: {:?} · reported by {} · member {:x}",
            alarm.alarm_type, alarm.node, alarm.member_id
        ));
    }
    lines.push(format!(
        "Correlated alarm records: {} (an unavailable alarm source is shown above)",
        member.alarms.len()
    ));
    bounded_text(&lines.join("\n"))
}

fn unmatched_details(data: &EtcdHealthSnapshot) -> String {
    let lines = data
        .unmatched_statuses
        .iter()
        .take(MAX_DETAIL_ITEMS)
        .map(|status| {
            format!(
                "Status: reporter {} · member {:x} · leader {:x}",
                status.node, status.member_id, status.leader_id
            )
        })
        .chain(
            data.unmatched_alarms
                .iter()
                .take(MAX_DETAIL_ITEMS)
                .map(|alarm| {
                    format!(
                        "Alarm: reporter {} · member {:x} · {:?}",
                        alarm.node, alarm.member_id, alarm.alarm_type
                    )
                }),
        )
        .collect::<Vec<_>>();
    bounded_text(&lines.join("\n"))
}

#[derive(Clone)]
struct WorkloadData {
    outcome: WorkloadCollectionOutcome,
    source: String,
    warning: Option<String>,
    truncated: bool,
}

impl WorkloadData {
    fn from_collection(
        mut outcome: WorkloadCollectionOutcome,
        source: String,
        warning: Option<String>,
    ) -> Result<Self, String> {
        if let WorkloadCollectionOutcome::Unavailable { errors, .. } = &outcome {
            return Err(format!(
                "All Kubernetes workload lists unavailable: {}",
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        let truncated = bound_workloads(&mut outcome);
        Ok(Self {
            outcome,
            source: bounded_text(&source),
            warning: warning.map(|warning| bounded_text(&warning)),
            truncated,
        })
    }
}

fn bound_workloads(outcome: &mut WorkloadCollectionOutcome) -> bool {
    let snapshot = match outcome {
        WorkloadCollectionOutcome::Complete(snapshot)
        | WorkloadCollectionOutcome::Partial { snapshot, .. } => snapshot,
        WorkloadCollectionOutcome::Unavailable { .. } => return false,
    };
    let mut truncated = snapshot.namespaces.len() > MAX_NAMESPACES;
    snapshot.namespaces.truncate(MAX_NAMESPACES);
    let mut workloads_left = MAX_ROWS;
    let mut pods_left = MAX_ROWS;
    for namespace in &mut snapshot.namespaces {
        truncated |=
            namespace.workloads.len() > workloads_left || namespace.problem_pods.len() > pods_left;
        namespace.workloads.truncate(workloads_left);
        namespace.problem_pods.truncate(pods_left);
        workloads_left -= namespace.workloads.len();
        pods_left -= namespace.problem_pods.len();
    }
    truncated
}

fn filtered_namespaces<'a>(
    snapshot: &'a WorkloadSnapshot,
    query: &str,
) -> Vec<&'a NamespaceSummary> {
    let query = query.trim().to_lowercase();
    snapshot
        .namespaces
        .iter()
        .filter(|namespace| query.is_empty() || namespace.name.to_lowercase().contains(&query))
        .collect()
}

fn workload_matches(workload: &WorkloadInfo, query: &str, only_problems: bool) -> bool {
    (!only_problems || workload.health != HealthState::Healthy)
        && (query.is_empty()
            || format!(
                "{} {} {} {}",
                workload.namespace,
                workload.name,
                workload.kind.label(),
                workload.issues.join(" ")
            )
            .to_lowercase()
            .contains(query))
}

fn pod_matches(pod: &PodInfo, query: &str) -> bool {
    query.is_empty()
        || format!(
            "{} {} {} {} {} {:?}",
            pod.namespace,
            pod.name,
            pod.node.as_deref().unwrap_or(""),
            pod.phase,
            pod.issue.label(),
            pod.issue
        )
        .to_lowercase()
        .contains(query)
}

#[derive(Clone, PartialEq)]
enum WorkloadSelection {
    Controller {
        namespace: String,
        name: String,
        kind: String,
    },
    Pod {
        namespace: String,
        name: String,
    },
}

impl WorkloadSelection {
    fn controller(workload: &WorkloadInfo) -> Self {
        Self::Controller {
            namespace: workload.namespace.clone(),
            name: workload.name.clone(),
            kind: workload.kind.label().into(),
        }
    }

    fn pod(pod: &PodInfo) -> Self {
        Self::Pod {
            namespace: pod.namespace.clone(),
            name: pod.name.clone(),
        }
    }
}

fn selected_details(
    snapshot: &WorkloadSnapshot,
    selected: &WorkloadSelection,
    cluster: &ClusterOverview,
) -> Option<(String, Option<LogRequest>)> {
    match selected {
        WorkloadSelection::Controller {
            namespace,
            name,
            kind,
        } => {
            let workload = snapshot
                .namespaces
                .iter()
                .find(|entry| &entry.name == namespace)?
                .workloads
                .iter()
                .find(|workload| &workload.name == name && workload.kind.label() == kind)?;
            Some((
                bounded_text(&format!(
                    "{namespace}/{name} · {kind}\nHealth: {:?}\nReady / desired: {} / {}\nIssues: {}",
                    workload.health,
                    workload.ready,
                    workload.desired,
                    if workload.issues.is_empty() {
                        "none reported".into()
                    } else {
                        workload
                            .issues
                            .iter()
                            .take(MAX_DETAIL_ITEMS)
                            .map(|issue| bounded_text(issue))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                )),
                None,
            ))
        }
        WorkloadSelection::Pod { namespace, name } => {
            let pod = snapshot
                .namespaces
                .iter()
                .find(|entry| &entry.name == namespace)?
                .problem_pods
                .iter()
                .find(|pod| &pod.name == name)?;
            Some((
                bounded_text(&format!(
                    "{namespace}/{name}\nNode: {}\nPhase: {}\nIssue: {:?}\nRestarts: {}\nCreated: {}",
                    pod.node.as_deref().unwrap_or("not scheduled"),
                    pod.phase,
                    pod.issue,
                    pod.restarts,
                    pod.created_at
                        .map(|date| date.to_rfc3339())
                        .unwrap_or_else(|| "unknown".into())
                )),
                pod_log_request(pod, cluster),
            ))
        }
    }
}

#[component]
pub(crate) fn WorkloadsPanel(ctx: FeatureContext, on_logs: EventHandler<LogRequest>) -> Element {
    let mut state = use_signal(SnapshotState::<WorkloadData>::default);
    let mut tick = use_signal(|| 0_u64);
    let mut namespace = use_signal(|| None::<String>);
    let mut namespace_search = use_signal(String::new);
    let mut search = use_signal(String::new);
    let mut only_problems = use_signal(|| false);
    let mut namespace_page = use_signal(|| 0_usize);
    let mut workload_page = use_signal(|| 0_usize);
    let mut pod_page = use_signal(|| 0_usize);
    let mut selected = use_signal(|| None::<WorkloadSelection>);
    use_poll(ctx.clone(), tick, state);
    let worker_ctx = ctx.clone();
    let _collection = use_resource(move || {
        let _ = tick();
        let ctx = worker_ctx.clone();
        async move {
            state.write().loading = true;
            let collect_ctx = ctx.clone();
            let result = ctx.run(async move {
                tokio::time::timeout(Duration::from_secs(60), async move {
                    // Source setup may retry the same cluster's pinned Talos
                    // kubeconfig, but must never fall back to an ambient client.
                    let (client, source, warning) = collect_ctx.kubernetes_with_source().await?;
                    let outcome = collect_workloads(collect_ctx.context.clone(), client).await;
                    WorkloadData::from_collection(outcome, source, warning)
                }).await.map_err(|_| "Kubernetes source validation / workload collection timed out; previous snapshot is retained.".to_owned())?
            }).await.and_then(|result| result);
            state.write().finish(result);
        }
    });
    let state_read = state.read();
    let loading = state_read.loading;
    let freshness = state_read.freshness();
    let error = state_read.error.clone();
    let data = state_read.data.as_ref();
    let snapshot = data.and_then(|data| data.outcome.snapshot());
    let source = data
        .map(|data| data.source.clone())
        .unwrap_or_else(|| "Not collected / unavailable".into());
    let warning = data.and_then(|data| data.warning.clone());
    let truncated = data.is_some_and(|data| data.truncated);
    let unavailable: Vec<_> = data
        .into_iter()
        .flat_map(|data| data.outcome.unavailable())
        .map(|error| bounded_text(&error.to_string()))
        .collect();
    let namespaces = snapshot
        .map(|snapshot| filtered_namespaces(snapshot, &namespace_search()))
        .unwrap_or_default();
    let namespace_count = namespaces.len();
    let (_, _, range) = page_range(namespace_count, namespace_page());
    let namespace_rows: Vec<_> = namespaces[range]
        .iter()
        .map(|entry| {
            (
                entry.name.clone(),
                entry.health,
                entry.healthy_workloads,
                entry.total_workloads,
                entry.problem_pods.len(),
            )
        })
        .collect();
    let chosen_namespace = namespace();
    let missing_namespace = chosen_namespace.as_ref().is_some_and(|name| {
        snapshot
            .is_some_and(|snapshot| !snapshot.namespaces.iter().any(|entry| &entry.name == name))
    });
    let query = search().trim().to_lowercase();
    let workloads: Vec<_> = snapshot
        .into_iter()
        .flat_map(|snapshot| snapshot.namespaces.iter())
        .filter(|entry| {
            chosen_namespace
                .as_ref()
                .is_none_or(|name| &entry.name == name)
        })
        .flat_map(|entry| entry.workloads.iter())
        .filter(|workload| workload_matches(workload, &query, only_problems()))
        .collect();
    let pods: Vec<_> = snapshot
        .into_iter()
        .flat_map(|snapshot| snapshot.namespaces.iter())
        .filter(|entry| {
            chosen_namespace
                .as_ref()
                .is_none_or(|name| &entry.name == name)
        })
        .flat_map(|entry| entry.problem_pods.iter())
        .filter(|pod| pod_matches(pod, &query))
        .collect();
    let workload_count = workloads.len();
    let pod_count = pods.len();
    let (_, _, range) = page_range(workload_count, workload_page());
    let workload_rows: Vec<_> = workloads[range]
        .iter()
        .map(|entry| (**entry).clone())
        .collect();
    let (_, _, range) = page_range(pod_count, pod_page());
    let pod_rows: Vec<_> = pods[range].iter().map(|entry| (**entry).clone()).collect();
    let chosen = selected();
    let detail = snapshot.and_then(|snapshot| {
        chosen
            .as_ref()
            .and_then(|selected| selected_details(snapshot, selected, &ctx.cluster))
    });
    let missing_detail = chosen.is_some() && detail.is_none();
    let counts = snapshot.map(|snapshot| format!("Deployments {} · StatefulSets {} · DaemonSets {} · Pods healthy {} / degraded {} / failing {}",
        snapshot.total_deployments, snapshot.total_statefulsets, snapshot.total_daemonsets,
        snapshot.total_pods_healthy, snapshot.total_pods_degraded, snapshot.total_pods_failing));
    drop(state_read);
    rsx! {
        section { class: "panel",
            div { class: "feature-heading", h2 { "Kubernetes workload health" }
                button { disabled: loading, onclick: move |_| tick += 1, "Refresh" }
            }
            p { class: "muted", "Context: {ctx.context} · Automatic refresh every 10s while visible" }
            p { class: "muted freshness", "{freshness}" }
            p { class: "muted text-review-region", style: WRAP_STYLE, tabindex: "0", role: "region", aria_label: "Snapshot Kubernetes source", "Snapshot Kubernetes source: {source}. Source setup may retry this cluster's pinned Talos kubeconfig; ambient kubeconfig fallback is never used." }
            if let Some(warning) = warning { div { class: "banner warning text-review-region", style: WRAP_STYLE, tabindex: "0", role: "region", aria_label: "Snapshot Kubernetes source warning", "{warning}" } }
            if loading { div { class: "banner", role: "status", "Validating selected Kubernetes source and loading workloads…" } }
            if let Some(error) = error { div { class: "banner warning text-review-region", style: WRAP_STYLE, tabindex: "0", aria_label: "Workload collection error", role: "alert", "Workload data unavailable: {error}" } }
            if !unavailable.is_empty() {
                div { class: "banner warning", "Partial snapshot: counts and readiness only cover available resource lists; missing kinds and pod health are unknown." }
                for error in unavailable { div { class: "banner warning text-review-region", style: WRAP_STYLE, tabindex: "0", role: "region", aria_label: "Unavailable Kubernetes resource list evidence", "{error}" } }
            }
            if truncated { div { class: "banner warning", "Retention limit reached: at most 1,000 namespaces, 10,000 controllers and 10,000 problematic pods are retained. Aggregate totals describe the collected sample; drilldown is incomplete." } }
            if let Some(counts) = counts { p { style: WRAP_STYLE, "{counts}" } }
            h3 { "Namespaces" }
            div { class: "toolbar",
                input { r#type: "search", aria_label: "Filter namespaces", placeholder: "Filter namespaces", value: namespace_search(), oninput: move |event| { namespace_search.set(event.value()); namespace_page.set(0); } }
                button { aria_pressed: chosen_namespace.is_none(), aria_current: chosen_namespace.is_none().then_some("true"), onclick: move |_| { namespace.set(None); workload_page.set(0); pod_page.set(0); selected.set(None); }, "All namespaces" }
                span { class: "muted", "Drilldown: " {chosen_namespace.as_deref().unwrap_or("all namespaces")} }
            }
            if missing_namespace { div { class: "banner warning", "Selected namespace is absent from this snapshot. Choose another namespace or All namespaces; no unrelated namespace was substituted." } }
            div { style: SCROLL_STYLE,
                table { class: "data-table", thead { tr { th { "Namespace" } th { "Health" } th { "Healthy controllers / total" } th { "Problematic pods" } } }
                    tbody {
                        for (name, health, healthy, total, problems) in namespace_rows {
                            {
                                let target_name = name.clone();
                                let is_selected = chosen_namespace.as_deref() == Some(name.as_str());
                                rsx! {
                                    tr { key: "{name}", class: if is_selected { "selected-row" } else { "" },
                                        td { button { aria_pressed: is_selected, aria_current: is_selected.then_some("true"), onclick: move |_| { namespace.set(Some(target_name.clone())); workload_page.set(0); pod_page.set(0); selected.set(None); }, "{name}" } }
                                        td { "{health:?}" }
                                        td { "{healthy} / {total}" }
                                        td { "{problems}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Pager { page: namespace_page, total: namespace_count, label: "namespaces".to_owned() }
            h3 { "Controllers" }
            div { class: "toolbar",
                input { r#type: "search", aria_label: "Filter workloads and pods", placeholder: "Filter workload / pod / node / issue", value: search(), oninput: move |event| { search.set(event.value()); workload_page.set(0); pod_page.set(0); } }
                label { input { r#type: "checkbox", checked: only_problems(), onchange: move |event| { only_problems.set(event.checked()); workload_page.set(0); } } "Only unhealthy controllers" }
            }
            div { style: SCROLL_STYLE,
                table { class: "data-table", thead { tr { th { "Namespace / name" } th { "Kind" } th { "Ready / desired" } th { "Health" } th { "Issues" } } }
                    tbody {
                        for workload in workload_rows {
                            {
                                let choice = WorkloadSelection::controller(&workload);
                                let is_selected = detail.is_some() && chosen.as_ref() == Some(&choice);
                                rsx! {
                                    tr { key: "{workload.namespace}/{workload.kind.label()}/{workload.name}", class: if is_selected { "selected-row" } else { "" },
                                        td { button { aria_pressed: is_selected, aria_current: is_selected.then_some("true"), onclick: move |_| selected.set(Some(choice.clone())), "{workload.namespace}/{workload.name}" } }
                                        td { "{workload.kind.label()}" }
                                        td { "{workload.ready} / {workload.desired}" }
                                        td { "{workload.health:?}" }
                                        td { style: "max-width:24rem;overflow-wrap:anywhere", {bounded_text(&workload.issues.join("; "))} }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if workload_count == 0 && !loading { p { class: "muted", "No controllers match the current snapshot and filters. Missing resource lists are not evidence of health." } }
            Pager { page: workload_page, total: workload_count, label: "controllers".to_owned() }
            h3 { "Problematic pods" }
            div { style: SCROLL_STYLE,
                table { class: "data-table", thead { tr { th { "Namespace / name" } th { "Node" } th { "Phase" } th { "Issue" } th { "Restarts" } } }
                    tbody {
                        for pod in pod_rows {
                            {
                                let choice = WorkloadSelection::pod(&pod);
                                let is_selected = detail.is_some() && chosen.as_ref() == Some(&choice);
                                rsx! {
                                    tr { key: "{pod.namespace}/{pod.name}", class: if is_selected { "selected-row" } else { "" },
                                        td { button { aria_pressed: is_selected, aria_current: is_selected.then_some("true"), onclick: move |_| selected.set(Some(choice.clone())), "{pod.namespace}/{pod.name}" } }
                                        td { {pod.node.as_deref().unwrap_or("not scheduled")} }
                                        td { "{pod.phase}" }
                                        td { style: "max-width:24rem;overflow-wrap:anywhere", {bounded_text(&format!("{:?}", pod.issue))} }
                                        td { "{pod.restarts}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if pod_count == 0 && !loading { p { class: "muted", "No problematic pods match. Pod health is unknown if the Pods list was unavailable." } }
            Pager { page: pod_page, total: pod_count, label: "pods".to_owned() }
            if let Some((details, request)) = detail {
                h3 { "Selected workload / pod details" }
                pre { class: "text-review-region", style: WRAP_STYLE, tabindex: "0", role: "region", aria_label: "Selected workload or pod details", "{details}" }
                if matches!(chosen, Some(WorkloadSelection::Pod { .. })) {
                    button { disabled: request.is_none(), onclick: move |_| { if let Some(request) = request.clone() { on_logs.call(request); } }, "Node kubelet logs" }
                    p { class: "muted", "Opens Talos kubelet logs on the pod's exact roster-mapped node, not the pod's container logs. Disabled if node identity cannot be mapped safely." }
                }
            }
            if missing_detail { p { class: "muted", "Selected item is no longer in this snapshot (it may have recovered). Select a current item; no replacement was silently selected." } }
        }
        ControlPlaneLogs { ctx, on_logs }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talos_pilot_core::{
        inspection::{
            InspectionSource, InspectionTarget, InspectionUnavailable, assemble_etcd_health,
        },
        workloads::{PodIssue, WorkloadKind, WorkloadSource, WorkloadSourceError},
    };

    fn member(id: u64, hostname: &str, peer_url: &str) -> EtcdMemberInfo {
        EtcdMemberInfo {
            id,
            hostname: hostname.into(),
            peer_urls: vec![peer_url.into()],
            client_urls: vec![],
            is_learner: false,
        }
    }

    fn workload(
        namespace: &str,
        name: &str,
        health: HealthState,
        ready: i32,
        desired: i32,
    ) -> WorkloadInfo {
        WorkloadInfo {
            namespace: namespace.into(),
            name: name.into(),
            kind: WorkloadKind::Deployment,
            ready,
            desired,
            health,
            issues: if ready < desired {
                vec!["Not all replicas ready".into()]
            } else {
                vec![]
            },
        }
    }

    fn pod(node: Option<&str>) -> PodInfo {
        PodInfo {
            name: "web-1".into(),
            namespace: "apps".into(),
            node: node.map(str::to_owned),
            phase: "Running".into(),
            restarts: 12,
            issue: PodIssue::HighRestarts(12),
            created_at: None,
        }
    }

    fn fixture() -> WorkloadSnapshot {
        WorkloadSnapshot {
            target: "chosen-context".into(),
            namespaces: vec![
                NamespaceSummary {
                    name: "apps".into(),
                    health: HealthState::Degraded,
                    workloads: vec![workload("apps", "web", HealthState::Degraded, 1, 3)],
                    problem_pods: vec![pod(Some("cp-a"))],
                    total_workloads: 1,
                    healthy_workloads: 0,
                },
                NamespaceSummary {
                    name: "system".into(),
                    health: HealthState::Healthy,
                    workloads: vec![
                        workload("system", "agent", HealthState::Healthy, 2, 2),
                        workload("system", "disabled", HealthState::Healthy, 0, 0),
                    ],
                    problem_pods: vec![],
                    total_workloads: 2,
                    healthy_workloads: 2,
                },
            ],
            total_deployments: 3,
            total_pods_degraded: 1,
            ..Default::default()
        }
    }

    #[test]
    fn namespace_filter_and_readiness_drilldown() {
        let snapshot = fixture();
        assert_eq!(
            filtered_namespaces(&snapshot, " APP ")
                .iter()
                .map(|namespace| namespace.name.as_str())
                .collect::<Vec<_>>(),
            vec!["apps"]
        );
        let apps = &snapshot.namespaces[0];
        assert!(workload_matches(&apps.workloads[0], "replicas", true));
        assert!(!workload_matches(
            &snapshot.namespaces[1].workloads[0],
            "",
            true
        ));
        assert!(!workload_matches(
            &snapshot.namespaces[1].workloads[1],
            "",
            true
        ));
        let selection = WorkloadSelection::Controller {
            namespace: "apps".into(),
            name: "web".into(),
            kind: "Deploy".into(),
        };
        let (details, _) =
            selected_details(&snapshot, &selection, &ClusterOverview::default()).unwrap();
        assert!(details.contains("1 / 3"));
        assert!(details.contains("Not all replicas ready"));
        assert!(pod_matches(&apps.problem_pods[0], "highrestarts"));
        assert_eq!(apps.problem_pods[0].restarts, 12);
    }

    #[test]
    fn unavailable_and_partial_never_become_healthy_empty_samples() {
        let errors = vec![WorkloadSourceError {
            source: WorkloadSource::Pods,
            message: "permission denied".into(),
        }];
        let unavailable = WorkloadCollectionOutcome::Unavailable {
            target: "chosen-context".into(),
            errors: errors.clone(),
        };
        assert!(unavailable.snapshot().is_none());
        let partial = WorkloadCollectionOutcome::Partial {
            snapshot: fixture(),
            unavailable: errors,
        };
        assert_eq!(partial.unavailable()[0].source, WorkloadSource::Pods);
        assert_eq!(
            partial.snapshot().unwrap().namespaces[0].workloads[0].ready,
            1
        );
        let mut state = SnapshotState::<WorkloadCollectionOutcome>::default();
        state.finish(Ok(partial));
        state.finish(Err(
            "Selected source and pinned Talos retry unavailable".into()
        ));
        assert!(state.data.as_ref().unwrap().snapshot().is_some());
        assert!(state.error.as_ref().unwrap().contains("unavailable"));
        assert!(state.freshness().contains("stale"));
    }

    #[test]
    fn effective_source_and_warning_update_with_snapshot_and_survive_failed_refresh() {
        let mut state = SnapshotState::<WorkloadData>::default();
        state.finish(WorkloadData::from_collection(
            WorkloadCollectionOutcome::Complete(fixture()),
            "Selected file /cluster/kubeconfig".into(),
            None,
        ));
        assert_eq!(
            state.data.as_ref().unwrap().source,
            "Selected file /cluster/kubeconfig"
        );
        assert!(state.data.as_ref().unwrap().warning.is_none());

        let retry_warning =
            "Selected file credentials failed; using this cluster's pinned Talos kubeconfig";
        let mut retry_snapshot = fixture();
        retry_snapshot.namespaces[0].workloads[0].ready = 2;
        state.finish(WorkloadData::from_collection(
            WorkloadCollectionOutcome::Complete(retry_snapshot),
            "Talos control plane (pinned same-cluster retry)".into(),
            Some(retry_warning.into()),
        ));
        let updated = state.data.as_ref().unwrap();
        assert_eq!(
            updated.source,
            "Talos control plane (pinned same-cluster retry)"
        );
        assert_eq!(updated.warning.as_deref(), Some(retry_warning));
        assert_eq!(
            updated.outcome.snapshot().unwrap().namespaces[0].workloads[0].ready,
            2
        );

        state.finish(WorkloadData::from_collection(
            WorkloadCollectionOutcome::Unavailable {
                target: "chosen-context".into(),
                errors: vec![WorkloadSourceError {
                    source: WorkloadSource::Pods,
                    message: "permission denied".into(),
                }],
            },
            "Source for the unsuccessful refresh must not relabel retained data".into(),
            Some("Warning for the unsuccessful refresh".into()),
        ));
        let retained = state.data.as_ref().unwrap();
        assert_eq!(
            retained.source,
            "Talos control plane (pinned same-cluster retry)"
        );
        assert_eq!(retained.warning.as_deref(), Some(retry_warning));
        assert_eq!(
            retained.outcome.snapshot().unwrap().namespaces[0].workloads[0].ready,
            2
        );
        assert!(state.error.as_ref().unwrap().contains("permission denied"));
        assert!(state.freshness().contains("stale"));

        state.finish(WorkloadData::from_collection(
            WorkloadCollectionOutcome::Complete(fixture()),
            "Selected file /cluster/kubeconfig".into(),
            None,
        ));
        assert_eq!(
            state.data.as_ref().unwrap().source,
            "Selected file /cluster/kubeconfig"
        );
        assert!(state.data.as_ref().unwrap().warning.is_none());
        assert!(state.error.is_none());
    }

    #[test]
    fn initial_etcd_selection_is_pinned_across_reordering_pages_filters_and_removal() {
        let mut snapshot = assemble_etcd_health(
            InspectionTarget::new("cp-a", "10.0.0.1"),
            (1..=30)
                .map(|id| member(id, &format!("cp-{id}"), "https://10.0.0.1:2380"))
                .collect(),
            vec![],
            vec![],
            vec![],
        );
        let selected = initial_member_selection(&snapshot.members, None);
        assert_eq!(selected, Some(1));
        snapshot.members.reverse();
        assert_eq!(
            initial_member_selection(&snapshot.members, selected),
            selected
        );
        let (_, _, first_page) = page_range(snapshot.members.len(), 0);
        assert!(
            !snapshot.members[first_page]
                .iter()
                .any(|member| Some(member.info.id) == selected)
        );
        let details = snapshot
            .members
            .iter()
            .find(|member| Some(member.info.id) == selected)
            .map(member_details)
            .unwrap();
        assert!(details.contains("cp-1"));
        assert!(
            snapshot
                .members
                .iter()
                .filter(|member| member.info.hostname.contains("cp-2"))
                .all(|member| Some(member.info.id) != selected)
        );
        snapshot
            .members
            .retain(|member| Some(member.info.id) != selected);
        assert_eq!(
            initial_member_selection(&snapshot.members, selected),
            selected
        );
        assert!(
            !snapshot
                .members
                .iter()
                .any(|member| Some(member.info.id) == selected)
        );
    }

    #[test]
    fn workload_row_identity_matches_details_across_pages_filters_and_reordering() {
        let mut snapshot = fixture();
        let controller = WorkloadSelection::controller(&snapshot.namespaces[0].workloads[0]);
        let pod = WorkloadSelection::pod(&snapshot.namespaces[0].problem_pods[0]);
        let cluster = ClusterOverview::default();
        let controller_details = selected_details(&snapshot, &controller, &cluster)
            .unwrap()
            .0;
        let pod_details = selected_details(&snapshot, &pod, &cluster).unwrap().0;
        snapshot.namespaces[0]
            .workloads
            .extend((0..PAGE_SIZE).map(|index| {
                workload(
                    "apps",
                    &format!("other-{index}"),
                    HealthState::Healthy,
                    1,
                    1,
                )
            }));
        snapshot.namespaces[0].workloads.reverse();
        let (_, _, range) = page_range(snapshot.namespaces[0].workloads.len(), 0);
        assert!(
            snapshot.namespaces[0].workloads[range]
                .iter()
                .all(|row| WorkloadSelection::controller(row) != controller)
        );
        assert!(!workload_matches(
            &snapshot.namespaces[0].workloads[PAGE_SIZE],
            "no-match",
            false
        ));
        assert!(!pod_matches(
            &snapshot.namespaces[0].problem_pods[0],
            "no-match"
        ));
        assert!(
            filtered_namespaces(&snapshot, "system")
                .iter()
                .all(|row| row.name != "apps")
        );
        snapshot.namespaces.reverse();
        assert_eq!(
            selected_details(&snapshot, &controller, &cluster)
                .unwrap()
                .0,
            controller_details
        );
        assert_eq!(
            selected_details(&snapshot, &pod, &cluster).unwrap().0,
            pod_details
        );
        assert!(WorkloadSelection::controller(&snapshot.namespaces[0].workloads[0]) != controller);
    }

    #[test]
    fn etcd_unknown_status_and_alarm_errors_remain_explicit() {
        let data = assemble_etcd_health(
            InspectionTarget::new("cp-a", "10.0.0.1"),
            vec![member(1, "cp-a", "https://10.0.0.1:2380")],
            vec![],
            vec![],
            vec![
                InspectionUnavailable {
                    source: InspectionSource::EtcdStatus,
                    message: "unavailable".into(),
                },
                InspectionUnavailable {
                    source: InspectionSource::EtcdAlarms,
                    message: "denied".into(),
                },
            ],
        );
        assert!(data.is_partial());
        assert_eq!(member_status(&data.members[0]), "Unknown / nonresponding");
        assert_eq!(leader_display(&data), "unknown");
        assert_eq!(quorum_display(&data), "UNKNOWN (status source unavailable)");
        assert!(member_details(&data.members[0]).contains("errors are unknown"));
        assert_eq!(data.responding_voting_members, 0);
    }

    #[test]
    fn stable_member_selection_and_roster_log_mapping() {
        let members = vec![
            member(1, "cp-a", "https://10.0.0.1:2380"),
            member(2, "cp-b", "https://[fd00::2]:2380"),
        ];
        assert_eq!(member_selection(&members, Some(2)), Some(2));
        assert_eq!(member_selection(&members, Some(42)), None);
        assert_eq!(member_selection(&members, None), Some(1));
        assert_eq!(member_selection(&[], Some(2)), None);
        let request = member_log_request(&members[1], &HashMap::new(), "etcd").unwrap();
        assert_eq!(request.node.as_deref(), Some("cp-b"));
        assert_eq!(request.address.as_deref(), Some("fd00::2"));
        assert_eq!(request.services, vec!["etcd"]);
        for service in CONTROL_PLANE_SERVICES {
            assert!(member_log_request(&members[0], &HashMap::new(), service).is_some());
        }
        assert!(
            member_log_request(
                &member(3, "", "https://10.0.0.3:2380"),
                &HashMap::new(),
                "etcd"
            )
            .is_none()
        );
        assert!(member_log_request(&members[0], &HashMap::new(), "unrelated").is_none());
    }

    #[test]
    fn pod_navigation_requires_exact_node_identity_and_selection_does_not_drift() {
        let mut cluster = ClusterOverview::default();
        cluster.node_ips.insert("cp-a".into(), "10.0.0.1".into());
        let request = pod_log_request(&pod(Some("cp-a")), &cluster).unwrap();
        assert_eq!(request.node.as_deref(), Some("cp-a"));
        assert_eq!(request.address.as_deref(), Some("10.0.0.1"));
        assert_eq!(request.services, vec!["kubelet"]);
        assert!(pod_log_request(&pod(Some("10.0.0.1")), &cluster).is_none());
        assert!(pod_log_request(&pod(None), &cluster).is_none());
        let chosen = WorkloadSelection::Pod {
            namespace: "apps".into(),
            name: "web-1".into(),
        };
        assert!(selected_details(&fixture(), &chosen, &cluster).is_some());
        let mut recovered = fixture();
        recovered.namespaces[0].problem_pods.clear();
        assert!(selected_details(&recovered, &chosen, &cluster).is_none());
    }

    #[test]
    fn all_controller_kinds_keep_ready_desired_and_namespace_identity() {
        let mut snapshot = fixture();
        for kind in [WorkloadKind::StatefulSet, WorkloadKind::DaemonSet] {
            let mut controller = workload("apps", "agent", HealthState::Degraded, 1, 2);
            controller.kind = kind;
            snapshot.namespaces[0].workloads.push(controller);
            let choice = WorkloadSelection::Controller {
                namespace: "apps".into(),
                name: "agent".into(),
                kind: kind.label().into(),
            };
            let (details, _) =
                selected_details(&snapshot, &choice, &ClusterOverview::default()).unwrap();
            assert!(details.contains(kind.label()));
            assert!(details.contains("1 / 2"));
        }
        let choice = WorkloadSelection::Controller {
            namespace: "system".into(),
            name: "agent".into(),
            kind: "Deploy".into(),
        };
        assert!(
            selected_details(&snapshot, &choice, &ClusterOverview::default())
                .unwrap()
                .0
                .contains("2 / 2")
        );
    }

    #[test]
    fn correlated_member_details_preserve_db_raft_errors_alarms_and_learner_quorum() {
        use talos_rs::{EtcdAlarm, EtcdAlarmType, EtcdMemberStatus};
        let status = EtcdMemberStatus {
            node: "cp-a".into(),
            member_id: 1,
            protocol_version: "3.5".into(),
            db_size: 1_000,
            db_size_in_use: 750,
            leader_id: 1,
            raft_index: 200,
            raft_term: 4,
            raft_applied_index: 198,
            errors: vec!["database issue".into()],
            is_learner: false,
        };
        let mut learner = member(2, "cp-b", "https://10.0.0.2:2380");
        learner.is_learner = true;
        let data = assemble_etcd_health(
            InspectionTarget::new("cp-a", "10.0.0.1"),
            vec![member(1, "cp-a", "https://10.0.0.1:2380"), learner],
            vec![status],
            vec![EtcdAlarm {
                node: "cp-a".into(),
                member_id: 1,
                alarm_type: EtcdAlarmType::Corrupt,
            }],
            vec![],
        );
        assert_eq!(data.voting_members, 1);
        assert_eq!(data.responding_voting_members, 1);
        assert!(data.quorum.has_quorum());
        assert_eq!(data.revision, 200);
        assert_eq!(data.largest_database_size, 1_000);
        assert!(leader_display(&data).contains("cp-a"));
        let details = member_details(&data.members[0]);
        for expected in [
            "75.0%",
            "applied 198",
            "term 4",
            "unapplied 2",
            "database issue",
            "Corrupt",
        ] {
            assert!(
                details.contains(expected),
                "Missing {expected} from {details}"
            );
        }
        assert_eq!(member_status(&data.members[0]), "Issues reported");
    }

    #[test]
    fn etcd_retention_preserves_full_sample_quorum_counts() {
        let members = (0..MAX_NAMESPACES + 1)
            .map(|id| member(id as u64, "cp", "https://10.0.0.1:2380"))
            .collect();
        let snapshot = assemble_etcd_health(
            InspectionTarget::new("cp", "10.0.0.1"),
            members,
            vec![],
            vec![],
            vec![],
        );
        let data = bound_etcd(snapshot);
        assert!(data.truncated);
        assert_eq!(data.snapshot.members.len(), MAX_NAMESPACES);
        assert_eq!(data.snapshot.voting_members, MAX_NAMESPACES + 1);
    }

    #[test]
    fn pages_and_retention_are_bounded_without_rewriting_totals() {
        assert_eq!(page_range(0, usize::MAX), (0, 1, 0..0));
        assert_eq!(page_range(26, 99), (1, 2, 25..26));
        let mut snapshot = fixture();
        snapshot.namespaces[0].workloads =
            vec![workload("apps", "many", HealthState::Healthy, 1, 1); MAX_ROWS + 1];
        snapshot.total_deployments = MAX_ROWS + 3;
        let mut outcome = WorkloadCollectionOutcome::Complete(snapshot);
        assert!(bound_workloads(&mut outcome));
        let snapshot = outcome.snapshot().unwrap();
        assert_eq!(
            snapshot
                .namespaces
                .iter()
                .map(|namespace| namespace.workloads.len())
                .sum::<usize>(),
            MAX_ROWS
        );
        assert_eq!(snapshot.total_deployments, MAX_ROWS + 3);
        assert!(bounded_text(&"ü".repeat(MAX_TEXT + 1)).ends_with("[text truncated]"));
    }
}
