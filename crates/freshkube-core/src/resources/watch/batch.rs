//! One watch's stream items mapped to events, and the window that coalesces
//! them into batches. Neither reads a clock: the caller passes `now`.

use std::time::Duration;

use kube::core::WatchEvent as KubeWatchEvent;
use tokio::time::Instant;

use super::{WatchEnd, WatchEvent};
use crate::resources::failure::Failure;
use crate::resources::table::{Table, TableRow};

/// Maps one item from a watch stream, pushing its changes onto `out` in
/// order and moving `resource_version` past every row it carries and to every
/// bookmark. Returns how the watch ends, if this item ends it.
pub(super) fn map_event(
    item: Result<KubeWatchEvent<Table>, kube::Error>,
    resource_version: &mut String,
    out: &mut Vec<WatchEvent>,
) -> Option<WatchEnd> {
    match item {
        Ok(event) => match event {
            KubeWatchEvent::Added(table) | KubeWatchEvent::Modified(table) => {
                for row in table.rows {
                    track_version(resource_version, &row);
                    out.push(WatchEvent::Upsert(row));
                }
                None
            }
            KubeWatchEvent::Deleted(table) => {
                for row in table.rows {
                    track_version(resource_version, &row);
                    out.push(WatchEvent::Delete(row));
                }
                None
            }
            KubeWatchEvent::Bookmark(bookmark) => {
                *resource_version = bookmark.metadata.resource_version;
                None
            }
            KubeWatchEvent::Error(status) if status.code == 410 => Some(WatchEnd::Relist),
            KubeWatchEvent::Error(status) => Some(WatchEnd::Failed(Failure::from_kube(
                kube::Error::Api(status),
            ))),
        },
        // A 410 can also arrive as the response itself rather than an event.
        Err(kube::Error::Api(status)) if status.code == 410 => Some(WatchEnd::Relist),
        Err(error) => Some(WatchEnd::Failed(Failure::from_kube(error))),
    }
}

fn track_version(resource_version: &mut String, row: &TableRow) {
    if let Some(metadata) = row.metadata()
        && !metadata.resource_version.is_empty()
    {
        resource_version.clone_from(&metadata.resource_version);
    }
}

/// Collects events until a window that opens with the first pending event
/// elapses, the watch ends or `max` events are pending, then hands them over
/// as one batch.
pub(super) struct Batcher {
    pending: Vec<WatchEvent>,
    deadline: Option<Instant>,
    window: Duration,
    max: usize,
}

impl Batcher {
    pub(super) fn new(window: Duration, max: usize) -> Self {
        Self {
            pending: Vec::new(),
            deadline: None,
            window,
            max,
        }
    }

    /// When the open window elapses, if events are pending.
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Where newly arrived events go; `flush` decides when they leave.
    pub(super) fn pending_mut(&mut self) -> &mut Vec<WatchEvent> {
        &mut self.pending
    }

