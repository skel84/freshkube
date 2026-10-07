//! Pure projections of Kubernetes evidence.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::{Event, Node};

use super::{EventSummary, NodeCondition, NodeSummary, Warning};

pub(super) fn summarize_nodes(nodes: Vec<Node>) -> Vec<NodeSummary> {
    nodes
        .into_iter()
        .map(|node| {
            let status = node.status.unwrap_or_default();
            let spec = node.spec.unwrap_or_default();
            let name = node.metadata.name.unwrap_or_default();
            NodeSummary {
                uid: node.metadata.uid.clone().unwrap_or_default(),
                pods: 0,
                requests: super::requests::ZERO,
                pods_current: true,
                pods_observed: true,
                name,
                conditions: status
                    .conditions
                    .unwrap_or_default()
                    .into_iter()
                    .map(|condition| NodeCondition {
                        kind: condition.type_,
                        status: condition.status,
                        reason: condition.reason.unwrap_or_default(),
                        message: condition.message.unwrap_or_default(),
                        since: condition.last_transition_time.map(|time| time.0),
                    })
                    .collect(),
                unschedulable: spec.unschedulable.unwrap_or(false),
                roles: node
                    .metadata
                    .labels
                    .unwrap_or_default()
                    .keys()
                    .filter_map(|key| {
                        key.strip_prefix("node-role.kubernetes.io/")
                            .map(str::to_owned)
                    })
                    .collect(),
                addresses: status
                    .addresses
                    .unwrap_or_default()
                    .into_iter()
                    .map(|address| (address.type_, address.address))
                    .collect(),
                kubelet_version: status
                    .node_info
                    .map(|info| info.kubelet_version)
                    .unwrap_or_default(),
                capacity: status.capacity.unwrap_or_default(),
                allocatable: status.allocatable.unwrap_or_default(),
                taints: spec
                    .taints
                    .unwrap_or_default()
                    .into_iter()
                    .map(|taint| format!("{}:{}", taint.key, taint.effect))
                    .collect(),
            }
        })
        .collect()
}

pub(super) fn summarize_events(events: Vec<Event>, now: DateTime<Utc>) -> EventSummary {
    let mut summary = EventSummary::default();
    let mut newest = BTreeMap::new();
    for event in events {
        if event.type_.as_deref() != Some("Warning") {
            continue;
        }
        let at = event
            .series
            .as_ref()
            .and_then(|series| series.last_observed_time.as_ref().map(|time| time.0))
            .or_else(|| event.last_timestamp.as_ref().map(|time| time.0))
            .or_else(|| event.event_time.as_ref().map(|time| time.0))
            .or_else(|| {
                event
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .map(|time| time.0)
            });
        let Some(at) = at.filter(|at| *at >= now - chrono::Duration::hours(1) && *at <= now) else {
            continue;
        };
        summary.total += 1;
        let warning = Warning {
            namespace: event.involved_object.namespace.unwrap_or_default(),
            kind: event.involved_object.kind.unwrap_or_default(),
            name: event.involved_object.name.unwrap_or_default(),
            reason: event.reason.unwrap_or_default(),
            message: event.message.unwrap_or_default(),
            at,
        };
        *summary.reasons.entry(warning.reason.clone()).or_default() += 1;
        let key = (
            warning.namespace.clone(),
            warning.kind.clone(),
            warning.name.clone(),
        );
        if newest
            .get(&key)
            .is_none_or(|previous: &Warning| previous.at < at)
        {
            newest.insert(key, warning);
        }
    }
    summary.newest = newest.into_values().collect();
    summary
        .newest
        .sort_by_key(|warning| std::cmp::Reverse(warning.at));
    summary
}
