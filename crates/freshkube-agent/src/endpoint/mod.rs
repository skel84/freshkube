//! The loopback read endpoint: MCP Streamable HTTP on `127.0.0.1`, one per
//! session, guarded by the session's bearer token.
//!
//! Each call is parsed by core's `Call::parse`, resolved to a target by the
//! app's [`Targets`], answered by `agent_reads::read` and given a read id. The
//! agent gets the answer as JSON text and as `structuredContent`; the app gets
//! a [`ReadEvent`] with the [`ViewState`] the window would show. The view never
//! goes on the wire.
//!
//! Dropping the [`Endpoint`] closes the port and kills the token.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use freshkube_core::agent_reads::{
    AgentAccess, AgentTarget, Call, Code, Cut, EntryId, Refusal, Request, TOOLS,
};
use freshkube_nav::ViewState;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
    Implementation, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
    Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::session::{ReadId, ReadIds, SessionId, Token};

mod guard;
mod tools;
mod view;

pub use freshkube_core::agent_reads::SERVER_NAME;

/// Resolves the cluster a call names to a target, in the app: `None` is the
/// session's own entry. An entry the session may not name answers `None`,
/// which the agent reads as `unknown_cluster`.
pub trait Targets: Send + Sync + 'static {
    fn resolve(&self, entry: Option<&EntryId>) -> Option<AgentTarget>;
}

/// How a read ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Answered, possibly cut.
    Answered { cut: Option<Cut> },
    /// Refused with a code.
    Refused(Code),
}

/// What the app is told of each read, refused ones included.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadEvent {
    pub session: SessionId,
    pub read: ReadId,
    /// One of the four tool names, or `unknown` for a call that named another.
    pub tool: &'static str,
    /// The entry the read was made on.
    pub cluster: String,
    /// Where the window would go; `None` for a refused read.
    pub view: Option<ViewState>,
    pub outcome: Outcome,
}

/// How long a connection may take to send its headers, and a request its
/// body, before it is dropped. A client on loopback sends both at once.
#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    pub header: Duration,
    pub body: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            header: Duration::from_secs(10),
            body: Duration::from_secs(10),
        }
    }
}

/// What the endpoint says of a cluster id the session cannot read, in place
/// of the agent's own text.
const UNKNOWN_CLUSTER: &str = "unknown";

struct Shared {
    /// Cancelled when the endpoint is dropped: a read in flight ends and
    /// publishes nothing.
    stop: CancellationToken,
    session: SessionId,
    /// The session's own entry.
    entry: String,
    access: AgentAccess,
    targets: Arc<dyn Targets>,
    events: UnboundedSender<ReadEvent>,
    ids: ReadIds,
    /// Whether the agent has asked for the tool list.
    listed: AtomicBool,
}

/// A running endpoint; dropping it stops the server and ends its token.
pub struct Endpoint {
    addr: SocketAddr,
    token: Token,
    session: SessionId,
    stop: CancellationToken,
    shared: Arc<Shared>,
}

impl Endpoint {
    /// Binds `127.0.0.1:0` and serves until dropped. `entry` is the session's
    /// own cluster; `access` is the gate, which this session reads through
    /// with a read budget of its own.
    pub async fn start(
        session: SessionId,
        entry: &str,
        access: &AgentAccess,
        targets: Arc<dyn Targets>,
        events: UnboundedSender<ReadEvent>,
    ) -> std::io::Result<Self> {
        Self::start_with(session, entry, access, targets, events, Timeouts::default()).await
    }

    /// As [`start`](Self::start), with the connection's time limits named.
    pub async fn start_with(
        session: SessionId,
        entry: &str,
        access: &AgentAccess,
        targets: Arc<dyn Targets>,
        events: UnboundedSender<ReadEvent>,
        timeouts: Timeouts,
    ) -> std::io::Result<Self> {
        let token = Token::generate()?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let stop = CancellationToken::new();
        let shared = Arc::new(Shared {
            stop: stop.clone(),
            session,
            entry: entry.to_owned(),
            access: access.for_session(),
            targets,
            events,
            ids: ReadIds::default(),
            listed: AtomicBool::new(false),
        });
        let handler = Handler {
            shared: Arc::clone(&shared),
            tools: Arc::new(tools::all()),
        };
        let service: StreamableHttpService<Handler, LocalSessionManager> =
            StreamableHttpService::new(
                move || Ok(handler.clone()),
                Default::default(),
                StreamableHttpServerConfig::default().with_cancellation_token(stop.clone()),
            );
        let guard = Arc::new(guard::Guard {
            token: token.clone(),
            host: addr.to_string(),
            ended: stop.clone(),
            body_timeout: timeouts.body,
        });
        let router = axum::Router::new()
            .nest_service("/mcp", service)
            .layer(axum::middleware::from_fn_with_state(guard, guard::check));
        tokio::spawn(serve(listener, router, stop.clone(), timeouts.header));
        Ok(Self {
            addr,
            token,
            session,
            stop,
            shared,
        })
    }

    /// Where the agent connects.
    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn session(&self) -> SessionId {
        self.session
    }

    /// Whether the agent has asked this endpoint for its tools. An agent
    /// that never did cannot have offered them to its model.
    pub fn listed(&self) -> bool {
        self.shared.listed.load(Ordering::SeqCst)
    }

    /// How many calls have reached the endpoint, refused ones included.
    pub fn calls(&self) -> u32 {
        self.shared.ids.count()
    }

