//! A scripted investigation for the offline `faux` provider: the shape of a
//! real run (thinking, tool calls, a final answer) with no model and no key.

use std::sync::Arc;

use rpi_ai::providers::faux::{
    FauxBlock, FauxProvider, FauxScript, FauxStep, faux_assistant_message,
};
use rpi_ai::types::StopReason;
use serde_json::json;

fn step(thinking: &str, tool: &str, args: serde_json::Value) -> FauxStep {
    FauxStep::message(faux_assistant_message(
        vec![
            FauxBlock::thinking(thinking),
            FauxBlock::tool_call(tool, args),
        ],
        StopReason::ToolUse,
    ))
}

pub fn steps() -> Vec<FauxStep> {
    vec![
        step(
            "Start from the workload's own status before reading any logs.",
            "get_object",
            json!({"kind": "Deployment", "namespace": "shop", "name": "checkout"}),
        ),
        step(
            "Revision 14 is not becoming ready. Find the new ReplicaSet's pod.",
            "list_objects",
            json!({"kind": "Pod", "namespace": "shop"}),
        ),
        step(
            "The new pod is crash looping. The previous instance shows why it exited.",
            "pod_logs",
            json!({"namespace": "shop", "pod": "checkout-7c9d8f6b4-q8zxk", "previous": true}),
        ),
        step(
            "Password authentication failed. The Reloader annotation says the rollout came from the checkout-db Secret.",
            "get_object",
            json!({"kind": "Secret", "namespace": "shop", "name": "checkout-db"}),
        ),
        step(
            "Rotated 11 minutes ago by external-secrets. Which Secret does the database role use?",
            "get_object",
            json!({"kind": "Cluster", "namespace": "shop", "name": "postgres"}),
        ),
        FauxStep::text(
            "Cause: the database password was rotated for the app but not for the database.\n\n\
             Evidence:\n\
             - ExternalSecret shop/checkout-db synced a new password into Secret shop/checkout-db 11m ago.\n\
             - Reloader rolled Deployment shop/checkout to revision 14; its pod crashes with \
             `password authentication failed for user \"checkout\"`.\n\
             - Cluster shop/postgres sets role checkout's password from Secret shop/checkout-db-role, unchanged for 41d.\n\
             - The revision 13 pods still work on connections opened before the rotation.\n\n\
             Suggested fix: point the role's passwordSecret at the rotated value (or rotate checkout-db-role from the \
             same Vault key), then let the rollout continue. Restarting the old pods now would take checkout down.",
        ),
    ]
}

/// `tokens_per_second` paces the stream like a model would; `None` is instant.
pub fn provider(tokens_per_second: Option<f64>) -> Arc<FauxProvider> {
    let mut script = FauxScript::new();
    if let Some(tps) = tokens_per_second {
        script = script.with_tokens_per_second(tps);
    }
    script.append_responses(steps());
    FauxProvider::new(script)
}
