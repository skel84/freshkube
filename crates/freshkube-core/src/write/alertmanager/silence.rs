//! The draft of a silence, and the rules that keep a narrow one narrow. All
//! of it is pure: a dialog draws [`classify`]'s answer and decides nothing.
//!
//! A silence is **narrow** only when it can mute nothing but the alert it was
//! made from, as far as its matchers say: an equality `alertname`, at least
//! one other equality matcher on an identifying label, no regex and no `!=`,
//! at most [`MAX_MUTED_NARROW`] alerts muted now, a short life, and, in
//! central mode, the entry's own exact `cluster` matcher. Anything else is
//! **broad** and says why. Some drafts are refused outright ([`Refusal`]).
use std::time::Duration;

use super::super::Protection;
use super::writer::{Muted, Preview};
use crate::monitoring::alerts::{Alert, SilenceMatcher};

/// The most alerts a narrow silence may mute now.
pub const MAX_MUTED_NARROW: usize = 10;
/// The shortest silence.
pub const MIN_DURATION: Duration = Duration::from_secs(60);
/// The longest silence of any class.
pub const MAX_DURATION: Duration = Duration::from_secs(24 * 3600);
/// The longest narrow silence on a `protected` cluster; longer is broad.
pub const PROTECTED_CAP: Duration = Duration::from_secs(4 * 3600);
/// The default length of a new silence.
pub const DEFAULT_DURATION: Duration = Duration::from_secs(2 * 3600);
/// Matchers in one silence.
pub const MAX_MATCHERS: usize = 12;
/// Characters in a matcher's name or value.
pub const MAX_VALUE: usize = 256;
/// Characters in the comment.
pub const MAX_COMMENT: usize = 512;
/// Labels that don't identify one alert: a silence needs another one.
const NOT_IDENTIFYING: [&str; 1] = ["severity"];

/// How the open entry's alerts are told apart from other clusters'.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The cluster's own Alertmanager: every alert is this cluster's.
    Cluster,
    /// A shared Alertmanager, whose alerts carry `label` and this entry's
    /// `value`. A silence without the exact matcher mutes other clusters.
    Central { label: String, value: String },
}

/// A silence as a person is editing it. The end is `duration` after the
/// moment of sending, never a time the page chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub matchers: Vec<SilenceMatcher>,
    pub comment: String,
    pub duration: Duration,
    pub mode: Mode,
}

fn equal(name: &str, value: &str) -> SilenceMatcher {
    SilenceMatcher {
        name: name.to_owned(),
        value: value.to_owned(),
        regex: false,
        equal: true,
    }
}

impl Draft {
    /// The silence for one alert: every label as an equality matcher, never a
    /// regex, so a value with regex characters stays literal. In central
    /// mode the entry's exact cluster matcher is first.
    pub fn for_alert(alert: &Alert, mode: Mode) -> Self {
        let mut matchers = Vec::new();
        if let Mode::Central { label, value } = &mode {
            matchers.push(equal(label, value));
        }
        for (name, value) in &alert.labels {
            let is_cluster = matches!(&mode, Mode::Central { label, .. } if label == name);
            if !is_cluster {
                matchers.push(equal(name, value));
            }
        }
        Self {
            matchers,
            comment: String::new(),
            duration: DEFAULT_DURATION,
            mode,
        }
    }

    /// The offered fix in central mode: puts the entry's exact cluster
    /// matcher back (first), dropping any other matcher on that label.
    pub fn limit_to_cluster(&mut self) {
        if let Mode::Central { label, value } = &self.mode {
            self.matchers.retain(|matcher| &matcher.name != label);
            self.matchers.insert(0, equal(label, value));
        }
    }

    /// Whether the entry's own exact cluster matcher is there (always true
    /// outside central mode).
    pub fn has_exact_cluster(&self) -> bool {
        match &self.mode {
            Mode::Cluster => true,
            Mode::Central { label, value } => self.matchers.iter().any(|matcher| {
                &matcher.name == label && &matcher.value == value && matcher.equal && !matcher.regex
            }),
        }
    }
}

