//! Which tool calls the agent may make: Freshkube's four reads, and nothing
//! else. Everything else is refused, with a note for the transcript.
//!
//! This is the second line. The first is the profile's lock-down, which
//! leaves the agent no other tool to ask for. A permission request may name
//! only a call's id (opencode's and Codex's do), so the decision rests on
//! what the call's own notifications said, as well as on what the request
//! says.
//!
//! A tool is ours only when its identity is an exact match of server
//! `freshkube` and one of the four tool names, in one of the forms agents use:
//!
//! - `freshkube_get_object`: opencode's name for an MCP tool;
//! - `mcp__freshkube__get_object`: Claude Code's;
//! - `mcp.freshkube.get_object`, or an input of `{server, tool}`: Codex's.
//!
//! A name that merely contains ours, such as `freshkube_get_object_extra` or
//! `other_freshkube_get_object`, is not ours. The identity is the agent's own
//! word; what keeps a shell from calling itself `freshkube_get_object` is
//! that the lock-down leaves none to call.

use std::collections::HashMap;
use std::sync::Mutex;

use agent_client_protocol::schema::v1::{
    PermissionOptionKind, RequestPermissionOutcome, RequestPermissionRequest,
    SelectedPermissionOutcome, SessionUpdate,
};
use serde_json::Value;

use crate::investigation::clip;
use crate::tools::{SERVER, TOOLS};

/// Calls remembered; a call past this is unknown, and unknown is refused.
const MAX_CALLS: usize = 4096;
const MAX_LABEL_BYTES: usize = 200;

/// Whether `identity` names one of our tools, exactly.
pub(super) fn is_ours(identity: &str) -> bool {
    [
        format!("{SERVER}_"),
        format!("mcp__{SERVER}__"),
        format!("mcp.{SERVER}."),
    ]
    .iter()
    .any(|prefix| {
        identity
            .strip_prefix(prefix.as_str())
            .is_some_and(|tool| TOOLS.contains(&tool))
    })
}

/// What a call says it is: its input's server and tool, else its name, else
/// its title. `None` when it says nothing.
fn identify(name: Option<&str>, title: Option<&str>, input: Option<&Value>) -> Option<bool> {
    if let Some(input) = input {
        let field = |key: &str| input.get(key).and_then(Value::as_str);
        if let (Some(server), Some(tool)) = (field("server"), field("tool")) {
            return Some(server == SERVER && TOOLS.contains(&tool));
        }
    }
    name.filter(|n| !n.is_empty())
        .or(title.filter(|t| !t.is_empty()))
        .map(is_ours)
}

#[derive(Default)]
pub(super) struct Calls(Mutex<HashMap<String, Known>>);

struct Known {
    label: String,
    ours: Option<bool>,
}

impl Calls {
    /// Remembers what a tool-call notification said about its call. The
    /// latest word wins, so a call that turns out to be something else is
    /// not ours.
    pub(super) fn note(&self, update: &SessionUpdate) {
        let (id, title, name, input) = match update {
            SessionUpdate::ToolCall(c) => (
                c.tool_call_id.to_string(),
                Some(c.title.as_str()),
                c.name.as_deref(),
                c.raw_input.as_ref(),
            ),
            SessionUpdate::ToolCallUpdate(u) => (
                u.tool_call_id.to_string(),
                u.fields.title.as_deref(),
                u.fields.name.as_deref(),
                u.fields.raw_input.as_ref(),
            ),
            _ => return,
        };
        let mut calls = self.0.lock().unwrap();
        if calls.len() >= MAX_CALLS && !calls.contains_key(&id) {
            return;
        }
        let call = calls.entry(id.clone()).or_insert_with(|| Known {
            label: clip(&id, MAX_LABEL_BYTES).to_string(),
            ours: None,
        });
        if let Some(title) = title.filter(|t| !t.is_empty()) {
            call.label = clip(title, MAX_LABEL_BYTES).to_string();
        }
        if let Some(ours) = identify(name, title, input) {
            call.ours = Some(ours);
        }
    }

    fn get(&self, id: &str) -> Option<(String, Option<bool>)> {
        let calls = self.0.lock().unwrap();
        calls.get(id).map(|c| (c.label.clone(), c.ours))
    }
}

/// Allows one of our tools once; refuses anything else. The second part is
/// what to tell the person when it refused.
pub(super) fn decide(
    request: &RequestPermissionRequest,
    calls: &Calls,
) -> (RequestPermissionOutcome, Option<String>) {
    let id = request.tool_call.tool_call_id.to_string();
    let fields = &request.tool_call.fields;
    let known = calls.get(&id);
    // Every word on record must say ours, and there must be one.
    let words = [
        known.as_ref().and_then(|(_, ours)| *ours),
        identify(
            fields.name.as_deref(),
            fields.title.as_deref(),
            fields.raw_input.as_ref(),
        ),
    ];
    let ours = words.iter().flatten().count() > 0 && words.iter().flatten().all(|ours| *ours);
    // What the call announced comes first: a refusal names the call, not
    // whatever the request claims it is.
    let label = known
        .map(|(label, _)| label)
        .or_else(|| fields.title.clone())
        .or_else(|| fields.name.clone())
        .unwrap_or(id);
    let wanted = if ours {
        [
            PermissionOptionKind::AllowOnce,
            PermissionOptionKind::AllowAlways,
        ]
    } else {
        [
            PermissionOptionKind::RejectOnce,
            PermissionOptionKind::RejectAlways,
        ]
    };
    let option = wanted
        .iter()
        .find_map(|kind| request.options.iter().find(|o| o.kind == *kind));
    let outcome = match option {
        Some(option) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
            option.option_id.clone(),
        )),
        None => RequestPermissionOutcome::Cancelled,
    };
    (
        outcome,
        (!ours).then(|| clip(&label, MAX_LABEL_BYTES).to_string()),
    )
}

