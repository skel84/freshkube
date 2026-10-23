//! Which tool calls the agent may make: Freshkube's four reads, and nothing
//! else. Everything else is refused, with a note for the transcript.
//!
//! This is the second line. The first is the profile's lock-down, which
//! leaves the agent no other tool to ask for. A permission request may name
//! only a call's id (opencode's does), so the decision rests on what the
//! call's own notifications said, as well as on what the request says.
//!
//! A call is ours only by its **name**, an exact match of server `freshkube`
//! and one of the four tool names: `freshkube_get_object` (how opencode names
//! an MCP tool), or the `mcp__freshkube__get_object` and
//! `mcp.freshkube.get_object` forms other agents use. Nothing else decides:
//! not the title (a shell call's title is its command) and not the input
//! (it is the model's own text, and may say anything). A name that merely
//! contains ours, such as `freshkube_get_object_extra`, is not ours.
//!
//! Once a call has read as not ours it stays not ours, whatever a later
//! update or the request says, and a call with no name anywhere is refused.
//! The name is still the agent's own word; what keeps a shell from calling
//! itself `freshkube_get_object` is that the lock-down leaves none to call.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::investigation::clip;
use crate::tools::{SERVER, TOOLS};
use agent_client_protocol::schema::v1::{
    PermissionOptionKind, RequestPermissionOutcome, RequestPermissionRequest,
    SelectedPermissionOutcome, SessionUpdate,
};

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

#[derive(Default)]
pub(super) struct Calls(Mutex<HashMap<String, Known>>);

struct Known {
    label: String,
    ours: Option<bool>,
}

impl Calls {
    /// Remembers what a tool-call notification said about its call. A call
    /// that has read as not ours stays so.
    pub(super) fn note(&self, update: &SessionUpdate) {
        let (id, title, name) = match update {
            SessionUpdate::ToolCall(c) => (
                c.tool_call_id.to_string(),
                Some(c.title.as_str()),
                c.name.as_deref(),
            ),
            SessionUpdate::ToolCallUpdate(u) => (
                u.tool_call_id.to_string(),
                u.fields.title.as_deref(),
                u.fields.name.as_deref(),
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
        if let Some(name) = name.filter(|n| !n.is_empty()) {
            // Never from not ours to ours.
            call.ours = Some(call.ours.unwrap_or(true) && is_ours(name));
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
    // Every name on record must be ours, and there must be one.
    let words = [
        known.as_ref().and_then(|(_, ours)| *ours),
        fields
            .name
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(is_ours),
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
        calls.note(&SessionUpdate::ToolCall(
            ToolCall::new("t2", "kubectl delete pod x").name("shell"),
        ));
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
    fn a_call_with_no_name_is_refused_whatever_its_title_says() {
        let calls = Calls::default();
        // opencode 2.1's updates carry no name; a title is only a label.
        calls.note(&SessionUpdate::ToolCall(ToolCall::new(
            "t1",
            "freshkube_get_object",
        )));
        let (outcome, _) = decide(&ask("t1", ToolCallUpdateFields::new()), &calls);
        assert!(!allowed(&outcome));
        let titled = ToolCallUpdateFields::new().title("freshkube_get_object");
        let (outcome, _) = decide(&ask("t2", titled), &calls);
        assert!(!allowed(&outcome));
    }

    #[test]
    fn a_shell_whose_input_says_it_is_ours_is_refused() {
        let calls = Calls::default();
        let call = ToolCall::new("t1", "curl http://169.254.169.254/")
            .name("shell")
            .raw_input(json!({"server": "freshkube", "tool": "get_object", "command": "curl x"}));
        calls.note(&SessionUpdate::ToolCall(call));
        let (outcome, _) = decide(&ask("t1", ToolCallUpdateFields::new()), &calls);
        assert!(!allowed(&outcome));
        // Nor does an input of that shape make an unnamed call ours.
        let unnamed = ToolCall::new("t2", "Run tool")
            .raw_input(json!({"server": "freshkube", "tool": "get_object"}));
        calls.note(&SessionUpdate::ToolCall(unnamed));
        let (outcome, _) = decide(&ask("t2", ToolCallUpdateFields::new()), &calls);
        assert!(!allowed(&outcome));
        let input = ToolCallUpdateFields::new()
            .raw_input(json!({"server": "freshkube", "tool": "get_object"}));
        let (outcome, _) = decide(&ask("t3", input), &calls);
        assert!(!allowed(&outcome));
    }

    #[test]
    fn a_call_that_read_as_not_ours_stays_not_ours() {
        let calls = Calls::default();
        calls.note(&SessionUpdate::ToolCall(
            ToolCall::new("t1", "ls").name("shell"),
        ));
        // A later update names it as ours: too late.
        calls.note(&SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "t1",
            ToolCallUpdateFields::new().name("freshkube_get_object"),
        )));
        let (outcome, _) = decide(&ask("t1", ToolCallUpdateFields::new()), &calls);
        assert!(!allowed(&outcome));
        // The request cannot claim it either.
        let claim = ToolCallUpdateFields::new().name("freshkube_get_object");
        let (outcome, _) = decide(&ask("t1", claim), &calls);
        assert!(!allowed(&outcome));
        // And the other direction: ours first, then something else.
        calls.note(&SessionUpdate::ToolCall(
            ToolCall::new("t2", "freshkube_get_object").name("freshkube_get_object"),
        ));
        calls.note(&SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "t2",
            ToolCallUpdateFields::new().name("shell"),
        )));
        let (outcome, _) = decide(&ask("t2", ToolCallUpdateFields::new()), &calls);
        assert!(!allowed(&outcome));
    }

    #[test]
    fn a_request_naming_something_else_than_the_call_is_refused() {
        let calls = Calls::default();
        calls.note(&SessionUpdate::ToolCall(
            ToolCall::new("t1", "freshkube_get_object").name("freshkube_get_object"),
        ));
        let other = ToolCallUpdateFields::new().name("shell");
        let (outcome, _) = decide(&ask("t1", other), &calls);
        assert!(!allowed(&outcome));
        let same = ToolCallUpdateFields::new().name("freshkube_get_object");
        let (outcome, _) = decide(&ask("t1", same), &calls);
        assert!(allowed(&outcome));
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
