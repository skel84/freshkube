//! What the three pages show, derived once when the data, the filter or the
//! toggle changes: rows, groups, the notes above the table and the state
//! that replaces it. Plain data, so a test reads it without drawing.
use chrono::{DateTime, Utc};
use freshkube_core::monitoring::alerts::{
    Alert, AlertState, AlertsData, Kind, LOOKED_FOR, Rule, RuleGroup, RuleHealth, RuleKind,
    RulesData, Severity, Silence, SilenceState, SilencesData, Unknown,
};
use freshkube_ui::ui::Tone;
use gpui_kit::SharedString;

use super::{Answer, Data, Which};

/// How many label chips a row shows before `+N`.
const CHIPS: usize = 3;

/// One cell: its words, and how they are set.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub text: SharedString,
    pub mono: bool,
    pub muted: bool,
}

impl Cell {
    pub fn plain(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            mono: false,
            muted: false,
        }
    }

    pub fn mono(text: impl Into<SharedString>) -> Self {
        Self {
            mono: true,
            ..Self::plain(text)
        }
    }

    pub fn muted(text: impl Into<SharedString>) -> Self {
        Self {
            muted: true,
            ..Self::plain(text)
        }
    }
}

/// The label chips of an alert row: `name=value`, and how many more.
#[derive(Clone, Debug, PartialEq)]
pub struct Chips {
    pub shown: Vec<SharedString>,
    pub more: usize,
}

/// A row's selection in the Inspector.
#[derive(Clone, Debug, PartialEq)]
pub struct Detail {
    pub title: SharedString,
    pub tone: Tone,
    pub state: SharedString,
    pub fields: Vec<(&'static str, SharedString, bool)>,
    pub summary: Option<SharedString>,
    pub description: Option<SharedString>,
    /// Every label, as the chips the Inspector draws.
    pub labels: Vec<(SharedString, SharedString)>,
    /// The runbook, when it is a web address.
    pub runbook: Option<SharedString>,
    /// Matchers of a silence, one per line.
    pub matchers: Vec<SharedString>,
    /// The alerts a silence mutes now, by name.
    pub muted: Vec<SharedString>,
    /// A rule's expression.
    pub query: Option<SharedString>,
    pub error: Option<SharedString>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub key: SharedString,
    pub tone: Tone,
    /// What the glyph says, which is also the row's accessible label.
    pub state: SharedString,
    pub label: SharedString,
    pub tooltip: SharedString,
    pub cells: Vec<Cell>,
    pub chips: Option<Chips>,
    pub group: usize,
    /// The lower-case text a filter matches.
    pub haystack: String,
    pub detail: Detail,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub key: SharedString,
    pub tone: Tone,
    pub label: SharedString,
    pub subject: Option<SharedString>,
    pub detail: Vec<String>,
}

/// One line of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Group(usize),
    Row(usize),
}

/// A note above the table.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub id: &'static str,
    pub tone: Tone,
    pub lead: Option<SharedString>,
    pub text: SharedString,
}

/// The state that replaces the table.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub id: &'static str,
    pub icon: StateIcon,
    pub title: SharedString,
    pub description: SharedString,
    pub detail: Option<SharedString>,
    pub retry: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateIcon {
    Quiet,
    Refused,
    Missing,
    Broken,
    Filtered,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    /// Nothing read yet.
    Loading,
    Table,
    State(State),
}

/// What a page derives from its answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    pub body: Body,
    pub rows: Vec<Row>,
    pub groups: Vec<Group>,
    pub lines: Vec<Entry>,
    pub notes: Vec<Note>,
    /// The status bar's parts: words and a tone.
    pub status: Vec<(String, Option<Tone>)>,
    /// Rows hidden by the filter or the toggle, for the footer's count.
    pub hidden: usize,
    pub total: usize,
    /// Whether the table has a cluster column.
    pub central: bool,
}

impl Display {
    pub fn loading() -> Self {
        Self {
            body: Body::Loading,
            rows: Vec::new(),
            groups: Vec::new(),
            lines: Vec::new(),
            notes: Vec::new(),
            status: Vec::new(),
            hidden: 0,
            total: 0,
            central: false,
        }
    }

    fn state(state: State) -> Self {
        Self {
            body: Body::State(state),
            ..Self::loading()
        }
    }
}

/// What decides which rows show.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    /// Lower-case text a row contains.
    pub query: String,
    /// Silenced and inhibited alerts show too.
    pub muted: bool,
}

