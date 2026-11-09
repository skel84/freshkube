//! `freshkube waiting-to-ship`: what is built or deployed somewhere but not
//! yet in the last Stage, oldest first, from the same read and derivation
//! as the Waiting to ship page. `--older-than` turns a wait into an exit
//! code, for a scheduled nag. `freshkube get application <app>` shows the
//! same read's Stages, without asking GitHub.

use chrono::{DateTime, Duration, Utc};
use freshkube_delivery::changes::{Gap, Note};
use freshkube_delivery::source::Source;
use freshkube_delivery::waiting::{App, Item, PullRead, State, Waiting, derive};
use serde_json::{Map, Value, json};

use crate::Report;
use crate::args::WaitingArgs;
use crate::cluster::{Stop, Target};
use crate::delivery::{self, Fetched};
use crate::exit::Findings;
use crate::notes::{ago, gap_findings, note_findings, notes_json, span};
use crate::read::{Class, Problem, Read};

pub struct Wanted {
    pub older_than: Option<Duration>,
}

pub fn wanted(args: &WaitingArgs) -> Result<Wanted, Stop> {
    let older_than = args
        .older_than
        .as_deref()
        .map(span)
        .transpose()
        .map_err(|why| Stop::Usage(format!("--older-than: {why}")))?;
    Ok(Wanted { older_than })
}

fn pull_json(pull: &Option<PullRead>) -> Value {
    match pull {
        None => Value::Null,
        Some(PullRead::Found(pull)) => json!({
            "read": "found",
            "number": pull.number,
            "title": pull.title,
            "author": pull.author,
            "state": pull.state.label(),
            "merged_at": pull.merged_at.map(|at| at.to_rfc3339()),
            "stale": pull.stale,
        }),
        Some(PullRead::NoPull) => json!({"read": "none"}),
        Some(PullRead::Unread { rate_limit, why }) => json!({
            "read": "not-read",
            "rate_limit": rate_limit,
            "why": why,
        }),
    }
}

fn pull_text(pull: &Option<PullRead>) -> Option<String> {
    match pull {
        None | Some(PullRead::NoPull) => None,
        Some(PullRead::Found(pull)) => Some(format!(
            "pull request #{} · {}{}",
            pull.number,
            pull.state.label(),
            pull.author
                .as_ref()
                .map(|author| format!(" · {author}"))
                .unwrap_or_default()
        )),
        Some(PullRead::Unread {
            rate_limit: true, ..
        }) => Some("pull request not read (rate limit)".into()),
        Some(PullRead::Unread { why, .. }) => Some(format!("pull request not read: {why}")),
    }
}

fn item_json(item: &Item, now: DateTime<Utc>) -> Value {
    json!({
        "freight": item.freight,
        "alias": item.alias,
        "title": item.title,
        "commit": item.commit,
        "created": item.created.map(|at| at.to_rfc3339()),
        "pull": pull_json(&item.pull),
        "in_stage": item.in_stage,
        "reached": item.reached.map(|at| at.to_rfc3339()),
        "waited_seconds": item.waited(now).map(|waited| waited.num_seconds()),
        "why": {
            "cause": item.why.word(),
            "needs": item.why.needs(),
            "blocked": item.why.blocked,
        },
        "promotable": item.promotable,
        "view": {"page": "waiting", "freight": item.freight},
    })
}

fn stages_json(app: &App) -> Vec<Value> {
    app.stages
        .iter()
        .map(|stage| {
            json!({
                "name": stage.name,
                "final": stage.is_final,
                "runs": stage.runs,
                "held": stage.held,
                "verified": stage.verified,
            })
        })
        .collect()
}

fn state_word(state: &State) -> &'static str {
    match state {
        State::Waiting => "waiting",
        State::CaughtUp => "caught-up",
        State::NotRead { .. } => "not-read",
    }
}

