//! Minimal summary evidence. Full API objects die before entering a store.
use std::{borrow::Cow, mem::size_of};

use chrono::{DateTime, Utc};
use k8s_openapi::api::{
    apps::v1::{DaemonSet, Deployment, StatefulSet},
    core::v1::{Event, Namespace, Node, PersistentVolume, PersistentVolumeClaim, Pod},
};
use kube::{Resource, runtime::reflector::Lookup};

use super::{NodeSummary, ObservationFailure, Source};
use crate::workloads::{self, PodIssue, WorkloadInfo};

#[derive(Clone, Debug)]
pub(super) struct PodFacts {
    pub node: Option<String>,
    pub phase: String,
    pub restarts: i32,
    pub issue: Option<PodIssue>,
    pub requests: crate::resources::Amounts,
}

#[derive(Clone, Debug)]
pub(super) enum Facts {
    Node(Box<NodeSummary>),
    Pod(PodFacts),
    Workload(Box<WorkloadInfo>),
    Event(Box<Event>),
    Claim(String),
    Volume(bool),
    Namespace,
}

/// A projected object accepted by the summary reflector. The representation
/// deliberately cannot be used to answer full-object or printer-table reads.
#[derive(Clone, Debug)]
pub struct RetainedObject {
    pub(super) name: String,
    pub(super) namespace: String,
    pub(super) uid: String,
    pub(super) version: String,
    pub(super) created: Option<DateTime<Utc>>,
    pub(super) facts: Facts,
    pub(super) bytes: usize,
}

impl RetainedObject {
    fn new<K: Resource>(object: &K, facts: Facts, extra_bytes: usize) -> Self {
        let metadata = object.meta();
        let mut result = Self {
            name: metadata.name.clone().unwrap_or_default(),
            namespace: metadata.namespace.clone().unwrap_or_default(),
            uid: metadata.uid.clone().unwrap_or_default(),
            version: metadata.resource_version.clone().unwrap_or_default(),
            created: metadata.creation_timestamp.as_ref().map(|time| time.0),
            facts,
            bytes: 0,
        };
        result.bytes = size_of::<Self>()
            + extra_bytes
            + result.name.capacity()
            + result.namespace.capacity()
            + result.uid.capacity()
            + result.version.capacity();
        result
    }

    pub(super) fn validate(&self, source: Source) -> Result<(), ObservationFailure> {
        if self.name.is_empty()
            || self.uid.is_empty()
            || self.version.is_empty()
            || (source.namespaced() && self.namespace.is_empty())
        {
            return Err(ObservationFailure::InvalidObject);
        }
        if source == Source::Pods && self.bytes > 4 * 1024 {
            return Err(ObservationFailure::Capacity);
        }
        Ok(())
    }
}

impl Lookup for RetainedObject {
    type DynamicType = Source;
    fn kind(source: &Source) -> Cow<'_, str> {
        source.kind().into()
    }
    fn group(source: &Source) -> Cow<'_, str> {
        source.group().into()
    }
    fn version(_: &Source) -> Cow<'_, str> {
        "v1".into()
    }
    fn plural(source: &Source) -> Cow<'_, str> {
        source.resource().into()
    }
    fn name(&self) -> Option<Cow<'_, str>> {
        Some(self.name.as_str().into())
    }
    fn namespace(&self) -> Option<Cow<'_, str>> {
        (!self.namespace.is_empty()).then(|| self.namespace.as_str().into())
    }
    fn resource_version(&self) -> Option<Cow<'_, str>> {
        Some(self.version.as_str().into())
    }
    fn uid(&self) -> Option<Cow<'_, str>> {
        Some(self.uid.as_str().into())
    }
}

/// Typed input shared by Kubernetes watchers and the offline fixture feed.
/// Implementations retain only the evidence required by the summary.
pub trait SummaryResource: Resource<DynamicType = ()> {
    const SOURCE: Source;
    fn retain(&self) -> RetainedObject;
}

