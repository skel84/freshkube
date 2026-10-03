//! Read-only cluster tools. Each one reports where the UI would go to show
//! what the agent is reading, which is what lets the window follow along.
//!
//! In the app these would call `freshkube-core` on the chosen connection; here
//! they read the canned [`Cluster`]. [`call`] is the tool logic; the embedded
//! loop wraps it as rpi tools ([`all`]) and the ACP backend serves it over MCP.

use std::sync::Arc;

use async_trait::async_trait;
use rpi_agent::{AgentError, AgentTool, AgentToolResult, ToolResultPartial};
use rpi_ai::types::{Schema, Tool};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::scenario::{Cluster, ObjectRef};

/// Where the window would show what a tool read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    Kind {
        kind: &'static str,
        namespace: String,
    },
    Object {
        object: ObjectRef,
        tab: Tab,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Logs { container: String, previous: bool },
    Events,
}

pub type Follow = Arc<dyn Fn(&str, Place) + Send + Sync>;

/// One tool's name, description and JSON Schema for its arguments.
pub struct Spec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

pub fn specs() -> Vec<Spec> {
    vec![
        Spec {
            name: "list_objects",
            description: "List objects of one kind in a namespace with a one-line status each.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "kind": {"type": "string", "description": "Kind, e.g. Pod, Deployment, Secret, ExternalSecret, Cluster (CloudNativePG)."},
                    "namespace": {"type": "string"}
                },
                "required": ["kind", "namespace"]
            }),
        },
        Spec {
            name: "get_object",
            description: "Read one object's spec and status. Secret values are never returned, only key names and metadata.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "kind": {"type": "string"},
                    "namespace": {"type": "string"},
                    "name": {"type": "string"}
                },
                "required": ["kind", "namespace", "name"]
            }),
        },
        Spec {
            name: "pod_logs",
            description: "Read a pod container's recent log lines. Set previous to read the instance before the last restart.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "namespace": {"type": "string"},
                    "pod": {"type": "string"},
                    "container": {"type": "string", "description": "Defaults to the pod's only container."},
                    "previous": {"type": "boolean"},
                    "tail": {"type": "integer", "minimum": 1, "maximum": 500}
                },
                "required": ["namespace", "pod"]
            }),
        },
        Spec {
            name: "list_events",
            description: "List recent events in a namespace, optionally for one object.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "namespace": {"type": "string"},
                    "kind": {"type": "string"},
                    "name": {"type": "string"}
                },
                "required": ["namespace"]
            }),
        },
    ]
}

/// Runs one tool: where the window would go, and the text the model reads.
pub fn call(cluster: &Cluster, tool: &str, params: &Value) -> Result<(Place, String), String> {
    match tool {
        "list_objects" => list_objects(cluster, params),
        "get_object" => get_object(cluster, params),
        "pod_logs" => pod_logs(cluster, params),
        "list_events" => list_events(cluster, params),
        other => Err(format!("unknown tool `{other}`")),
    }
}

/// The tools as rpi tools for the embedded loop.
pub fn all(cluster: Arc<Cluster>, follow: Follow) -> Vec<Arc<dyn AgentTool>> {
    specs()
        .into_iter()
        .map(|spec| {
            Arc::new(ClusterTool {
                schema: Tool {
                    name: spec.name.into(),
                    description: spec.description.into(),
                    parameters: Schema(spec.parameters),
                    constrained_sampling: None,
                },
                cluster: Arc::clone(&cluster),
                follow: Arc::clone(&follow),
            }) as Arc<dyn AgentTool>
        })
        .collect()
}

struct ClusterTool {
    schema: Tool,
    cluster: Arc<Cluster>,
    follow: Follow,
}

fn arg<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn required<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    arg(params, key).ok_or_else(|| format!("missing argument `{key}`"))
}

#[async_trait]
impl AgentTool for ClusterTool {
    fn schema(&self) -> &Tool {
        &self.schema
    }

    fn label(&self) -> &str {
        &self.schema.name
    }

    async fn execute(
        &self,
        tool_call_id: &str,
        params: Value,
        _signal: CancellationToken,
        _on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let (place, text) =
            call(&self.cluster, &self.schema.name, &params).map_err(AgentError::Tool)?;
        (self.follow)(tool_call_id, place);
        Ok(AgentToolResult::text(text))
    }
}