/// The display of `answer` for `which`. `stale` is the words of a refresh
/// that failed over this answer.
pub fn derive(
    which: Which,
    answer: Option<&Answer>,
    stale: Option<&str>,
    filter: &Filter,
    central: Option<&str>,
    now: DateTime<Utc>,
) -> Display {
    let Some(answer) = answer else {
        return Display::loading();
    };
    let mut display = match answer {
        Answer::Unknown(unknown) => return Display::state(unknown_state(which, unknown)),
        Answer::Read(Data::Alerts(data)) => alerts(data, filter, central, now),
        Answer::Read(Data::Rules(data)) => rules(data, filter, now),
        Answer::Read(Data::Silences(data)) => silences(data, filter, now),
    };
    if let Some(why) = stale {
        display.notes.insert(
            0,
            Note {
                id: "stale",
                tone: Tone::Warn,
                lead: Some("Couldn't read again".into()),
                text: format!("Showing the last read, which may be out of date. {why}").into(),
            },
        );
        display.status.push(("last read".into(), Some(Tone::Warn)));
    }
    display
}

/// The state for an answer that says nothing is known, in the page's words.
fn unknown_state(which: Which, unknown: &Unknown) -> State {
    let what = match which {
        Which::Alerts => "alerts",
        Which::Rules => "rules",
        Which::Silences => "silences",
    };
    match unknown {
        Unknown::NotFound(reason) => State {
            id: "alerts-not-found",
            icon: StateIcon::Missing,
            title: match which {
                Which::Alerts => "No Alertmanager or vmalert found".into(),
                Which::Rules => "No rules found".into(),
                Which::Silences => "No Alertmanager found".into(),
            },
            description: format!(
                "Looked for {LOOKED_FOR}. Nothing answered, so nothing is known of {what}: \
                 this isn't an empty list."
            )
            .into(),
            detail: Some(reason.clone().into()),
            retry: true,
        },
        Unknown::Refused(reason) => State {
            id: "alerts-refused",
            icon: StateIcon::Refused,
            title: format!("Can't read {what}").into(),
            description: "Alertmanager and vmalert are reached through the Kubernetes API's \
                          service proxy, and it refused this one. A refused read isn't an \
                          empty one."
                .into(),
            detail: Some(reason.clone().into()),
            retry: true,
        },
        Unknown::Unread(reason) => State {
            id: "alerts-failed",
            icon: StateIcon::Broken,
            title: format!("Couldn't read {what}").into(),
            description: format!("Nothing is known of {what} until a read answers.").into(),
            detail: Some(reason.clone().into()),
            retry: true,
        },
        Unknown::NoClusterLabel(label) => State {
            id: "alerts-no-label",
            icon: StateIcon::Missing,
            title: "The central store's alerts have no cluster label".into(),
            description: format!(
                "Its alerts carry no `{label}` label, so none can be told to be this \
                 cluster's, and none are shown. Add the label to the alerts, or name the \
                 one they carry as metrics.label in the setup file."
            )
            .into(),
            detail: None,
            retry: true,
        },
        Unknown::RulesOnly => State {
            id: "alerts-rules-only",
            icon: StateIcon::Missing,
            title: "Only PrometheusRule objects were found".into(),
            description: "They define rules, but nothing here evaluates them, so whether any \
                          alert fires isn't known. The Rules page lists them."
                .into(),
            detail: None,
            retry: true,
        },
    }
}

fn words(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// `3 m`, `2 h 5 m`, `4 d`: how long ago, in the table's short words.
pub fn ago(at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(at) = at else {
        return "–".into();
    };
    let seconds = (now - at).num_seconds().max(0);
    match seconds {
        0..=59 => format!("{seconds} s"),
        60..=3599 => format!("{} m", seconds / 60),
        3600..=86_399 => format!("{} h {} m", seconds / 3600, seconds % 3600 / 60),
        _ => format!("{} d", seconds / 86_400),
    }
}

/// When, for an end: `in 1 h 5 m`, or how long ago it passed.
fn until(at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(at) = at else {
        return "–".into();
    };
    if at > now {
        format!("in {}", ago(Some(now - (at - now)), now))
    } else {
        format!("{} ago", ago(Some(at), now))
    }
}

fn stamp(at: Option<DateTime<Utc>>) -> String {
    at.map_or_else(
        || "–".into(),
        |at| at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
    )
}

fn severity_tone(severity: Severity) -> Tone {
    match severity {
        Severity::Critical => Tone::Crit,
        Severity::Warning => Tone::Warn,
        Severity::Info | Severity::Other => Tone::Info,
    }
}

/// The glyph's tone and words for an alert: trouble by its severity,
/// pending as waiting, and what is muted as information.
fn alert_tone(alert: &Alert) -> Tone {
    match alert.state {
        AlertState::Firing => severity_tone(alert.severity),
        AlertState::Pending => Tone::Waiting,
        AlertState::Silenced | AlertState::Inhibited => Tone::Info,
    }
}

/// The labels the row's chips show: every one but those the table has a
/// column or a group for.
fn chip_labels(alert: &Alert) -> Vec<SharedString> {
    alert
        .labels
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "alertname" | "severity" | "cluster"))
        .map(|(name, value)| format!("{name}={value}").into())
        .collect()
}