impl SummaryResource for Pod {
    const SOURCE: Source = Source::Pods;
    fn retain(&self) -> RetainedObject {
        let (restarts, issue) = workloads::analyze_pod(self);
        let facts = PodFacts {
            node: self.spec.as_ref().and_then(|spec| spec.node_name.clone()),
            phase: self
                .status
                .as_ref()
                .and_then(|status| status.phase.clone())
                .unwrap_or_else(|| "Unknown".into()),
            restarts,
            issue,
            requests: super::requests::of(self),
        };
        let bytes = facts.node.as_ref().map_or(0, String::capacity)
            + facts.phase.capacity()
            + match &facts.issue {
                Some(
                    PodIssue::Unknown(reason)
                    | PodIssue::FailingReason(reason)
                    | PodIssue::WaitingReason(reason),
                ) => reason.capacity(),
                _ => 0,
            };
        RetainedObject::new(self, Facts::Pod(facts), bytes)
    }
}

impl SummaryResource for Node {
    const SOURCE: Source = Source::Nodes;
    fn retain(&self) -> RetainedObject {
        let node = super::summarize::summarize_nodes(vec![self.clone()])
            .pop()
            .expect("one node");
        let bytes = size_of::<NodeSummary>()
            + node.uid.capacity()
            + node.name.capacity()
            + node.kubelet_version.capacity()
            + node.conditions.capacity() * size_of::<super::NodeCondition>()
            + node
                .conditions
                .iter()
                .map(|c| {
                    c.kind.capacity()
                        + c.status.capacity()
                        + c.reason.capacity()
                        + c.message.capacity()
                })
                .sum::<usize>()
            + node.roles.capacity() * size_of::<String>()
            + node.roles.iter().map(String::capacity).sum::<usize>()
            + node.addresses.capacity() * size_of::<(String, String)>()
            + node
                .addresses
                .iter()
                .map(|(k, v)| k.capacity() + v.capacity())
                .sum::<usize>()
            + node.taints.capacity() * size_of::<String>()
            + node.taints.iter().map(String::capacity).sum::<usize>()
            + node
                .capacity
                .iter()
                .chain(node.allocatable.iter())
                .map(|(k, v)| {
                    size_of::<(
                        String,
                        k8s_openapi::apimachinery::pkg::api::resource::Quantity,
                    )>() + k.capacity()
                        + v.0.capacity()
                        + 4 * size_of::<usize>()
                })
                .sum::<usize>();
        RetainedObject::new(self, Facts::Node(Box::new(node)), bytes)
    }
}

fn workload<K: Resource>(object: &K, snapshot: workloads::WorkloadSnapshot) -> RetainedObject {
    let info = snapshot
        .namespaces
        .into_iter()
        .next()
        .expect("one namespace")
        .workloads
        .into_iter()
        .next()
        .expect("one workload");
    let bytes = size_of::<WorkloadInfo>()
        + info.name.capacity()
        + info.namespace.capacity()
        + info.issues.capacity() * size_of::<String>()
        + info.issues.iter().map(String::capacity).sum::<usize>();
    RetainedObject::new(object, Facts::Workload(Box::new(info)), bytes)
}

