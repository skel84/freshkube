//! What a page says when a read has nothing to show (docs/DESIGN.md,
//! "Words"): the tool-neutral part of the change page's cause and next step.
//! A page maps what it already knows onto a [`ReadState`] and
//! [`read_state`] draws it through `empty_state` and `banner`, so the title,
//! the icon, the tone and which buttons show are decided in one place.
//!
//! Nothing here reads anything. A page that can't tell which state it is in
//! says [`ReadState::NotReadYet`] or an unknown of its own, never an empty list.

use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, App, ElementId, InteractiveElement as _, IntoElement as _, SharedString,
};

use crate::ui::{self, Tone};

/// Why a page has nothing to show, in words a first-time user can act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadState {
    /// An optional tool isn't on the cluster. `adds` finishes "it adds …".
    NotInstalled {
        tool: String,
        adds: String,
        cluster: String,
    },
    /// Freshkube doesn't know something it needs. `gap` is the plain
    /// sentence ("Freshkube doesn't know where builds run."); `setting` is
    /// the Settings section that changes it.
    NotSetUp {
        what: String,
        gap: String,
        setting: Option<String>,
    },
    /// The cluster refused the read. `manifest` is the read-only Role to ask
    /// an administrator for; `reason` is the server's own words.
    Refused {
        what: String,
        cluster: String,
        manifest: Option<String>,
        reason: Option<String>,
    },
    /// The read failed in some other way.
    Unreachable {
        what: String,
        cluster: String,
        reason: Option<String>,
    },
    /// The first read hasn't answered.
    NotReadYet { what: String },
    /// The read worked and nothing matched.
    Empty {
        looked_for: String,
        cluster: String,
        hint: Option<String>,
    },
    /// Data kept after a failed refresh; `at` is the time it was read.
    Stale { what: String, at: String },
}

/// The buttons a page can offer; [`read_state`] shows the ones the state
/// allows and drops the rest.
#[derive(Default)]
pub struct ReadActions {
    pub retry: Option<AnyElement>,
    pub open_settings: Option<AnyElement>,
    pub copy_access: Option<AnyElement>,
    pub docs: Option<AnyElement>,
    /// Buttons of the page's own, such as a candidate to use, after the rest.
    pub extra: Vec<AnyElement>,
    /// Details for the mono box, such as what was looked for; the state's
    /// own reason is shown when there are none.
    pub details: Option<String>,
}

impl ReadActions {
    /// The buttons that apply to a state.
    pub fn for_state(self, state: &ReadState) -> Vec<AnyElement> {
        let mut buttons = Vec::new();
        buttons.extend(self.open_settings);
        buttons.extend(
            self.copy_access
                .filter(|_| state.access_request().is_some()),
        );
        buttons.extend(self.retry.filter(|_| state.allows_retry()));
        buttons.extend(self.docs);
        buttons.extend(self.extra);
        buttons
    }
}

impl ReadState {
    /// The title: four to eight words, the meaning and not the page's name.
    pub fn title(&self) -> String {
        match self {
            Self::NotInstalled { tool, cluster, .. } => {
                format!("{tool} isn't installed on {cluster}")
            }
            Self::NotSetUp { what, .. } => format!("{what} isn't set up"),
            Self::Refused { what, .. } => format!("Not allowed to read {what}"),
            Self::Unreachable { what, .. } => format!("Couldn't reach {what}"),
            Self::NotReadYet { what } => format!("Haven't read {what} yet"),
            Self::Empty { looked_for, .. } => format!("No {looked_for} found"),
            Self::Stale { what, .. } => format!("Showing {what} as last seen"),
        }
    }

    /// What was checked, what was found, and what it means.
    pub fn description(&self) -> String {
        match self {
            Self::NotInstalled {
                tool,
                adds,
                cluster,
            } => format!(
                "Looked for {tool} in {cluster} and didn't find it. {tool} is optional: it adds {adds}. The rest of Freshkube works without it."
            ),
            Self::NotSetUp { gap, setting, .. } => match setting {
                Some(setting) => format!("{gap} Set it in Settings › {setting}."),
                None => gap.clone(),
            },
            Self::Refused { cluster, .. } => {
                format!("{cluster} refused the read. This says nothing about whether any exist.")
            }
            Self::Unreachable { cluster, .. } => {
                format!(
                    "Nothing is shown as missing. Freshkube couldn't get an answer from {cluster}."
                )
            }
            Self::NotReadYet { what } => format!("Freshkube hasn't finished reading {what}."),
            Self::Empty {
                looked_for,
                cluster,
                hint,
            } => {
                let found = format!("Looked for {looked_for} in {cluster} and found none.");
                match hint {
                    Some(hint) => format!("{found} {hint}"),
                    None => found,
                }
            }
            Self::Stale { at, .. } => format!("Read at {at}; the latest read failed."),
        }
    }

