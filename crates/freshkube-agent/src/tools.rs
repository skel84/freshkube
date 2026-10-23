//! Freshkube's four reads, by the names the agent sees: the endpoint's own
//! list (`freshkube_core::agent_reads`), so the allow-list can't drift from it.

pub use freshkube_core::agent_reads::{SERVER_NAME as SERVER, TOOLS};
