//! The four tools as MCP lists them: the design's names, arguments and
//! bounds. The names are core's `TOOLS`; a test keeps the two in step.

use rmcp::model::Tool;
use serde_json::{Map, Value, json};

use freshkube_core::agent_reads::TOOLS;

fn schema(properties: Value, required: &[&str]) -> Map<String, Value> {
    let Value::Object(schema) = json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    }) else {
        unreachable!("a schema is an object")
    };
    schema
}

/// The common `cluster` argument.
fn cluster() -> Value {
    json!({"type": "string", "maxLength": 64,
        "description": "A cluster entry's id. Defaults to the session's own cluster."})
}

fn tool(name: &'static str, description: &'static str, input: Map<String, Value>) -> Tool {
    // Some MCP clients defer a server's tools behind a search tool of their
    // own, which a session without built-in tools lacks.
    let mut meta = Map::new();
    meta.insert("anthropic/alwaysLoad".into(), Value::Bool(true));
    Tool::new(name, description, input).with_meta(meta.into())
}

pub(super) fn all() -> Vec<Tool> {
    vec![
        tool(
            TOOLS[0],
            "List objects of one kind with a one-line status each. Read-only, with your own access.",
            schema(
                json!({
                    "cluster": cluster(),
                    "kind": {"type": "string", "maxLength": 253,
                        "description": "What kubectl takes: pods, Deployment, deploy.apps, certificates.cert-manager.io."},
                    "namespace": {"type": "string", "maxLength": 63, "description": "Omit to list every namespace."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 100},
                }),
                &["kind"],
            ),
        ),
        tool(
            TOOLS[1],
            "Read one object's spec and status. A Secret is only its metadata: values and key names are never read.",
            schema(
                json!({
                    "cluster": cluster(),
                    "kind": {"type": "string", "maxLength": 253},
                    "namespace": {"type": "string", "maxLength": 63, "description": "Required for a namespaced kind."},
                    "name": {"type": "string", "maxLength": 253},
                }),
                &["kind", "name"],
            ),
        ),
        tool(
            TOOLS[2],
            "Read a pod container's recent log lines. Set previous to read the instance before the last restart.",
            schema(
                json!({
                    "cluster": cluster(),
                    "namespace": {"type": "string", "maxLength": 63},
                    "pod": {"type": "string", "maxLength": 253},
                    "container": {"type": "string", "maxLength": 63, "description": "Defaults to the pod's default container."},
                    "previous": {"type": "boolean", "default": false},
                    "tail_lines": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200},
                    "since_seconds": {"type": "integer", "minimum": 1, "maximum": 86400},
                }),
                &["namespace", "pod"],
            ),
        ),
        tool(
            TOOLS[3],
            "List recent events, newest first, optionally for one object.",
            schema(
                json!({
                    "cluster": cluster(),
                    "namespace": {"type": "string", "maxLength": 63},
                    "kind": {"type": "string", "maxLength": 253},
                    "name": {"type": "string", "maxLength": 253},
                    "since_seconds": {"type": "integer", "minimum": 1, "maximum": 86400, "default": 3600},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 100},
                }),
                &[],
            ),
        ),
    ]
}
