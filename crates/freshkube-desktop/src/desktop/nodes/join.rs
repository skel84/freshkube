use gpui_kit::SharedString;
// A joined row is derived when either cluster summary changes.
use crate::{
    presentation::{NodeSummary as TalosNode, Role},
    ui::Tone,
};
use freshkube_core::{
    HasHealth, HealthIndicator,
    kubernetes_summary::NodeSummary as KubernetesNode,
    node_health::{
        KubernetesNodeState, NodeAssessment, NodeProblem, TalosNodeFacts, TalosNodeState,
    },
};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct NodeKey {
    pub(crate) kubernetes: Option<String>,
    pub(crate) talos: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct NodeRow {
    pub(crate) key: NodeKey,
    pub(crate) id: SharedString,
    pub(crate) open_id: SharedString,
    pub(crate) pod_label: SharedString,
    pub(crate) kubelet_pods: SharedString,
    pub(crate) service_problem: bool,
    pub(crate) name: SharedString,
    pub(crate) role: Role,
    pub(crate) tone: Tone,
    pub(crate) kubernetes: Option<KubernetesNode>,
    pub(crate) kubernetes_current: bool,
    pub(crate) talos: Option<TalosNode>,
    pub(crate) ready: &'static str,
    pub(crate) talos_state: SharedString,
    pub(crate) address: SharedString,
    pub(crate) load: SharedString,
    pub(crate) memory: SharedString,
    pub(crate) pods: SharedString,
    pub(crate) services: SharedString,
    pub(crate) note: SharedString,
    pub(crate) chips: Vec<SharedString>,
    pub(crate) facts: Vec<(SharedString, SharedString)>,
    pub(crate) problems: Vec<SharedString>,
}

pub(crate) fn join(
    talos: &[TalosNode],
    kubernetes: &[KubernetesNode],
    talos_available: bool,
    kubernetes_available: bool,
) -> Vec<NodeRow> {
    let mut matched = BTreeSet::new();
    // Reserve all name matches before considering addresses.
    let mut pairs: Vec<_> = talos
        .iter()
        .map(|talos| {
            let partner = kubernetes
                .iter()
                .enumerate()
                .find(|(ix, node)| node.name == talos.name && !matched.contains(ix))
                .map(|(ix, _)| ix);
            if let Some(ix) = partner {
                matched.insert(ix);
            }
            (talos, partner)
        })
        .collect();
    for (talos, partner) in &mut pairs {
        if partner.is_none() {
            *partner = kubernetes
                .iter()
                .enumerate()
                .find(|(ix, node)| {
                    !matched.contains(ix)
                        && node.addresses.iter().any(|(kind, address)| {
                            matches!(kind.as_str(), "InternalIP" | "ExternalIP")
                                && address == &talos.address
                        })
                })
                .map(|(ix, _)| ix);
            if let Some(ix) = partner {
                matched.insert(*ix);
            }
        }
    }
    let mut rows: Vec<_> = pairs
        .into_iter()
        .map(|(talos, partner)| {
            row(
                Some(talos),
                partner.map(|ix| &kubernetes[ix]),
                talos_available,
                kubernetes_available,
            )
        })
        .collect();
    rows.extend(
        kubernetes
            .iter()
            .enumerate()
            .filter(|(ix, _)| !matched.contains(ix))
            .map(|(_, node)| row(None, Some(node), talos_available, kubernetes_available)),
    );
    rows.sort_by(|left, right| {
        left.role
            .cmp(&right.role)
            .then_with(|| left.name.cmp(&right.name))
    });
    rows
}

fn row(
    talos: Option<&TalosNode>,
    kubernetes: Option<&KubernetesNode>,
    talos_available: bool,
    kubernetes_available: bool,
) -> NodeRow {
    let assessment = NodeAssessment::from_sources(
        talos.map(|node| {
            TalosNodeFacts::new(
                node.role.into(),
                node.responding,
                &node.services,
                node.memory.map(|memory| memory.percent()),
            )
        }),
        kubernetes,
        talos_available,
        kubernetes_available,
    )
    .with_kubernetes_current(kubernetes_available);
    let name = kubernetes
        .map(|node| node.name.as_str())
        .or_else(|| talos.map(|node| node.name.as_str()))
        .unwrap_or_default();
    let role = Role::from(assessment.role());
    let counts = talos.map(|node| node.health_counts()).unwrap_or_default();
    let memory = memory_text(talos);
    let note = compact_note(
        assessment.problems().first(),
        counts.unhealthy,
        role,
        &memory,
    );
    let pod_count = kubernetes
        .map(|node| {
            if !node.pods_observed {
                "—".into()
            } else if !node.pods_current {
                format!("{} last known", node.pods)
            } else {
                node.pods.to_string()
            }
        })
        .unwrap_or_else(|| "—".into());
    let mut row = NodeRow {
        kubernetes_current: kubernetes_available,
        key: NodeKey {
            kubernetes: kubernetes.map(|node| node.name.clone()),
            talos: talos.map(|node| node.name.clone()),
        },
        id: format!("node-{name}").into(),
        open_id: format!("node-{name}-open").into(),
        pod_label: kubernetes
            .map(|_| format!("Pods {pod_count}"))
            .unwrap_or_else(|| "Pods".into())
            .into(),
        kubelet_pods: kubernetes
            .map(|_| format!("Pods on this node ({pod_count})"))
            .unwrap_or_else(|| "Pods on this node".into())
            .into(),
        service_problem: counts.unhealthy > 0,
        name: name.into(),
        role,
        tone: node_tone(assessment.health()),
        kubernetes: kubernetes.cloned(),
        talos: talos.cloned(),
        ready: readiness_label(assessment.kubernetes()),
        talos_state: talos_label(
            assessment.talos(),
            talos.and_then(|node| node.version.as_deref()),
        ),
        address: node_address(talos, kubernetes).into(),
        load: load_text(talos),
        memory,
        pods: pod_count.into(),
        services: match assessment.talos() {
            TalosNodeState::Responding => format!(
                "{} healthy · {} unhealthy",
                counts.healthy, counts.unhealthy
            )
            .into(),
            _ => "—".into(),
        },
        note,
        chips: Vec::new(),
        facts: Vec::new(),
        problems: assessment.problems().iter().map(problem_text).collect(),
    };
    row.chips = chips(&row);
    row.facts = facts(&row);
    row
}

fn node_tone(health: HealthIndicator) -> Tone {
    match health {
        HealthIndicator::Healthy => Tone::Good,
        HealthIndicator::Warning => Tone::Warn,
        HealthIndicator::Error => Tone::Crit,
        HealthIndicator::Pending | HealthIndicator::Unknown => Tone::Unknown,
        HealthIndicator::Info => Tone::Accent,
    }
}

fn readiness_label(state: KubernetesNodeState) -> &'static str {
    match state {
        KubernetesNodeState::Ready => "Ready",
        KubernetesNodeState::NotReady => "NotReady",
        KubernetesNodeState::Stale => "Last known",
        KubernetesNodeState::Unavailable => "Kubernetes unavailable",
        KubernetesNodeState::Absent => "Not in Kubernetes",
    }
}

