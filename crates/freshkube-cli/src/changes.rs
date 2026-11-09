//! `freshkube changes <app>`: an application's changes, from the same read
//! and derivation the Changes list uses, one block per Freight with how sure
//! it is that each Stage's pods run it.

use chrono::{DateTime, Duration, Utc};
use freshkube_delivery::change::Reach;
use freshkube_delivery::changes::{Changes, Fold, Named, Note, Row, derive};
use freshkube_delivery::source::Source;
use serde_json::{Map, Value, json};

use crate::Report;
use crate::args::ChangesArgs;
use crate::cluster::{Stop, Target};
use crate::delivery;
use crate::exit::Findings;
use crate::notes::{ago, note_findings, notes_json, span};
use crate::read::{Class, Problem, Read};

/// The options that need no cluster, checked before anything is read.
pub struct Wanted {
    pub since: Option<Duration>,
}

pub fn wanted(args: &ChangesArgs) -> Result<Wanted, Stop> {
    let since = args
        .since
        .as_deref()
        .map(span)
        .transpose()
        .map_err(|why| Stop::Usage(format!("--since: {why}")))?;
    Ok(Wanted { since })
}

fn reach_word(reach: &Reach) -> &'static str {
    match reach {
        Reach::NotReached => "not-reached",
        Reach::Refused(_) => "refused",
        Reach::Linked(confidence) => confidence.word(),
    }
}

fn glyph(reach: &Reach) -> char {
    match reach {
        Reach::NotReached => '-',
        Reach::Refused(_) => '?',
        Reach::Linked(confidence) => match confidence.word() {
            "confirmed" => '+',
            "claimed" => '!',
            _ => '?',
        },
    }
}

fn fold_word(fold: Fold) -> &'static str {
    match fold {
        Fold::NotConfirmed => "not-confirmed",
        Fold::Confirmed => "confirmed",
        Fold::NotRunning => "not-running",
    }
}

fn named(row: &Row) -> (String, Value) {
    match &row.named {
        Named::PullRequest(number) => (
            format!("pull request #{number}"),
            json!({"kind": "pull-request", "number": number}),
        ),
        Named::Commit(commit) => (
            format!("commit {}", commit.chars().take(7).collect::<String>()),
            json!({"kind": "commit", "commit": commit}),
        ),
        Named::Nothing => ("no commit".into(), json!({"kind": "none"})),
    }
}

fn missing(
    app: &str,
    projects: &Source<Vec<freshkube_delivery::changes::Project>>,
) -> Option<String> {
    let all = projects.read()?;
    if all.iter().any(|project| project.name == app) {
        return None;
    }
    let mut names: Vec<&str> = all.iter().map(|project| project.name.as_str()).collect();
    names.sort_unstable();
    Some(format!(
        "no Kargo project \"{app}\". Projects: {}",
        if names.is_empty() {
            "none".to_owned()
        } else {
            names.join(", ")
        }
    ))
}

pub async fn run(args: &ChangesArgs, target: &Target) -> Result<Report, Stop> {
    let Wanted { since } = wanted(args)?;
    let (read, now) = match delivery::changes(target).await {
        Ok(read) => read,
        Err(problem) => return Ok(problem_report(args, &problem)),
    };
    let list = derive(&read);
    let mut findings = Findings::default();
    let mut problems: Vec<Problem> = Vec::new();
    if let Some(why) = missing(&args.app, &read.projects) {
        let problem = Problem {
            class: Class::NotFound,
            read: Box::new(Read {
                verb: "list",
                kind: "Kargo project".into(),
                namespaced: false,
                namespace: None,
                context: target.context.clone(),
            }),
            message: why,
        };
        findings.merge(problem.findings());
        problems.push(problem);
    }
    // Every row carries its project's Stages in promotion order.
    let stage_names: Vec<String> = list
        .rows
        .iter()
        .find(|row| row.project == args.app)
        .map(|row| row.stages.iter().map(|stage| stage.name.clone()).collect())
        .unwrap_or_default();
    if let Some(wanted) = &args.stage
        && !stage_names.is_empty()
        && !stage_names.contains(wanted)
    {
        let problem = Problem {
            class: Class::NotFound,
            read: Box::new(Read {
                verb: "list",
                kind: "Kargo stage".into(),
                namespaced: false,
                namespace: None,
                context: target.context.clone(),
            }),
            message: format!(
                "{} has no Stage \"{wanted}\". Stages: {}",
                args.app,
                stage_names.join(", ")
            ),
        };
        findings.merge(problem.findings());
        problems.push(problem);
    }
    let rows = chosen(&list, args, since, now);
    let notes: Vec<&Note> = list
        .notes
        .iter()
        .filter(|note| note.project.as_deref().is_none_or(|name| name == args.app))
        .collect();
    for note in &notes {
        findings.merge(note_findings(note));
    }
    for row in &rows {
        for stage in shown(row, args) {
            if matches!(stage.reach, Reach::Refused(_)) {
                findings.refused = true;
            }
        }
    }
    let text = text(args, target, &stage_names, &rows, &notes, &problems, now);
    let mut json = Map::new();
    json.insert("application".into(), json!(args.app));
    json.insert("stages".into(), json!(stage_names));
    json.insert("stage".into(), json!(args.stage));
    json.insert("since".into(), json!(since.map(|d| d.num_seconds())));
    json.insert("github".into(), json!("anonymous"));
    json.insert(
        "changes".into(),
        Value::Array(rows.iter().map(|row| row_json(row, args, now)).collect()),
    );
    json.insert("notes".into(), Value::Array(notes_json(&notes)));
    json.insert(
        "problems".into(),
        Value::Array(problems.iter().map(Problem::json).collect()),
    );
    Ok(Report {
        json,
        text,
        findings,
    })
}