fn app_json(app: &App, now: DateTime<Utc>) -> Value {
    let not_read = match &app.state {
        State::NotRead { cause, text, .. } => Some(json!({"cause": cause.word(), "text": text})),
        _ => None,
    };
    json!({
        "name": app.name,
        "state": state_word(&app.state),
        "not_read": not_read,
        "furthest": app.furthest,
        "items": app.items.iter().map(|item| item_json(item, now)).collect::<Vec<_>>(),
        "finals": app.finals.iter().map(|gap| json!({
            "stage": gap.stage,
            "runs": gap.runs,
            "ahead": gap.ahead,
            "at_least": gap.at_least,
            "next": gap.next.as_ref().map(|next| json!({
                "freight": next.freight,
                "brings": next.brings,
            })),
        })).collect::<Vec<_>>(),
        "stages": stages_json(app),
        "older": app.older,
        "pulls_read": app.pulls_read,
        "view": {"page": "waiting", "project": app.name},
    })
}

fn app_text(app: &App, now: DateTime<Utc>, out: &mut String) {
    match &app.state {
        State::Waiting => out.push_str(&format!(
            "\n{}  waiting{}\n",
            app.name,
            app.gap()
                .map(|gap| format!(
                    " · {} {} ahead of {}",
                    gap.ahead.len(),
                    "Freight",
                    gap.stage
                ))
                .unwrap_or_default()
        )),
        State::CaughtUp => out.push_str(&format!("\n{}  caught up\n", app.name)),
        State::NotRead { cause, text, .. } => out.push_str(&format!(
            "\n{}  ? not read · {}\n  {text}\n",
            app.name,
            cause.word()
        )),
    }
    for item in &app.items {
        let name = item.alias.as_deref().unwrap_or(&item.freight);
        let waited = match (item.waited(now), &item.in_stage) {
            (Some(waited), Some(stage)) => format!("  waited {} in {stage}", ago(waited)),
            (Some(waited), None) => format!("  waited {}", ago(waited)),
            (None, _) => "  wait not dated by Kargo".into(),
        };
        let pull = pull_text(&item.pull)
            .map(|text| format!(" · {text}"))
            .unwrap_or_default();
        out.push_str(&format!("  {name}{pull}{waited}\n"));
        if let Some(title) = &item.title {
            out.push_str(&format!("    {title}\n"));
        }
        out.push_str(&format!(
            "    {}: {}{}\n",
            item.why.word(),
            item.why.needs(),
            if item.promotable {
                " · can be promoted now"
            } else {
                ""
            }
        ));
    }
}

fn notes_of(waiting: &Waiting) -> Vec<&Note> {
    waiting.notes.iter().collect()
}

fn state_findings(app: &App) -> Findings {
    match &app.state {
        State::NotRead { gap, .. } => gap_findings(*gap),
        _ => Findings::default(),
    }
}

fn problem_report(problem: &Problem, apps_key: &str) -> Report {
    // The same keys as an answer has, with nothing found.
    let mut json = Map::new();
    if apps_key == "apps" {
        json.insert("observed_at".into(), Value::Null);
        json.insert("undated".into(), json!([]));
        json.insert("sign_in".into(), json!([]));
        json.insert("github".into(), json!("anonymous"));
        json.insert("older_than".into(), Value::Null);
        json.insert("overdue".into(), json!([]));
    }
    json.insert(apps_key.into(), json!([]));
    json.insert("notes".into(), json!([]));
    json.insert("problems".into(), json!([problem.json()]));
    Report {
        json,
        text: format!("? {}\n", problem.sentence()),
        findings: problem.findings(),
    }
}

