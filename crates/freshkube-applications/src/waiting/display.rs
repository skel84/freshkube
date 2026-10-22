//! What the page shows, derived when a read arrives or the find box
//! changes: the groups with their rows, each row's words and tone, the
//! Inspector's sections, the notes on what may be missing, and, with no
//! rows, the state that says why. Drawing only reads it.
use chrono::{DateTime, Local, Utc};
use freshkube_delivery::change::Cause;
use freshkube_delivery::changes::Gap;
use freshkube_delivery::waiting::{
    App, Fold, Item, PullRead, Query, Shipped, Standing, State, Waiting,
};
use freshkube_ui::ui::Tone;
use gpui_kit::SharedString;

/// The find box's hint, when what was typed asks nothing.
pub(crate) const HINT: &str =
    "Type a pull request number (418) or the start of a commit (a1f3c9e).";

/// One line of the Inspector's trace: a Stage, and where the Freight stands
/// in it.
#[derive(Clone, Debug, PartialEq)]
pub struct TraceLine {
    pub stage: SharedString,
    pub production: bool,
    pub tone: Option<Tone>,
    pub word: SharedString,
}

/// The Inspector's sections for a row.
#[derive(Clone, Debug, Default)]
pub struct Detail {
    pub heading: SharedString,
    pub tag: (Tone, SharedString),
    pub fields: Vec<(&'static str, SharedString, bool)>,
    /// Why it is stuck, in words.
    pub why: Option<SharedString>,
    pub trace: Vec<TraceLine>,
    /// Which Stages count as production, and each one's gap.
    pub production: Vec<SharedString>,
    /// What promoting now would ship, per production Stage.
    pub ship: Vec<SharedString>,
    /// What was read, and when.
    pub basis: SharedString,
}

/// What a row stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A Freight in the application's gap.
    Item { app: usize, item: usize },
    /// A listed Freight the find box matched that isn't in a gap.
    Listed { app: usize, listed: usize },
    /// An application with nothing waiting, or that wasn't read.
    App { app: usize },
}

#[derive(Clone, Debug)]
pub struct WaitRow {
    pub key: SharedString,
    pub kind: Kind,
    pub group: usize,
    pub tone: Tone,
    /// The glyph's tooltip and the Inspector's tag.
    pub state: SharedString,
    /// `#418`, or the commit, or `–`.
    pub named: SharedString,
    pub by_commit: bool,
    /// Where the pull request stands, dimmer when it is what was kept.
    pub pull: Option<SharedString>,
    pub pull_stale: bool,
    /// Why the pull request wasn't read, or how stale it is.
    pub pull_note: Option<SharedString>,
    pub change: SharedString,
    pub title_len: usize,
    pub merged: SharedString,
    pub stage: SharedString,
    pub waited: SharedString,
    pub why_word: SharedString,
    pub why: SharedString,
    pub label: SharedString,
    pub tooltip: SharedString,
    pub detail: Detail,
}

/// One group: an application with something waiting, or a fold's.
#[derive(Clone, Debug)]
pub struct Group {
    pub key: SharedString,
    pub fold: Fold,
    pub title: SharedString,
    pub detail: Vec<String>,
    pub tone: Tone,
    pub rows: usize,
    /// Whether the group folds while anything waits: only caught up.
    pub folds: bool,
}

/// What shows in place of the table when there are no rows.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Table,
    Empty {
        title: SharedString,
        description: SharedString,
    },
    Refused {
        reason: SharedString,
    },
    Failed {
        reason: SharedString,
    },
}

#[derive(Clone, Debug)]
pub struct Display {
    pub groups: Vec<Group>,
    pub rows: Vec<WaitRow>,
    /// What may be missing, in one sentence, and the same as a label.
    pub missing: Option<(SharedString, SharedString)>,
    pub body: Body,
    pub waiting: usize,
    pub items: usize,
    /// The query the find box asked, when it asked one.
    pub query: Option<Query>,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            rows: Vec::new(),
            missing: None,
            body: Body::Table,
            waiting: 0,
            items: 0,
            query: None,
        }
    }
}

