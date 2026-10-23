//! One session update in the transcript's terms. A tool call may arrive
//! before its input is known and be described again later; the transcript
//! updates the step in place.

use agent_client_protocol::schema::v1::{ContentBlock, SessionUpdate, ToolCallStatus};
use serde_json::Value;

use crate::investigation::{Args, Event, clip};

const MAX_LABEL_BYTES: usize = 256;

pub(super) fn events(update: SessionUpdate) -> Vec<Event> {
    match update {
        SessionUpdate::AgentThoughtChunk(chunk) => text(chunk.content)
            .map(Event::Thought)
            .into_iter()
            .collect(),
        SessionUpdate::AgentMessageChunk(chunk) => {
            text(chunk.content).map(Event::Said).into_iter().collect()
        }
        SessionUpdate::ToolCall(call) => {
            let id = call.tool_call_id.to_string();
            let mut out = vec![Event::StepStarted {
                id: id.clone(),
                tool: label(&call.title, call.name.as_deref()),
                args: Args::from_value(&call.raw_input.unwrap_or(Value::Null)),
            }];
            out.extend(ended(id, call.status));
            out
        }
        SessionUpdate::ToolCallUpdate(update) => {
            let id = update.tool_call_id.to_string();
            let fields = update.fields;
            let mut out = Vec::new();
            if fields.title.is_some() || fields.raw_input.is_some() {
                out.push(Event::StepDescribed {
                    id: id.clone(),
                    tool: fields
                        .title
                        .as_deref()
                        .map(|t| label(t, fields.name.as_deref())),
                    args: fields.raw_input.as_ref().map(Args::from_value),
                });
            }
            out.extend(fields.status.and_then(|s| ended(id, s)));
            out
        }
        _ => Vec::new(),
    }
}

/// The title, with the tool's name when the title doesn't say it.
pub(super) fn label(title: &str, name: Option<&str>) -> String {
    let title = clip(title, MAX_LABEL_BYTES);
    match name {
        Some(name) if !title.contains(name) => {
            format!("{title} ({})", clip(name, MAX_LABEL_BYTES))
        }
        _ => title.to_string(),
    }
}

fn ended(id: String, status: ToolCallStatus) -> Option<Event> {
    match status {
        ToolCallStatus::Completed => Some(Event::StepEnded { id, failed: false }),
        ToolCallStatus::Failed => Some(Event::StepEnded { id, failed: true }),
        _ => None,
    }
}

fn text(content: ContentBlock) -> Option<String> {
    match content {
        ContentBlock::Text(t) => Some(t.text),
        _ => None,
    }
}
