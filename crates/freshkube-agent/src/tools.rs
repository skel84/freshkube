//! Freshkube's four reads, by the names the agent sees.
//!
//! This is a copy of the endpoint's list (the design comment on the agent
//! issue fixes the names) until `agent_reads::TOOLS` exists; the slice that
//! lands second switches to it.

/// The MCP server the app serves, as the agent names it.
pub const SERVER: &str = "freshkube";

/// The four tools of that server, and nothing else.
pub const TOOLS: [&str; 4] = ["list_objects", "get_object", "pod_logs", "list_events"];
