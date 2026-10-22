//! What the alert sources say, in one shape: alerts, rule groups and
//! silences, whichever server they came from. No UI types.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

/// How urgent an alert says it is, from its `severity` label. Declared most
/// urgent first, so sorting puts critical alerts first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Critical,
    Warning,
    Info,
    /// No `severity` label, or one this doesn't know.
    Other,
}

impl Severity {
    pub fn parse(label: Option<&str>) -> Self {
        match label.map(str::to_ascii_lowercase).as_deref() {
            Some("critical" | "crit" | "page" | "error") => Self::Critical,
            Some("warning" | "warn") => Self::Warning,
            Some("info" | "none") => Self::Info,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Critical => "Critical",
            Self::Warning => "Warning",
            Self::Info => "Info",
            Self::Other => "No severity",
        }
    }
}

/// Where an alert stands. Pending alerts come from vmalert and Prometheus
/// rules; Alertmanager only holds firing ones, which it may mute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AlertState {
    Firing,
    Pending,
    Silenced,
    Inhibited,
}

impl AlertState {
    /// Firing or pending: the alerts that count as trouble.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Firing | Self::Pending)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Firing => "Firing",
            Self::Pending => "Pending",
            Self::Silenced => "Silenced",
            Self::Inhibited => "Inhibited",
        }
    }
}

/// Which kind of server an alert or a rule was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Alertmanager,
    Vmalert,
    /// A PrometheusRule object: a definition, not a server's answer.
    PrometheusRule,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Alertmanager => "Alertmanager",
            Self::Vmalert => "vmalert",
            Self::PrometheusRule => "PrometheusRule",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    pub name: String,
    pub state: AlertState,
    pub severity: Severity,
    pub labels: BTreeMap<String, String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub runbook: Option<String>,
    /// Since when it has been in this state, as the source reports it.
    pub since: Option<DateTime<Utc>>,
    /// The source's id for it; alerts with one are deduplicated by it.
    pub fingerprint: String,
    /// The silences that mute it, by id.
    pub silenced_by: Vec<String>,
    pub inhibited_by: Vec<String>,
    pub source: Kind,
}

impl Alert {
    pub fn label(&self, name: &str) -> Option<&str> {
        self.labels.get(name).map(String::as_str)
    }
}

/// A rule's health as its server reports it. A definition nobody evaluates
/// is [`RuleHealth::Unknown`], never Ok.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RuleHealth {
    /// The last evaluation failed.
    Error,
    Unknown,
    Ok,
}

impl RuleHealth {
    pub fn parse(health: Option<&str>) -> Self {
        match health {
            Some("ok") => Self::Ok,
            Some("err" | "error") => Self::Error,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::Unknown => "Unknown",
            Self::Ok => "OK",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RuleKind {
    Alerting,
    Recording,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub name: String,
    pub kind: RuleKind,
    pub health: RuleHealth,
    pub last_error: Option<String>,
    pub last_evaluation: Option<DateTime<Utc>>,
    pub query: String,
    /// An alerting rule's alerts now, firing or pending.
    pub alerts: usize,
    pub firing: bool,
    pub source: Kind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RuleGroup {
    pub name: String,
    /// Where the group is defined: a file, or a PrometheusRule's
    /// `namespace/name`.
    pub origin: Option<String>,
    pub interval_seconds: Option<f64>,
    pub last_evaluation: Option<DateTime<Utc>>,
    pub rules: Vec<Rule>,
}

impl RuleGroup {
    /// The rules whose last evaluation failed.
    pub fn broken(&self) -> usize {
        self.rules
            .iter()
            .filter(|rule| rule.health == RuleHealth::Error)
            .count()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SilenceState {
    Active,
    Pending,
    Expired,
}

impl SilenceState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Pending => "Pending",
            Self::Expired => "Expired",
        }
    }
}

/// One matcher of a silence: `name`, an operator and a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SilenceMatcher {
    pub name: String,
    pub value: String,
    pub regex: bool,
    pub equal: bool,
}

impl SilenceMatcher {
    /// As Alertmanager writes it: `name="value"`, `!=`, `=~`, `!~`.
    pub fn text(&self) -> String {
        let operator = match (self.equal, self.regex) {
            (true, false) => "=",
            (false, false) => "!=",
            (true, true) => "=~",
            (false, true) => "!~",
        };
        format!("{}{operator}\"{}\"", self.name, self.value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Silence {
    pub id: String,
    pub state: SilenceState,
    pub matchers: Vec<SilenceMatcher>,
    pub created_by: String,
    pub comment: String,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
}

impl Silence {
    /// The alerts it mutes now: those the source says it silences, which is
    /// what Alertmanager decided, not a guess from the matchers.
    pub fn muted<'a>(&self, alerts: &'a [Alert]) -> Vec<&'a Alert> {
        alerts
            .iter()
            .filter(|alert| alert.silenced_by.contains(&self.id))
            .collect()
    }
}
