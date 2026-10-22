//! Reading the audit logs for Settings › Audit log: the writes log and
//! Talos Operations' per-context files side by side, across contexts, with a
//! Source filter. Read-only; nothing here writes.

use std::path::Path;

use chrono::{DateTime, Utc};

use super::{audit::AuditStore, audit::scrub};
use crate::operations::AuditLog;

/// Which log a row came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// Changes to Kubernetes objects through the write framework.
    Writes,
    /// Talos Operations (cordon, drain, reboot, shutdown).
    Operations,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Writes => "Writes",
            Source::Operations => "Operations",
        }
    }
}

/// The Source filter's choices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SourceFilter {
    #[default]
    All,
    Only(Source),
}

impl SourceFilter {
    pub const ALL: [SourceFilter; 3] = [
        SourceFilter::All,
        SourceFilter::Only(Source::Writes),
        SourceFilter::Only(Source::Operations),
    ];

    pub fn label(self) -> &'static str {
        match self {
            SourceFilter::All => "All",
            SourceFilter::Only(source) => source.label(),
        }
    }

    pub fn admits(self, source: Source) -> bool {
        match self {
            SourceFilter::All => true,
            SourceFilter::Only(only) => only == source,
        }
    }
}

/// One line of either log, in one shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub at: DateTime<Utc>,
    pub source: Source,
    /// The context (writes) or cluster label (Operations).
    pub context: String,
    pub actor: String,
    pub action: String,
    /// The object or node it reached.
    pub object: String,
    pub phase: String,
    /// Field paths for a write; empty for Operations.
    pub fields: Vec<String>,
    pub result: String,
}

/// Everything both logs hold, newest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    pub rows: Vec<Row>,
    /// Lines or files that could not be read, so the view can say so rather
    /// than look complete.
    pub unreadable: usize,
}

impl History {
    pub fn shown(&self, filter: SourceFilter) -> impl Iterator<Item = &Row> {
        self.rows
            .iter()
            .filter(move |row| filter.admits(row.source))
    }
}

fn phase_word(phase: impl serde::Serialize) -> String {
    serde_json::to_value(phase)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Reads the writes log and every `operations-*.yaml` in `operations_dir`.
pub fn load(writes: &AuditStore, operations_dir: &Path) -> History {
    let mut history = History::default();
    match writes.read() {
        Ok(read) => {
            history.unreadable += read.skipped;
            history.rows.extend(read.records.into_iter().map(|record| {
                Row {
                    at: record.at,
                    source: Source::Writes,
                    context: record.context,
                    actor: record.actor,
                    action: record.action,
                    object: record
                        .objects
                        .iter()
                        .map(|o| o.name.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                    phase: phase_word(record.phase),
                    fields: record.objects.into_iter().flat_map(|o| o.fields).collect(),
                    result: record.result,
                }
            }));
        }
        Err(_) => history.unreadable += 1,
    }
    let files = std::fs::read_dir(operations_dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            name.starts_with("operations-") && name.ends_with(".yaml")
                        })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for file in files {
        match AuditLog::new(&file, "").read_entries() {
            Ok(entries) => history.rows.extend(entries.into_iter().map(|entry| Row {
                at: entry.timestamp,
                source: Source::Operations,
                context: entry.cluster,
                actor: entry.actor,
                action: entry.operation.label().to_owned(),
                object: entry.target.name,
                phase: phase_word(entry.phase),
                fields: vec![],
                result: scrub(&entry.message),
            })),
            Err(_) => history.unreadable += 1,
        }
    }
    history.rows.sort_by(|a, b| b.at.cmp(&a.at));
    history
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::{AuditEntry, AuditPhase, NodeTarget, OperationKind, OperationStep};
    use crate::write::testing::{self as fake, Cluster};
    use crate::write::{Confirmation, Protection, Verdict};

    fn operations_entry(cluster: &str, at: &str) -> AuditEntry {
        AuditEntry {
            timestamp: at.parse().unwrap(),
            cluster: cluster.into(),
            actor: "ops".into(),
            operation: OperationKind::Cordon,
            target: NodeTarget::new("node-1", "10.0.0.1"),
            phase: AuditPhase::Success,
            step: OperationStep::Cordon,
            message: "cordoned".into(),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn both_logs_are_read_across_contexts_newest_first_with_a_source_filter() {
        let dir = tempfile::tempdir().unwrap();
        let operations = dir.path().join("ops");
        AuditLog::with_actor(operations.join("operations-a.yaml"), "cluster-a", "ops")
            .append(&operations_entry("cluster-a", "2020-01-01T00:00:00Z"))
            .unwrap();
        AuditLog::with_actor(operations.join("operations-b.yaml"), "cluster-b", "ops")
            .append(&operations_entry("cluster-b", "2020-01-02T00:00:00Z"))
            .unwrap();
        std::fs::write(operations.join("notes.txt"), "not a log").unwrap();
        let cluster = Cluster::new();
        let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
        let Verdict::Ready(prepared) = runner.preflight(fake::scale(3)).await else {
            panic!()
        };
        runner
            .execute(*prepared, Confirmation::Confirmed, || false, |_| {})
            .await;
        let history = load(&AuditStore::new(dir.path().join("audit")), &operations);
        assert_eq!(history.unreadable, 0);
        assert!(history.rows.windows(2).all(|w| w[0].at >= w[1].at));
        let writes: Vec<_> = history.shown(SourceFilter::Only(Source::Writes)).collect();
        let ops: Vec<_> = history
            .shown(SourceFilter::Only(Source::Operations))
            .collect();
        assert_eq!((writes.len(), ops.len()), (4, 2));
        assert_eq!(history.shown(SourceFilter::All).count(), 6);
        assert_eq!(ops[0].context, "cluster-b");
        assert_eq!(
            (ops[0].action.as_str(), ops[0].phase.as_str()),
            ("cordon", "success")
        );
        assert!(
            writes
                .iter()
                .any(|r| r.phase == "sending" && r.fields == ["spec.replicas"])
        );
    }

    #[test]
    fn a_missing_log_is_empty_and_a_broken_one_is_counted() {
        let dir = tempfile::tempdir().unwrap();
        let empty = load(
            &AuditStore::new(dir.path().join("none")),
            &dir.path().join("nothing"),
        );
        assert_eq!(empty, History::default());
        std::fs::write(dir.path().join("operations-x.yaml"), "---\nnot: [valid").unwrap();
        assert_eq!(
            load(&AuditStore::new(dir.path().join("none")), dir.path()).unreadable,
            1
        );
    }
}
