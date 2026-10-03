//! The ACP backend: the operator's own agent (Claude Code or Codex, signed in
//! with their subscription) runs as a child process speaking ACP over stdio and
//! reads the cluster only through our MCP endpoint. Its session updates become
//! the same dock events the embedded loop produces.
//!
//! Each agent is locked down the same way: no built-in tools (shell, files,
//! web), none of the operator's own settings, hooks or MCP servers, an empty
//! scratch directory as its workspace, and a permission handler that refuses
//! anything but our tools.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    CancelNotification, ContentBlock, HttpHeader, InitializeRequest, McpServer, McpServerHttp,
    NewSessionRequest, PermissionOptionKind, PromptRequest, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome,
    SessionNotification, SessionUpdate, StopReason, TextContent, ToolCallStatus,
};
use agent_client_protocol::{AcpAgent, AcpAgentConfig, ConnectionTo, LineDirection};
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_util::sync::CancellationToken;

use crate::investigation::{Event, Outcome};
use crate::mcp::{self, SERVER_NAME};
use crate::scenario::Cluster;
use crate::session::SYSTEM_PROMPT;
use crate::tools;

/// How long a cancelled prompt may take to answer before the agent is dropped.
const CANCEL_GRACE: Duration = Duration::from_secs(5);

pub enum AgentKind {
    /// `@agentclientprotocol/claude-agent-acp`: Claude Code on the operator's login.
    Claude,
    /// `@agentclientprotocol/codex-acp`: Codex on the operator's ChatGPT login.
    Codex,
    /// Any ACP agent command, such as the offline fake.
    Command(AcpAgentConfig),
}

impl AgentKind {
    fn config(self) -> AcpAgentConfig {
        match self {
            // `strictMcpConfig` doesn't cover the operator's claude.ai connectors.
            AgentKind::Claude => AcpAgentConfig::new("npx")
                .args(["-y", "@agentclientprotocol/claude-agent-acp@0.85.1"])
                .env("ENABLE_CLAUDEAI_MCP_SERVERS", "false"),
            AgentKind::Codex => AcpAgentConfig::new("npx")
                .args(["-y", "@agentclientprotocol/codex-acp@2.1.1"])
                .env("INITIAL_AGENT_MODE", "read-only")
                .env("CODEX_CONFIG", codex_config().to_string()),
            AgentKind::Command(config) => config,
        }
    }
}

/// Codex's own tools off, our instructions and reasoning summaries on. Codex reads the operator's
/// `~/.codex/config.toml` for its login and model; these override the rest.
fn codex_config() -> Value {
    let off = [
        "shell_tool",
        "unified_exec",
        "apps",
        "plugins",
        "hooks",
        "multi_agent",
        "browser_use",
        "computer_use",
        "in_app_browser",
        "image_generation",
        "view_image",
        "goals",
        "memories",
        "skill_search",
        "tool_suggest",
    ];
    json!({
        "developer_instructions": SYSTEM_PROMPT,
        "model_reasoning_summary": "detailed",
        "web_search": "disabled",
        "features": off.iter().map(|f| (f.to_string(), json!(false))).collect::<serde_json::Map<_, _>>(),
    })
}

/// Claude Code's session options: our system prompt in place of its own, no
/// built-in tools, no settings files (so no hooks, CLAUDE.md or user MCP
/// servers), our tools allowed without a prompt, and summarized thinking.
fn claude_meta() -> serde_json::Map<String, Value> {
    let allowed: Vec<String> = tools::specs()
        .iter()
        .map(|s| format!("mcp__{SERVER_NAME}__{}", s.name))
        .collect();
    let mut meta = json!({
        "systemPrompt": SYSTEM_PROMPT,
        "claudeCode": {
            "options": {
                "tools": [],
                "settingSources": [],
                "strictMcpConfig": true,
                "allowedTools": allowed,
                "thinking": {"type": "adaptive", "display": "summarized"},
            }
        }
    });
    // FRESHKUBE_CLAUDE_DEBUG_FILE=<path> keeps Claude Code's debug log.
    if let Ok(path) = std::env::var("FRESHKUBE_CLAUDE_DEBUG_FILE") {
        meta["claudeCode"]["options"]["debugFile"] = json!(path);
    }
    let Value::Object(meta) = meta else {
        unreachable!()
    };
    meta
}