    /// Called after each stream item, and when the window elapses, at `now`.
    /// Returns the batch to deliver, if one is due; `ending` delivers whatever
    /// is pending. Otherwise it opens the window for newly pending events.
    pub(super) fn flush(&mut self, now: Instant, ending: bool) -> Option<Vec<WatchEvent>> {
        let window_elapsed = self.deadline.is_some_and(|at| now >= at);
        if !self.pending.is_empty() && (ending || window_elapsed || self.pending.len() >= self.max)
        {
            self.deadline = None;
            return Some(std::mem::take(&mut self.pending));
        } else if !self.pending.is_empty() && self.deadline.is_none() {
            self.deadline = Some(now + self.window);
        } else if self.pending.is_empty() {
            self.deadline = None;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::FailureKind;
    use serde_json::{Value, json};

    const WINDOW: Duration = Duration::from_millis(16);

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn row(name: &str, version: &str) -> Value {
        json!({"cells": [name], "object": {"kind": "PartialObjectMetadata",
            "metadata": {"name": name, "namespace": "default", "resourceVersion": version}}})
    }

    fn table_event(kind: &str, rows: &[(&str, &str)]) -> KubeWatchEvent<Table> {
        let rows: Vec<Value> = rows
            .iter()
            .map(|(name, version)| row(name, version))
            .collect();
        serde_json::from_value(json!({"type": kind, "object": {"kind": "Table",
            "apiVersion": "meta.k8s.io/v1", "metadata": {}, "columnDefinitions": null,
            "rows": rows}}))
        .unwrap()
    }

    fn status(code: u16) -> Value {
        json!({"kind": "Status", "apiVersion": "v1", "metadata": {}, "status": "Failure",
            "message": "for test", "reason": "ForTest", "code": code})
    }

    fn error_event(code: u16) -> KubeWatchEvent<Table> {
        serde_json::from_value(json!({"type": "ERROR", "object": status(code)})).unwrap()
    }

    fn error_response(code: u16) -> kube::Error {
        kube::Error::Api(serde_json::from_value(status(code)).unwrap())
    }

    fn upsert(name: &str) -> WatchEvent {
        WatchEvent::Upsert(serde_json::from_value(row(name, "1")).unwrap())
    }

    fn summary(events: &[WatchEvent]) -> Vec<String> {
        events
            .iter()
            .map(|event| match event {
                WatchEvent::Upsert(row) => format!("upsert {}", row.metadata().unwrap().name),
                WatchEvent::Delete(row) => format!("delete {}", row.metadata().unwrap().name),
                WatchEvent::Reset { .. } => "reset".into(),
                WatchEvent::Failed { .. } => "failed".into(),
            })
            .collect()
    }

    /// The changes one item maps to; it must not end the watch.
    fn events(
        item: Result<KubeWatchEvent<Table>, kube::Error>,
        version: &mut String,
    ) -> Vec<String> {
        let mut out = Vec::new();
        assert!(
            map_event(item, version, &mut out).is_none(),
            "the watch ended"
        );
        summary(&out)
    }

    /// How one item ends the watch; it must add no changes.
    fn end(item: Result<KubeWatchEvent<Table>, kube::Error>, version: &mut String) -> WatchEnd {
        let mut out = Vec::new();
        let end = map_event(item, version, &mut out).expect("the watch goes on");
        assert!(out.is_empty());
        end
    }

    #[test]
    fn rows_become_events_and_move_the_version() {
        let mut version = "10".to_string();
        let added = events(Ok(table_event("ADDED", &[("a", "11")])), &mut version);
        assert_eq!(added, ["upsert a"]);
        assert_eq!(version, "11");
        let modified = events(
            Ok(table_event("MODIFIED", &[("a", "12"), ("b", "13")])),
            &mut version,
        );
        assert_eq!(modified, ["upsert a", "upsert b"]);
        assert_eq!(version, "13");
        let deleted = events(Ok(table_event("DELETED", &[("b", "14")])), &mut version);
        assert_eq!(deleted, ["delete b"]);
        assert_eq!(version, "14");
    }

    #[test]
    fn a_row_without_a_version_keeps_the_last_one() {
        let mut version = "10".to_string();
        let added = events(Ok(table_event("ADDED", &[("a", "")])), &mut version);
        assert_eq!(added, ["upsert a"]);
        assert_eq!(version, "10");
    }

    #[test]
    fn a_bookmark_moves_the_version_alone() {
        let mut version = "10".to_string();
        let bookmark = serde_json::from_value(json!({"type": "BOOKMARK", "object": {
            "kind": "Table", "apiVersion": "meta.k8s.io/v1",
            "metadata": {"resourceVersion": "40"}}}))
        .unwrap();
        assert!(events(Ok(bookmark), &mut version).is_empty());
        assert_eq!(version, "40");
    }

    #[test]
    fn gone_relists_as_an_event_or_a_response() {
        let mut version = "10".to_string();
        assert!(matches!(
            end(Ok(error_event(410)), &mut version),
            WatchEnd::Relist
        ));
        assert!(matches!(
            end(Err(error_response(410)), &mut version),
            WatchEnd::Relist
        ));
        assert_eq!(version, "10", "a relist takes the list's version");
    }

    #[test]
    fn other_errors_fail_the_watch() {
        let mut version = "10".to_string();
        assert!(matches!(
            end(Ok(error_event(403)), &mut version),
            WatchEnd::Failed(failure) if failure.kind == FailureKind::Forbidden
        ));
        assert!(matches!(
            end(Err(error_response(500)), &mut version),
            WatchEnd::Failed(failure) if failure.kind == FailureKind::Other
        ));
    }

    #[test]
    fn a_burst_inside_one_window_publishes_once() {
        let start = Instant::now();
        let mut batcher = Batcher::new(WINDOW, 512);
        for (at, name) in [(0, "a"), (5, "b"), (15, "c")] {
            batcher.pending_mut().extend(vec![upsert(name)]);
            assert!(batcher.flush(start + ms(at), false).is_none());
            assert_eq!(batcher.deadline(), Some(start + WINDOW));
        }
        let batch = batcher.flush(start + WINDOW, false).unwrap();
        assert_eq!(summary(&batch), ["upsert a", "upsert b", "upsert c"]);
        assert_eq!(batcher.deadline(), None);
        assert!(batcher.flush(start + ms(40), false).is_none());
    }

    #[test]
    fn an_idle_gap_publishes_one_window_after_the_event() {
        let start = Instant::now();
        let mut batcher = Batcher::new(WINDOW, 512);
        batcher.pending_mut().extend(vec![upsert("a")]);
        assert!(batcher.flush(start, false).is_none());
        assert_eq!(batcher.deadline(), Some(start + WINDOW));
        // Nothing more arrives: the window elapsing delivers the event.
        let batch = batcher.flush(start + WINDOW, false).unwrap();
        assert_eq!(summary(&batch), ["upsert a"]);
        // After a quiet second, the next event opens a window of its own.
        let later = start + ms(1000);
        batcher.pending_mut().extend(vec![upsert("b")]);
        assert!(batcher.flush(later, false).is_none());
        assert_eq!(batcher.deadline(), Some(later + WINDOW));
        let batch = batcher.flush(later + WINDOW, false).unwrap();
        assert_eq!(summary(&batch), ["upsert b"]);
    }

    #[test]
    fn nothing_pending_opens_no_window() {
        let start = Instant::now();
        let mut batcher = Batcher::new(WINDOW, 512);
        batcher.pending_mut().extend(Vec::new());
        assert!(batcher.flush(start, false).is_none());
        assert!(
            batcher.flush(start, true).is_none(),
            "an empty batch is never sent"
        );
        assert_eq!(batcher.deadline(), None);
    }

    #[test]
    fn a_full_batch_publishes_at_once() {
        let start = Instant::now();
        let mut batcher = Batcher::new(WINDOW, 3);
        batcher.pending_mut().extend(vec![upsert("a"), upsert("b")]);
        assert!(batcher.flush(start, false).is_none());
        batcher.pending_mut().extend(vec![upsert("c")]);
        let batch = batcher.flush(start + ms(1), false).unwrap();
        assert_eq!(summary(&batch), ["upsert a", "upsert b", "upsert c"]);
        assert_eq!(batcher.deadline(), None);
    }

    #[test]
    fn an_ending_watch_delivers_what_is_pending_and_keeps_nothing() {
        let start = Instant::now();
        let mut version = "10".to_string();
        let mut batcher = Batcher::new(WINDOW, 512);
        let added = Ok(table_event("ADDED", &[("a", "11")]));
        assert!(map_event(added, &mut version, batcher.pending_mut()).is_none());
        assert!(batcher.flush(start, false).is_none());
        // A 410 inside the window: the pending change goes out before the
        // relist, whose complete list then replaces it in a batch of its own.
        let gone = map_event(Ok(error_event(410)), &mut version, batcher.pending_mut());
        assert!(matches!(gone, Some(WatchEnd::Relist)));
        let batch = batcher.flush(start + ms(2), true).unwrap();
        assert_eq!(summary(&batch), ["upsert a"]);
        assert_eq!(batcher.deadline(), None);
        assert!(batcher.flush(start + ms(40), true).is_none());
    }
}
