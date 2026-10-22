//! `freshkube check`: the cluster's health from the same lists the Resources
//! page shows, with what to do about each failure.

use freshkube_core::resources::{ResourceKind, StatusTone, find_kind};
use futures::future::join_all;
use serde_json::{Map, Value, json};

use crate::Report;
use crate::args::{CheckArgs, FailOn};
use crate::cluster::{Stop, Target};
use crate::exit::Findings;
use crate::read::{Class, Listing, Problem, Row, list, view};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Severity {
    Pass,
    Unknown,
    Warn,
    Fail,
}

impl Severity {
    fn word(self) -> &'static str {
        match self {
            Severity::Pass => "pass",
            Severity::Unknown => "unknown",
            Severity::Warn => "warn",
            Severity::Fail => "fail",
        }
    }
}

struct Spec {
    id: &'static str,
    title: &'static str,
    kind: &'static str,
    /// Namespaced checks honour `--namespace`.
    judge: fn(&Row) -> (Severity, Option<&'static str>),
}

const SPECS: [Spec; 4] = [
    Spec {
        id: "nodes-ready",
        title: "Nodes are ready",
        kind: "nodes",
        judge: node,
    },
    Spec {
        id: "pods-healthy",
        title: "Pods are healthy",
        kind: "pods",
        judge: pod,
    },
    Spec {
        id: "deployments-ready",
        title: "Deployments have their replicas",
        kind: "deployments.apps",
        judge: workload,
    },
    Spec {
        id: "statefulsets-ready",
        title: "StatefulSets have their replicas",
        kind: "statefulsets.apps",
        judge: workload,
    },
];

fn reason(row: &Row) -> &str {
    row.tone.as_ref().map_or("", |(_, reason)| reason)
}

fn node(row: &Row) -> (Severity, Option<&'static str>) {
    let reason = reason(row);
    if reason.starts_with("NotReady") {
        (
            Severity::Fail,
            Some(
                "The node's kubelet or network is down. Read its conditions on the Nodes page; on Talos, check the kubelet service.",
            ),
        )
    } else if reason.contains("SchedulingDisabled") {
        (
            Severity::Warn,
            Some("The node is cordoned. Uncordon it when its work is done."),
        )
    } else {
        by_tone(row, "Read the object's events on the Resources page.")
    }
}

fn pod(row: &Row) -> (Severity, Option<&'static str>) {
    by_tone(
        row,
        "Read the pod's events and its container's logs on the Resources page.",
    )
}

fn workload(row: &Row) -> (Severity, Option<&'static str>) {
    by_tone(
        row,
        "Read the workload's pods and events on the Resources page.",
    )
}

fn by_tone(row: &Row, fix: &'static str) -> (Severity, Option<&'static str>) {
    match row.tone.as_ref().map(|(tone, _)| tone) {
        Some(StatusTone::Success) => (Severity::Pass, None),
        Some(StatusTone::Warning) => (Severity::Warn, Some(fix)),
        Some(StatusTone::Danger) => (Severity::Fail, Some(fix)),
        Some(StatusTone::Neutral) | None => (Severity::Unknown, None),
    }
}

/// A row with how bad it is and what to do about it.
type Judged = (Row, Severity, Option<&'static str>);

struct Outcome {
    id: &'static str,
    title: &'static str,
    kind: ResourceKind,
    result: Result<(Listing, Vec<Judged>), Problem>,
    state: String,
    severity: Option<Severity>,
}

pub async fn run(args: &CheckArgs, target: &Target) -> Result<Report, Stop> {
    // All the reads go out together, so a hung cluster costs one deadline.
    let reads = SPECS.iter().map(|spec| async {
        let kind = find_kind(spec.kind).expect("a built-in kind");
        let namespace = args.namespace.as_deref().filter(|_| kind.namespaced);
        let listed = list(target, &kind, namespace, None, false).await;
        (kind, listed)
    });
    let mut listed = join_all(reads).await;
    // A pod on a node that is not ready warns, as in the pods list.
    let not_ready = listed
        .iter()
        .find(|(kind, _)| kind.key() == "nodes")
        .and_then(|(_, result)| result.as_ref().ok())
        .map(|listing| listing.not_ready_nodes())
        .unwrap_or_default();
    for (kind, result) in &mut listed {
        if let (true, Ok(listing)) = (kind.is_pod(), result) {
            listing.note_nodes(&not_ready);
        }
    }
    let outcomes = SPECS
        .iter()
        .zip(listed)
        .map(|(spec, (kind, result))| outcome(spec, kind, result))
        .collect();
    Ok(report(args, outcomes))
}

fn outcome(spec: &Spec, kind: ResourceKind, result: Result<Listing, Problem>) -> Outcome {
    let result = result.map(|listing| {
        let judged: Vec<_> = listing
            .rows
            .iter()
            .map(|row| {
                let (severity, fix) = (spec.judge)(row);
                (row.clone(), severity, fix)
            })
            .collect();
        (listing, judged)
    });
    let (state, severity) = match &result {
        Ok((_, judged)) => {
            let worst = judged
                .iter()
                .map(|(_, severity, _)| *severity)
                .max()
                .unwrap_or(Severity::Pass);
            (worst.word().to_owned(), Some(worst))
        }
        Err(problem) => (
            match problem.class {
                Class::Failed | Class::NotFound => "fail".to_owned(),
                other => other.word().to_owned(),
            },
            None,
        ),
    };
    Outcome {
        id: spec.id,
        title: spec.title,
        kind,
        result,
        state,
        severity,
    }
}

fn report(args: &CheckArgs, outcomes: Vec<Outcome>) -> Report {
    let mut findings = Findings::default();
    let mut text = String::new();
    let mut checks = Vec::new();
    let mut counts = Map::new();
    for outcome in &outcomes {
        match (&outcome.result, outcome.severity) {
            (Ok(_), Some(Severity::Fail)) => findings.failed = true,
            (Ok(_), Some(Severity::Warn)) => findings.failed |= args.fail_on == FailOn::Warn,
            (Ok(_), Some(Severity::Unknown)) => findings.unknown = true,
            (Ok(_), _) => {}
            (Err(problem), _) => findings.merge(problem.findings()),
        }
        let count = counts
            .entry(outcome.state.clone())
            .or_insert_with(|| json!(0));
        *count = json!(count.as_u64().unwrap_or(0) + 1);
        let (glyph, summary, items, problem) = describe(outcome);
        text.push_str(&format!("{glyph} {} - {summary}\n", outcome.title));
        for (row, severity, fix) in shown(outcome) {
            text.push_str(&format!(
                "    {} {} {}\n",
                if severity == Severity::Fail { "x" } else { "!" },
                label(&row),
                reason(&row)
            ));
            if let Some(fix) = fix {
                text.push_str(&format!("      fix: {fix}\n"));
            }
        }
        if let Some(problem) = &problem {
            text.push_str(&format!("    {}\n", problem.sentence()));
        }
        let mut check = json!({
            "id": outcome.id,
            "title": outcome.title,
            "state": outcome.state,
            "summary": summary,
            "items": items,
        });
        if let Some(problem) = problem {
            check["problem"] = problem.json();
        }
        checks.push(check);
    }
    let mut json = Map::new();
    json.insert("checks".into(), Value::Array(checks));
    json.insert("summary".into(), Value::Object(counts));
    json.insert(
        "fail_on".into(),
        json!(match args.fail_on {
            FailOn::Fail => "fail",
            FailOn::Warn => "warn",
        }),
    );
    Report {
        json,
        text,
        findings,
    }
}

fn label(row: &Row) -> String {
    match &row.namespace {
        Some(namespace) => format!("{namespace}/{}", row.name),
        None => row.name.clone(),
    }
}

fn shown(outcome: &Outcome) -> Vec<Judged> {
    match &outcome.result {
        Ok((_, judged)) => judged
            .iter()
            .filter(|(_, severity, _)| *severity >= Severity::Warn)
            .cloned()
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// The glyph, the summary, the items that are not fine and the problem, if any.
fn describe(outcome: &Outcome) -> (&'static str, String, Vec<Value>, Option<Problem>) {
    match &outcome.result {
        Err(problem) => {
            let glyph = match problem.class {
                Class::Unknown => "?",
                Class::Refused | Class::SignIn | Class::Failed | Class::NotFound => "x",
            };
            (
                glyph,
                problem.class.word().to_owned(),
                Vec::new(),
                Some(problem.clone()),
            )
        }
        Ok((_, judged)) => {
            let bad = shown(outcome);
            let count = |wanted: Severity| judged.iter().filter(|(_, s, _)| *s == wanted).count();
            let summary = format!(
                "{} of {} fine, {} failing, {} warning",
                count(Severity::Pass),
                judged.len(),
                count(Severity::Fail),
                count(Severity::Warn)
            );
            let glyph = match outcome.severity {
                Some(Severity::Fail) => "x",
                Some(Severity::Warn) => "!",
                Some(Severity::Unknown) => "?",
                _ => "+",
            };
            let items = bad
                .iter()
                .map(|(row, severity, fix)| {
                    json!({
                        "namespace": row.namespace,
                        "name": row.name,
                        "state": severity.word(),
                        "reason": reason(row),
                        "fix": fix,
                        "view": view(&outcome.kind, row),
                    })
                })
                .collect();
            (glyph, summary, items, None)
        }
    }
}