pub struct AcpSession {
    cancel: CancellationToken,
}

impl AcpSession {
    pub fn start(
        agent: AgentKind,
        cluster: Arc<Cluster>,
        prompt: impl Into<String>,
    ) -> (Self, UnboundedReceiver<Event>) {
        Self::start_with(agent, cluster, prompt, |_| {})
    }

    /// `trace` sees every session update as it arrives, for the spike's logs.
    pub fn start_with(
        agent: AgentKind,
        cluster: Arc<Cluster>,
        prompt: impl Into<String>,
        trace: impl Fn(&SessionUpdate) + Send + Sync + 'static,
    ) -> (Self, UnboundedReceiver<Event>) {
        let (tx, rx) = unbounded_channel();
        let cancel = CancellationToken::new();
        let prompt = prompt.into();
        let claude = matches!(agent, AgentKind::Claude);
        let run = run(
            agent.config(),
            claude,
            cluster,
            prompt,
            tx.clone(),
            cancel.clone(),
            Arc::new(trace),
        );
        tokio::spawn(async move {
            let outcome = run.await.unwrap_or_else(|e| Outcome::Failed(e.to_string()));
            let _ = tx.send(Event::Ended(outcome));
        });
        (Self { cancel }, rx)
    }

    /// Asks the agent to stop; it answers the prompt as cancelled.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl Drop for AcpSession {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

type Trace = Arc<dyn Fn(&SessionUpdate) + Send + Sync>;

async fn run(
    config: AcpAgentConfig,
    claude: bool,
    cluster: Arc<Cluster>,
    prompt: String,
    tx: UnboundedSender<Event>,
    cancel: CancellationToken,
    trace: Trace,
) -> Result<Outcome, agent_client_protocol::Error> {
    let internal = agent_client_protocol::Error::into_internal_error;
    // An empty workspace: nothing on disk for the agent to read.
    let workspace = tempfile::tempdir().map_err(internal)?;
    let reads = tx.clone();
    let server = mcp::serve(
        cluster,
        Arc::new(move |tool, place| {
            let _ = reads.send(Event::Read {
                step: None,
                tool: tool.into(),
                place,
            });
        }),
    )
    .await
    .map_err(internal)?;

    let updates = tx.clone();
    let refusals = tx.clone();
    let calls = Arc::new(Calls::default());
    let seen = Arc::clone(&calls);
    agent_client_protocol::Client
        .builder()
        .name("freshkube")
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                trace(&notification.update);
                seen.note(&notification.update);
                for event in events(notification.update) {
                    let _ = updates.send(event);
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _cx| {
                let (outcome, refused) = decide(&request, &calls);
                if let Some(what) = refused {
                    let _ = refusals.send(Event::Note(format!("Refused: {what}")));
                }
                responder.respond(RequestPermissionResponse::new(outcome))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(
            // The agent's own log goes to our stderr.
            AcpAgent::new(config).with_debug(|line, direction| {
                if matches!(direction, LineDirection::Stderr) {
                    eprintln!("[agent] {line}");
                }
            }),
            async move |cx: ConnectionTo<agent_client_protocol::Agent>| {
                cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
                    .block_task()
                    .await?;
                let mut request =
                    NewSessionRequest::new(workspace.path()).mcp_servers(vec![McpServer::Http(
                        McpServerHttp::new(SERVER_NAME, &server.url).headers(vec![
                            HttpHeader::new("Authorization", format!("Bearer {}", server.token)),
                        ]),
                    )]);
                if claude {
                    request = request.meta(claude_meta());
                }
                let session = cx.send_request(request).block_task().await?.session_id;

                let answer = cx
                    .send_request(PromptRequest::new(
                        session.clone(),
                        vec![ContentBlock::Text(TextContent::new(prompt))],
                    ))
                    .block_task();
                tokio::pin!(answer);
                let response = tokio::select! {
                    response = &mut answer => response?,
                    () = cancel.cancelled() => {
                        cx.send_notification(CancelNotification::new(session))?;
                        match tokio::time::timeout(CANCEL_GRACE, answer).await {
                            Ok(response) => response?,
                            Err(_) => return Ok(Outcome::Cancelled),
                        }
                    }
                };
                drop(server);
                Ok(match response.stop_reason {
                    StopReason::EndTurn => Outcome::Answered,
                    StopReason::Cancelled => Outcome::Cancelled,
                    other => Outcome::Failed(format!("stopped: {other:?}")),
                })
            },
        )
        .await
}

/// What the agent said about each tool call, by id. A permission request may
/// name only the call's id (Codex's do), so the decision rests on what the
/// call's own notification said.
#[derive(Default)]
struct Calls(Mutex<HashMap<String, Call>>);

#[derive(Clone)]
struct Call {
    label: String,
    ours: bool,
}

impl Calls {
    fn note(&self, update: &SessionUpdate) {
        let (id, title, name, input) = match update {
            SessionUpdate::ToolCall(c) => (
                &c.tool_call_id,
                Some(c.title.as_str()),
                c.name.as_deref(),
                c.raw_input.as_ref(),
            ),
            SessionUpdate::ToolCallUpdate(u) => (
                &u.tool_call_id,
                u.fields.title.as_deref(),
                u.fields.name.as_deref(),
                u.fields.raw_input.as_ref(),
            ),
            _ => return,
        };
        let mut calls = self.0.lock().unwrap();
        let call = calls.entry(id.to_string()).or_insert_with(|| Call {
            label: id.to_string(),
            ours: false,
        });
        if let Some(title) = title.filter(|t| !t.is_empty()) {
            call.label = title.into();
        }
        call.ours |= ours(title, name, input);
    }

    fn get(&self, id: &str) -> Option<Call> {
        self.0.lock().unwrap().get(id).cloned()
    }
}

/// Allows our tools once; refuses anything else, naming it for the dock.
fn decide(
    request: &RequestPermissionRequest,
    calls: &Calls,
) -> (RequestPermissionOutcome, Option<String>) {
    let id = request.tool_call.tool_call_id.to_string();
    let fields = &request.tool_call.fields;
    let known = calls.get(&id);
    let ours = ours(
        fields.title.as_deref(),
        fields.name.as_deref(),
        fields.raw_input.as_ref(),
    ) || known.as_ref().is_some_and(|c| c.ours);
    let label = fields
        .title
        .clone()
        .or_else(|| fields.name.clone())
        .or(known.map(|c| c.label))
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
    (outcome, (!ours).then_some(label))
}

/// Claude Code names our tools `mcp__freshkube__get_object`, Codex
/// `mcp.freshkube.get_object` with the server in its input.
fn ours(title: Option<&str>, name: Option<&str>, input: Option<&Value>) -> bool {
    let named = |label: &str| {
        label.contains(SERVER_NAME) && tools::specs().iter().any(|s| label.contains(s.name))
    };
    title.is_some_and(named)
        || name.is_some_and(named)
        || input.is_some_and(|i| i.get("server").and_then(Value::as_str) == Some(SERVER_NAME))
}

/// One session update in dock terms. A tool call may arrive before its input
/// is known and be described again later; the dock updates the step in place.
fn events(update: SessionUpdate) -> Vec<Event> {
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
                args: call.raw_input.unwrap_or(Value::Null),
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
                    args: fields.raw_input,
                });
            }
            out.extend(fields.status.and_then(|s| ended(id, s)));
            out
        }
        _ => Vec::new(),
    }
}

fn label(title: &str, name: Option<&str>) -> String {
    match name {
        Some(name) if !title.contains(name) => format!("{title} ({name})"),
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