/// A web address, and only that: a runbook that is another scheme is shown
/// as text and never opened.
fn web_address(runbook: &Option<String>) -> Option<SharedString> {
    let runbook = runbook.as_deref()?.trim();
    (runbook.starts_with("https://") || runbook.starts_with("http://"))
        .then(|| runbook.to_owned().into())
}

fn alert_row(alert: &Alert, group: usize, central: Option<&str>, now: DateTime<Utc>) -> Row {
    let labels = chip_labels(alert);
    let tone = alert_tone(alert);
    let since = alert.since;
    let key: SharedString = format!("{}-{}", alert.source.label(), alert.fingerprint).into();
    let state = format!("{} · {}", alert.state.label(), alert.severity.label());
    let summary = alert.summary.clone().unwrap_or_default();
    let mut cells = vec![
        Cell::plain(alert.name.clone()),
        Cell::plain(alert.state.label()),
        Cell::mono(ago(since, now)),
        Cell::plain(summary.clone()),
    ];
    if central.is_some() {
        cells.push(Cell::mono(alert.label("cluster").unwrap_or("–").to_owned()));
    }
    let haystack = format!(
        "{} {} {} {} {}",
        alert.name,
        alert.state.label(),
        summary,
        alert.description.as_deref().unwrap_or_default(),
        alert
            .labels
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join(" ")
    )
    .to_lowercase();
    let mut fields = vec![
        ("Severity", alert.severity.label().into(), false),
        ("State", alert.state.label().into(), false),
        (
            "Since",
            format!("{} · {} ago", stamp(since), ago(since, now)).into(),
            false,
        ),
        ("Source", alert.source.label().into(), false),
    ];
    if let Some(cluster) = alert.label("cluster") {
        fields.push(("Cluster", cluster.to_owned().into(), true));
    }
    if !alert.silenced_by.is_empty() {
        fields.push(("Silenced by", alert.silenced_by.join(", ").into(), true));
    }
    if !alert.inhibited_by.is_empty() {
        fields.push(("Inhibited by", alert.inhibited_by.join(", ").into(), true));
    }
    Row {
        key,
        tone,
        state: state.clone().into(),
        label: format!("{}: {state}", alert.name).into(),
        tooltip: if summary.is_empty() {
            alert.name.clone().into()
        } else {
            summary.clone().into()
        },
        chips: Some(Chips {
            shown: labels.iter().take(CHIPS).cloned().collect(),
            more: labels.len().saturating_sub(CHIPS),
        }),
        cells,
        group,
        haystack,
        detail: Detail {
            title: alert.name.clone().into(),
            tone,
            state: alert.state.label().into(),
            fields,
            summary: alert.summary.clone().map(Into::into),
            description: alert.description.clone().map(Into::into),
            labels: alert
                .labels
                .iter()
                .map(|(name, value)| (name.clone().into(), value.clone().into()))
                .collect(),
            runbook: web_address(&alert.runbook),
            matchers: Vec::new(),
            muted: Vec::new(),
            query: None,
            error: None,
        },
    }
}

