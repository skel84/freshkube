//! Our tools over MCP, for an agent that runs outside the app (the ACP
//! backend). A loopback Streamable HTTP endpoint that answers only requests
//! carrying the session's bearer token; each call reports its place, as the
//! embedded tools do.

use std::sync::Arc;

use axum::extract::Request;
use axum::http::{StatusCode, header::AUTHORIZATION};
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
    Implementation, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
    Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::scenario::Cluster;
use crate::tools::{self, Place};

/// Called with the tool's name and where the window would go.
pub type OnRead = Arc<dyn Fn(&str, Place) + Send + Sync>;

pub const SERVER_NAME: &str = "freshkube";

/// A running endpoint; dropping it stops the server.
pub struct McpServer {
    pub url: String,
    pub token: String,
    stop: CancellationToken,
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

pub async fn serve(cluster: Arc<Cluster>, on_read: OnRead) -> std::io::Result<McpServer> {
    let stop = CancellationToken::new();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let handler = Handler {
        cluster,
        on_read,
        tools: Arc::new(
            tools::specs()
                .into_iter()
                .map(|spec| {
                    let Value::Object(schema) = spec.parameters else {
                        unreachable!("tool schemas are objects")
                    };
                    // Claude Code otherwise defers MCP tools behind its own
                    // ToolSearch, which a session without built-in tools lacks.
                    let mut meta = serde_json::Map::new();
                    meta.insert("anthropic/alwaysLoad".into(), Value::Bool(true));
                    Tool::new(spec.name, spec.description, schema).with_meta(meta.into())
                })
                .collect(),
        ),
    };
    let service: StreamableHttpService<Handler, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Default::default(),
        StreamableHttpServerConfig::default().with_cancellation_token(stop.clone()),
    );
    let expected = format!("Bearer {token}");
    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let authorized = request
                .headers()
                .get(AUTHORIZATION)
                .is_some_and(|v| v.as_bytes() == expected.as_bytes());
            let line = format!("{} {}", request.method(), request.uri());
            async move {
                let response = if authorized {
                    next.run(request).await
                } else {
                    StatusCode::UNAUTHORIZED.into_response()
                };
                eprintln!("[mcp] {line} → {}", response.status());
                response
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/mcp", listener.local_addr()?);
    let until = stop.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(until.cancelled_owned())
            .await;
    });
    Ok(McpServer { url, token, stop })
}

#[derive(Clone)]
struct Handler {
    cluster: Arc<Cluster>,
    on_read: OnRead,
    tools: Arc<Vec<Tool>>,
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        // Claude Code's MCP 2026-07-28 client rejects a list without a
        // lifetime and cache scope, which rmcp leaves out unless set.
        Ok(ListToolsResult::with_all_items(self.tools.to_vec())
            .with_ttl_ms(3_600_000)
            .with_cache_scope(CacheScope::Private))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let params = Value::Object(request.arguments.unwrap_or_default());
        Ok(match tools::call(&self.cluster, &request.name, &params) {
            Ok((place, text)) => {
                (self.on_read)(&request.name, place);
                CallToolResult::success(vec![ContentBlock::text(text)])
            }
            Err(e) => CallToolResult::error(vec![ContentBlock::text(e)]),
        }
        .into())
    }
}
