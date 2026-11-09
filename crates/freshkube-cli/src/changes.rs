//! `freshkube changes <app>`: an application's changes, from the same read
//! and derivation the Changes list uses, one block per Freight with how sure
//! it is that each Stage's pods run it.

use chrono::{DateTime, Duration, Utc};
use freshkube_core::failure::FailureKind;
use freshkube_delivery::change::Reach;
use freshkube_delivery::changes::{Changes, Fold, Gap, Named, Note, Row, derive};
use freshkube_delivery::source::Source;
use serde_json::{Map, Value, json};

use crate::Report;
use crate::args::ChangesArgs;
use crate::cluster::{Stop, Target};
use crate::delivery::{self, Fetched};
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

fn not_found(target: &Target, kind: &str, message: String) -> Problem {
    Problem {
        class: Class::NotFound,
        read: Box::new(Read {
            verb: "list",
            kind: kind.into(),
            namespaced: false,
            namespace: None,
            context: target.context.clone(),
        }),
        message,
    }
}

/// What the application's own changes could not be told, from the changes
/// this command shows: the lines each change's trail is missing and the
/// changes that were not followed at all, each once. Other applications'
/// changes are not looked at, so what they miss never reaches this one.
fn followed(rows: &[&Row], app: &str, findings: &mut Findings) -> Vec<Note> {
    let mut notes: Vec<Note> = Vec::new();
    let mut keep = |note: Note| {
        if !notes.iter().any(|held| held.text == note.text) {
            notes.push(note);
        }
    };
    for row in rows {
        match &row.trail {
            Ok(change) => {
                for line in &change.missing {
                    keep(Note {
                        github: line.github,
                        gap: match line.gap {
                            freshkube_delivery::change::Gap::NotServed => Gap::NotServed,
                            freshkube_delivery::change::Gap::Refused => Gap::Refused,
                            freshkube_delivery::change::Gap::Unreadable => Gap::Unreadable,
                        },
                        project: Some(app.to_owned()),
                        text: line.text.clone(),
                    });
                }
            }
            // A change no Stage runs, or whose Stages weren't read (a note
            // of the project says so), was never meant to be followed.
            Err(_) if row.fold == Fold::NotRunning || row.stages.is_empty() => {}
            Err(failure) => {
                findings.sign_in |= failure.kind == FailureKind::Unauthorized;
                keep(Note {
                    github: false,
                    gap: match failure.kind {
                        FailureKind::Forbidden => Gap::Refused,
                        _ => Gap::Unreadable,
                    },
                    project: Some(app.to_owned()),
                    text: format!("{} wasn't followed: {}", row.freight, failure.message),
                });
            }
        }
    }
    notes
}

pub async fn run(args: &ChangesArgs, target: &Target) -> Result<Report, Stop> {
    let Wanted { since } = wanted(args)?;
    let Fetched {
        read: (read, now),
        sign_in,
    } = match delivery::changes(target).await {
        Ok(fetched) => fetched,
        Err(problem) => return Ok(problem_report(args, &problem)),
    };
    let list = derive(&read);
    let mut findings = Findings::default();
    let mut problems: Vec<Problem> = Vec::new();
    let mut notes: Vec<Note> = Vec::new();
    let all = match &read.projects {
        Source::Read(all) | Source::Capped(all, _) => Some(all),
        _ => None,
    };
    let project = all.and_then(|all| all.iter().find(|project| project.name == args.app));
    match &read.projects {
        Source::Read(all) if project.is_none() => {
            let mut names: Vec<&str> = all.iter().map(|project| project.name.as_str()).collect();
            names.sort_unstable();
            problems.push(not_found(
                target,
                "Kargo project",
                format!(
                    "no Kargo project \"{}\". Projects: {}",
                    args.app,
                    if names.is_empty() {
                        "none".to_owned()
                    } else {
                        names.join(", ")
                    }
                ),
            ));
        }
        // The list stopped at its cap: the application may be past it, which
        // is not the same as not being there.
        Source::Capped(_, truncation) if project.is_none() => notes.push(Note {
            github: false,
            gap: Gap::Unreadable,
            project: None,
            text: format!(
                "Only {} Kargo projects on {} were read; {} may be among the others",
                truncation.read, read.cluster, args.app
            ),
        }),
        _ => {}
    }
    // When Kargo itself wasn't read, the list is that one note. Otherwise a
    // note with no project is about every project (a cap, a change that
    // wasn't followed in time, a destination one of them lacks) and is left
    // to the changes below, which say what they miss themselves.
    if all.is_none() || read.unreached.is_some() {
        notes.extend(list.notes.iter().cloned());
    } else {
        notes.extend(
            list.notes
                .iter()
                .filter(|note| note.project.as_deref() == Some(args.app.as_str()))
                .cloned(),
        );
    }
    // The Stages as Kargo lists them; in promotion order when a change
    // carries them.
    let mut stage_set: Vec<String> = project
        .and_then(|project| project.stages.read())
        .map(|stages| stages.iter().map(|stage| stage.name.clone()).collect())
        .unwrap_or_default();
    stage_set.sort();
    let stages_read = project.is_some_and(|project| project.stages.read().is_some());
    let stage_names: Vec<String> = list
        .rows
        .iter()
        .find(|row| row.project == args.app)
        .map(|row| row.stages.iter().map(|stage| stage.name.clone()).collect())
        .unwrap_or_else(|| stage_set.clone());
    if let Some(wanted) = &args.stage
        && stages_read
        && !stage_set.contains(wanted)
    {
        problems.push(not_found(
            target,
            "Kargo stage",
            format!(
                "{} has no Stage \"{wanted}\". Stages: {}",
                args.app,
                stage_set.join(", ")
            ),
        ));
    }
    let rows = chosen(&list, args, since, now);
    notes.extend(followed(&rows, &args.app, &mut findings));
    for problem in &problems {
        findings.merge(problem.findings());
    }
    for note in &notes {
        findings.merge(note_findings(note));
    }
    if !sign_in.is_empty() {
        findings.sign_in = true;
    }
    for row in &rows {
        for stage in shown(row, args) {
            if matches!(stage.reach, Reach::Refused(_)) {
                findings.refused = true;
            }
        }
    }
    let text = text(args, target, &stage_names, &rows, &notes, &problems, now);
    let mut text = text;
    for entry in &sign_in {
        text.push_str(&format!(
            "\n? entry {entry} signs in with a credential plugin and was not connected; run it again with --allow-login\n"
        ));
    }
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
    json.insert("sign_in".into(), json!(sign_in));
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
    notes: &[Note],
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