fn alerts(
    data: &AlertsData,
    filter: &Filter,
    central: Option<&str>,
    now: DateTime<Utc>,
) -> Display {
    let total = data.alerts.len();
    let shown: Vec<&Alert> = data
        .alerts
        .iter()
        .filter(|alert| filter.muted || alert.state.is_active())
        .collect();
    let muted = data.alerts.iter().filter(|a| !a.state.is_active()).count();
    let mut display = Display {
        central: central.is_some(),
        total,
        ..Display::loading()
    };
    display.body = Body::Table;
    // Groups: severity, then alert name, in the order the alerts came
    // (already most urgent first).
    for alert in &shown {
        let key: SharedString = format!("{}-{}", alert.severity.label(), alert.name).into();
        let group = match display.groups.iter().position(|group| group.key == key) {
            Some(group) => group,
            None => {
                display.groups.push(Group {
                    key,
                    tone: severity_tone(alert.severity),
                    label: alert.name.clone().into(),
                    subject: Some(alert.severity.label().into()),
                    detail: Vec::new(),
                });
                display.groups.len() - 1
            }
        };
        let row = alert_row(alert, group, central, now);
        if filter.query.is_empty() || row.haystack.contains(&filter.query) {
            display.rows.push(row);
        }
    }
    finish(&mut display);
    display.hidden = total - display.rows.len();
    for group in 0..display.groups.len() {
        let count = display.rows.iter().filter(|row| row.group == group).count();
        display.groups[group].detail = vec![words(count, "alert", "alerts")];
    }
    let firing = data
        .alerts
        .iter()
        .filter(|alert| alert.state.is_active())
        .count();
    display.status = vec![(words(firing, "active alert", "active alerts"), None)];
    if muted > 0 {
        display.status.push((format!("{muted} muted"), None));
    }
    if data.more > 0 {
        display.notes.push(Note {
            id: "alerts-more",
            tone: Tone::Info,
            lead: Some("Showing the first alerts".into()),
            text: format!(
                "{} more alerts weren't kept; filter by a label to see them",
                data.more
            )
            .into(),
        });
    }
    sources_note(&mut display, &data.others);
    if display.rows.is_empty() {
        display.body = Body::State(if shown.is_empty() {
            quiet(&data.read, filter.muted, muted)
        } else {
            filtered("alerts")
        });
    }
    display
}

fn sources_note(display: &mut Display, others: &[String]) {
    if others.is_empty() {
        return;
    }
    display.notes.push(Note {
        id: "alerts-several",
        tone: Tone::Info,
        lead: Some("Several Alertmanagers answered".into()),
        text: format!(
            "Another also answered ({}); only the first is read. Replicas of one Alertmanager \
             are read as one.",
            others.join(", ")
        )
        .into(),
    });
}

fn filtered(what: &str) -> State {
    State {
        id: "alerts-filtered",
        icon: StateIcon::Filtered,
        title: format!("No {what} match").into(),
        description: format!("The filter hides every one of the {what} read.").into(),
        detail: None,
        retry: false,
    }
}

/// All quiet, naming what was read.
fn quiet(
    read: &[freshkube_core::monitoring::alerts::ReadFrom],
    muted_shown: bool,
    muted: usize,
) -> State {
    let from = read
        .iter()
        .map(|from| from.words())
        .collect::<Vec<_>>()
        .join(" and ");
    State {
        id: "alerts-quiet",
        icon: StateIcon::Quiet,
        title: "All quiet".into(),
        description: format!(
            "No active alerts in {from}.{}",
            if muted > 0 && !muted_shown {
                format!(" {muted} muted; show silenced and inhibited alerts to see them.")
            } else {
                String::new()
            }
        )
        .into(),
        detail: None,
        retry: false,
    }
}

/// Groups' rows in the order the groups are, and the table's lines.
fn finish(display: &mut Display) {
    display.lines.clear();
    let mut order: Vec<usize> = (0..display.rows.len()).collect();
    order.sort_by_key(|&ix| display.rows[ix].group);
    let mut last = None;
    for ix in order {
        let group = display.rows[ix].group;
        if last != Some(group) {
            display.lines.push(Entry::Group(group));
            last = Some(group);
        }
        display.lines.push(Entry::Row(ix));
    }
}

fn rule_tone(rule: &Rule) -> Tone {
    match rule.health {
        RuleHealth::Error => Tone::Crit,
        RuleHealth::Ok => Tone::Good,
        RuleHealth::Unknown => Tone::Unknown,
    }
}

