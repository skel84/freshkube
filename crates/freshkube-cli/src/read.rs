//! One read of a kind, through the same `list_table` the Resources page
//! uses, and what the commands make of its rows.

use std::time::Duration;

use freshkube_core::constants::SCREEN_DEADLINE;
use freshkube_core::failure::{Failure, FailureKind};
use freshkube_core::resources::{ResourceKind, StatusTone, Table, list_table, row_tone};
use serde_json::{Value, json};

use crate::cluster::Target;
use crate::exit::Findings;

/// How a read that did not return rows ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Failed,
    Refused,
    SignIn,
    Unknown,
}

impl Class {
    pub fn of(kind: FailureKind) -> Self {
        match kind {
            FailureKind::Forbidden => Class::Refused,
            FailureKind::Unauthorized => Class::SignIn,
            FailureKind::Unreachable | FailureKind::Timeout => Class::Unknown,
            FailureKind::Config | FailureKind::NotFound | FailureKind::Other => Class::Failed,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Class::Failed => "failed",
            Class::Refused => "refused",
            Class::SignIn => "sign-in",
            Class::Unknown => "unknown",
        }
    }
}

/// The read a problem is about: its verb, kind, namespace and context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    pub verb: &'static str,
    pub kind: String,
    pub namespaced: bool,
    pub namespace: Option<String>,
    pub context: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub class: Class,
    pub read: Read,
    pub message: String,
}

impl Problem {
    pub fn findings(&self) -> Findings {
        let mut findings = Findings::default();
        match self.class {
            Class::Failed => findings.failed = true,
            Class::Refused => findings.refused = true,
            Class::SignIn => findings.sign_in = true,
            Class::Unknown => findings.unknown = true,
        }
        findings
    }

    /// `refused: list pods in namespace "shop" on context "acme": …`
    pub fn sentence(&self) -> String {
        let read = &self.read;
        let place = match (&read.namespace, read.namespaced) {
            (Some(namespace), true) => format!(" in namespace \"{namespace}\""),
            (None, true) => " in all namespaces".to_owned(),
            _ => String::new(),
        };
        format!(
            "{}: {} {}{place} on context \"{}\": {}",
            self.class.word(),
            read.verb,
            read.kind,
            read.context,
            self.message
        )
    }

    pub fn json(&self) -> Value {
        let read = &self.read;
        json!({
            "state": self.class.word(),
            "verb": read.verb,
            "kind": read.kind,
            "namespace": read.namespace,
            "context": read.context,
            "message": self.message,
        })
    }
}

/// A listed row with the state its printed columns give it.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub namespace: Option<String>,
    pub name: String,
    pub cells: Vec<String>,
    pub tone: Option<(StatusTone, String)>,
}

impl Row {
    /// The text a state shows as: `good`, `warn`, `crit` or `unknown`.
    pub fn state(&self) -> Option<&'static str> {
        self.tone.as_ref().map(|(tone, _)| tone_word(*tone))
    }
}

pub fn tone_word(tone: StatusTone) -> &'static str {
    match tone {
        StatusTone::Success => "good",
        StatusTone::Warning => "warn",
        StatusTone::Danger => "crit",
        StatusTone::Neutral => "unknown",
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listing {
    pub columns: Vec<String>,
    pub rows: Vec<Row>,
}

/// Where a row's object opens in the app: the location a terminal route
/// can hand to the window.
pub fn view(kind: &ResourceKind, row: &Row) -> Value {
    json!({
        "page": "resources",
        "kind": kind.key(),
        "namespace": row.namespace,
        "name": row.name,
    })
}

fn text(cell: &Value) -> String {
    match cell {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The rows of a table, in the columns kubectl shows by default (all of them
/// when `wide`). A pod that says Running but is not fully ready is a warning,
/// as in the pods list.
pub fn listing(kind: &ResourceKind, table: Table, wide: bool) -> Listing {
    let shown: Vec<usize> = table
        .column_definitions
        .iter()
        .enumerate()
        .filter(|(_, column)| wide || column.priority == 0)
        .map(|(ix, _)| ix)
        .collect();
    let columns: Vec<String> = shown
        .iter()
        .map(|ix| table.column_definitions[*ix].name.clone())
        .collect();
    let rows = table
        .rows
        .iter()
        .map(|row| {
            let cells: Vec<String> = shown
                .iter()
                .map(|ix| row.cells.get(*ix).map(text).unwrap_or_default())
                .collect();
            let metadata = row.metadata();
            let name = metadata
                .map(|m| m.name.clone())
                .filter(|name| !name.is_empty())
                .or_else(|| cells.first().cloned())
                .unwrap_or_default();
            let namespace = metadata
                .and_then(|m| m.namespace.clone())
                .filter(|namespace| !namespace.is_empty());
            let tone = row_tone(&columns, &cells).map(|found| {
                if kind.is_pod() {
                    pod_tone(&columns, &cells, found)
                } else {
                    found
                }
            });
            Row {
                namespace,
                name,
                cells,
                tone,
            }
        })
        .collect();
    Listing { columns, rows }
}

fn pod_tone(
    columns: &[String],
    cells: &[String],
    (tone, label): (StatusTone, String),
) -> (StatusTone, String) {
    if tone != StatusTone::Success || label != "Running" {
        return (tone, label);
    }
    let ready = columns
        .iter()
        .position(|name| name.eq_ignore_ascii_case("ready"))
        .and_then(|ix| cells.get(ix))
        .and_then(|text| freshkube_core::resources::ready_tone(text));
    match ready {
        Some(StatusTone::Success) | None => (tone, label),
        Some(not_ready) => (
            if not_ready == StatusTone::Danger {
                StatusTone::Warning
            } else {
                not_ready
            },
            "Running, not ready".to_owned(),
        ),
    }
}

/// Lists a kind. Every read is a GET of a collection; a failure is
/// classified, never raised.
pub async fn list(
    target: &Target,
    kind: &ResourceKind,
    namespace: Option<&str>,
    selector: Option<&str>,
    wide: bool,
) -> Result<Listing, Problem> {
    let read = Read {
        verb: "list",
        kind: kind.key(),
        namespaced: kind.namespaced,
        namespace: namespace.map(str::to_owned),
        context: target.context.clone(),
    };
    let problem = |failure: Failure| Problem {
        class: Class::of(failure.kind),
        read: read.clone(),
        message: failure.message,
    };
    let client = target.source.access.client().await.map_err(&problem)?;
    let table = match tokio::time::timeout(
        deadline(),
        list_table(&client, kind, namespace, selector),
    )
    .await
    {
        Ok(result) => result.map_err(&problem)?,
        Err(_) => return Err(problem(Failure::timeout("The read"))),
    };
    Ok(listing(kind, table, wide))
}

fn deadline() -> Duration {
    SCREEN_DEADLINE
}