    /// The session's token, to hand to the agent once.
    pub fn token(&self) -> &Token {
        &self.token
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint")
            .field("session", &self.session)
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

/// Accepts connections until `stop`, then closes the port and every
/// connection still open, so nothing outlives the endpoint. A connection that
/// is slow to send its headers is dropped.
async fn serve(
    listener: tokio::net::TcpListener,
    router: axum::Router,
    stop: CancellationToken,
    header_timeout: Duration,
) {
    loop {
        let accepted = tokio::select! {
            biased;
            () = stop.cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        let Ok((stream, _)) = accepted else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        let service = TowerToHyperService::new(router.clone());
        let stop = stop.clone();
        tokio::spawn(async move {
            let connection = hyper::server::conn::http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(header_timeout)
                .serve_connection(TokioIo::new(stream), service);
            tokio::select! {
                () = stop.cancelled() => {}
                _ = connection => {}
            }
        });
    }
}

#[derive(Clone)]
struct Handler {
    shared: Arc<Shared>,
    tools: Arc<Vec<Tool>>,
}

/// What a read comes to, before it is told to the agent and the app.
struct Done {
    cluster: String,
    target: String,
    view: Option<ViewState>,
    result: Result<(Value, Option<Cut>), Refusal>,
}

impl Shared {
    async fn run(&self, tool: &str, arguments: Value, read: ReadId) -> Done {
        let call = match Call::parse(tool, &arguments) {
            Ok(call) => call,
            Err(refusal) => {
                return Done {
                    cluster: self.entry.clone(),
                    target: "-".into(),
                    view: None,
                    result: Err(refusal),
                };
            }
        };
        let target_text = describe(&call.request);
        let Some(target) = self.targets.resolve(call.cluster.as_ref()) else {
            // The id is the agent's text: neither the log nor the app is
            // told it.
            return Done {
                cluster: UNKNOWN_CLUSTER.to_owned(),
                target: target_text,
                view: None,
                result: Err(Refusal::new(
                    Code::UnknownCluster,
                    "this session cannot read that cluster",
                )),
            };
        };
        match freshkube_core::agent_reads::read(&self.access, &target, &call.request).await {
            Ok(answer) => Done {
                cluster: target.entry,
                target: target_text,
                view: view::view_of(&answer.place),
                result: Ok((answer.body.wire(&read.to_string()), answer.body.cut())),
            },
            Err(refusal) => Done {
                cluster: target.entry,
                target: target_text,
                view: None,
                result: Err(refusal),
            },
        }
    }
}

/// What a request reads, from its checked fields only.
fn describe(request: &Request) -> String {
    let namespaced = |namespace: Option<&str>, name: &str| match namespace {
        Some(namespace) => format!("{namespace}/{name}"),
        None => name.to_owned(),
    };
    match request {
        Request::List(list) => format!(
            "{} {}",
            list.kind.as_str(),
            list.namespace.as_ref().map_or("*", |n| n.as_str())
        ),
        Request::Get(get) => format!(
            "{} {}",
            get.kind.as_str(),
            namespaced(
                get.namespace.as_ref().map(|n| n.as_str()),
                get.name.as_str()
            )
        ),
        Request::Logs(logs) => format!(
            "logs {}",
            namespaced(Some(logs.namespace.as_str()), logs.pod.as_str())
        ),
        Request::Events(events) => format!(
            "events {}",
            events.namespace.as_ref().map_or("*", |n| n.as_str())
        ),
    }
}

/// The tool's name as a log and an event may repeat it.
fn known_tool(tool: &str) -> &'static str {
    TOOLS
        .iter()
        .find(|known| **known == tool)
        .copied()
        .unwrap_or("unknown")
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
        self.shared.listed.store(true, Ordering::SeqCst);
        // An MCP 2026-07-28 client may reject a list without a lifetime and
        // cache scope, which rmcp leaves out unless set.
        Ok(ListToolsResult::with_all_items(self.tools.to_vec())
            .with_ttl_ms(3_600_000)
            .with_cache_scope(CacheScope::Private))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let shared = &self.shared;
        let read = shared.ids.next();
        let tool = known_tool(&request.name);
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        // A read ends with the endpoint or with the client's own cancel, and
        // then nothing is logged, told to the app or answered.
        let done = tokio::select! {
            biased;
            () = shared.stop.cancelled() => {
                return Err(ErrorData::internal_error("the endpoint is closed", None));
            }
            () = context.ct.cancelled() => {
                return Err(ErrorData::internal_error("the call was cancelled", None));
            }
            done = shared.run(&request.name, arguments, read) => done,
        };
        // A read that finished as the endpoint was dropped publishes nothing
        // either: not the log line, not the `ReadEvent`.
        if shared.stop.is_cancelled() {
            return Err(ErrorData::internal_error("the endpoint is closed", None));
        }
        let (wire, outcome, code) = match done.result {
            Ok((wire, cut)) => (wire, Outcome::Answered { cut }, "ok"),
            Err(refusal) => (
                json!({"read": read.to_string(),
                    "error": {"code": refusal.code.as_str(), "message": refusal.message()}}),
                Outcome::Refused(refusal.code),
                refusal.code.as_str(),
            ),
        };
        // One line per read: who, what and how it ended. No answer data, no
        // token, and nothing the agent typed that was refused.
        tracing::info!(
            session = %shared.session.short(),
            read = %read,
            entry = %done.cluster,
            tool,
            target = %done.target,
            code,
            "agent read"
        );
        let failed = matches!(outcome, Outcome::Refused(_));
        let _ = shared.events.send(ReadEvent {
            session: shared.session,
            read,
            tool,
            cluster: done.cluster,
            view: done.view,
            outcome,
        });
        let content = vec![ContentBlock::text(wire.to_string())];
        let mut result = if failed {
            CallToolResult::error(content)
        } else {
            CallToolResult::success(content)
        };
        result.structured_content = Some(wire);
        Ok(result.into())
    }
}