fn rule_row(rule: &Rule, group: &RuleGroup, ix: usize, group_ix: usize, now: DateTime<Utc>) -> Row {
    let tone = rule_tone(rule);
    let kind = match rule.kind {
        RuleKind::Alerting => "Alerting",
        RuleKind::Recording => "Recording",
    };
    let state = format!("{} · {kind}", rule.health.label());
    let evaluated = if rule.source == Kind::PrometheusRule {
        "not evaluated".to_owned()
    } else {
        format!("{} ago", ago(rule.last_evaluation, now))
    };
    let alerts = match (rule.kind, rule.alerts) {
        (RuleKind::Recording, _) => String::new(),
        (_, 0) => "–".into(),
        (_, n) if rule.firing => format!("{n} firing"),
        (_, n) => format!("{n} pending"),
    };
    let error = rule.last_error.clone();
    let mut fields = vec![
        ("Kind", kind.into(), false),
        ("Health", rule.health.label().into(), false),
        ("Evaluated", evaluated.clone().into(), false),
        ("Group", group.name.clone().into(), false),
    ];
    if let Some(origin) = &group.origin {
        fields.push(("Defined in", origin.clone().into(), true));
    }
    Row {
        key: format!("{group_ix}-{ix}-{}", rule.name).into(),
        tone,
        state: state.clone().into(),
        label: format!("{}: {state}", rule.name).into(),
        tooltip: error.clone().unwrap_or_else(|| rule.name.clone()).into(),
        cells: vec![
            Cell::plain(rule.name.clone()),
            Cell::plain(kind),
            Cell::plain(rule.health.label()),
            Cell::mono(evaluated),
            Cell::plain(alerts),
            Cell::muted(error.clone().unwrap_or_default()),
        ],
        chips: None,
        group: group_ix,
        haystack: format!("{} {} {}", rule.name, group.name, rule.query).to_lowercase(),
        detail: Detail {
            title: rule.name.clone().into(),
            tone,
            state: rule.health.label().into(),
            fields,
            summary: None,
            description: None,
            labels: Vec::new(),
            runbook: None,
            matchers: Vec::new(),
            muted: Vec::new(),
            query: (!rule.query.is_empty()).then(|| rule.query.clone().into()),
            error: error.map(Into::into),
        },
    }
}

fn rules(data: &RulesData, filter: &Filter, now: DateTime<Utc>) -> Display {
    let mut display = Display {
        body: Body::Table,
        ..Display::loading()
    };
    let mut total = 0;
    for (group_ix, group) in data.groups.iter().enumerate() {
        total += group.rules.len();
        let broken = group.broken();
        let interval = group
            .interval_seconds
            .map(|seconds| format!("every {} s", seconds.round() as u64));
        let evaluated = data
            .evaluated
            .then(|| format!("evaluated {} ago", ago(group.last_evaluation, now)));
        let mut detail: Vec<String> = interval.into_iter().chain(evaluated).collect();
        detail.push(words(group.rules.len(), "rule", "rules"));
        if broken > 0 {
            detail.push(format!("{broken} failing"));
        }
        display.groups.push(Group {
            key: format!("rules-{group_ix}").into(),
            tone: if broken > 0 {
                Tone::Crit
            } else if data.evaluated {
                Tone::Good
            } else {
                Tone::Unknown
            },
            label: group.name.clone().into(),
            subject: group.origin.clone().map(Into::into),
            detail,
        });
        for (ix, rule) in group.rules.iter().enumerate() {
            let row = rule_row(rule, group, ix, group_ix, now);
            if filter.query.is_empty() || row.haystack.contains(&filter.query) {
                display.rows.push(row);
            }
        }
    }
    display.total = total;
    // A group with no row left under the filter is no group.
    let kept: Vec<usize> = (0..display.groups.len())
        .filter(|&group| display.rows.iter().any(|row| row.group == group))
        .collect();
    finish(&mut display);
    display.lines.retain(|entry| match entry {
        Entry::Group(group) => kept.contains(group),
        Entry::Row(_) => true,
    });
    display.hidden = total - display.rows.len();
    let broken = data.groups.iter().map(RuleGroup::broken).sum::<usize>();
    display.status = vec![(words(total, "rule", "rules"), None)];
    if broken > 0 {
        display
            .status
            .push((format!("{broken} failing"), Some(Tone::Crit)));
    }
    if !data.evaluated {
        display
            .status
            .push(("health unknown".into(), Some(Tone::Unknown)));
        display.notes.push(Note {
            id: "rules-unevaluated",
            tone: Tone::Info,
            lead: Some("Health is Unknown".into()),
            text: "These are definitions. Nothing here evaluates them, so whether a rule is \
                   healthy isn't known."
                .into(),
        });
    }
    if let Some(note) = &data.note {
        display.notes.push(Note {
            id: "rules-note",
            tone: Tone::Info,
            lead: None,
            text: note.clone().into(),
        });
    }
    if display.rows.is_empty() {
        display.body = Body::State(if total == 0 {
            State {
                id: "rules-none",
                icon: StateIcon::Quiet,
                title: "No rules".into(),
                description: "The source answered and holds no rule groups.".into(),
                detail: None,
                retry: false,
            }
        } else {
            filtered("rules")
        });
    }
    display
}