fn talos_label(state: TalosNodeState, version: Option<&str>) -> SharedString {
    match state {
        TalosNodeState::Responding => version.unwrap_or("Version not reported").to_owned().into(),
        TalosNodeState::Unresponsive => "No response".into(),
        TalosNodeState::Unavailable => "Talos unavailable".into(),
        TalosNodeState::Absent => "No Talos data".into(),
    }
}

fn node_address<'a>(
    talos: Option<&'a TalosNode>,
    kubernetes: Option<&'a KubernetesNode>,
) -> &'a str {
    talos
        .map(|node| node.address.as_str())
        .or_else(|| {
            kubernetes.and_then(|node| {
                node.addresses
                    .iter()
                    .find(|(kind, _)| kind == "InternalIP")
                    .or_else(|| node.addresses.first())
                    .map(|(_, address)| address.as_str())
            })
        })
        .unwrap_or_default()
}

fn load_text(talos: Option<&TalosNode>) -> SharedString {
    talos
        .and_then(|node| node.load)
        .map(|load| format!("{:.2} · {:.2} · {:.2}", load[0], load[1], load[2]))
        .unwrap_or_else(|| "—".into())
        .into()
}

fn memory_text(talos: Option<&TalosNode>) -> SharedString {
    talos
        .and_then(|node| node.memory)
        .map(|memory| {
            format!(
                "{:.0}% · {}",
                memory.percent(),
                freshkube_core::formatting::format_bytes(memory.total)
            )
        })
        .unwrap_or_else(|| "—".into())
        .into()
}

