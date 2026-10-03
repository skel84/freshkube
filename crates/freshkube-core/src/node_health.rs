//! Health interpretation for nodes observed by Talos, Kubernetes, or both.
//! Source availability stays distinct from the health of an observed node.

use talos_rs::ServiceInfo;

use crate::{
    HasHealth, HealthIndicator, NodeRole, constants::NODE_MEMORY_WARNING_PERCENT,
    kubernetes_summary::NodeSummary as KubernetesNode,
};

/// The Talos facts needed to assess a node, independent of any view model.
#[derive(Debug)]
pub struct TalosNodeFacts<'a> {
    role: NodeRole,
    responding: bool,
    services: &'a [ServiceInfo],
    memory_percent: Option<f64>,
}

impl<'a> TalosNodeFacts<'a> {
    pub fn new(
        role: NodeRole,
        responding: bool,
        services: &'a [ServiceInfo],
        memory_percent: Option<f64>,
    ) -> Self {
        Self {
            role,
            responding,
            services,
            memory_percent,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KubernetesNodeState {
    Unavailable,
    Absent,
    Ready,
    NotReady,
    Stale,
}

impl HasHealth for KubernetesNodeState {
    fn health(&self) -> HealthIndicator {
        match self {
            Self::Ready => HealthIndicator::Healthy,
            Self::NotReady => HealthIndicator::Error,
            Self::Unavailable | Self::Absent | Self::Stale => HealthIndicator::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TalosNodeState {
    Unavailable,
    Absent,
    Responding,
    Unresponsive,
}

impl HasHealth for TalosNodeState {
    fn health(&self) -> HealthIndicator {
        match self {
            Self::Responding => HealthIndicator::Healthy,
            Self::Unresponsive => HealthIndicator::Error,
            Self::Unavailable | Self::Absent => HealthIndicator::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NodeProblem<'a> {
    KubernetesNotReady,
    TalosUnresponsive,
    UnhealthyService(&'a str),
    HighMemory(f64),
}

impl HasHealth for NodeProblem<'_> {
    fn health(&self) -> HealthIndicator {
        match self {
            Self::KubernetesNotReady | Self::TalosUnresponsive => HealthIndicator::Error,
            Self::UnhealthyService(_) | Self::HighMemory(_) => HealthIndicator::Warning,
        }
    }
}

#[derive(Debug)]
pub struct NodeAssessment<'a> {
    role: NodeRole,
    kubernetes: KubernetesNodeState,
    talos: TalosNodeState,
    problems: Vec<NodeProblem<'a>>,
}

impl<'a> NodeAssessment<'a> {
    pub fn from_sources(
        talos: Option<TalosNodeFacts<'a>>,
        kubernetes: Option<&KubernetesNode>,
        talos_available: bool,
        kubernetes_available: bool,
    ) -> Self {
        let role = node_role(talos.as_ref(), kubernetes);
        let kubernetes = match kubernetes {
            Some(node) if node.is_ready() => KubernetesNodeState::Ready,
            Some(_) => KubernetesNodeState::NotReady,
            None if kubernetes_available => KubernetesNodeState::Absent,
            None => KubernetesNodeState::Unavailable,
        };
        let talos_state = match &talos {
            Some(node) if node.responding => TalosNodeState::Responding,
            Some(_) => TalosNodeState::Unresponsive,
            None if talos_available => TalosNodeState::Absent,
            None => TalosNodeState::Unavailable,
        };
        let mut problems = Vec::new();
        if kubernetes == KubernetesNodeState::NotReady {
            problems.push(NodeProblem::KubernetesNotReady);
        }
        if talos_state == TalosNodeState::Unresponsive {
            problems.push(NodeProblem::TalosUnresponsive);
        }
        if let Some(node) = talos {
            problems.extend(
                node.services
                    .iter()
                    .filter(|service| service.health() == HealthIndicator::Error)
                    .map(|service| NodeProblem::UnhealthyService(&service.id)),
            );
            if let Some(percent) = node
                .memory_percent
                .filter(|percent| *percent >= NODE_MEMORY_WARNING_PERCENT)
            {
                problems.push(NodeProblem::HighMemory(percent));
            }
        }
        Self {
            role,
            kubernetes,
            talos: talos_state,
            problems,
        }
    }

    pub fn role(&self) -> &NodeRole {
        &self.role
    }

    pub fn kubernetes(&self) -> KubernetesNodeState {
        self.kubernetes
    }

    /// Retained evidence from an interrupted observation is not current health.
    /// Keep independent Talos findings and the role inferred from Node facts.
    pub fn with_kubernetes_current(mut self, current: bool) -> Self {
        if !current
            && matches!(
                self.kubernetes,
                KubernetesNodeState::Ready | KubernetesNodeState::NotReady
            )
        {
            self.kubernetes = KubernetesNodeState::Stale;
            self.problems
                .retain(|problem| !matches!(problem, NodeProblem::KubernetesNotReady));
        }
        self
    }

    pub fn talos(&self) -> TalosNodeState {
        self.talos
    }

    /// Problems in priority order: readiness, response, services, then memory.
    /// Service order follows the input snapshot.
    pub fn problems(&self) -> &[NodeProblem<'a>] {
        &self.problems
    }
}

impl HasHealth for NodeAssessment<'_> {
    /// Summarize observed problems. Unavailable or absent sources alone do not
    /// make the node unhealthy; their state is exposed separately.
    fn health(&self) -> HealthIndicator {
        self.problems.iter().fold(
            if self.kubernetes == KubernetesNodeState::Stale {
                HealthIndicator::Unknown
            } else {
                HealthIndicator::Healthy
            },
            |health, problem| health.worst(problem.health()),
        )
    }
}

fn node_role(talos: Option<&TalosNodeFacts<'_>>, kubernetes: Option<&KubernetesNode>) -> NodeRole {
    if kubernetes.is_some_and(|node| {
        node.roles
            .iter()
            .any(|role| matches!(role.as_str(), "control-plane" | "master"))
    }) {
        NodeRole::ControlPlane
    } else {
        talos
            .map(|node| node.role.clone())
            .unwrap_or(NodeRole::Worker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kubernetes_summary::NodeCondition;

    fn kubernetes() -> KubernetesNode {
        KubernetesNode {
            uid: "uid".into(),
            name: "node".into(),
            conditions: vec![NodeCondition {
                kind: "Ready".into(),
                status: "True".into(),
                reason: String::new(),
                message: String::new(),
                since: None,
            }],
            unschedulable: false,
            roles: Vec::new(),
            addresses: Vec::new(),
            kubelet_version: String::new(),
            capacity: Default::default(),
            taints: Vec::new(),
            pods: 0,
            pods_current: true,
            pods_observed: true,
        }
    }

    #[test]
    fn stale_kubernetes_evidence_keeps_independent_talos_problems() {
        let mut kube = kubernetes();
        kube.conditions[0].status = "False".into();
        let stale = NodeAssessment::from_sources(None, Some(&kube), false, false)
            .with_kubernetes_current(false);
        assert_eq!(stale.kubernetes(), KubernetesNodeState::Stale);
        assert_eq!(stale.health(), HealthIndicator::Unknown);
        assert!(stale.problems().is_empty());
        let talos_failure = NodeAssessment::from_sources(
            Some(TalosNodeFacts::new(NodeRole::Worker, false, &[], None)),
            Some(&kube),
            true,
            false,
        ).with_kubernetes_current(false);
        assert_eq!(talos_failure.health(), HealthIndicator::Error);
        assert_eq!(talos_failure.problems(), [NodeProblem::TalosUnresponsive]);
    }

    #[test]
    fn source_availability_is_distinct_from_absence_and_node_health() {
        for (available, kube_state, talos_state) in [
            (
                false,
                KubernetesNodeState::Unavailable,
                TalosNodeState::Unavailable,
            ),
            (true, KubernetesNodeState::Absent, TalosNodeState::Absent),
        ] {
            let empty = NodeAssessment::from_sources(None, None, available, available);
            assert_eq!(empty.kubernetes(), kube_state);
            assert_eq!(empty.talos(), talos_state);
            assert_eq!(empty.health(), HealthIndicator::Healthy);
            assert_eq!(empty.kubernetes().health(), HealthIndicator::Unknown);
            assert_eq!(empty.talos().health(), HealthIndicator::Unknown);

            let kube_only =
                NodeAssessment::from_sources(None, Some(&kubernetes()), available, true);
            assert_eq!(kube_only.kubernetes(), KubernetesNodeState::Ready);
            assert_eq!(kube_only.talos(), talos_state);
            assert_eq!(kube_only.health(), HealthIndicator::Healthy);

            let talos_only = NodeAssessment::from_sources(
                Some(TalosNodeFacts::new(NodeRole::Unknown, true, &[], None)),
                None,
                true,
                available,
            );
            assert_eq!(talos_only.kubernetes(), kube_state);
            assert_eq!(talos_only.talos(), TalosNodeState::Responding);
            assert_eq!(talos_only.health(), HealthIndicator::Healthy);
        }
    }

    #[test]
    fn observed_readiness_and_response_take_precedence_over_source_flags() {
        let mut kube = kubernetes();
        for ready in ["False", "Unknown", ""] {
            kube.conditions[0].status = ready.into();
            let node = NodeAssessment::from_sources(
                Some(TalosNodeFacts::new(NodeRole::Worker, false, &[], None)),
                Some(&kube),
                false,
                false,
            );
            assert_eq!(node.kubernetes(), KubernetesNodeState::NotReady);
            assert_eq!(node.talos(), TalosNodeState::Unresponsive);
            assert_eq!(node.health(), HealthIndicator::Error);
            assert_eq!(
                node.problems(),
                [
                    NodeProblem::KubernetesNotReady,
                    NodeProblem::TalosUnresponsive
                ]
            );
        }
        kube.conditions.clear();
        let missing_ready = NodeAssessment::from_sources(None, Some(&kube), true, true);
        assert_eq!(missing_ready.kubernetes(), KubernetesNodeState::NotReady);
    }

    #[test]
    fn kubernetes_control_plane_roles_win_then_talos_then_worker() {
        let mut kube = kubernetes();
        for role in ["control-plane", "master"] {
            kube.roles = vec![role.into()];
            for talos_role in [NodeRole::Worker, NodeRole::Unknown] {
                let node = NodeAssessment::from_sources(
                    Some(TalosNodeFacts::new(talos_role, true, &[], None)),
                    Some(&kube),
                    true,
                    true,
                );
                assert_eq!(node.role(), &NodeRole::ControlPlane);
            }
        }
        kube.roles = vec!["worker".into()];
        for role in [NodeRole::ControlPlane, NodeRole::Worker, NodeRole::Unknown] {
            let node = NodeAssessment::from_sources(
                Some(TalosNodeFacts::new(role.clone(), true, &[], None)),
                Some(&kube),
                true,
                true,
            );
            assert_eq!(node.role(), &role);
        }
        assert_eq!(
            NodeAssessment::from_sources(None, Some(&kube), false, true).role(),
            &NodeRole::Worker
        );
    }

    #[test]
    fn service_and_memory_warnings_do_not_override_critical_problems() {
        let services = [ServiceInfo {
            id: "kubelet".into(),
            state: "Running".into(),
            health: Some(talos_rs::ServiceHealth {
                unknown: false,
                healthy: false,
                last_message: String::new(),
            }),
        }];
        for responding in [false, true] {
            let node = NodeAssessment::from_sources(
                Some(TalosNodeFacts::new(
                    NodeRole::Worker,
                    responding,
                    &services,
                    Some(96.),
                )),
                None,
                true,
                false,
            );
            let warnings = [
                NodeProblem::UnhealthyService("kubelet"),
                NodeProblem::HighMemory(96.),
            ];
            if responding {
                assert_eq!(node.health(), HealthIndicator::Warning);
                assert_eq!(node.problems(), warnings);
            } else {
                assert_eq!(node.health(), HealthIndicator::Error);
                assert_eq!(node.problems()[0], NodeProblem::TalosUnresponsive);
                assert_eq!(node.problems()[1..], warnings);
            }
        }
    }

    #[test]
    fn node_memory_warning_starts_at_ninety_percent() {
        for (percent, expected) in [
            (None, HealthIndicator::Healthy),
            (Some(85.), HealthIndicator::Healthy),
            (Some(89.99), HealthIndicator::Healthy),
            (Some(90.), HealthIndicator::Warning),
            (Some(100.), HealthIndicator::Warning),
        ] {
            let node = NodeAssessment::from_sources(
                Some(TalosNodeFacts::new(NodeRole::Worker, true, &[], percent)),
                None,
                true,
                false,
            );
            assert_eq!(node.health(), expected, "memory {percent:?}");
        }
    }

    #[test]
    fn unknown_services_do_not_create_node_problems() {
        let services = [
            ServiceInfo {
                id: "unknown".into(),
                state: "Failed".into(),
                health: Some(talos_rs::ServiceHealth {
                    unknown: true,
                    healthy: false,
                    last_message: String::new(),
                }),
            },
            ServiceInfo {
                id: "missing".into(),
                state: "Stopped".into(),
                health: None,
            },
        ];
        let node = NodeAssessment::from_sources(
            Some(TalosNodeFacts::new(NodeRole::Worker, true, &services, None)),
            None,
            true,
            false,
        );
        assert!(node.problems().is_empty());
        assert_eq!(node.health(), HealthIndicator::Healthy);
    }
}