fn silence_tone(silence: &Silence) -> Tone {
    match silence.state {
        SilenceState::Active => Tone::Info,
        SilenceState::Pending => Tone::Waiting,
        SilenceState::Expired => Tone::Outline,
    }
}

fn silence_row(silence: &Silence, alerts: &[Alert], group: usize, now: DateTime<Utc>) -> Row {
    let matchers: Vec<SharedString> = silence.matchers.iter().map(|m| m.text().into()).collect();
    let muted = silence.muted(alerts);
    let tone = silence_tone(silence);
    let ends = match silence.state {
        SilenceState::Pending => format!("starts {}", until(silence.starts_at, now)),
        _ => until(silence.ends_at, now),
    };
    let muting = match silence.state {
        SilenceState::Active => words(muted.len(), "alert", "alerts"),
        _ => "–".into(),
    };
    let line = matchers
        .iter()
        .map(SharedString::as_ref)
        .collect::<Vec<&str>>()
        .join(", ");
    let state = silence.state.label();
    Row {
        key: silence.id.clone().into(),
        tone,
        state: state.into(),
        label: format!("{line}: {state}").into(),
        tooltip: silence.comment.clone().into(),
        cells: vec![
            Cell::mono(line.clone()),
            Cell::plain(silence.created_by.clone()),
            Cell::plain(silence.comment.clone()),
            Cell::mono(ends.clone()),
            Cell::plain(muting),
        ],
        chips: None,
        group,
        haystack: format!("{line} {} {}", silence.created_by, silence.comment).to_lowercase(),
        detail: Detail {
            title: line.into(),
            tone,
            state: state.into(),
            fields: vec![
                ("Id", silence.id.clone().into(), true),
                ("Created by", silence.created_by.clone().into(), false),
                ("Starts", stamp(silence.starts_at).into(), false),
                (
                    "Ends",
                    format!("{} · {ends}", stamp(silence.ends_at)).into(),
                    false,
                ),
            ],
            summary: Some(silence.comment.clone().into()).filter(|c: &SharedString| !c.is_empty()),
            description: None,
            labels: Vec::new(),
            runbook: None,
            matchers,
            muted: muted
                .iter()
                .map(|alert| format!("{} · {}", alert.name, alert.severity.label()).into())
                .collect(),
            query: None,
            error: None,
        },
    }
}

fn silences(data: &SilencesData, filter: &Filter, now: DateTime<Utc>) -> Display {
    let mut display = Display {
        body: Body::Table,
        total: data.silences.len(),
        ..Display::loading()
    };
    for (group, state) in [
        SilenceState::Active,
        SilenceState::Pending,
        SilenceState::Expired,
    ]
    .into_iter()
    .enumerate()
    {
        let tone = match state {
            SilenceState::Active => Tone::Info,
            SilenceState::Pending => Tone::Waiting,
            SilenceState::Expired => Tone::Outline,
        };
        display.groups.push(Group {
            key: format!("silences-{}", state.label().to_lowercase()).into(),
            tone,
            label: state.label().into(),
            subject: None,
            detail: Vec::new(),
        });
        for silence in data.silences.iter().filter(|s| s.state == state) {
            let row = silence_row(silence, &data.alerts, group, now);
            if filter.query.is_empty() || row.haystack.contains(&filter.query) {
                display.rows.push(row);
            }
        }
    }
    finish(&mut display);
    for group in 0..display.groups.len() {
        let count = display.rows.iter().filter(|row| row.group == group).count();
        display.groups[group].detail = vec![words(count, "silence", "silences")];
    }
    display.hidden = display.total - display.rows.len();
    let active = data
        .silences
        .iter()
        .filter(|s| s.state == SilenceState::Active)
        .count();
    display.status = vec![(words(active, "active silence", "active silences"), None)];
    if display.rows.is_empty() {
        display.body = Body::State(if data.silences.is_empty() {
            State {
                id: "silences-none",
                icon: StateIcon::Quiet,
                title: "No silences".into(),
                description: format!(
                    "Alertmanager holds no silences, read from {}.",
                    data.read
                        .iter()
                        .map(|from| from.words())
                        .collect::<Vec<_>>()
                        .join(" and ")
                )
                .into(),
                detail: None,
                retry: false,
            }
        } else {
            filtered("silences")
        });
    }
    display
}
