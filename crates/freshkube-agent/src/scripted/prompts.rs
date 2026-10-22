//! What the scripted agent does for each prompt.

use std::time::Duration;

use agent_client_protocol::schema::v1::{
    PermissionOption, PermissionOptionKind, RequestPermissionOutcome, RequestPermissionRequest,
    SessionId, SessionUpdate, StopReason, ToolCall, ToolCallStatus, ToolCallUpdate,
    ToolCallUpdateFields, ToolKind,
};
use agent_client_protocol::{Client, ConnectionTo};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ContentBlock as McpContent};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{Mcp, chunk, fail, send};

pub(super) struct Turn<'a> {
    pub cx: &'a ConnectionTo<Client>,
    pub session: &'a SessionId,
    pub mcp: Option<Mcp>,
    pub model: String,
    pub cancel: CancellationToken,
}

pub(super) enum Ending {
    Stop(StopReason),
    /// Drop the connection without answering.
    Crash,
}

pub(super) async fn run(turn: &Turn<'_>, prompt: &str) -> agent_client_protocol::Result<Ending> {
    let stop = if prompt.contains("slow") {
        think_until_cancelled(turn).await?
    } else if prompt.contains("flood") {
        flood(turn)?
    } else if prompt.contains("crash") {
        send(
            turn.cx,
            turn.session,
            SessionUpdate::AgentThoughtChunk(chunk("About to fall over.")),
        )?;
        tokio::time::sleep(Duration::from_millis(50)).await;
        return Ok(Ending::Crash);
    } else if prompt.contains("environment") {
        say(turn, &environment().to_string())?;
        StopReason::EndTurn
    } else if prompt.contains("grandchild") {
        grandchild(turn)?
    } else if prompt.contains("tools") {
        tools(turn).await?
    } else if prompt.contains("model") {
        say(turn, &format!("model={}", turn.model))?;
        StopReason::EndTurn
    } else {
        investigate(turn).await?
    };
    Ok(Ending::Stop(stop))
}

fn say(turn: &Turn<'_>, text: &str) -> agent_client_protocol::Result<()> {
    send(
        turn.cx,
        turn.session,
        SessionUpdate::AgentMessageChunk(chunk(text)),
    )
}

async fn think_until_cancelled(turn: &Turn<'_>) -> agent_client_protocol::Result<StopReason> {
    for _ in 0..500 {
        send(
            turn.cx,
            turn.session,
            SessionUpdate::AgentThoughtChunk(chunk("still thinking ")),
        )?;
        tokio::select! {
            () = turn.cancel.cancelled() => return Ok(StopReason::Cancelled),
            () = tokio::time::sleep(Duration::from_millis(20)) => {}
        }
    }
    Ok(StopReason::EndTurn)
}

/// 10 MiB of thoughts, in chunks of 64 KiB.
fn flood(turn: &Turn<'_>) -> agent_client_protocol::Result<StopReason> {
    let piece = "x".repeat(64 * 1024);
    for _ in 0..160 {
        send(
            turn.cx,
            turn.session,
            SessionUpdate::AgentThoughtChunk(chunk(&piece)),
        )?;
    }
    say(turn, "done flooding")?;
    Ok(StopReason::EndTurn)
}

/// The arguments, environment and directories this process has, as JSON.
fn environment() -> Value {
    let env: serde_json::Map<String, Value> =
        std::env::vars().map(|(k, v)| (k, json!(v))).collect();
    let cwd = std::env::current_dir().unwrap_or_default();
    let entries =
        |dir: &std::path::Path| std::fs::read_dir(dir).map_or(-1, |entries| entries.count() as i64);
    let size = |name: &str| {
        std::env::var(name)
            .ok()
            .and_then(|path| std::fs::metadata(path).ok())
            .map_or(-1, |meta| meta.len() as i64)
    };
    let config_home = std::env::var("XDG_CONFIG_HOME").unwrap_or_default();
    json!({
        "args": std::env::args().collect::<Vec<_>>(),
        "env": env,
        "cwd": cwd,
        "cwd_entries": entries(&cwd),
        "kubeconfig_len": size("KUBECONFIG"),
        "talosconfig_len": size("TALOSCONFIG"),
        "config_home_entries": entries(std::path::Path::new(&config_home)),
        "pid": std::process::id(),
    })
}

/// Starts a process of its own (this binary, idling) and says both pids.
fn grandchild(turn: &Turn<'_>) -> agent_client_protocol::Result<StopReason> {
    let exe = std::env::current_exe().map_err(fail)?;
    let child = std::process::Command::new(exe)
        .arg("--idle")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn()
        .map_err(fail)?;
    say(
        turn,
        &json!({"agent": std::process::id(), "grandchild": child.id()}).to_string(),
    )?;
    // Not waited for: it outlives this call, and must die with the group.
    std::mem::forget(child);
    Ok(StopReason::EndTurn)
}

/// One call: announced, then asked about by its id only (or with a claim of
/// its own). Returns whether the client allowed it.
struct Call<'a> {
    id: &'a str,
    title: &'a str,
    name: &'a str,
    kind: ToolKind,
    input: Value,
    /// A claim the permission request makes about the call.
    claim: Option<&'a str>,
}

