//! A scripted ACP agent. It reads through the MCP endpoint it is handed in
//! `session/new`, as the real agents do, and does what the prompt names:
//!
//! | The prompt contains | The agent |
//! | --- | --- |
//! | `slow` | thinks until it is cancelled |
//! | `flood` | sends 10 MiB of thoughts |
//! | `crash` | sends a thought and drops the connection |
//! | `environment` | says its arguments, environment and directories as JSON |
//! | `grandchild` | starts a process of its own and says both pids (the binary) |
//! | `tools` | asks to run each of the calls a locked-down agent must be refused |
//! | `model` | says which model it uses |
//! | anything else | reads a Deployment through MCP, then asks to run a shell command |
//!
//! It runs in process over a pipe ([`in_process`]), which the dock's UI tests
//! and `--fixture` can use, and as the `fake_acp_agent` binary for the tests
//! that need a process. The app bundle never ships the binary.

mod prompts;

use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, InitializeRequest,
    InitializeResponse, McpCapabilities, McpServer, NewSessionRequest, NewSessionResponse,
    PromptRequest, PromptResponse, SessionConfigOption, SessionConfigOptionCategory,
    SessionConfigOptionValue, SessionConfigSelectOption, SessionId, SessionNotification,
    SessionUpdate, SetSessionConfigOptionRequest, SetSessionConfigOptionResponse,
};
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo};
use tokio::sync::Notify;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tokio_util::sync::CancellationToken;

use crate::child::Link;

/// One model the scripted agent offers.
#[derive(Debug, Clone)]
pub struct ScriptModel {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Script {
    pub models: Vec<ScriptModel>,
    /// Never answers `initialize`.
    pub never_initialize: bool,
}

impl Default for Script {
    /// Two models, the second of them local.
    fn default() -> Self {
        let model = |id: &str, name: &str| ScriptModel {
            id: id.into(),
            name: name.into(),
        };
        Self {
            models: vec![
                model("hosted/large", "hosted/Large"),
                model("ollama/small", "ollama/Small"),
            ],
            never_initialize: false,
        }
    }
}

impl Script {
    /// The scripted binary's flags: `--never-initialize`.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Self {
        let mut script = Self::default();
        for arg in args {
            if arg == "--never-initialize" {
                script.never_initialize = true;
            }
        }
        script
    }
}

struct State {
    script: Script,
    mcp: Option<Mcp>,
    model: String,
    cancel: CancellationToken,
}

#[derive(Clone)]
pub(crate) struct Mcp {
    pub(crate) url: String,
    pub(crate) token: String,
}

type Shared = Arc<Mutex<State>>;

/// An agent in this process, over pipes. Ending the session stops it.
pub fn in_process(script: Script) -> Link {
    let (client_end, agent_end) = tokio::io::duplex(256 * 1024);
    let (agent_reader, agent_writer) = tokio::io::split(agent_end);
    let (client_reader, client_writer) = tokio::io::split(client_end);
    let task = tokio::spawn(async move {
        let _ = serve(agent_reader.compat(), agent_writer.compat_write(), script).await;
    });
    Link::in_process(
        "the scripted agent",
        client_reader,
        client_writer,
        std::env::temp_dir(),
        task.abort_handle(),
    )
}

/// Serves the script over the given streams until the client hangs up.
pub async fn serve(
    reader: impl futures::AsyncRead + Send + 'static,
    writer: impl futures::AsyncWrite + Send + 'static,
    script: Script,
) -> agent_client_protocol::Result<()> {
    let model = script
        .models
        .first()
        .map(|m| m.id.clone())
        .unwrap_or_default();
    let never = script.never_initialize;
    let state: Shared = Arc::new(Mutex::new(State {
        script,
        mcp: None,
        model,
        cancel: CancellationToken::new(),
    }));
    let crashed = Arc::new(Notify::new());
    let (initial, sessions, models, prompts, cancels) = (
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state,
    );
    let crash = Arc::clone(&crashed);
    let connection = Agent
        .builder()
        .name("scripted")
        .on_receive_request(
            async move |init: InitializeRequest, responder, _cx| {
                if never {
                    std::future::pending::<()>().await;
                }
                let _ = &initial;
                responder.respond(
                    InitializeResponse::new(init.protocol_version).agent_capabilities(
                        AgentCapabilities::new()
                            .mcp_capabilities(McpCapabilities::new().http(true)),
                    ),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, _cx| {
                let mcp = request.mcp_servers.iter().find_map(|s| match s {
                    McpServer::Http(http) => {
                        let token = http
                            .headers
                            .iter()
                            .find(|h| h.name == "Authorization")?
                            .value
                            .strip_prefix("Bearer ")?
                            .to_string();
                        Some(Mcp {
                            url: http.url.clone(),
                            token,
                        })
                    }
                    _ => None,
                });
                let mut state = sessions.lock().unwrap();
                state.mcp = mcp;
                let options = model_options(&state);
                drop(state);
                responder.respond(NewSessionResponse::new("s1").config_options(options))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: SetSessionConfigOptionRequest, responder, _cx| {
                let SessionConfigOptionValue::ValueId { value } = request.value else {
                    return responder
                        .respond_with_error(agent_client_protocol::Error::invalid_params());
                };
                let mut state = models.lock().unwrap();
                state.model = value.to_string();
                let options = model_options(&state);
                drop(state);
                responder.respond(SetSessionConfigOptionResponse::new(options))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, cx: ConnectionTo<Client>| {
                let (mcp, model, cancel) = {
                    let mut state = prompts.lock().unwrap();
                    state.cancel = CancellationToken::new();
                    (state.mcp.clone(), state.model.clone(), state.cancel.clone())
                };
                let text = request
                    .prompt
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .collect::<String>();
                let session = request.session_id;
                let task_cx = cx.clone();
                let crash = Arc::clone(&crash);
                cx.spawn(async move {
                    let turn = prompts::Turn {
                        cx: &task_cx,
                        session: &session,
                        mcp,
                        model,
                        cancel,
                    };
                    let stop = match prompts::run(&turn, &text).await? {
                        prompts::Ending::Stop(stop) => stop,
                        prompts::Ending::Crash => {
                            crash.notify_one();
                            return Ok(());
                        }
                    };
                    responder.respond(PromptResponse::new(stop))
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |_: CancelNotification, _cx| {
                cancels.lock().unwrap().cancel.cancel();
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_to(ByteStreams::new(writer, reader));
    tokio::select! {
        result = connection => result,
        () = crashed.notified() => Ok(()),
    }
}

fn model_options(state: &State) -> Vec<SessionConfigOption> {
    let choices: Vec<_> = state
        .script
        .models
        .iter()
        .map(|m| SessionConfigSelectOption::new(m.id.clone(), m.name.clone()))
        .collect();
    vec![
        SessionConfigOption::select("model", "Model", state.model.clone(), choices)
            .category(SessionConfigOptionCategory::Model),
    ]
}

pub(crate) fn send(
    cx: &ConnectionTo<Client>,
    session: &SessionId,
    update: SessionUpdate,
) -> agent_client_protocol::Result<()> {
    cx.send_notification(SessionNotification::new(session.clone(), update))
}

pub(crate) fn chunk(text: &str) -> ContentChunk {
    ContentChunk::new(ContentBlock::Text(
        agent_client_protocol::schema::v1::TextContent::new(text),
    ))
}

pub(crate) fn fail(e: impl std::fmt::Display) -> agent_client_protocol::Error {
    agent_client_protocol::Error::into_internal_error(std::io::Error::other(e.to_string()))
}