    /// The server's own words, for the mono box.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Refused { reason, .. } | Self::Unreachable { reason, .. } => reason.as_deref(),
            _ => None,
        }
    }

    /// An optional tool that is missing is a note. A refusal, a failure and
    /// stale data are warnings; the rest judge nothing.
    pub fn tone(&self) -> Tone {
        match self {
            Self::NotInstalled { .. } | Self::NotSetUp { .. } | Self::Empty { .. } => Tone::Info,
            Self::Refused { .. } | Self::Unreachable { .. } | Self::Stale { .. } => Tone::Warn,
            Self::NotReadYet { .. } => Tone::Waiting,
        }
    }

    /// Whether Retry makes sense: never after a refusal (asking again gets
    /// the same answer), and not for what isn't there to read.
    pub fn allows_retry(&self) -> bool {
        matches!(
            self,
            Self::Unreachable { .. } | Self::Stale { .. } | Self::NotReadYet { .. }
        )
    }

    /// The Role to ask an administrator for, for a refusal that has one.
    pub fn access_request(&self) -> Option<&str> {
        match self {
            Self::Refused { manifest, .. } => manifest.as_deref(),
            _ => None,
        }
    }

    fn icon(&self) -> IconName {
        match self {
            Self::Refused { .. } => IconName::Lock,
            Self::Unreachable { .. } => IconName::TriangleAlert,
            Self::NotReadYet { .. } => IconName::Loader,
            _ => IconName::Info,
        }
    }
}

/// Draws a state: a banner for stale data, otherwise an empty state. The
/// Retry button shows only where the state allows it.
pub fn read_state(
    id: impl Into<ElementId>,
    state: &ReadState,
    actions: ReadActions,
    cx: &App,
) -> AnyElement {
    let details = actions.details.clone();
    if let ReadState::Stale { .. } = state {
        let lead: SharedString = state.title().into();
        let retry = actions.retry.filter(|_| state.allows_retry());
        return ui::banner(state.tone(), Some(lead), state.description(), retry, cx)
            .id(id)
            .into_any_element();
    }
    let buttons = actions.for_state(state);
    ui::empty_state(
        state.icon(),
        state.title(),
        state.description(),
        details.or_else(|| state.reason().map(str::to_owned)),
        buttons,
        cx,
    )
    .id(id)
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn not_installed() -> ReadState {
        ReadState::NotInstalled {
            tool: "Prometheus".into(),
            adds: "charts of CPU and memory over time".into(),
            cluster: "prod-fra".into(),
        }
    }

    fn refused() -> ReadState {
        ReadState::Refused {
            what: "pods".into(),
            cluster: "prod-fra".into(),
            manifest: Some("kind: ClusterRole".into()),
            reason: Some("forbidden".into()),
        }
    }

    #[test]
    fn no_retry_after_a_refusal() {
        assert!(!refused().allows_retry());
        assert!(
            ReadState::Unreachable {
                what: "pods".into(),
                cluster: "prod-fra".into(),
                reason: None
            }
            .allows_retry()
        );
    }

    #[test]
    fn a_missing_optional_tool_is_a_note_not_a_warning() {
        let state = not_installed();
        assert_eq!(state.tone(), Tone::Info);
        assert!(!state.allows_retry());
        let text = state.description();
        assert!(text.contains("Looked for Prometheus in prod-fra"));
        assert!(text.contains("optional"));
        assert!(text.contains("works without it"));
    }

    #[test]
    fn refusal_keeps_apart_from_empty() {
        assert_eq!(refused().title(), "Not allowed to read pods");
        assert!(
            refused()
                .description()
                .contains("says nothing about whether any exist")
        );
        assert_eq!(refused().reason(), Some("forbidden"));
        assert_eq!(refused().access_request(), Some("kind: ClusterRole"));
        let empty = ReadState::Empty {
            looked_for: "applications".into(),
            cluster: "prod-fra".into(),
            hint: None,
        };
        assert_eq!(empty.title(), "No applications found");
        assert_eq!(empty.access_request(), None);
    }

    #[test]
    fn titles_are_four_to_eight_words_without_jargon() {
        for state in [
            not_installed(),
            refused(),
            ReadState::NotSetUp {
                what: "Where builds run".into(),
                gap: "Freshkube doesn't know where builds run.".into(),
                setting: Some("Delivery sources".into()),
            },
            ReadState::NotReadYet {
                what: "applications".into(),
            },
        ] {
            let words = state.title().split_whitespace().count();
            assert!((3..=8).contains(&words), "{}", state.title());
        }
    }

    #[test]
    fn not_set_up_points_at_settings() {
        let state = ReadState::NotSetUp {
            what: "Prometheus".into(),
            gap: "Freshkube doesn't know where Prometheus is.".into(),
            setting: Some("Monitoring".into()),
        };
        assert!(
            state
                .description()
                .ends_with("Set it in Settings › Monitoring.")
        );
    }

    #[test]
    fn stale_is_a_warning_that_can_retry() {
        let state = ReadState::Stale {
            what: "pods".into(),
            at: "14:02:11".into(),
        };
        assert_eq!(state.tone(), Tone::Warn);
        assert!(state.allows_retry());
    }
}

#[cfg(test)]
mod actions_tests {
    use super::*;
    use gpui_kit::div;

    fn buttons() -> ReadActions {
        ReadActions {
            retry: Some(div().into_any_element()),
            open_settings: Some(div().into_any_element()),
            copy_access: Some(div().into_any_element()),
            ..ReadActions::default()
        }
    }

    #[test]
    fn a_refusal_drops_retry_and_keeps_the_access_request() {
        let state = ReadState::Refused {
            what: "pods".into(),
            cluster: "prod-fra".into(),
            manifest: Some("kind: ClusterRole".into()),
            reason: None,
        };
        // Open Settings and Copy access request; no Retry.
        assert_eq!(buttons().for_state(&state).len(), 2);
    }

    #[test]
    fn a_failure_keeps_retry() {
        let state = ReadState::Unreachable {
            what: "pods".into(),
            cluster: "prod-fra".into(),
            reason: None,
        };
        // Open Settings and Retry; no access request to copy.
        assert_eq!(buttons().for_state(&state).len(), 2);
    }
}
