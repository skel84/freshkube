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
