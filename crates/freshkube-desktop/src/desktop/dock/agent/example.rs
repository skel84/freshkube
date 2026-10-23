//! The fixture's agent: the scripted one, told to read the example cluster's
//! `api` Deployment, one of its pods, and the events around them, through
//! Freshkube's four tools.

use std::time::Duration;

use freshkube_agent::scripted::{Script, TourRead};
use freshkube_resources::example;
use serde_json::json;

use crate::agent_shape::AgentShape;

/// What the example question asks.
pub(crate) const QUESTION: &str = "Why is the api Deployment restarting?";

fn read(tool: &str, arguments: serde_json::Value) -> TourRead {
    TourRead {
        tool: tool.into(),
        arguments,
    }
}

/// The scripted agent for `context`'s example objects and `shape`.
pub(crate) fn script(context: &str, other_entry: Option<&str>, shape: AgentShape) -> Script {
    let now = chrono::Utc::now().timestamp();
    let (namespace, deployment) = example::read(context, "deployments.apps", None, now)
        .and_then(|(_, rows)| {
            let row = rows
                .iter()
                .find(|row| row.identity.name == "api")
                .or(rows.first())?;
            Some((row.identity.namespace.clone(), row.identity.name.clone()))
        })
        .unwrap_or_else(|| ("payments".into(), "api".into()));
    let pod = example::read(context, "pods", Some(&namespace), now)
        .and_then(|(_, rows)| {
            rows.iter()
                .find(|row| {
                    row.identity.namespace == namespace
                        && row.identity.name.starts_with(&deployment)
                })
                .or(rows.first())
                .map(|row| row.identity.name.clone())
        })
        .unwrap_or_else(|| format!("{deployment}-0"));
    let mut tour = vec![
        read(
            "list_objects",
            json!({"kind": "pods", "namespace": namespace}),
        ),
        read(
            "get_object",
            json!({"kind": "deployments.apps", "namespace": namespace, "name": deployment}),
        ),
        read("pod_logs", json!({"namespace": namespace, "pod": pod})),
        read(
            "list_events",
            json!({"namespace": namespace, "since_seconds": 86400}),
        ),
    ];
    if shape == AgentShape::Refused {
        // A kind the example cluster doesn't serve, and a read on another
        // cluster, which needs access of its own.
        tour.insert(
            1,
            read("list_objects", json!({"kind": "widgets.example.io"})),
        );
        tour.push(read(
            "list_objects",
            json!({
                "cluster": other_entry.unwrap_or("stage-fra"),
                "kind": "pods",
                "namespace": namespace,
            }),
        ));
    }
    Script {
        tour,
        // A shell is always asked for last, and always refused.
        tour_shell: matches!(shape, AgentShape::Refused | AgentShape::Answered),
        tour_pause: match shape {
            // A step that stays running.
            AgentShape::Running | AgentShape::Stopped => Duration::from_secs(3600),
            _ => Duration::ZERO,
        },
        ..Script::default()
    }
}
