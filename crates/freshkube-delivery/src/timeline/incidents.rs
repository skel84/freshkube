//! The Incidents source: Coroot's incidents of the application's workloads.
//!
//! Coroot is read through Observability's one connection and the open
//! entry's project link, not a cluster [`Reader`](crate::read::Reader), so
//! the page hands this the sample it got, or why it got none. Coroot's list
//! is the project's latest 100 incidents across all states, never filtered
//! by the window, so the source shows what that sample holds and vouches for
//! no quiet hour ([`SourceRead::state_only`]); a full sample is also cut.
//! Nothing here guesses: an incident is the application's by its Coroot
//! application id (the entry's cluster, namespace, kind and name).
use freshkube_core::coroot::{Incident, IncidentState, ReadError, Status};

use super::live::Workload;
use super::{Item, Severity, Source, SourceRead};

/// How many incidents Coroot's list holds at most (core's request limit).
pub const SAMPLE: usize = 100;

/// Why Coroot's incidents weren't read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Absent {
    /// Observability holds no connection to Coroot.
    NotConnected,
    /// The open entry names no Coroot cluster.
    NotLinked,
    /// It names one, and neither it nor the connection names a project.
    NoProject,
    /// Coroot answered with a failure.
    Failed(ReadError),
    /// The read itself failed (timed out, stopped).
    Other(String),
}

impl Absent {
    /// The chip's reason.
    pub fn why(&self) -> String {
        match self {
            Self::NotConnected => "Coroot isn't connected".into(),
            Self::NotLinked => "the cluster isn't linked to a Coroot cluster".into(),
            Self::NoProject => "no Coroot project is named for the cluster".into(),
            Self::Failed(ReadError::Refused | ReadError::Authentication) => {
                "Forbidden: Coroot refused the incidents".into()
            }
            Self::Failed(ReadError::Unreachable) => "Unreachable: Coroot didn't answer".into(),
            Self::Failed(ReadError::Timeout) => "Timed out: Coroot didn't answer in time".into(),
            Self::Failed(other) => format!("can't read Coroot's incidents: {other}"),
            Self::Other(why) => why.clone(),
        }
    }
}

/// The incidents source from what Coroot answered for the window of `hours`
/// ending at `now`. `cluster` is the Coroot cluster the entry links to.
pub fn read(
    from: &str,
    cluster: &str,
    workloads: &[(String, Vec<Workload>)],
    sample: Result<Vec<Incident>, Absent>,
    now: i64,
    hours: u32,
) -> SourceRead {
    let since = now - i64::from(hours) * 3600;
    let incidents = match sample {
        Ok(incidents) => incidents,
        Err(absent) => return SourceRead::not_read(Source::Incidents, from, now, &absent.why()),
    };
    let ours = |incident: &&Incident| {
        let app = &incident.app;
        app.cluster_id() == cluster
            && workloads.iter().any(|(namespace, list)| {
                app.namespace() == Some(namespace.as_str())
                    && list
                        .iter()
                        .any(|w| w.name == app.name() && w.kind_name() == app.kind())
            })
    };
    let (mut items, mut unplaced) = (Vec::new(), 0);
    for incident in incidents.iter().filter(ours) {
        let Some(opened) = incident.opened_at.map(|at| at.timestamp()) else {
            unplaced += 1;
            continue;
        };
        let open = incident.state == IncidentState::Open;
        let ended = incident.resolved_at.map(|at| at.timestamp());
        // It overlaps the window: still open, or ended inside it.
        if !open && ended.unwrap_or(opened) < since {
            continue;
        }
        let mut detail = Vec::new();
        if opened < since {
            detail.push("started before this window".to_owned());
        }
        detail.push(if open {
            "open".to_owned()
        } else {
            format!(
                "resolved after {}",
                super::span_words(incident.duration.as_secs() as i64)
            )
        });
        if incident.impact_percent > 0.0 {
            detail.push(format!(
                "{:.1}% of requests affected",
                incident.impact_percent
            ));
        }
        if !incident.description.is_empty() {
            detail.push(incident.description.clone());
        }
        let critical = incident.severity == Status::Critical;
        items.push(Item {
            at: opened.max(since),
            severity: if critical {
                Severity::Bad
            } else {
                Severity::Warn
            },
            title: format!(
                "Incident {}: {}",
                if critical { "Critical" } else { "Warning" },
                incident.app.name()
            ),
            detail: detail.join(" · "),
        });
    }
    let mut read = SourceRead::read(Source::Incidents, from, now, items).state_only();
    let mut cuts = Vec::new();
    if incidents.len() >= SAMPLE {
        cuts.push(format!(
            "only Coroot's latest {SAMPLE} incidents were listed"
        ));
    }
    if unplaced > 0 {
        cuts.push(format!("{unplaced} without a start time"));
    }
    if !cuts.is_empty() {
        read = read.cut(&cuts.join("; "));
    }
    read
}
