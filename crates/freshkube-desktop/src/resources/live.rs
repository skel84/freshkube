//! Converts the server-printed tables from core into resource rows.

use freshkube_core::resources::{FailureKind, TableColumn, TableRow, WatchEvent};

use super::model::{ColumnKind, ReadState, ResourceColumn, ResourceIdentity, ResourceRow};
use super::store::ResourceEvent;

pub(crate) fn columns(definitions: &[TableColumn]) -> Vec<ResourceColumn> {
    definitions
        .iter()
        .map(|definition| {
            let kind = if definition.name.eq_ignore_ascii_case("age") {
                ColumnKind::Age
            } else if matches!(definition.column_type.as_str(), "integer" | "number") {
                ColumnKind::Number
            } else {
                ColumnKind::Text
            };
            ResourceColumn {
                name: definition.name.clone(),
                kind,
                wide: definition.priority > 0,
            }
        })
        .collect()
}

/// A printed row with its identity. Rows without object metadata, which the
/// server includes for every real object, are skipped.
pub(crate) fn row(connection: &str, resource: &str, row: &TableRow) -> Option<ResourceRow> {
    let metadata = row.metadata()?;
    if metadata.name.is_empty() {
        return None;
    }
    Some(ResourceRow {
        identity: ResourceIdentity {
            connection: connection.into(),
            resource: resource.into(),
            namespace: metadata.namespace.clone().unwrap_or_default(),
            name: metadata.name.clone(),
            uid: metadata.uid.clone(),
        },
        cells: row.cells.iter().map(cell_text).collect(),
        created: metadata
            .creation_timestamp
            .as_deref()
            .and_then(parse_timestamp),
        terminating: metadata.deletion_timestamp.is_some(),
    })
}

/// Cells are usually strings, but integer columns (an event's count) arrive
/// as JSON numbers.
fn cell_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(number) => number.to_string(),
        other => other.to_string(),
    }
}

/// An RFC 3339 timestamp, as Kubernetes writes them, in Unix seconds.
pub(crate) fn parse_timestamp(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.timestamp())
}

/// Unix seconds now.
pub(crate) fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// One watch batch as store events. A reset also marks the read loaded; a
/// refusal (403) is told apart from other failures, which the store shows as
/// stale while rows remain.
pub(crate) fn convert(
    connection: &str,
    resource: &str,
    events: Vec<WatchEvent>,
) -> Vec<ResourceEvent> {
    let mut converted = Vec::with_capacity(events.len() + 1);
    for event in events {
        match event {
            WatchEvent::Reset { columns, rows } => {
                converted.push(ResourceEvent::Reset {
                    columns: self::columns(&columns),
                    rows: rows
                        .iter()
                        .filter_map(|table_row| row(connection, resource, table_row))
                        .collect(),
                });
                converted.push(ResourceEvent::Read(ReadState::Loaded));
            }
            WatchEvent::Upsert(table_row) => {
                if let Some(row) = row(connection, resource, &table_row) {
                    converted.push(ResourceEvent::Upsert(row));
                }
            }
            WatchEvent::Delete(table_row) => {
                if let Some(row) = row(connection, resource, &table_row) {
                    converted.push(ResourceEvent::Delete(row.identity));
                }
            }
            WatchEvent::Failed { failure, .. } => {
                let state = if failure.kind == FailureKind::Forbidden {
                    ReadState::Refused(failure.message)
                } else {
                    ReadState::Failed(failure.to_string())
                };
                converted.push(ResourceEvent::Read(state));
            }
        }
    }
    converted
}

#[cfg(test)]
mod tests {
    use super::*;
    use freshkube_core::resources::{Failure, Table};

    fn table() -> Table {
        serde_json::from_str(
            r#"{"columnDefinitions":[
                  {"name":"Name","type":"string","priority":0},
                  {"name":"Desired","type":"integer","priority":0},
                  {"name":"Age","type":"string","priority":0},
                  {"name":"Selector","type":"string","priority":1}],
                "rows":[
                  {"cells":["web",3,"5d",null],"object":{"metadata":{"name":"web","namespace":"shop","uid":"u1",
                    "resourceVersion":"7","creationTimestamp":"2026-09-30T10:00:00Z",
                    "deletionTimestamp":"2026-10-01T00:00:00Z","labels":{"app":"web"}}}},
                  {"cells":["node-1"],"object":{"metadata":{"name":"node-1","uid":"u2"}}},
                  {"cells":["orphan"]}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn columns_keep_kind_and_wide_priority() {
        let columns = columns(&table().column_definitions);
        assert_eq!(columns[0].kind, ColumnKind::Text);
        assert_eq!(columns[1].kind, ColumnKind::Number);
        assert_eq!(columns[2].kind, ColumnKind::Age);
        assert!(!columns[2].wide);
        assert!(columns[3].wide);
    }

    #[test]
    fn rows_carry_identity_cells_creation_and_termination() {
        let table = table();
        let first = row("ctx", "deployments.apps", &table.rows[0]).unwrap();
        assert_eq!(first.identity.address(), "shop/web");
        assert_eq!(first.identity.resource, "deployments.apps");
        assert_eq!(first.cells, ["web", "3", "5d", ""]);
        assert_eq!(first.created, parse_timestamp("2026-09-30T10:00:00Z"));
        assert!(first.terminating);
        // Cluster-scoped rows have no namespace.
        let node = row("ctx", "nodes", &table.rows[1]).unwrap();
        assert_eq!(node.identity.namespace, "");
        assert_eq!(node.identity.address(), "node-1");
        assert!(!node.terminating);
        // A row without object metadata has no identity and is skipped.
        assert!(row("ctx", "nodes", &table.rows[2]).is_none());
    }

    #[test]
    fn timestamps_parse_as_utc_seconds() {
        assert_eq!(parse_timestamp("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_timestamp("2000-03-01T00:00:01Z"), Some(951_868_801));
        assert_eq!(
            parse_timestamp("2026-09-30T10:00:00.123Z"),
            Some(1_790_762_400)
        );
        assert_eq!(parse_timestamp("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_timestamp("not a time"), None);
    }

    #[test]
    fn batches_convert_resets_changes_and_refusals() {
        let table = table();
        let events = convert(
            "ctx",
            "deployments.apps",
            vec![
                WatchEvent::Reset {
                    columns: table.column_definitions.clone(),
                    rows: table.rows.clone(),
                },
                WatchEvent::Upsert(table.rows[2].clone()),
                WatchEvent::Delete(table.rows[0].clone()),
                WatchEvent::Failed {
                    failure: Failure::new(FailureKind::Forbidden, "no"),
                    retrying: false,
                },
                WatchEvent::Failed {
                    failure: Failure::new(FailureKind::Unreachable, "down"),
                    retrying: true,
                },
            ],
        );
        assert!(matches!(&events[0], ResourceEvent::Reset { rows, .. } if rows.len() == 2));
        assert!(matches!(events[1], ResourceEvent::Read(ReadState::Loaded)));
        // The orphan upsert has no identity and is dropped.
        assert!(matches!(&events[2], ResourceEvent::Delete(identity) if identity.name == "web"));
        assert!(
            matches!(&events[3], ResourceEvent::Read(ReadState::Refused(reason)) if reason == "no")
        );
        assert!(
            matches!(&events[4], ResourceEvent::Read(ReadState::Failed(reason)) if reason == "Unreachable · down")
        );
        assert_eq!(events.len(), 5);
    }
}
