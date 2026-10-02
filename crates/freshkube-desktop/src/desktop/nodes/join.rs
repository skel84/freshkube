use gpui_kit::SharedString;
// A joined row is derived when either cluster summary changes.
use crate::{
    presentation::{NodeSummary as TalosNode, Role},
    ui::Tone,
};
use freshkube_core::kubernetes_summary::NodeSummary as KubernetesNode;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
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
    let name = kubernetes
        .map(|node| node.name.as_str())
        .or_else(|| talos.map(|node| node.name.as_str()))
        .unwrap_or_default();
    let role = if kubernetes.is_some_and(|node| {
        node.roles
            .iter()
            .any(|role| matches!(role.as_str(), "control-plane" | "master"))
    }) {
        Role::ControlPlane
    } else {
        talos.map(|node| node.role).unwrap_or(Role::Worker)
    };
    let ready = match kubernetes {
        Some(node) if node.is_ready() => "Ready",
        Some(_) => "NotReady",
        None if !kubernetes_available => "Kubernetes unavailable",
        None => "Not in Kubernetes",
    };
    let talos_state = match talos {
        Some(node) if !node.responding => "No response".to_owned(),
        Some(node) => node
            .version
            .clone()
            .unwrap_or_else(|| "Version not reported".into()),
        None if !talos_available => "Talos unavailable".into(),
        None => "No Talos data".into(),
    };
    let address = talos
        .map(|node| node.address.clone())
        .or_else(|| {
            kubernetes.and_then(|node| {
                node.addresses
                    .iter()
                    .find(|(kind, _)| kind == "InternalIP")
                    .or_else(|| node.addresses.first())
                    .map(|(_, address)| address.clone())
            })
        })
        .unwrap_or_default();
    let mut problems: Vec<SharedString> = Vec::new();
    let mut tone = Tone::Good;
    if kubernetes.is_some_and(|node| !node.is_ready()) {
        tone = Tone::Crit;
        problems.push("Kubernetes NotReady".into());
    }
    if let Some(node) = talos {
        if !node.responding {
            tone = Tone::Crit;
            problems.push("Talos API not answering".into());
        }
        for service in node.unhealthy_services() {
            if tone != Tone::Crit {
                tone = Tone::Warn;
            }
            problems.push(format!("{} unhealthy", service.id).into());
        }
        if let Some(memory) = node.memory.filter(|memory| memory.percent() >= 90.) {
            if tone != Tone::Crit {
                tone = Tone::Warn;
            }
            problems.push(format!("Memory {:.0}%", memory.percent()).into());
        }
    }
    let load = talos
        .and_then(|node| node.load)
        .map(|load| format!("{:.2} · {:.2} · {:.2}", load[0], load[1], load[2]))
        .unwrap_or_else(|| "—".into());
    let memory = talos
        .and_then(|node| node.memory)
        .map(|memory| {
            format!(
                "{:.0}% · {}",
                memory.percent(),
                freshkube_core::formatting::format_bytes(memory.total)
            )
        })
        .unwrap_or_else(|| "—".into());
    let counts = talos.map(|node| node.health_counts()).unwrap_or_default();
    let services = talos
        .filter(|node| node.responding)
        .map(|_| {
            format!(
                "{} healthy · {} unhealthy",
                counts.healthy, counts.unhealthy
            )
        })
        .unwrap_or_else(|| "—".into());
    let note = if ready == "NotReady" {
        ready.into()
    } else if talos.is_some_and(|node| !node.responding) {
        "No response".into()
    } else if counts.unhealthy > 0 {
        format!("{} svc", counts.unhealthy)
    } else if talos
        .and_then(|node| node.memory)
        .is_some_and(|memory| memory.percent() >= 90.)
    {
        memory.clone()
    } else if role == Role::ControlPlane {
        "cp".into()
    } else {
        "worker".into()
    };
    let mut chips: Vec<SharedString> = vec![role.label().into(), address.clone().into()];
    let mut facts: Vec<(SharedString, SharedString)> = vec![
        ("Kubernetes".into(), ready.into()),
        ("Talos".into(), talos_state.clone().into()),
        ("Load".into(), load.clone().into()),
        ("Memory".into(), memory.clone().into()),
        ("System services".into(), services.clone().into()),
    ];
    if let Some(node) = talos {
        if let Some(version) = &node.version {
            chips.push(format!("Talos {version}").into());
        }
        if let Some(cores) = node.cores {
            chips.push(format!("{cores} cores").into());
        }
        if let Some(memory) = node.memory {
            chips.push(freshkube_core::formatting::format_bytes(memory.total).into());
        }
        facts.push((
            "etcd member".into(),
            if node.etcd_member { "Yes" } else { "No" }.into(),
        ));
    }
    if let Some(node) = kubernetes {
        if talos.is_none() {
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
    NodeRow {
        key: NodeKey {
            kubernetes: kubernetes.map(|node| node.name.clone()),
            talos: talos.map(|node| node.name.clone()),
        },
        id: format!("node-{name}").into(),
        open_id: format!("node-{name}-open").into(),
        pod_label: kubernetes
            .map(|node| format!("Pods {}", node.pods))
            .unwrap_or_else(|| "Pods".into())
            .into(),
        kubelet_pods: kubernetes
            .map(|node| format!("Pods on this node ({})", node.pods))
            .unwrap_or("Pods on this node".into())
            .into(),
        service_problem: counts.unhealthy > 0,
        name: name.into(),
        role,
        tone,
        kubernetes: kubernetes.cloned(),
        talos: talos.cloned(),
        ready,
        talos_state: talos_state.into(),
        address: address.into(),
        load: load.into(),
        memory: memory.into(),
        pods: kubernetes
            .map(|node| node.pods.to_string())
            .unwrap_or_else(|| "—".into())
            .into(),
        services: services.into(),
        note: note.into(),
        chips,
        facts,
        problems,
    }
}
