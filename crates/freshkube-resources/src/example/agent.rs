//! The example cluster as the agent's reads see it (`ExampleObjects`): the
//! same rows, documents, log lines and events the Resources page shows,
//! written as a server would return them. Core applies its bounds, its Secret
//! rule and its places to these as to a live answer.

use std::sync::Arc;

use freshkube_core::agent_reads::{ExampleObjects, example_objects};
use freshkube_core::resources::{PodLogUpdate, ResourceKind};
use serde_json::{Value, json};

use super::{document, events, pod_log, read};
use crate::live;

/// Kinds the example cluster lists whose rows have no document of their own
/// are still listed, with just their metadata.
struct Example {
    context: String,
}

/// The example cluster `context` for an agent session's `Targets`.
pub fn agent_objects(context: &str) -> Arc<dyn ExampleObjects> {
    example_objects(Example {
        context: context.to_owned(),
    })
}

impl Example {
    fn rows(&self, key: &str, namespace: Option<&str>) -> Vec<crate::model::ResourceRow> {
        let Some((_, rows)) = read(&self.context, key, None, live::now()) else {
            return Vec::new();
        };
        rows.into_iter()
            .filter(|row| namespace.is_none_or(|namespace| row.identity.namespace == namespace))
            .collect()
    }
}

impl ExampleObjects for Example {
    fn objects(&self, kind: &ResourceKind, namespace: Option<&str>) -> Vec<Value> {
        let now = live::now();
        self.rows(&kind.key(), namespace)
            .into_iter()
            .map(|row| {
                document(&row.identity, now)
                    .and_then(|doc| serde_yaml::from_str::<Value>(&doc.yaml).ok())
                    .unwrap_or_else(|| {
                        json!({
                            "apiVersion": kind.api_version(),
                            "kind": kind.kind,
                            "metadata": {
                                "name": row.identity.name,
                                "namespace": row.identity.namespace,
                                "uid": row.identity.uid,
                            },
                        })
                    })
            })
            .collect()
    }

    fn log_lines(
        &self,
        namespace: &str,
        pod: &str,
        container: Option<&str>,
        previous: bool,
    ) -> Option<Vec<String>> {
        let now = live::now();
        let identity = self
            .rows("pods", Some(namespace))
            .into_iter()
            .find(|row| row.identity.name == pod)?
            .identity;
        let container = match container {
            Some(container) => container.to_owned(),
            None => document(&identity, now)?.overview.pod?.default?,
        };
        let (updates, _) = pod_log(&identity, &container, previous, now);
        let mut lines = Vec::new();
        for update in updates {
            match update {
                PodLogUpdate::Line(line) => lines.push(line),
                PodLogUpdate::Failed(_) => return None,
                _ => {}
            }
        }
        Some(lines)
    }

    fn events(&self, namespace: Option<&str>) -> Vec<Value> {
        let now = live::now();
        let mut out = Vec::new();
        for (key, kind) in [("pods", "Pod"), ("deployments.apps", "Deployment")] {
            for row in self.rows(key, namespace) {
                for event in events(&row.identity, now) {
                    let seen = event.last_seen.map(|at| at.to_rfc3339());
                    out.push(json!({
                        "metadata": {
                            "name": event.uid,
                            "namespace": row.identity.namespace,
                        },
                        "type": event.event_type,
                        "reason": event.reason,
                        "message": event.message,
                        "count": event.count,
                        "lastTimestamp": seen,
                        "involvedObject": {
                            "kind": kind,
                            "name": row.identity.name,
                            "namespace": row.identity.namespace,
                            "uid": row.identity.uid,
                        },
                    }));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
