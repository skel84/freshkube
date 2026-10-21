//! What the session alert read knows, and the one place that decides what
//! it may say. An alert source that wasn't read, is stale or refused is
//! Unknown, never "no alerts": only a current read with no active alerts is
//! quiet, and a PrometheusRule alone says nothing of health.
use std::time::Duration;

use super::model::{Alert, Severity};
use super::read::Active;

/// How often the session reads the active alerts.
pub const POLL: Duration = Duration::from_secs(60);

/// Why nothing is known of the alerts, for the page to say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unknown {
    /// No Alertmanager or vmalert answered; `LOOKED_FOR` says what was tried.
    NotFound(String),
    /// The identity may not reach it (`get` on `services/proxy`, or a
    /// URL's token).
    Refused(String),
    /// It was found but didn't answer.
    Unread(String),
    /// The central store's alerts carry no such label, so none can be told
    /// to be this cluster's. Nothing is shown.
    NoClusterLabel(String),
    /// Only PrometheusRule objects: definitions nobody here evaluates.
    RulesOnly,
}

impl Unknown {
    /// The words for the state, as the page shows them.
    pub fn words(&self) -> String {
        match self {
            Self::NotFound(looked) => format!("No Alertmanager or vmalert found: {looked}"),
            Self::Refused(why) => format!("Can't read alerts: {why}"),
            Self::Unread(why) => format!("Couldn't read alerts: {why}"),
            Self::NoClusterLabel(label) => format!(
                "The central store's alerts have no `{label}` label, so none can be \
                 shown for this cluster"
            ),
            Self::RulesOnly => "Only PrometheusRule objects were found, which say nothing \
                 of whether a rule is healthy or firing"
                .into(),
        }
    }
}

/// What the session read last, for one source and one context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Nothing read yet.
    Reading,
    /// The last read, which succeeded.
    Known(Active),
    /// The last read failed; what was known before is kept as evidence and
    /// never counts as quiet.
    Stale {
        last: Active,
        why: String,
    },
    Unknown(Unknown),
}

/// How many active alerts of each severity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub critical: usize,
    pub warning: usize,
    pub other: usize,
}

impl Counts {
    pub fn of(alerts: &[Alert]) -> Self {
        let mut counts = Self::default();
        for alert in alerts.iter().filter(|alert| alert.state.is_active()) {
            match alert.severity {
                Severity::Critical => counts.critical += 1,
                Severity::Warning => counts.warning += 1,
                Severity::Info | Severity::Other => counts.other += 1,
            }
        }
        counts
    }

    pub fn total(self) -> usize {
        self.critical + self.warning + self.other
    }
}

/// What the alerts say of the cluster's health.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Health {
    /// Not known, and why. Never read as quiet.
    Unknown(String),
    /// A current read found nothing active, naming what was read.
    Quiet(String),
    Alerting(Counts),
}

impl State {
    /// The next state after a read. A success is the state; a failure keeps
    /// what was known as stale evidence, except a refusal, after which
    /// nothing is vouched for.
    pub fn after(self, read: Result<Active, Unknown>) -> Self {
        match read {
            Ok(active) => Self::Known(active),
            Err(Unknown::Unread(why)) => match self {
                Self::Known(last) | Self::Stale { last, .. } => Self::Stale { last, why },
                _ => Self::Unknown(Unknown::Unread(why)),
            },
            Err(unknown) => Self::Unknown(unknown),
        }
    }

    pub fn health(&self) -> Health {
        match self {
            Self::Reading => Health::Unknown("Alerts haven't been read yet".into()),
            Self::Unknown(unknown) => Health::Unknown(unknown.words()),
            Self::Stale { why, .. } => Health::Unknown(format!(
                "The alerts couldn't be read again ({why}); what was read before is stale"
            )),
            Self::Known(active) => {
                let counts = Counts::of(&active.alerts);
                if counts.total() > 0 || active.more > 0 {
                    Health::Alerting(counts)
                } else {
                    Health::Quiet(format!(
                        "No active alerts in {}",
                        active
                            .read
                            .iter()
                            .map(|from| from.words())
                            .collect::<Vec<_>>()
                            .join(" and ")
                    ))
                }
            }
        }
    }

    /// The alerts a screen may show as current: only a current read's.
    pub fn current(&self) -> Option<&Active> {
        match self {
            Self::Known(active) => Some(active),
            _ => None,
        }
    }

    /// The alerts of the last read, current or stale, for a screen that
    /// marks stale evidence as such.
    pub fn evidence(&self) -> Option<&Active> {
        match self {
            Self::Known(active) | Self::Stale { last: active, .. } => Some(active),
            _ => None,
        }
    }

    /// The firing and pending alerts at least as urgent as warnings.
    pub fn urgent(&self) -> impl Iterator<Item = &Alert> {
        self.evidence()
            .into_iter()
            .flat_map(|active| active.alerts.iter())
            .filter(|alert| {
                alert.state.is_active()
                    && matches!(alert.severity, Severity::Critical | Severity::Warning)
            })
    }

    /// Whether the rail's Monitoring mark shows: a current read with a
    /// critical or warning alert. Unknown has its own mark, never this one.
    pub fn marks_problem(&self) -> bool {
        matches!(self.health(), Health::Alerting(counts) if counts.critical + counts.warning > 0)
    }
}
