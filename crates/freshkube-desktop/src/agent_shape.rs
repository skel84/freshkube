//! `FRESHKUBE_AGENT`: what the Agent dock tab's example shows with
//! `--fixture`. The tab runs the scripted agent in process instead of
//! opencode; a shape picks the setup state or a point in a session, so a
//! capture or a test can open the tab there.

/// One shape of the Agent tab's example.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentShape {
    /// opencode is found and supported, access is off and nothing is asked
    /// yet (the default).
    #[default]
    Ready,
    /// opencode is not found: the install command, Check again and Choose a
    /// file.
    NotInstalled,
    /// opencode is below the supported range: Start is off.
    Unsupported,
    /// opencode is newer than the version checked: a warning, Start is on.
    Newer,
    /// opencode reports no sign-in: `opencode auth login` is shown.
    SignedOut,
    /// A question is asked with access off, so the egress card shows.
    AccessOff,
    /// Access is on and a read is held mid-way, so a step shows as running.
    Running,
    /// Access is on and the question was answered.
    Answered,
    /// Access is on, the question was asked and Stop pressed.
    Stopped,
    /// Access is on and the question is asked; the example agent reads a
    /// place every two seconds, so the page follows it: the list, the
    /// Deployment, a pod's log in the follow-owned dock tab, and the events.
    Following,
    /// Access is on; the trail shows a refused shell, a refused read and a
    /// read on another cluster.
    Refused,
    /// Access was turned off and the save failed: the tab says access comes
    /// back after a restart.
    SaveFailed,
    /// Access is on and the agent answers without asking for Freshkube's
    /// tools, so the tab says they weren't offered.
    NoTools,
    /// Access is on and the question's turn failed with the provider's message.
    TurnFailed,
    /// `data: shared` is set: the dock says investigations land in the
    /// person's own opencode history.
    Shared,
}

impl AgentShape {
    /// The names `FRESHKUBE_AGENT` takes.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const NAMES: [&'static str; 15] = [
        "ready",
        "not-installed",
        "unsupported",
        "newer",
        "signed-out",
        "access-off",
        "running",
        "answered",
        "stopped",
        "following",
        "refused",
        "save-failed",
        "no-tools",
        "turn-failed",
        "shared",
    ];

    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "ready" => Self::Ready,
            "not-installed" => Self::NotInstalled,
            "unsupported" => Self::Unsupported,
            "newer" => Self::Newer,
            "signed-out" => Self::SignedOut,
            "access-off" => Self::AccessOff,
            "running" => Self::Running,
            "answered" => Self::Answered,
            "stopped" => Self::Stopped,
            "following" => Self::Following,
            "refused" => Self::Refused,
            "save-failed" => Self::SaveFailed,
            "no-tools" => Self::NoTools,
            "turn-failed" => Self::TurnFailed,
            "shared" => Self::Shared,
            _ => return None,
        })
    }

    /// Whether the shape grants access before it starts.
    pub fn grants(self) -> bool {
        matches!(
            self,
            Self::Running
                | Self::Answered
                | Self::Stopped
                | Self::Following
                | Self::Refused
                | Self::SaveFailed
                | Self::NoTools
                | Self::TurnFailed
        )
    }

    /// Whether opening the tab asks the example question at once.
    pub fn asks(self) -> bool {
        matches!(
            self,
            Self::AccessOff
                | Self::Running
                | Self::Answered
                | Self::Stopped
                | Self::Following
                | Self::Refused
                | Self::NoTools
                | Self::TurnFailed
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_parses_to_its_own_shape() {
        let shapes: Vec<_> = AgentShape::NAMES
            .iter()
            .map(|name| AgentShape::parse(name).unwrap())
            .collect();
        for (ix, shape) in shapes.iter().enumerate() {
            assert!(!shapes[ix + 1..].contains(shape));
        }
        assert_eq!(AgentShape::parse("Ready"), None);
        assert_eq!(AgentShape::parse(""), None);
    }
}
