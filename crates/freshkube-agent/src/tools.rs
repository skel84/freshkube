//! Freshkube's four reads, by the names the agent sees: the endpoint's own
//! list (`freshkube_core::agent_reads`), so the allow-list can't drift from it.

pub use freshkube_core::agent_reads::{SERVER_NAME as SERVER, TOOLS};

/// The tool Code Mode offers in place of an MCP server's own. The profile
/// denies it; an agent that asks for it anyway has been refused it.
pub const EXECUTE: &str = "execute";

/// Why Freshkube's tools were not offered to the agent's model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotOffered {
    /// The agent never asked the endpoint for its tools.
    NeverListed,
    /// It listed them, made no read, and asked for `execute`, which was
    /// refused.
    ExecuteRefused,
}

/// Whether an answered turn shows the tools weren't offered, from what the
/// endpoint and the permission gate saw, never from the model's text. ACP
/// carries no list of the model's tools, so this needs a session with no
/// read at all: one read, of any of the four, proves they were offered.
///
/// Not caught: an agent that lists the tools and reads nothing without
/// asking for `execute`. Under the pinned opencode that is what a plugin
/// that didn't load looks like (its `*` deny means `execute` is never
/// offered, so there is no request to refuse), and it is also a model that
/// chose not to read. The tab stays silent on both (#967).
pub fn not_offered(listed: bool, reads: u32, execute_refused: u32) -> Option<NotOffered> {
    if reads > 0 {
        None
    } else if !listed {
        Some(NotOffered::NeverListed)
    } else if execute_refused > 0 {
        Some(NotOffered::ExecuteRefused)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_that_never_listed_the_tools_has_not_been_offered_them() {
        assert_eq!(not_offered(false, 0, 0), Some(NotOffered::NeverListed));
        assert_eq!(not_offered(false, 0, 3), Some(NotOffered::NeverListed));
    }

    #[test]
    fn listing_and_being_refused_execute_without_a_read_says_so() {
        assert_eq!(not_offered(true, 0, 1), Some(NotOffered::ExecuteRefused));
    }

    #[test]
    fn one_read_of_any_tool_says_the_tools_were_offered() {
        // Only some of the four used, with or without a refusal beside it.
        assert_eq!(not_offered(true, 1, 0), None);
        assert_eq!(not_offered(true, 1, 2), None);
        assert_eq!(not_offered(false, 1, 0), None);
    }

    #[test]
    fn an_answer_with_no_reads_and_no_refused_execute_says_nothing() {
        assert_eq!(not_offered(true, 0, 0), None);
    }
}