fn problem_report(args: &ChangesArgs, problem: &Problem) -> Report {
    let mut json = Map::new();
    json.insert("application".into(), json!(args.app));
    json.insert("changes".into(), json!([]));
    json.insert("notes".into(), json!([]));
    json.insert("problems".into(), json!([problem.json()]));
    Report {
        json,
        text: format!("? {}\n", problem.sentence()),
        findings: problem.findings(),
    }
}

/// The rows of the application, newest first as the list orders them, within
/// `--since` and, with `--stage`, those the Stage runs.
fn chosen<'a>(
    list: &'a Changes,
    args: &ChangesArgs,
    since: Option<Duration>,
    now: DateTime<Utc>,
) -> Vec<&'a Row> {
    list.rows
        .iter()
        .filter(|row| row.project == args.app)
        .filter(|row| match since {
            Some(span) => row.created.is_some_and(|created| now - created <= span),
            None => true,
        })
        .filter(|row| match &args.stage {
            Some(stage) => row
                .stages
                .iter()
                .any(|s| &s.name == stage && s.reach != Reach::NotReached),
            None => true,
        })
        .collect()
}

fn shown<'a>(
    row: &'a Row,
    args: &'a ChangesArgs,
) -> impl Iterator<Item = &'a freshkube_delivery::changes::StageReach> {
    row.stages.iter().filter(move |stage| {
        args.stage
            .as_ref()
            .is_none_or(|wanted| &stage.name == wanted)
    })
}

fn row_json(row: &Row, args: &ChangesArgs, now: DateTime<Utc>) -> Value {
    let (_, change) = named(row);
    let stages: Vec<Value> = shown(row, args)
        .map(|stage| {
            let detail = match &stage.reach {
                Reach::Refused(why) => Some(why.clone()),
                _ => None,
            };
            json!({"name": stage.name, "reach": reach_word(&stage.reach), "detail": detail})
        })
        .collect();
    json!({
        "key": row.key,
        "project": row.project,
        "freight": row.freight,
        "alias": row.alias,
        "change": change,
        "pull_state": row.pull_state.map(|state| state.label()),
        "pull_note": row.pull_note,
        "title": row.title,
        "created": row.created.map(|created| created.to_rfc3339()),
        "age_seconds": row.created.map(|created| (now - created).num_seconds().max(0)),
        "state": reach_word(&row.state),
        "fold": fold_word(row.fold),
        "stages": stages,
        "view": {"page": "change", "project": row.project, "freight": row.freight},
    })
}

fn text(
    args: &ChangesArgs,
    target: &Target,
    stage_names: &[String],
    rows: &[&Row],
    notes: &[&Note],
    problems: &[Problem],
    now: DateTime<Utc>,
) -> String {
    let mut out = format!(
        "Changes of {} on {} ({} {})\n",
        args.app,
        target.context,
        rows.len(),
        if rows.len() == 1 { "change" } else { "changes" }
    );
    if !stage_names.is_empty() {
        out.push_str(&format!("Stages: {}\n", stage_names.join(" → ")));
    }
    for problem in problems {
        out.push_str(&format!("x {}\n", problem.sentence()));
    }
    for row in rows {
        let (what, _) = named(row);
        let name = row.alias.as_deref().unwrap_or(&row.freight);
        let age = row
            .created
            .map(|created| format!("  {} ago", ago(now - created)))
            .unwrap_or_default();
        let pull = row
            .pull_state
            .map(|state| format!(" · {}", state.label()))
            .unwrap_or_default();
        out.push_str(&format!("\n{name}  {what}{pull}{age}\n"));
        if let Some(title) = &row.title {
            out.push_str(&format!("  {title}\n"));
        }
        if let Some(note) = &row.pull_note {
            out.push_str(&format!("  ? {note}\n"));
        }
        let cells: Vec<String> = shown(row, args)
            .map(|stage| {
                format!(
                    "{} {} {}",
                    glyph(&stage.reach),
                    stage.name,
                    reach_word(&stage.reach)
                )
            })
            .collect();
        if !cells.is_empty() {
            out.push_str(&format!("  {}\n", cells.join("   ")));
        }
    }
    for note in notes {
        out.push_str(&format!("\n? {}\n", note.text));
    }
    if let Some(note) = delivery::github_note(target) {
        out.push_str(&format!("\n{note}\n"));
    }
    out
}