#[cfg(test)]
mod tests {
    use agent_client_protocol::schema::v1::{
        PermissionOption, ToolCall, ToolCallUpdate, ToolCallUpdateFields,
    };
    use serde_json::json;

    use super::*;

    fn options() -> Vec<PermissionOption> {
        vec![
            PermissionOption::new("once", "Allow", PermissionOptionKind::AllowOnce),
            PermissionOption::new("always", "Always", PermissionOptionKind::AllowAlways),
            PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
        ]
    }

    fn ask(id: &str, fields: ToolCallUpdateFields) -> RequestPermissionRequest {
        RequestPermissionRequest::new("s1", ToolCallUpdate::new(id.to_string(), fields), options())
    }

    fn allowed(outcome: &RequestPermissionOutcome) -> bool {
        matches!(outcome, RequestPermissionOutcome::Selected(s) if s.option_id.to_string() == "once")
    }

    #[test]
    fn only_an_exact_match_of_our_server_and_tool_is_ours() {
        for ours in [
            "freshkube_list_objects",
            "freshkube_get_object",
            "freshkube_pod_logs",
            "freshkube_list_events",
            "mcp__freshkube__get_object",
            "mcp.freshkube.pod_logs",
        ] {
            assert!(is_ours(ours), "{ours}");
        }
        for other in [
            "freshkube_get_object_extra",
            "freshkube_get_objects",
            "xfreshkube_get_object",
            "other_freshkube_get_object",
            "other_get_object",
            "mcp__other__get_object",
            "mcp.other.get_object",
            "mcp__freshkube__get_object_extra",
            "freshkube_",
            "freshkube",
            "get_object",
            "shell",
            "Freshkube_get_object",
            "freshkube_get_object ",
            "",
        ] {
            assert!(!is_ours(other), "{other:?}");
        }
    }

    #[test]
    fn a_request_naming_only_an_id_follows_what_the_call_said() {
        let calls = Calls::default();
        calls.note(&SessionUpdate::ToolCall(
            ToolCall::new("t1", "freshkube_get_object").name("freshkube_get_object"),
        ));
        calls.note(&SessionUpdate::ToolCall(ToolCall::new(
            "t2",
            "kubectl delete pod x",
        )));
        let (outcome, refused) = decide(&ask("t1", ToolCallUpdateFields::new()), &calls);
        assert!(allowed(&outcome) && refused.is_none());
        let (outcome, refused) = decide(&ask("t2", ToolCallUpdateFields::new()), &calls);
        assert!(!allowed(&outcome));
        assert_eq!(refused.as_deref(), Some("kubectl delete pod x"));
        // An id nothing announced is refused, named by its id.
        let (outcome, refused) = decide(&ask("t9", ToolCallUpdateFields::new()), &calls);
        assert!(!allowed(&outcome));
        assert_eq!(refused.as_deref(), Some("t9"));
    }

    #[test]
    fn a_call_that_turns_out_to_be_something_else_is_not_ours() {
        let calls = Calls::default();
        calls.note(&SessionUpdate::ToolCall(ToolCall::new(
            "t1",
            "freshkube_get_object",
        )));
        calls.note(&SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "t1",
            ToolCallUpdateFields::new().title("shell"),
        )));
        let (outcome, _) = decide(&ask("t1", ToolCallUpdateFields::new()), &calls);
        assert!(!allowed(&outcome));
        // And a request cannot claim a name the call did not have.
        calls.note(&SessionUpdate::ToolCall(ToolCall::new("t2", "shell")));
        let claim = ToolCallUpdateFields::new().title("freshkube_get_object");
        let (outcome, _) = decide(&ask("t2", claim), &calls);
        assert!(!allowed(&outcome));
    }

    #[test]
    fn codex_names_the_server_in_the_input() {
        let calls = Calls::default();
        let call = ToolCall::new("t1", "Run tool")
            .raw_input(json!({"server": "freshkube", "tool": "get_object", "arguments": {}}));
        calls.note(&SessionUpdate::ToolCall(call));
        let (outcome, _) = decide(&ask("t1", ToolCallUpdateFields::new()), &calls);
        assert!(allowed(&outcome));
        let other = ToolCall::new("t2", "Run tool")
            .raw_input(json!({"server": "other", "tool": "get_object"}));
        calls.note(&SessionUpdate::ToolCall(other));
        let (outcome, _) = decide(&ask("t2", ToolCallUpdateFields::new()), &calls);
        assert!(!allowed(&outcome));
    }

    #[test]
    fn a_refusal_without_a_reject_option_cancels() {
        let calls = Calls::default();
        let mut request = ask("t1", ToolCallUpdateFields::new().title("shell"));
        request.options = vec![PermissionOption::new(
            "once",
            "Allow",
            PermissionOptionKind::AllowOnce,
        )];
        let (outcome, refused) = decide(&request, &calls);
        assert!(matches!(outcome, RequestPermissionOutcome::Cancelled));
        assert_eq!(refused.as_deref(), Some("shell"));
    }
}