async fn attempt(turn: &Turn<'_>, call: Call<'_>) -> agent_client_protocol::Result<bool> {
    send(
        turn.cx,
        turn.session,
        SessionUpdate::ToolCall(
            ToolCall::new(call.id.to_string(), call.title.to_string())
                .name(call.name.to_string())
                .kind(call.kind)
                .status(ToolCallStatus::Pending)
                .raw_input(call.input),
        ),
    )?;
    let fields = match call.claim {
        Some(claim) => ToolCallUpdateFields::new().title(claim.to_string()),
        None => ToolCallUpdateFields::new(),
    };
    let allowed = ask(turn, call.id, fields).await?;
    send(
        turn.cx,
        turn.session,
        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            call.id.to_string(),
            ToolCallUpdateFields::new().status(if allowed {
                ToolCallStatus::Completed
            } else {
                ToolCallStatus::Failed
            }),
        )),
    )?;
    Ok(allowed)
}

async fn ask(
    turn: &Turn<'_>,
    id: &str,
    fields: ToolCallUpdateFields,
) -> agent_client_protocol::Result<bool> {
    // opencode offers these three; the others follow the same shape.
    let options = vec![
        PermissionOption::new("once", "Allow once", PermissionOptionKind::AllowOnce),
        PermissionOption::new("always", "Always allow", PermissionOptionKind::AllowAlways),
        PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
    ];
    let response = turn
        .cx
        .send_request(RequestPermissionRequest::new(
            turn.session.clone(),
            ToolCallUpdate::new(id.to_string(), fields),
            options,
        ))
        .block_task()
        .await?;
    Ok(matches!(
        response.outcome,
        RequestPermissionOutcome::Selected(s) if s.option_id.to_string() == "once"
    ))
}

/// Every kind of call a locked-down agent must be refused, and ours.
async fn tools(turn: &Turn<'_>) -> agent_client_protocol::Result<StopReason> {
    let mut allowed = Vec::new();
    let get_object = json!({"kind": "Deployment", "namespace": "shop", "name": "checkout"});
    let calls = [
        Call {
            id: "ours",
            title: "freshkube_get_object",
            name: "freshkube_get_object",
            kind: ToolKind::Other,
            input: get_object.clone(),
            claim: None,
        },
        Call {
            id: "shell",
            title: "kubectl delete pod checkout-7c9d8f6b4-q8zxk",
            name: "shell",
            kind: ToolKind::Execute,
            input: json!({"command": "kubectl delete pod checkout-7c9d8f6b4-q8zxk"}),
            claim: None,
        },
        Call {
            id: "read",
            title: "~/.kube/config",
            name: "read",
            kind: ToolKind::Read,
            input: json!({"filePath": "~/.kube/config"}),
            claim: None,
        },
        Call {
            id: "edit",
            title: "/etc/hosts",
            name: "edit",
            kind: ToolKind::Edit,
            input: json!({"filePath": "/etc/hosts"}),
            claim: None,
        },
        Call {
            id: "fetch",
            title: "https://example.com/payload",
            name: "webfetch",
            kind: ToolKind::Fetch,
            input: json!({"url": "https://example.com/payload"}),
            claim: None,
        },
        Call {
            id: "other",
            title: "other_get_object",
            name: "other_get_object",
            kind: ToolKind::Other,
            input: get_object.clone(),
            claim: None,
        },
        Call {
            id: "extra",
            title: "freshkube_get_object_extra",
            name: "freshkube_get_object_extra",
            kind: ToolKind::Other,
            input: get_object.clone(),
            claim: None,
        },
        // Announced as a shell, then asked about under our name.
        Call {
            id: "claim",
            title: "curl http://169.254.169.254/",
            name: "shell",
            kind: ToolKind::Execute,
            input: json!({"command": "curl http://169.254.169.254/"}),
            claim: Some("freshkube_get_object"),
        },
    ];
    for call in calls {
        let id = call.id;
        if attempt(turn, call).await? {
            allowed.push(id);
        }
    }
    // A call nothing announced, asked about by its id alone.
    if ask(turn, "never-announced", ToolCallUpdateFields::new()).await? {
        allowed.push("never-announced");
    }
    say(turn, &format!("allowed={}", allowed.join(",")))?;
    Ok(StopReason::EndTurn)
}

/// Reads the Deployment through MCP, then asks to run a shell command.
async fn investigate(turn: &Turn<'_>) -> agent_client_protocol::Result<StopReason> {
    let mcp = turn
        .mcp
        .clone()
        .ok_or_else(|| fail("no MCP server in session/new"))?;
    send(
        turn.cx,
        turn.session,
        SessionUpdate::AgentThoughtChunk(chunk("Reading the Deployment.")),
    )?;
    let arguments = json!({"kind": "Deployment", "namespace": "shop", "name": "checkout"});
    send(
        turn.cx,
        turn.session,
        SessionUpdate::ToolCall(
            ToolCall::new("t1", "freshkube_get_object")
                .name("freshkube_get_object")
                .status(ToolCallStatus::InProgress)
                .raw_input(arguments.clone()),
        ),
    )?;
    let status = if ask(turn, "t1", ToolCallUpdateFields::new()).await? {
        let client = ()
            .serve(StreamableHttpClientTransport::from_config(
                StreamableHttpClientTransportConfig::with_uri(mcp.url).auth_header(mcp.token),
            ))
            .await
            .map_err(fail)?;
        let Value::Object(args) = arguments else {
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
        turn.cx,
        turn.session,
        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "t1",
            ToolCallUpdateFields::new().status(status),
        )),
    )?;
    attempt(
        turn,
        Call {
            id: "t2",
            title: "kubectl delete pod checkout-7c9d8f6b4-q8zxk",
            name: "shell",
            kind: ToolKind::Execute,
            input: json!({"command": "kubectl -n shop delete pod checkout-7c9d8f6b4-q8zxk"}),
            claim: None,
        },
    )
    .await?;
    say(turn, "Cause: scripted.")?;
    Ok(StopReason::EndTurn)
}