fn problem_text(problem: &NodeProblem<'_>) -> SharedString {
    match problem {
        NodeProblem::KubernetesNotReady => "Kubernetes NotReady".into(),
        NodeProblem::TalosUnresponsive => "Talos API not answering".into(),
        NodeProblem::UnhealthyService(id) => format!("{id} unhealthy").into(),
        NodeProblem::HighMemory(percent) => format!("Memory {percent:.0}%").into(),
    }
}

fn compact_note(
    problem: Option<&NodeProblem<'_>>,
    unhealthy_services: usize,
    role: Role,
    memory: &SharedString,
) -> SharedString {
    match problem {
        Some(NodeProblem::KubernetesNotReady) => "NotReady".into(),
        Some(NodeProblem::TalosUnresponsive) => "No response".into(),
        Some(NodeProblem::UnhealthyService(_)) => format!("{unhealthy_services} svc").into(),
        Some(NodeProblem::HighMemory(_)) => memory.clone(),
        None if role == Role::ControlPlane => "cp".into(),
        None => "worker".into(),
    }
}

fn chips(row: &NodeRow) -> Vec<SharedString> {
    let mut chips = vec![row.role.label().into(), row.address.clone()];
    if let Some(node) = &row.talos {
        talos_chips(node, &mut chips);
    }
    if let Some(node) = &row.kubernetes {
        kubernetes_chips(node, row.talos.is_none(), &mut chips);
    }
    chips
}

fn talos_chips(node: &TalosNode, chips: &mut Vec<SharedString>) {
    if let Some(version) = &node.version {
        chips.push(format!("Talos {version}").into());
    }
    if let Some(cores) = node.cores {
        chips.push(format!("{cores} cores").into());
    }
    if let Some(memory) = node.memory {
        chips.push(freshkube_core::formatting::format_bytes(memory.total).into());
    }
}

fn kubernetes_chips(node: &KubernetesNode, show_capacity: bool, chips: &mut Vec<SharedString>) {
    if show_capacity {
        if let Some(cores) = node.capacity.get("cpu") {
            chips.push(format!("{} cores", cores.0).into());
        }
        if let Some(memory) = node.capacity.get("memory") {
            chips.push(format!("{} memory", memory.0).into());
        }
    }
    if !node.kubelet_version.is_empty() {
        chips.push(format!("kubelet {}", node.kubelet_version).into());
    }
    if node.unschedulable {
        chips.push("Cordoned".into());
    }
}

fn facts(row: &NodeRow) -> Vec<(SharedString, SharedString)> {
    let mut facts = vec![
        ("Kubernetes".into(), row.ready.into()),
        ("Talos".into(), row.talos_state.clone()),
        ("Load".into(), row.load.clone()),
        ("Memory".into(), row.memory.clone()),
        ("System services".into(), row.services.clone()),
    ];
    if let Some(node) = &row.talos {
        facts.push((
            "etcd member".into(),
            if node.etcd_member { "Yes" } else { "No" }.into(),
        ));
    }
    if let Some(node) = &row.kubernetes {
        kubernetes_facts(node, &mut facts);
    }
    facts
}

fn kubernetes_facts(node: &KubernetesNode, facts: &mut Vec<(SharedString, SharedString)>) {
    for condition in &node.conditions {
        facts.push((
            condition.kind.clone().into(),
            format!(
                "{} · {} · {}",
                condition.status, condition.reason, condition.message
            )
            .into(),
        ));
    }
    for (kind, address) in &node.addresses {
        facts.push((kind.clone().into(), address.clone().into()));
    }
    for (kind, amount) in &node.capacity {
        facts.push((format!("Capacity {kind}").into(), amount.0.clone().into()));
    }
    for taint in &node.taints {
        facts.push(("Taint".into(), taint.clone().into()));
    }
}