/// Why a silence is broad.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// No equality `alertname` matcher.
    NoAlertname,
    /// Only `alertname`: every instance of the rule.
    OnlyAlertname,
    /// A regex matcher, which can match more than it reads as.
    Regex,
    /// A `!=` matcher, which matches everything it doesn't name.
    Negative,
    /// More than [`MAX_MUTED_NARROW`] alerts muted now.
    TooManyMuted(usize),
    /// More alerts than the preview counts: not listed, so not narrow.
    MoreThanCounted(usize),
    /// Longer than [`PROTECTED_CAP`] on a protected cluster.
    PastProtectedCap,
    /// Central mode without the entry's exact cluster matcher: it can mute
    /// other clusters' alerts.
    NoExactCluster,
}

impl Reason {
    /// The words for the dialog.
    pub fn words(&self) -> String {
        match self {
            Self::NoAlertname => "It has no alertname matcher".into(),
            Self::OnlyAlertname => {
                "It matches only the alert's name, so every instance of it".into()
            }
            Self::Regex => "It uses a regex matcher".into(),
            Self::Negative => "It uses a != matcher".into(),
            Self::TooManyMuted(count) => format!("It mutes {count} alerts now"),
            Self::MoreThanCounted(bound) => {
                format!("It mutes more than the {bound} alerts the preview counts")
            }
            Self::PastProtectedCap => "It lasts longer than 4 h on a protected cluster".into(),
            Self::NoExactCluster => {
                "It has no exact cluster matcher, so it can mute other clusters' alerts".into()
            }
        }
    }
}

/// How a silence is classed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Class {
    Narrow,
    Broad(Vec<Reason>),
}

impl Class {
    pub fn is_narrow(&self) -> bool {
        matches!(self, Self::Narrow)
    }
}

/// Why a draft can't be sent at any class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The cluster is read-only.
    ReadOnly,
    NoMatchers,
    TooManyMatchers,
    EmptyName,
    /// A label name Alertmanager's filter and API would read as something
    /// else: only letters, digits and underscores, not starting with a digit.
    BadName(String),
    /// A matcher with an empty value, which matches alerts that lack the
    /// label as well.
    EmptyValue(String),
    ValueTooLong(String),
    /// A regex that matches every value of the label.
    MatchesEverything(String),
    EmptyComment,
    CommentTooLong,
    DurationTooShort,
    DurationTooLong,
}

impl Refusal {
    pub fn words(&self) -> String {
        match self {
            Self::ReadOnly => "This cluster is read-only.".into(),
            Self::NoMatchers => "A silence needs at least one matcher".into(),
            Self::TooManyMatchers => format!("A silence takes at most {MAX_MATCHERS} matchers"),
            Self::EmptyName => "A matcher needs a label name".into(),
            Self::BadName(name) => format!("{name} isn't a label name"),
            Self::EmptyValue(name) => format!("The matcher on {name} has no value"),
            Self::ValueTooLong(name) => {
                format!("The matcher on {name} is longer than {MAX_VALUE} characters")
            }
            Self::MatchesEverything(name) => format!("The regex on {name} matches every value"),
            Self::EmptyComment => "A silence needs a comment".into(),
            Self::CommentTooLong => format!("The comment is longer than {MAX_COMMENT} characters"),
            Self::DurationTooShort => "A silence lasts at least a minute".into(),
            Self::DurationTooLong => "A silence lasts at most 24 h".into(),
        }
    }
}

/// A Prometheus label name: ASCII letters, digits and underscores, not
/// starting with a digit. UTF-8 label names are refused.
fn is_label_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A regex that selects every value, written the ways people write it.
fn matches_everything(value: &str) -> bool {
    let core = value
        .strip_prefix('^')
        .unwrap_or(value)
        .strip_suffix('$')
        .unwrap_or_else(|| value.strip_prefix('^').unwrap_or(value));
    matches!(core, "" | ".*" | "(.*)" | ".*.*" | "(?s).*")
}