fn kind(asked: &str) -> Result<&'static str, String> {
    Cluster::canonical_kind(asked).ok_or_else(|| {
        format!(
            "unknown kind `{asked}`; known kinds: {}",
            Cluster::kinds().join(", ")
        )
    })
}

fn list_objects(cluster: &Cluster, params: &Value) -> Result<(Place, String), String> {
    let kind = kind(required(params, "kind")?)?;
    let namespace = required(params, "namespace")?;
    let rows: Vec<String> = cluster
        .objects
        .iter()
        .filter(|o| o.id.kind == kind && o.id.namespace == namespace)
        .map(|o| format!("{}  {}", o.id.name, o.status))
        .collect();
    let text = if rows.is_empty() {
        format!("No {kind} objects in {namespace}.")
    } else {
        rows.join("\n")
    };
    let place = Place::Kind {
        kind,
        namespace: namespace.into(),
    };
    Ok((place, text))
}

fn get_object(cluster: &Cluster, params: &Value) -> Result<(Place, String), String> {
    let kind = required(params, "kind")?;
    let namespace = required(params, "namespace")?;
    let name = required(params, "name")?;
    let object = cluster
        .find(kind, namespace, name)
        .ok_or_else(|| format!("{kind} {namespace}/{name} not found"))?;
    let mut text = format!(
        "{}\nstatus: {}\n{}",
        object.id, object.status, object.document
    );
    if let Some(keys) = object.secret_keys {
        text.push_str(&format!(
            "\ndata keys: {} (values withheld)",
            keys.join(", ")
        ));
    }
    let place = Place::Object {
        object: object.id.clone(),
        tab: Tab::Overview,
    };
    Ok((place, text))
}

fn pod_logs(cluster: &Cluster, params: &Value) -> Result<(Place, String), String> {
    let namespace = required(params, "namespace")?;
    let pod = required(params, "pod")?;
    let object = cluster
        .find("Pod", namespace, pod)
        .ok_or_else(|| format!("Pod {namespace}/{pod} not found"))?;
    let container = arg(params, "container");
    let log = cluster
        .logs
        .iter()
        .find(|l| l.pod == pod && container.is_none_or(|c| c == l.container))
        .ok_or_else(|| format!("no such container in {pod}"))?;
    let previous = params
        .get("previous")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let tail = params.get("tail").and_then(Value::as_u64).unwrap_or(100) as usize;
    let lines = if previous { log.previous } else { log.current };
    let text = if previous && lines.is_empty() {
        "No previous instance: the container has not restarted.".to_string()
    } else {
        lines[lines.len().saturating_sub(tail)..].join("\n")
    };
    let place = Place::Object {
        object: object.id.clone(),
        tab: Tab::Logs {
            container: log.container.into(),
            previous,
        },
    };
    Ok((place, text))
}

fn list_events(cluster: &Cluster, params: &Value) -> Result<(Place, String), String> {
    let namespace = required(params, "namespace")?;
    let kind = arg(params, "kind").map(kind).transpose()?;
    let name = arg(params, "name");
    let rows: Vec<String> = cluster
        .events
        .iter()
        .filter(|e| e.object.namespace == namespace)
        .filter(|e| kind.is_none_or(|k| e.object.kind == k))
        .filter(|e| name.is_none_or(|n| e.object.name == n))
        .map(|e| {
            format!(
                "{:<8} {:<18} {:<20} {}  {}",
                e.kind, e.reason, e.age, e.object, e.message
            )
        })
        .collect();
    let text = if rows.is_empty() {
        "No events.".to_string()
    } else {
        rows.join("\n")
    };
    let place = match (kind, name) {
        (Some(kind), Some(name)) => match cluster.find(kind, namespace, name) {
            Some(object) => Place::Object {
                object: object.id.clone(),
                tab: Tab::Events,
            },
            None => Place::Kind {
                kind: "Event",
                namespace: namespace.into(),
            },
        },
        _ => Place::Kind {
            kind: "Event",
            namespace: namespace.into(),
        },
    };
    Ok((place, text))
}