impl SummaryResource for Deployment {
    const SOURCE: Source = Source::Deployments;
    fn retain(&self) -> RetainedObject {
        workload(
            self,
            workloads::build_snapshot(String::new(), vec![self.clone()], vec![], vec![], vec![]),
        )
    }
}
impl SummaryResource for DaemonSet {
    const SOURCE: Source = Source::DaemonSets;
    fn retain(&self) -> RetainedObject {
        workload(
            self,
            workloads::build_snapshot(String::new(), vec![], vec![], vec![self.clone()], vec![]),
        )
    }
}
impl SummaryResource for StatefulSet {
    const SOURCE: Source = Source::StatefulSets;
    fn retain(&self) -> RetainedObject {
        workload(
            self,
            workloads::build_snapshot(String::new(), vec![], vec![self.clone()], vec![], vec![]),
        )
    }
}
impl SummaryResource for PersistentVolumeClaim {
    const SOURCE: Source = Source::Claims;
    fn retain(&self) -> RetainedObject {
        let phase = self
            .status
            .as_ref()
            .and_then(|status| status.phase.clone())
            .unwrap_or_default();
        let bytes = phase.capacity();
        RetainedObject::new(self, Facts::Claim(phase), bytes)
    }
}
impl SummaryResource for PersistentVolume {
    const SOURCE: Source = Source::Volumes;
    fn retain(&self) -> RetainedObject {
        RetainedObject::new(
            self,
            Facts::Volume(
                self.status
                    .as_ref()
                    .and_then(|status| status.phase.as_deref())
                    == Some("Available"),
            ),
            0,
        )
    }
}
impl SummaryResource for Namespace {
    const SOURCE: Source = Source::Namespaces;
    fn retain(&self) -> RetainedObject {
        RetainedObject::new(self, Facts::Namespace, 0)
    }
}
impl SummaryResource for Event {
    const SOURCE: Source = Source::Events;
    fn retain(&self) -> RetainedObject {
        let event = Event {
            involved_object: k8s_openapi::api::core::v1::ObjectReference {
                namespace: self.involved_object.namespace.clone(),
                name: self.involved_object.name.clone(),
                kind: self.involved_object.kind.clone(),
                uid: self.involved_object.uid.clone(),
                ..Default::default()
            },
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                creation_timestamp: self.metadata.creation_timestamp.clone(),
                ..Default::default()
            },
            type_: self.type_.clone(),
            reason: self.reason.clone(),
            message: self.message.clone(),
            series: self.series.clone(),
            last_timestamp: self.last_timestamp.clone(),
            event_time: self.event_time.clone(),
            ..Default::default()
        };
        let bytes = size_of::<Event>()
            + [
                &event.involved_object.namespace,
                &event.involved_object.name,
                &event.involved_object.kind,
                &event.involved_object.uid,
                &event.type_,
                &event.reason,
                &event.message,
            ]
            .into_iter()
            .map(|value| value.as_ref().map_or(0, String::capacity))
            .sum::<usize>();
        RetainedObject::new(self, Facts::Event(Box::new(event)), bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::workloads::{HealthState, WorkloadKind};
    use serde::de::DeserializeOwned;
    use serde_json::Value;

    fn retained_workload<K: SummaryResource + DeserializeOwned>(object: Value) -> WorkloadInfo {
        let resource: K = serde_json::from_value(object).unwrap();
        let Facts::Workload(info) = resource.retain().facts else {
            panic!("expected workload facts");
        };
        *info
    }

    fn assert_workload<K: SummaryResource + DeserializeOwned>(
        kind: WorkloadKind,
        status: Value,
        desired: i32,
        ready: i32,
        health: HealthState,
        issues: &[&str],
    ) {
        assert_eq!(
            retained_workload::<K>(json!({
                "metadata": {"name": "app", "namespace": "example"},
                "spec": {"replicas": 9, "selector": {}, "template": {}, "serviceName": "app"},
                "status": status,
            })),
            WorkloadInfo {
                name: "app".into(),
                namespace: "example".into(),
                kind,
                ready,
                desired,
                health,
                issues: issues.iter().map(|issue| (*issue).into()).collect(),
            }
        );
    }

    fn assert_workload_defaults<K: SummaryResource + DeserializeOwned>(kind: WorkloadKind) {
        assert_eq!(
            retained_workload::<K>(json!({})),
            WorkloadInfo {
                name: "unknown".into(),
                namespace: "default".into(),
                kind,
                ready: 0,
                desired: 0,
                health: HealthState::Healthy,
                issues: Vec::new(),
            }
        );
    }

    #[test]
    fn deployment_conversion_preserves_replica_health_and_defaults() {
        let kind = WorkloadKind::Deployment;
        for (status, desired, ready, health, issues) in [
            (Value::Null, 0, 0, HealthState::Healthy, vec![]),
            (json!({}), 0, 0, HealthState::Healthy, vec![]),
            (
                json!({"replicas": 0, "readyReplicas": 0}),
                0,
                0,
                HealthState::Healthy,
                vec![],
            ),
            (
                json!({"replicas": 3, "readyReplicas": 3, "availableReplicas": 3}),
                3,
                3,
                HealthState::Healthy,
                vec![],
            ),
            (
                json!({"replicas": 3}),
                3,
                0,
                HealthState::Failing,
                vec!["No pods ready (0/3)"],
            ),
            (
                json!({"replicas": 3, "readyReplicas": 2, "availableReplicas": 2}),
                3,
                2,
                HealthState::Degraded,
                vec!["Partial: 2/3 ready"],
            ),
            (
                json!({"replicas": 3, "readyReplicas": 3}),
                3,
                3,
                HealthState::Healthy,
                vec![],
            ),
            (
                json!({"replicas": 3, "readyReplicas": 4, "availableReplicas": 2}),
                3,
                4,
                HealthState::Healthy,
                vec![],
            ),
        ] {
            assert_workload::<Deployment>(kind, status, desired, ready, health, &issues);
        }
        assert_workload_defaults::<Deployment>(kind);
    }

    #[test]
    fn statefulset_conversion_preserves_replica_health_and_defaults() {
        let kind = WorkloadKind::StatefulSet;
        for (status, desired, ready, health, issues) in [
            (Value::Null, 0, 0, HealthState::Healthy, vec![]),
            (json!({"replicas": 0}), 0, 0, HealthState::Healthy, vec![]),
            (
                json!({"replicas": 3, "readyReplicas": 3}),
                3,
                3,
                HealthState::Healthy,
                vec![],
            ),
            (
                json!({"replicas": 3}),
                3,
                0,
                HealthState::Failing,
                vec!["No pods ready (0/3)"],
            ),
            (
                json!({"replicas": 3, "readyReplicas": 2}),
                3,
                2,
                HealthState::Degraded,
                vec!["Partial: 2/3 ready"],
            ),
            (
                json!({"replicas": 3, "readyReplicas": 4}),
                3,
                4,
                HealthState::Healthy,
                vec![],
            ),
        ] {
            assert_workload::<StatefulSet>(kind, status, desired, ready, health, &issues);
        }
        assert_workload_defaults::<StatefulSet>(kind);
    }

    #[test]
    fn daemonset_conversion_preserves_replica_health_and_defaults() {
        let kind = WorkloadKind::DaemonSet;
        for (status, desired, ready, health, issues) in [
            (Value::Null, 0, 0, HealthState::Healthy, vec![]),
            (
                json!({"desiredNumberScheduled": 0, "numberReady": 0, "currentNumberScheduled": 0, "numberMisscheduled": 0}),
                0,
                0,
                HealthState::Healthy,
                vec![],
            ),
            (
                json!({"desiredNumberScheduled": 3, "numberReady": 3, "currentNumberScheduled": 3, "numberMisscheduled": 0}),
                3,
                3,
                HealthState::Healthy,
                vec![],
            ),
            (
                json!({"desiredNumberScheduled": 3, "numberReady": 0, "currentNumberScheduled": 3, "numberMisscheduled": 0}),
                3,
                0,
                HealthState::Failing,
                vec!["No pods ready (0/3)"],
            ),
            (
                json!({"desiredNumberScheduled": 3, "numberReady": 2, "currentNumberScheduled": 3, "numberMisscheduled": 0}),
                3,
                2,
                HealthState::Degraded,
                vec!["Partial: 2/3 ready"],
            ),
            (
                json!({"desiredNumberScheduled": 3, "numberReady": 4, "currentNumberScheduled": 3, "numberMisscheduled": 0}),
                3,
                4,
                HealthState::Healthy,
                vec![],
            ),
        ] {
            assert_workload::<DaemonSet>(kind, status, desired, ready, health, &issues);
        }
        assert_workload_defaults::<DaemonSet>(kind);
    }

    #[test]
    fn diagnosed_reason_strings_are_counted_against_the_pod_byte_limit() {
        let reason = "example".repeat(1024);
        for status in [
            json!({"phase":"Failed","reason":reason}),
            json!({"phase":"Pending","initContainerStatuses":[{
                "name":"migrate","image":"example","imageID":"example",
                "ready":false,"restartCount":0,"state":{"waiting":{"reason":reason}}
            }]}),
        ] {
            let pod: Pod = serde_json::from_value(json!({
                "metadata":{"name":"app","namespace":"example","uid":"app","resourceVersion":"1"},
                "status":status
            }))
            .unwrap();
            let retained = pod.retain();
            assert!(retained.bytes >= reason.len());
            assert_eq!(
                retained.validate(Source::Pods),
                Err(ObservationFailure::Capacity)
            );
        }
    }
}