fn age(at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> SharedString {
    at.map_or_else(
        || "–".into(),
        |at| {
            let seconds = now.signed_duration_since(at).num_seconds().max(0);
            crate::resources::model::format_age(seconds as u64).into()
        },
    )
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

impl Display {
    /// From a read, or why nothing was read, narrowed by what the find box
    /// asked. `text` is what was typed; one that asks nothing shows
    /// nothing narrowed and the hint.
    pub(crate) fn new(
        waiting: Option<&Waiting>,
        error: Option<&str>,
        cluster: &str,
        text: &str,
    ) -> Self {
        let Some(waiting) = waiting else {
            return match error {
                Some(why) => Self {
                    body: Body::Failed {
                        reason: why.to_owned().into(),
                    },
                    ..Self::default()
                },
                None => Self::default(),
            };
        };
        let now = waiting.observed_at.unwrap_or_else(Utc::now);
        let query = Query::parse(text);
        let mut display = Self {
            waiting: waiting.waiting(),
            items: waiting.items(),
            query: query.clone(),
            ..Self::default()
        };
        display.fill(waiting, now, text);
        display.missing = (!waiting.notes.is_empty()).then(|| {
            let text = waiting
                .notes
                .iter()
                .map(|note| note.text.as_str())
                .collect::<Vec<_>>()
                .join(". ");
            let label = format!("May be missing: {text}");
            (text.into(), label.into())
        });
        display.body = if !display.rows.is_empty() {
            Body::Table
        } else {
            body(waiting, cluster, text, query.as_ref())
        };
        // Beside rows, and over an empty page, a note says what wasn't
        // read; refused and failed say it themselves, as does Kargo not
        // installed.
        let banner = match &display.body {
            Body::Table => true,
            Body::Empty { .. } => !uninstalled(waiting),
            Body::Refused { .. } | Body::Failed { .. } => false,
        };
        display.missing = display.missing.take().filter(|_| banner);
        display
    }

    fn fill(&mut self, waiting: &Waiting, now: DateTime<Utc>, text: &str) {
        if !text.trim().is_empty() {
            // A search: one group for each application it found something
            // in, whatever the application's state.
            let Some(query) = Query::parse(text) else {
                return;
            };
            for (ix, app) in waiting.apps.iter().enumerate() {
                let found = app.find(&query);
                if found.is_empty() {
                    continue;
                }
                let group = self.group_of(app, Fold::Waiting, found.len());
                for freight in found {
                    self.push_found(waiting, ix, app, freight, group, now);
                }
            }
            return;
        }
        for (ix, app) in waiting.apps.iter().enumerate() {
            match app.fold() {
                Fold::Waiting => {
                    let group = self.group_of(app, Fold::Waiting, app.items.len());
                    for item in 0..app.items.len() {
                        self.push_item(waiting, ix, item, group, now);
                    }
                }
                fold => {
                    let group = match self.groups.iter().position(|g| g.fold == fold) {
                        Some(group) => group,
                        None => self.fold_group(fold),
                    };
                    self.groups[group].rows += 1;
                    self.push_app(waiting, ix, group, now);
                }
            }
        }
        for group in &mut self.groups {
            if group.fold != Fold::Waiting {
                group.detail = vec![plural(group.rows, "application", "applications")];
            }
        }
    }

    fn fold_group(&mut self, fold: Fold) -> usize {
        self.groups.push(Group {
            key: format!("fold-{}", fold.label().to_lowercase().replace(' ', "-")).into(),
            fold,
            title: fold.label().into(),
            detail: Vec::new(),
            tone: match fold {
                Fold::Waiting => Tone::Waiting,
                Fold::NotRead => Tone::Unknown,
                Fold::CaughtUp => Tone::Good,
            },
            rows: 0,
            folds: fold == Fold::CaughtUp,
        });
        self.groups.len() - 1
    }

    /// An application's own group: its name, and its gap to production.
    fn group_of(&mut self, app: &App, fold: Fold, rows: usize) -> usize {
        let behind = app.gap().map(|gap| {
            format!(
                "{} Freight ahead of {}{}",
                gap.ahead.len(),
                gap.stage,
                if gap.at_least { " or more" } else { "" },
            )
        });
        let detail = match &app.state {
            State::Waiting => behind.into_iter().chain(older_words(app)).collect(),
            State::CaughtUp => vec!["caught up".to_owned()],
            State::NotRead { .. } => vec!["not read".to_owned()],
        };
        let tone = match app.state {
            State::Waiting => Tone::Waiting,
            State::CaughtUp => Tone::Good,
            State::NotRead { .. } => Tone::Unknown,
        };
        self.groups.push(Group {
            key: format!("app-{}", app.name).into(),
            fold,
            title: app.name.clone().into(),
            detail,
            tone,
            rows,
            folds: false,
        });
        self.groups.len() - 1
    }

    fn push_item(
        &mut self,
        waiting: &Waiting,
        app_ix: usize,
        item_ix: usize,
        group: usize,
        now: DateTime<Utc>,
    ) {
        let app = &waiting.apps[app_ix];
        let item = &app.items[item_ix];
        let row = item_row(waiting, app, app_ix, item, item_ix, group, now);
        self.rows.push(row);
    }

    fn push_app(&mut self, waiting: &Waiting, app_ix: usize, group: usize, now: DateTime<Utc>) {
        let row = app_row(waiting, app_ix, group, now);
        self.rows.push(row);
    }

    fn push_found(
        &mut self,
        waiting: &Waiting,
        app_ix: usize,
        app: &App,
        freight: &str,
        group: usize,
        now: DateTime<Utc>,
    ) {
        if let Some(item) = app.items.iter().position(|item| item.freight == freight) {
            self.push_item(waiting, app_ix, item, group, now);
        } else if let Some(listed) = app.listed.iter().position(|l| l.freight == freight) {
            let row = listed_row(waiting, app, app_ix, listed, group, now);
            self.rows.push(row);
        }
    }
}

/// With no rows: Kargo refused or unread is its own state, and only when
/// it answered is it empty.
fn body(waiting: &Waiting, cluster: &str, text: &str, query: Option<&Query>) -> Body {
    if !text.trim().is_empty() {
        return Body::Empty {
            title: match query {
                Some(Query::Pull(number)) => {
                    format!("No Freight read names pull request {number}")
                }
                Some(Query::Commit(commit)) => format!("No Freight read has commit {commit}"),
                None => "That isn't a pull request or a commit".to_owned(),
            }
            .into(),
            description: match query {
                Some(Query::Pull(_)) => "Pull requests are asked for only the Freight that waits. \
                    If the change has shipped, find it by its commit."
                    .to_owned(),
                Some(Query::Commit(_)) => format!(
                    "Only the {} newest Freight of each project are read, and a commit \
                     older than those isn't found.",
                    freshkube_delivery::changes::MAX_FREIGHT
                ),
                None => HINT.to_owned(),
            }
            .into(),
        };
    }
    let note = |gap| waiting.notes.iter().find(|note| note.gap == gap);
    let join = |gap| {
        waiting
            .notes
            .iter()
            .filter(|note| note.gap == gap)
            .map(|note| note.text.as_str())
            .collect::<Vec<_>>()
            .join(". ")
    };
    if note(Gap::Refused).is_some() && waiting.apps.is_empty() {
        return Body::Refused {
            reason: join(Gap::Refused).into(),
        };
    }
    if note(Gap::Unreadable).is_some() && waiting.apps.is_empty() {
        return Body::Failed {
            reason: join(Gap::Unreadable).into(),
        };
    }
    if uninstalled(waiting) {
        return Body::Empty {
            title: "Kargo isn't installed".into(),
            description: format!(
                "Kargo's projects aren't served on {cluster}, so there's nothing to compare."
            )
            .into(),
        };
    }
    Body::Empty {
        title: "No Kargo project found".into(),
        description: format!("No Kargo project on {cluster} has Stages and Freight yet.").into(),
    }
}

/// Whether Kargo's projects aren't served at all.
fn uninstalled(waiting: &Waiting) -> bool {
    waiting
        .notes
        .iter()
        .any(|note| note.gap == Gap::NotServed && note.project.is_none())
}

/// What the PR cell says of a Freight: its pull request, else its commit,
/// and for a pull request that wasn't read, that it wasn't, never the bare
/// commit a pull request GitHub doesn't name shows.
struct Named {
    text: SharedString,
    by_commit: bool,
    state: Option<SharedString>,
    stale: bool,
    /// The PR cell's tooltip: why the pull request wasn't read.
    note: Option<SharedString>,
}

fn named_of(item: &Item) -> Named {
    if let Some(pull) = item.pull() {
        return Named {
            text: format!("#{}", pull.number).into(),
            by_commit: false,
            state: Some(pull.state.label().into()),
            stale: pull.stale.is_some(),
            note: pull.stale.clone().map(Into::into),
        };
    }
    if let Some(PullRead::Unread { rate_limit, why }) = &item.pull {
        return Named {
            text: if *rate_limit {
                "not read (rate limit)"
            } else {
                "not read"
            }
            .into(),
            by_commit: false,
            state: None,
            stale: true,
            note: Some(why.clone().into()),
        };
    }
    match &item.commit {
        Some(commit) => Named {
            text: commit.chars().take(7).collect::<String>().into(),
            by_commit: true,
            state: None,
            stale: false,
            note: None,
        },
        None => Named {
            text: "–".into(),
            by_commit: false,
            state: None,
            stale: false,
            note: None,
        },
    }
}

/// `10 older Freight not listed`, for a project the cap cut.
fn older_words(app: &App) -> Option<String> {
    (app.older > 0).then(|| {
        format!(
            "{} older Freight not listed: only the {} newest are read",
            app.older,
            freshkube_delivery::changes::MAX_FREIGHT
        )
    })
}

fn item_row(
    waiting: &Waiting,
    app: &App,
    app_ix: usize,
    item: &Item,
    item_ix: usize,
    group: usize,
    now: DateTime<Utc>,
) -> WaitRow {
    let named = named_of(item);
    let title = item.title.clone().unwrap_or_else(|| item.freight.clone());
    let merged = item.pull().and_then(|pull| pull.merged_at);
    let tone = if item.why.blocked {
        Tone::Warn
    } else {
        Tone::Waiting
    };
    let state: SharedString = if item.why.blocked {
        "Stuck: what it waits for has failed".into()
    } else {
        "Waiting: not in production yet".into()
    };
    let waited = item
        .waited(now)
        .map_or_else(|| "–".into(), |d| age_of(d.num_seconds()));
    let stage: SharedString = item.in_stage.clone().unwrap_or_else(|| "–".into()).into();
    let why = format!("{} {}", item.why.word(), item.why.needs());
    let label = format!(
        "{} {title} · {} · {}, waited {waited} · {why}",
        named.text,
        app.name,
        if item.in_stage.is_some() {
            format!("in {stage}")
        } else {
            "in no Stage".to_owned()
        },
    );
    WaitRow {
        key: format!("{}/{}", app.name, item.freight).into(),
        kind: Kind::Item {
            app: app_ix,
            item: item_ix,
        },
        group,
        tone,
        state: state.clone(),
        named: named.text.clone(),
        by_commit: named.by_commit,
        pull: named.state.clone(),
        pull_stale: named.stale,
        pull_note: named.note.clone(),
        change: title.clone().into(),
        title_len: title.len(),
        merged: age(merged, now),
        stage,
        waited,
        why_word: item.why.word().into(),
        why: item.why.needs().to_owned().into(),
        label: label.into(),
        tooltip: format!("{title} · {} · Freight {}", app.name, item.freight).into(),
        detail: item_detail(waiting, app, item, now),
    }
}

fn age_of(seconds: i64) -> SharedString {
    crate::resources::model::format_age(seconds.max(0) as u64).into()
}

fn listed_row(
    waiting: &Waiting,
    app: &App,
    app_ix: usize,
    listed_ix: usize,
    group: usize,
    now: DateTime<Utc>,
) -> WaitRow {
    let listed = &app.listed[listed_ix];
    let commit = listed.commits.first().cloned().unwrap_or_default();
    let title = listed
        .title
        .clone()
        .unwrap_or_else(|| listed.freight.clone());
    let (tone, state, why) = shipped_words(app, &listed.freight);
    WaitRow {
        key: format!("{}/{}", app.name, listed.freight).into(),
        kind: Kind::Listed {
            app: app_ix,
            listed: listed_ix,
        },
        group,
        tone,
        state: state.clone(),
        named: commit.chars().take(7).collect::<String>().into(),
        by_commit: true,
        pull: None,
        pull_stale: false,
        pull_note: None,
        change: title.clone().into(),
        title_len: title.len(),
        merged: "–".into(),
        stage: "–".into(),
        waited: "–".into(),
        why_word: state.clone(),
        why: why.clone(),
        label: format!("{title} · {} · {state}. {why}", app.name).into(),
        tooltip: format!("{title} · {} · Freight {}", app.name, listed.freight).into(),
        detail: listed_detail(waiting, app, &listed.freight, &title, now),
    }
}

/// Whether a listed Freight that isn't waiting reached production.
fn shipped_words(app: &App, freight: &str) -> (Tone, SharedString, SharedString) {
    let production = app.production().join(", ");
    match app.shipped(freight) {
        Shipped::Everywhere => (
            Tone::Good,
            "In production".into(),
            format!("Every production Stage ({production}) has run it").into(),
        ),
        Shipped::Partly => {
            let missing: Vec<String> = app
                .trace(freight)
                .into_iter()
                .zip(&app.stages)
                .filter(|(_, line)| line.is_final)
                .filter(|((_, how), _)| matches!(how, Standing::Not | Standing::Verified))
                .map(|((stage, _), _)| stage)
                .collect();
            (
                Tone::Unknown,
                "Partly in production".into(),
                format!("Not yet in {}", missing.join(", ")).into(),
            )
        }
        Shipped::No => (
            Tone::Outline,
            "Not in production".into(),
            format!("No production Stage ({production}) has run it").into(),
        ),
        Shipped::Unknown => (
            Tone::Unknown,
            "Unknown".into(),
            "The project has no Stage to call production".into(),
        ),
    }
}

fn app_row(waiting: &Waiting, app_ix: usize, group: usize, now: DateTime<Utc>) -> WaitRow {
    let app = &waiting.apps[app_ix];
    let (tone, state, word, why) = match &app.state {
        State::NotRead { cause, text, .. } => (
            Tone::Unknown,
            SharedString::from("Not read"),
            SharedString::from(cause.word()),
            SharedString::from(text.clone()),
        ),
        _ => (
            Tone::Good,
            "Caught up".into(),
            "Caught up".into(),
            format!(
                "Every production Stage ({}) runs the newest Freight it takes",
                app.production().join(", ")
            )
            .into(),
        ),
    };
    WaitRow {
        key: app.name.clone().into(),
        kind: Kind::App { app: app_ix },
        group,
        tone,
        state: state.clone(),
        named: "–".into(),
        by_commit: false,
        pull: None,
        pull_stale: false,
        pull_note: None,
        change: app.name.clone().into(),
        title_len: app.name.len(),
        merged: "–".into(),
        stage: "–".into(),
        waited: "–".into(),
        why_word: word,
        why: why.clone(),
        label: format!("{} · {state}. {why}", app.name).into(),
        tooltip: format!("{} · {state}", app.name).into(),
        detail: app_detail(waiting, app, now),
    }
}

fn basis(waiting: &Waiting, app: &App) -> SharedString {
    let at = waiting
        .observed_at
        .map(|at| at.with_timezone(&Local).format("%H:%M").to_string())
        .unwrap_or_default();
    let pulls = app.items.iter().any(|item| item.pull().is_some());
    format!(
        "Freight, Stages and their history read from Kargo at {at}{}. Nothing here says whether \
         pods run it.{}",
        if pulls {
            "; pull requests from GitHub"
        } else {
            ""
        },
        older_words(app).map_or_else(String::new, |older| format!(" {older}."))
    )
    .into()
}

fn trace_lines(app: &App, freight: &str) -> Vec<TraceLine> {
    app.trace(freight)
        .into_iter()
        .zip(&app.stages)
        .map(|((stage, how), line)| {
            let (tone, word) = match how {
                Standing::Runs => (Some(Tone::Good), "Runs it now"),
                Standing::Ran => (Some(Tone::Good), "Ran it"),
                Standing::Verified => (Some(Tone::Info), "Verified it, history doesn't hold it"),
                Standing::Not => (None, "Not reached"),
            };
            TraceLine {
                stage: stage.into(),
                production: line.is_final,
                tone,
                word: word.into(),
            }
        })
        .collect()
}

fn production_lines(app: &App) -> Vec<SharedString> {
    let mut lines = vec![
        format!(
            "Production is {}: the Stages nothing takes Freight from.",
            app.production().join(" and ")
        )
        .into(),
    ];
    for gap in &app.finals {
        lines.push(
            format!(
                "{} runs {}; {} Freight ahead{}",
                gap.stage,
                if gap.runs.is_empty() {
                    "nothing".to_owned()
                } else {
                    gap.runs.join(", ")
                },
                gap.ahead.len(),
                if gap.at_least {
                    " (at least: what it runs isn't among those read)"
                } else {
                    ""
                },
            )
            .into(),
        );
    }
    lines.extend(older_words(app).map(SharedString::from));
    if app.finals.len() > 1 {
        lines.push(
            format!(
                "The application shows {}, the furthest behind.",
                app.furthest.as_deref().unwrap_or_default()
            )
            .into(),
        );
    }
    lines
}

/// What promoting now would ship to each production Stage.
fn ship_lines(app: &App) -> Vec<SharedString> {
    app.finals
        .iter()
        .map(|gap| match &gap.next {
            Some(next) => format!(
                "{}: {} brings {} Freight",
                gap.stage,
                next.freight,
                next.brings.len()
            ),
            None if gap.ahead.is_empty() => format!("{}: nothing, it is caught up", gap.stage),
            None => format!(
                "{}: nothing can be promoted yet; no Freight ahead of it is verified in the Stage that feeds it",
                gap.stage
            ),
        }
        .into())
        .collect()
}

fn item_detail(waiting: &Waiting, app: &App, item: &Item, now: DateTime<Utc>) -> Detail {
    let title = item.title.clone().unwrap_or_else(|| item.freight.clone());
    let named = named_of(item).text;
    let pull = item.pull();
    let mut fields: Vec<(&'static str, SharedString, bool)> = vec![
        ("Application", app.name.clone().into(), true),
        ("Freight", item.freight.clone().into(), true),
    ];
    if let Some(commit) = &item.commit {
        fields.push((
            "Commit",
            commit.chars().take(12).collect::<String>().into(),
            true,
        ));
    }
    if let Some(pull) = pull {
        fields.push((
            "Pull request",
            format!(
                "#{} · {}{}",
                pull.number,
                pull.state.label(),
                pull.author
                    .as_ref()
                    .map_or_else(String::new, |author| format!(" · {author}"))
            )
            .into(),
            false,
        ));
        if let Some(note) = &pull.stale {
            fields.push(("GitHub", note.clone().into(), false));
        }
    }
    match &item.pull {
        Some(PullRead::Unread { rate_limit, why }) => fields.push((
            "Pull request",
            format!(
                "Not read{}: {why}",
                if *rate_limit { " (rate limit)" } else { "" }
            )
            .into(),
            false,
        )),
        Some(PullRead::NoPull) => fields.push((
            "Pull request",
            "GitHub names none for this commit".into(),
            false,
        )),
        _ => {}
    }
    if let Some(merged) = pull.and_then(|pull| pull.merged_at) {
        fields.push((
            "Merged",
            format!(
                "{} · {} ago",
                merged.with_timezone(&Local).format("%Y-%m-%d %H:%M"),
                age(Some(merged), now)
            )
            .into(),
            false,
        ));
    }
    let since = match (&item.in_stage, item.reached) {
        (Some(stage), Some(at)) => format!(
            "In {stage} since {} ({} ago): its first verification there started then",
            at.with_timezone(&Local).format("%Y-%m-%d %H:%M"),
            age(Some(at), now)
        ),
        (Some(stage), None) => format!(
            "In {stage}; Kargo records no verification there, so when it arrived isn't known"
        ),
        (None, _) => "In no Stage yet".to_owned(),
    };
    fields.push(("Waiting", since.into(), false));
    Detail {
        heading: format!("{named} {title}").into(),
        tag: (
            if item.why.blocked {
                Tone::Warn
            } else {
                Tone::Waiting
            },
            if item.why.blocked { "Stuck" } else { "Waiting" }.into(),
        ),
        fields,
        why: Some(format!("{} · {}", item.why.word(), item.why.needs()).into()),
        trace: trace_lines(app, &item.freight),
        production: production_lines(app),
        ship: ship_lines(app),
        basis: basis(waiting, app),
    }
}

fn listed_detail(
    waiting: &Waiting,
    app: &App,
    freight: &str,
    title: &str,
    _now: DateTime<Utc>,
) -> Detail {
    let (tone, word, why) = shipped_words(app, freight);
    let commit = app
        .listed
        .iter()
        .find(|l| l.freight == freight)
        .and_then(|l| l.commits.first())
        .cloned();
    let mut fields: Vec<(&'static str, SharedString, bool)> = vec![
        ("Application", app.name.clone().into(), true),
        ("Freight", freight.to_owned().into(), true),
    ];
    if let Some(commit) = commit {
        fields.push((
            "Commit",
            commit.chars().take(12).collect::<String>().into(),
            true,
        ));
    }
    Detail {
        heading: title.to_owned().into(),
        tag: (tone, word),
        fields,
        why: Some(why),
        trace: trace_lines(app, freight),
        production: production_lines(app),
        ship: ship_lines(app),
        basis: basis(waiting, app),
    }
}

fn app_detail(waiting: &Waiting, app: &App, _now: DateTime<Utc>) -> Detail {
    let fields: Vec<(&'static str, SharedString, bool)> =
        vec![("Application", app.name.clone().into(), true)];
    match &app.state {
        State::NotRead { cause, text, .. } => Detail {
            heading: app.name.clone().into(),
            tag: (Tone::Unknown, "Not read".into()),
            fields,
            why: Some(format!("{} · {} · {text}", cause.word(), cause_needs(cause)).into()),
            trace: Vec::new(),
            production: Vec::new(),
            ship: Vec::new(),
            basis: format!(
                "Not read: it is shown, never left out. Nothing is known about what waits in {}.",
                app.name
            )
            .into(),
        },
        _ => Detail {
            heading: app.name.clone().into(),
            tag: (Tone::Good, "Caught up".into()),
            fields,
            why: None,
            trace: Vec::new(),
            production: production_lines(app),
            ship: ship_lines(app),
            basis: basis(waiting, app),
        },
    }
}

fn cause_needs(cause: &Cause) -> &str {
    cause.needs()
}
