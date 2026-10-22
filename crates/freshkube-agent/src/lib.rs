//! The agent's way to the cluster (`docs/features/agent.md`): a loopback read
//! endpoint, the session token that guards it, and the per-cluster access a
//! person grants.
//!
//! No GPUI, and nothing of desktop, `freshkube-ui` or the feature crates. The
//! reads themselves are core's [`freshkube_core::agent_reads`]; this crate
//! serves them over MCP and tells the app what each one looked at.
//!
//! - [`endpoint`]: the MCP Streamable HTTP server, one per session, and the
//!   [`Targets`] the app implements.
//! - [`session`]: [`SessionId`], [`ReadId`] and the [`Token`].
//! - [`saved`]: `agent.json` and its audit line, behind [`Access`].

pub mod endpoint;
pub mod saved;
pub mod session;

pub use endpoint::{Endpoint, Outcome, ReadEvent, Targets, Timeouts};
pub use saved::Access;
pub use session::{ReadId, SessionId, Token};