/// What no class can send. A regex that doesn't compile isn't looked for
/// here: it is always broad, and Alertmanager refuses it with its own reason
/// before anything is created.
pub fn validate(draft: &Draft) -> Result<(), Refusal> {
    if draft.matchers.is_empty() {
        return Err(Refusal::NoMatchers);
    }
    if draft.matchers.len() > MAX_MATCHERS {
        return Err(Refusal::TooManyMatchers);
    }
    for matcher in &draft.matchers {
        if matcher.name.trim().is_empty() {
            return Err(Refusal::EmptyName);
        }
        if !is_label_name(&matcher.name) {
            return Err(Refusal::BadName(matcher.name.clone()));
        }
        if matcher.name.chars().count() > MAX_VALUE || matcher.value.chars().count() > MAX_VALUE {
            return Err(Refusal::ValueTooLong(matcher.name.clone()));
        }
        if matcher.value.is_empty() {
            return Err(Refusal::EmptyValue(matcher.name.clone()));
        }
        if matcher.regex && matcher.equal && matches_everything(&matcher.value) {
            return Err(Refusal::MatchesEverything(matcher.name.clone()));
        }
    }
    let comment = draft.comment.trim();
    if comment.is_empty() {
        return Err(Refusal::EmptyComment);
    }
    if comment.chars().count() > MAX_COMMENT {
        return Err(Refusal::CommentTooLong);
    }
    if draft.duration < MIN_DURATION {
        return Err(Refusal::DurationTooShort);
    }
    if draft.duration > MAX_DURATION {
        return Err(Refusal::DurationTooLong);
    }
    Ok(())
}

/// How the draft is classed, from what it would mute now (`preview`, read
/// again just before the send) and the cluster's level. The level's own
/// block is the first refusal: nothing is classed under `read-only`.
pub fn classify(draft: &Draft, preview: &Preview, level: Protection) -> Result<Class, Refusal> {
    if level == Protection::ReadOnly {
        return Err(Refusal::ReadOnly);
    }
    validate(draft)?;
    let mut reasons = Vec::new();
    let exact = |name: &str| {
        draft
            .matchers
            .iter()
            .any(|matcher| matcher.name == name && matcher.equal && !matcher.regex)
    };
    if !exact("alertname") {
        reasons.push(Reason::NoAlertname);
    } else if !draft.matchers.iter().any(|matcher| {
        matcher.name != "alertname"
            // The central cluster matcher says which cluster, not which
            // instance: it doesn't narrow the rule's own alerts.
            && !matches!(&draft.mode, Mode::Central { label, .. } if *label == matcher.name)
            && !NOT_IDENTIFYING.contains(&matcher.name.as_str())
            && matcher.equal
            && !matcher.regex
    }) {
        reasons.push(Reason::OnlyAlertname);
    }
    if draft.matchers.iter().any(|matcher| matcher.regex) {
        reasons.push(Reason::Regex);
    }
    if draft.matchers.iter().any(|matcher| !matcher.equal) {
        reasons.push(Reason::Negative);
    }
    match preview.muted {
        Muted::Exactly(count) if count > MAX_MUTED_NARROW => {
            reasons.push(Reason::TooManyMuted(count))
        }
        Muted::Exactly(_) => {}
        Muted::AtLeast(bound) => reasons.push(Reason::MoreThanCounted(bound)),
    }
    if level == Protection::Protected && draft.duration > PROTECTED_CAP {
        reasons.push(Reason::PastProtectedCap);
    }
    if !draft.has_exact_cluster() {
        reasons.push(Reason::NoExactCluster);
    }
    Ok(if reasons.is_empty() {
        Class::Narrow
    } else {
        Class::Broad(reasons)
    })
}
