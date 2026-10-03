//! A scripted ACP agent for offline tests of the ACP backend. It reads through
//! the MCP endpoint the client hands it, as Claude Code and Codex do.
//!
//! A prompt containing "slow" thinks until cancelled. Any other prompt reads
//! the Deployment through MCP after asking permission by call id only (as Codex
//! does), then asks to run a shell command, which the client must refuse.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, InitializeRequest,
    InitializeResponse, McpCapabilities, McpServer, NewSessionRequest, NewSessionResponse,
    PermissionOption, PermissionOptionKind, PromptRequest, PromptResponse,
    RequestPermissionOutcome, RequestPermissionRequest, SessionNotification, SessionUpdate,
    StopReason, TextContent, ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields,
    ToolKind,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Stdio};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ContentBlock as McpContent};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct State {
    mcp: Option<(String, String)>,
    cancel: CancellationToken,
}

type Shared = Arc<Mutex<State>>;

#[tokio::main]
async fn main() -> agent_client_protocol::Result<()> {
    let state: Shared = Arc::default();
    let (sessions, prompts, cancels) = (state.clone(), state.clone(), state);
    Agent
        .builder()
        .name("fake")
        .on_receive_request(
            async move |init: InitializeRequest, responder, _cx| {
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
                        Some((http.url.clone(), token))
                    }
                    _ => None,
                });
                sessions.lock().unwrap().mcp = mcp;
                responder.respond(NewSessionResponse::new("s1"))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, cx: ConnectionTo<Client>| {
                let (mcp, cancel) = {
                    let state = prompts.lock().unwrap();
                    (state.mcp.clone(), state.cancel.clone())
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
                cx.spawn(async move {
                    let stop = if text.contains("slow") {
                        think_until_cancelled(&task_cx, &session, &cancel).await?
                    } else {
                        investigate(&task_cx, &session, mcp).await?
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
        .connect_to(Stdio::new())
        .await
}

fn send(
    cx: &ConnectionTo<Client>,
    session: &agent_client_protocol::schema::v1::SessionId,
    update: SessionUpdate,
) -> agent_client_protocol::Result<()> {
    cx.send_notification(SessionNotification::new(session.clone(), update))
}

fn fail(e: impl std::fmt::Display) -> agent_client_protocol::Error {
    agent_client_protocol::Error::into_internal_error(std::io::Error::other(e.to_string()))
}

fn chunk(text: &str) -> ContentChunk {
    ContentChunk::new(ContentBlock::Text(TextContent::new(text)))
}

async fn think_until_cancelled(
    cx: &ConnectionTo<Client>,
    session: &agent_client_protocol::schema::v1::SessionId,
    cancel: &CancellationToken,
) -> agent_client_protocol::Result<StopReason> {
    for _ in 0..500 {
        send(
            cx,
            session,
            SessionUpdate::AgentThoughtChunk(chunk("still thinking ")),
        )?;
        tokio::select! {
            () = cancel.cancelled() => return Ok(StopReason::Cancelled),
            () = tokio::time::sleep(Duration::from_millis(20)) => {}
        }
    }
    Ok(StopReason::EndTurn)
}

async fn investigate(
    cx: &ConnectionTo<Client>,
    session: &agent_client_protocol::schema::v1::SessionId,
    mcp: Option<(String, String)>,
) -> agent_client_protocol::Result<StopReason> {
    let (url, token) = mcp.ok_or_else(|| fail("no MCP server in session/new"))?;
    send(
        cx,
        session,
        SessionUpdate::AgentThoughtChunk(chunk("Reading the Deployment.")),
    )?;

    // Our tool, announced in full, then a permission request by id only.
    let arguments = json!({"kind": "Deployment", "namespace": "shop", "name": "checkout"});
    send(
        cx,
        session,
        SessionUpdate::ToolCall(
            ToolCall::new("t1", "mcp.freshkube.get_object")
                .kind(ToolKind::Execute)
                .status(ToolCallStatus::InProgress)
                .raw_input(
                    json!({"server": "freshkube", "tool": "get_object", "arguments": arguments}),
                ),
        ),
    )?;
    let allowed = ask(cx, session, "t1").await?;
    let status = if allowed {
        let client = ()
            .serve(StreamableHttpClientTransport::from_config(
                StreamableHttpClientTransportConfig::with_uri(url).auth_header(token),
            ))
            .await
            .map_err(fail)?;
        let serde_json::Value::Object(args) = arguments else {
            unreachable!()
        };
        let result = client
            .call_tool(CallToolRequestParams::new("get_object").with_arguments(args))
            .await
            .map_err(fail)?;
        let read = result.content.iter().any(
            |c| matches!(c, McpContent::Text(t) if t.text.contains("Deployment/shop/checkout")),
        );
        if read {
            ToolCallStatus::Completed
        } else {
            ToolCallStatus::Failed
        }
    } else {
        ToolCallStatus::Failed
    };
    send(
        cx,
        session,
        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "t1",
            ToolCallUpdateFields::new().status(status),
        )),
    )?;

    // Not ours: the client must refuse it.
    send(
        cx,
        session,
        SessionUpdate::ToolCall(
            ToolCall::new("t2", "kubectl delete pod checkout-7c9d8f6b4-q8zxk")
                .kind(ToolKind::Execute)
                .raw_input(
                    json!({"command": "kubectl -n shop delete pod checkout-7c9d8f6b4-q8zxk"}),
                ),
        ),
    )?;
    let allowed = ask(cx, session, "t2").await?;
    send(
        cx,
        session,
        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "t2",
            ToolCallUpdateFields::new().status(if allowed {
                ToolCallStatus::Completed
            } else {
                ToolCallStatus::Failed
            }),
        )),
    )?;
    send(
        cx,
        session,
        SessionUpdate::AgentMessageChunk(chunk("Cause: scripted.")),
    )?;
    Ok(StopReason::EndTurn)
}

/// Asks permission naming only the call's id; true when allowed.
async fn ask(
    cx: &ConnectionTo<Client>,
    session: &agent_client_protocol::schema::v1::SessionId,
    id: &str,
) -> agent_client_protocol::Result<bool> {
    let options = vec![
        PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce),
        PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
    ];
    let response = cx
        .send_request(RequestPermissionRequest::new(
            session.clone(),
            ToolCallUpdate::new(id.to_string(), ToolCallUpdateFields::new()),
            options,
        ))
        .block_task()
        .await?;
    Ok(matches!(
        response.outcome,
        RequestPermissionOutcome::Selected(s) if s.option_id.to_string() == "allow"
    ))
}