pub async fn run(args: &WaitingArgs, target: &Target) -> Result<Report, Stop> {
    let Wanted { older_than } = wanted(args)?;
    let Fetched { read, sign_in } = match delivery::waiting(target, true).await {
        Ok(fetched) => fetched,
        Err(problem) => return Ok(problem_report(&problem, "apps")),
    };
    let now = read.observed_at;
    let waiting = derive(&read);
    let mut findings = Findings::default();
    let notes = notes_of(&waiting);
    for note in &notes {
        findings.merge(note_findings(note));
    }
    for app in &waiting.apps {
        findings.merge(state_findings(app));
    }
    if !sign_in.is_empty() {
        findings.sign_in = true;
    }
    let overdue: Vec<(&App, &Item)> = match older_than {
        Some(limit) => waiting
            .apps
            .iter()
            .flat_map(|app| app.items.iter().map(move |item| (app, item)))
            .filter(|(_, item)| item.waited(now).is_some_and(|waited| waited >= limit))
            .collect(),
        None => Vec::new(),
    };
    if !overdue.is_empty() {
        findings.failed = true;
    }
    // A wait Kargo doesn't date can't be told to be short: with --older-than
    // it is unknown, never quietly fine.
    let undated: Vec<(&App, &Item)> = match older_than {
        Some(_) => waiting
            .apps
            .iter()
            .flat_map(|app| app.items.iter().map(move |item| (app, item)))
            .filter(|(_, item)| item.waited(now).is_none())
            .collect(),
        None => Vec::new(),
    };
    if !undated.is_empty() {
        findings.unknown = true;
    }
    let mut text = format!(
        "Waiting to ship on {}: {} {} across {} {}\n",
        target.context,
        waiting.items(),
        "Freight",
        waiting.waiting(),
        if waiting.waiting() == 1 {
            "application"
        } else {
            "applications"
        },
    );
    for app in &waiting.apps {
        app_text(app, now, &mut text);
    }
    for note in &notes {
        text.push_str(&format!("\n? {}\n", note.text));
    }
    if let (Some(limit), false) = (older_than, overdue.is_empty()) {
        text.push_str(&format!(
            "\nx {} {} waited {} or longer\n",
            overdue.len(),
            if overdue.len() == 1 {
                "Freight has"
            } else {
                "Freight have"
            },
            ago(limit)
        ));
    }
    if !undated.is_empty() {
        text.push_str(&format!(
            "\n? {} {} no date from Kargo for how long {} waited, so {} older than {} can't be told\n",
            undated.len(),
            if undated.len() == 1 { "Freight has" } else { "Freight have" },
            if undated.len() == 1 { "it has" } else { "they have" },
            if undated.len() == 1 { "it is" } else { "they are" },
            older_than.map(ago).unwrap_or_default()
        ));
    }
    for entry in &sign_in {
        text.push_str(&format!(
            "\n? entry {entry} signs in with a credential plugin and was not connected; run it again with --allow-login\n"
        ));
    }
    if let Some(note) = delivery::github_note(target) {
        text.push_str(&format!("\n{note}\n"));
    }
    let mut json = Map::new();
    json.insert("observed_at".into(), json!(now.to_rfc3339()));
    json.insert(
        "undated".into(),
        json!(
            undated
                .iter()
                .map(|(app, item)| json!({"application": app.name, "freight": item.freight}))
                .collect::<Vec<_>>()
        ),
    );
    json.insert("sign_in".into(), json!(sign_in));
    json.insert("github".into(), json!("anonymous"));
    json.insert(
        "older_than".into(),
        json!(older_than.map(|d| d.num_seconds())),
    );
    json.insert(
        "overdue".into(),
        json!(
            overdue
                .iter()
                .map(|(app, item)| json!({"application": app.name, "freight": item.freight}))
                .collect::<Vec<_>>()
        ),
    );
    json.insert(
        "apps".into(),
        Value::Array(waiting.apps.iter().map(|app| app_json(app, now)).collect()),
    );
    json.insert(
        "notes".into(),
        Value::Array(notes_json(notes.iter().copied())),
    );
    json.insert("problems".into(), json!([]));
    Ok(Report {
        json,
        text,
        findings,
    })
}

