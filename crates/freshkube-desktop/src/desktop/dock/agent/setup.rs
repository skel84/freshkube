//! What the setup card knows about opencode: whether it is found and its
//! version, from D1b's check, and the commands to show when it is not.

use std::path::PathBuf;

use freshkube_agent::profile::{Checked, Data, Host, Profile, Version, VersionState, locate};

/// The command a person runs to install opencode; Freshkube never does.
pub(crate) const INSTALL_COMMAND: &str = "curl -fsSL https://opencode.ai/install | bash";
/// Where #945 is tracked.
pub(crate) const ISSUE_945: &str = "https://github.com/skel84/freshkube/issues/945";
/// The read-only identity how-to.
pub(crate) const HOW_TO: &str =
    "https://github.com/skel84/freshkube/blob/main/docs/features/agent.md#a-read-only-identity";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Setup {
    /// The check is running.
    Checking,
    /// Found, supported.
    Ready {
        version: String,
    },
    /// Newer than the version checked: a warning, and Start is on.
    Newer {
        found: String,
        checked: String,
    },
    /// Below the supported range, or not a version: Start is off.
    Unsupported {
        found: String,
        need: String,
    },
    NotInstalled,
    /// opencode reports no sign-in.
    SignedOut {
        version: String,
    },
}

impl Setup {
    pub(crate) fn can_start(&self) -> bool {
        matches!(
            self,
            Self::Ready { .. } | Self::Newer { .. } | Self::SignedOut { .. }
        )
    }

    pub(crate) fn from_state(state: &VersionState) -> Self {
        match state {
            VersionState::NotInstalled => Self::NotInstalled,
            VersionState::Unsupported { found, need } => Self::Unsupported {
                found: found.clone(),
                need: need.clone(),
            },
            VersionState::Newer { found, checked } => Self::Newer {
                found: show(*found),
                checked: show(*checked),
            },
            VersionState::Supported { found } => Self::Ready {
                version: show(*found),
            },
        }
    }
}

fn show(version: Version) -> String {
    version.to_string()
}

/// A passing check and the profile it is for, kept to start with.
#[derive(Clone)]
pub(crate) struct Found {
    pub(crate) profile: Profile,
    pub(crate) checked: Checked,
}

/// Looks for opencode (a chosen file first) and checks its version. Runs on
/// Tokio: it starts `opencode --version` with the locked environment.
pub(crate) async fn check(chosen: Option<PathBuf>, data: Data) -> (Setup, Option<Found>) {
    let host = Host::current();
    let Some(binary) = locate(chosen.as_deref(), &host.search_dirs()) else {
        return (Setup::NotInstalled, None);
    };
    let profile = Profile::opencode(binary, data);
    match profile.check(&host).await {
        Ok(checked) => {
            let setup = checked
                .state()
                .map(Setup::from_state)
                .unwrap_or(Setup::NotInstalled);
            (setup, Some(Found { profile, checked }))
        }
        Err(state) => (Setup::from_state(&state), None),
    }
}

/// The example's setup state for `FRESHKUBE_AGENT`.
pub(crate) fn example(shape: crate::agent_shape::AgentShape) -> Setup {
    use crate::agent_shape::AgentShape::*;
    let version = || "2.1.3".to_owned();
    match shape {
        NotInstalled => Setup::NotInstalled,
        Unsupported => Setup::Unsupported {
            found: "1.4.2".into(),
            need: ">= 2.1.0, < 3.0.0".into(),
        },
        Newer => Setup::Newer {
            found: "2.4.0".into(),
            checked: version(),
        },
        SignedOut => Setup::SignedOut { version: version() },
        _ => Setup::Ready { version: version() },
    }
}

/// The upgrade command shown beside an unsupported version.
pub(crate) const UPGRADE_COMMAND: &str = "opencode upgrade";
