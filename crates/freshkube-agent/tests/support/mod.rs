//! Shared by the integration tests: a stub for the MCP endpoint, a way to
//! drive a session to the end of a turn, and the scripted binary's profile.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::Request;
use axum::http::{StatusCode, header::AUTHORIZATION};
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use freshkube_agent::acp::{AcpSession, Endpoint};
use freshkube_agent::investigation::{Event, Investigation};
use freshkube_agent::profile::{Host, Profile};
use freshkube_agent::scripted::{self, Script};
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
    Implementation, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
    Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::sync::CancellationToken;

pub const TOKEN: &str = "stub-session-token-0123456789abcdef";

/// A stand-in for the app's endpoint: the four tools, answering only the
/// session's bearer token, and a record of the Authorization headers it saw.
pub struct Stub {
    pub url: String,
    pub token: String,
    pub seen: Arc<Mutex<Vec<String>>>,
    stop: CancellationToken,
}

impl Stub {
    pub fn endpoint(&self) -> Endpoint {
        Endpoint::new(self.url.clone(), self.token.clone())
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

pub async fn stub() -> Stub {
    let stop = CancellationToken::new();
    let tools: Vec<Tool> = freshkube_agent::tools::TOOLS
        .iter()
        .map(|name| {
            let Value::Object(schema) = json!({"type": "object", "properties": {}}) else {
                unreachable!()
            };
            Tool::new(*name, "stub", schema)
        })
        .collect();
    let handler = Handler {
        tools: Arc::new(tools),
    };
    let service: StreamableHttpService<Handler, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Default::default(),
        StreamableHttpServerConfig::default().with_cancellation_token(stop.clone()),
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    let expected = format!("Bearer {TOKEN}");
    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let header = request
                .headers()
                .get(AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let authorized = header.as_deref() == Some(expected.as_str());
            record.lock().unwrap().extend(header);
            async move {
                if authorized {
                    next.run(request).await
                } else {
                    StatusCode::UNAUTHORIZED.into_response()
                }
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let until = stop.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(until.cancelled_owned())
            .await;
    });
    Stub {
        url,
        token: TOKEN.into(),
        seen,
        stop,
    }
}

#[derive(Clone)]
struct Handler {
    tools: Arc<Vec<Tool>>,
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("freshkube", "0"))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.to_vec())
            .with_ttl_ms(3_600_000)
            .with_cache_scope(CacheScope::Private))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let arguments = request.arguments.unwrap_or_else(Map::new);
        let name = arguments.get("name").and_then(Value::as_str).unwrap_or("?");
        let kind = arguments.get("kind").and_then(Value::as_str).unwrap_or("?");
        let namespace = arguments
            .get("namespace")
            .and_then(Value::as_str)
            .unwrap_or("?");
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "{kind}/{namespace}/{name}"
        ))])
        .into())
    }
}

/// The scripted agent as a process, with the locked environment and none of
/// an agent's own switches.
pub fn scripted_binary(args: &[&str]) -> Profile {
    Profile::command(
        "the scripted agent",
        PathBuf::from(env!("CARGO_BIN_EXE_fake_acp_agent")),
        args.iter().copied(),
    )
}

pub fn start_binary(args: &[&str], endpoint: Endpoint) -> (AcpSession, UnboundedReceiver<Event>) {
    let link = scripted_binary(args)
        .spawn(&Host::default())
        .expect("the scripted agent starts");
    AcpSession::start(link, endpoint)
}

pub fn start_in_process(endpoint: Endpoint) -> (AcpSession, UnboundedReceiver<Event>) {
    AcpSession::start(scripted::in_process(Script::default()), endpoint)
}

/// Records events until the turn ends; panics if it doesn't within 20 s.
pub async fn until_ended(rx: &mut UnboundedReceiver<Event>, inv: &mut Investigation) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(20), rx.recv())
            .await
            .expect("the turn ends")
            .expect("the session reports the end of the turn");
        let ended = matches!(event, Event::Ended(_));
        inv.record(event);
        if ended {
            return;
        }
    }
}

/// Everything the agent said in the turn, joined.
pub fn said(inv: &Investigation) -> String {
    use freshkube_agent::investigation::Entry;
    inv.entries()
        .iter()
        .filter_map(|entry| match entry {
            Entry::Said(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect()
}

pub fn notes(inv: &Investigation) -> Vec<String> {
    use freshkube_agent::investigation::Entry;
    inv.entries()
        .iter()
        .filter_map(|entry| match entry {
            Entry::Note(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Whether a process is still running; a zombie is not.
#[cfg(unix)]
pub fn alive(pid: i32) -> bool {
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        // The state is the field after the parenthesised name.
        let state = stat
            .rsplit(')')
            .next()
            .and_then(|rest| rest.trim().chars().next());
        return state != Some('Z') && state != Some('X');
    }
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc_kill(pid, 0) == 0 }
}

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}
