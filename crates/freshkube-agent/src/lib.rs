//! The agent in the dock, without GPUI and without anything of desktop.
//!
//! - [`acp`]: the ACP client over a child's stdio. It starts the agent, hands
//!   it Freshkube's MCP endpoint as a URL and a token, and refuses every tool
//!   that isn't one of Freshkube's four reads.
//! - [`profile`]: what an agent profile is, the opencode profile with its
//!   lock-down, where opencode is found and which versions are supported.
//! - [`investigation`]: the transcript the agent's session updates become,
//!   bounded in every dimension.
//! - `scripted` (feature `scripted`): an ACP agent that follows a script, for tests and for the
//!   fixture; in process over a pipe, or as the `fake_acp_agent` binary.
//!
//! Docs: `docs/features/agent.md`.

pub mod acp;
mod child;
pub mod investigation;
pub mod profile;
#[cfg(feature = "scripted")]
pub mod scripted;
pub mod tools;

pub use child::Link;

/// Log directives that keep the ACP and MCP libraries at info. At debug they
/// log every message they send, and `session/new` carries the session's
/// token in a header. The app's logger adds these whatever its level.
pub const QUIET_LOG_TARGETS: [&str; 2] = ["agent_client_protocol=info", "rmcp=info"];

/// Whether a log target belongs to the ACP or MCP library. A logger that
/// drops these libraries' events below info (whatever the filter says: a
/// more specific `RUST_LOG` target such as `agent_client_protocol::jsonrpc`
/// beats a directive) keeps the token out of the log. `src/main.rs` has the
/// same predicate and the same clamp.
pub fn is_agent_library_target(target: &str) -> bool {
    let crate_name = target.split("::").next().unwrap_or(target);
    crate_name == "rmcp" || crate_name.starts_with("agent_client_protocol")
}