/// The keys every answer of `get application` has, whether it found the
/// application or not: `application` is null when it didn't.
fn application_json(
    application: Value,
    notes: &[&Note],
    sign_in: &[String],
    problems: &[Problem],
) -> Map<String, Value> {
    let mut json = Map::new();
    json.insert("kind".into(), json!({"key": "application"}));
    json.insert("namespace".into(), Value::Null);
    json.insert("application".into(), application);
    json.insert(
        "notes".into(),
        Value::Array(notes_json(notes.iter().copied())),
    );
    json.insert("sign_in".into(), json!(sign_in));
    json.insert(
        "problems".into(),
        Value::Array(problems.iter().map(Problem::json).collect()),
    );
    json
}

/// `freshkube get application <app>`: the application's Stages, in promotion
/// order, with what each runs and has verified. GitHub is not asked.
pub async fn application(name: &str, target: &Target) -> Result<Report, Stop> {
    let Fetched { read, sign_in } = match delivery::waiting(target, false).await {
        Ok(fetched) => fetched,
        Err(problem) => {
            return Ok(Report {
                json: application_json(Value::Null, &[], &[], std::slice::from_ref(&problem)),
                text: format!("? {}\n", problem.sentence()),
                findings: problem.findings(),
            });
        }
    };
    let waiting = derive(&read);
    let mut findings = Findings::default();
    let mut problems: Vec<Problem> = Vec::new();
    let mut said: Vec<Note> = Vec::new();
    let app = waiting.apps.iter().find(|app| app.name == name);
    match &read.projects {
        // Applications are Kargo projects; one that isn't in the list that
        // was read in full is not there. Past a cap it may only be unread.
        Source::Read(_) if app.is_none() => problems.push(Problem {
            class: Class::NotFound,
            read: Box::new(Read {
                verb: "get",
                kind: "application".into(),
                namespaced: false,
                namespace: None,
                context: target.context.clone(),
            }),
            message: format!("no application \"{name}\""),
        }),
        Source::Capped(_, truncation) if app.is_none() => said.push(Note {
            github: false,
            gap: Gap::Unreadable,
            project: None,
            text: format!(
                "Only {} Kargo projects on {} were read; {name} may be among the others",
                truncation.read, read.cluster
            ),
        }),
        _ => {}
    }
    // This command asks GitHub nothing, so what is said of GitHub is moot.
    said.extend(
        waiting
            .notes
            .iter()
            .filter(|note| {
                note.project
                    .as_deref()
                    .is_none_or(|project| project == name)
            })
            .filter(|note| !note.github)
            .cloned(),
    );
    let notes: Vec<&Note> = said.iter().collect();
    for note in &notes {
        findings.merge(note_findings(note));
    }
    for problem in &problems {
        findings.merge(problem.findings());
    }
    if !sign_in.is_empty() {
        findings.sign_in = true;
    }
    let mut text = String::new();
    let application = match app {
        Some(app) => {
            findings.merge(state_findings(app));
            text.push_str(&format!("Application {name} on {}\n", target.context));
            if let State::NotRead {
                cause, text: why, ..
            } = &app.state
            {
                text.push_str(&format!("? Stages not read · {}: {why}\n", cause.word()));
            }
            for stage in &app.stages {
                let runs = if stage.runs.is_empty() {
                    "runs nothing".to_owned()
                } else {
                    format!("runs {}", stage.runs.join(", "))
                };
                text.push_str(&format!(
                    "  {}{}  {runs} · {} verified\n",
                    stage.name,
                    if stage.is_final { " (last)" } else { "" },
                    stage.verified.len()
                ));
            }
            json!({
                "name": app.name,
                "state": state_word(&app.state),
                "stages": stages_json(app),
                "view": {"page": "application", "project": app.name},
            })
        }
        None => Value::Null,
    };
    for problem in &problems {
        text.push_str(&format!("x {}\n", problem.sentence()));
    }
    for note in &notes {
        text.push_str(&format!("? {}\n", note.text));
    }
    for entry in &sign_in {
        text.push_str(&format!(
            "? entry {entry} signs in with a credential plugin and was not connected; run it again with --allow-login\n"
        ));
    }
    Ok(Report {
        json: application_json(application, &notes, &sign_in, &problems),
        text,
        findings,
    })
}
