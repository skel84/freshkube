//! One watch's stream items mapped to events, and the window that coalesces
//! them into batches. Neither reads a clock: the caller passes `now`.

use std::time::Duration;

use kube::core::WatchEvent as KubeWatchEvent;
use tokio::time::Instant;

use super::{WatchEnd, WatchEvent};
use crate::resources::failure::Failure;
use crate::resources::table::{Table, TableRow};

/// What one item from a watch stream asks of the watch.
pub(super) enum Mapped {
    /// Changes to deliver, in order; none for a bookmark.
    Events(Vec<WatchEvent>),
    /// The watch ends here.
    End(WatchEnd),
}

/// Maps one item from a watch stream, moving `resource_version` past every
/// row it carries and to every bookmark.
pub(super) fn map_event(
    item: Result<KubeWatchEvent<Table>, kube::Error>,
    resource_version: &mut String,
) -> Mapped {
    match item {
        Ok(event) => match event {
            KubeWatchEvent::Added(table) | KubeWatchEvent::Modified(table) => Mapped::Events(
                table
                    .rows
                    .into_iter()
                    .map(|row| {
                        track_version(resource_version, &row);
                        WatchEvent::Upsert(row)
                    })
                    .collect(),
            ),
            KubeWatchEvent::Deleted(table) => Mapped::Events(
                table
                    .rows
                    .into_iter()
                    .map(|row| {
                        track_version(resource_version, &row);
                        WatchEvent::Delete(row)
                    })
                    .collect(),
            ),
            KubeWatchEvent::Bookmark(bookmark) => {
                *resource_version = bookmark.metadata.resource_version;
                Mapped::Events(Vec::new())
            }
            KubeWatchEvent::Error(status) if status.code == 410 => Mapped::End(WatchEnd::Relist),
            KubeWatchEvent::Error(status) => Mapped::End(WatchEnd::Failed(Failure::from_kube(
                kube::Error::Api(status),
            ))),
        },
        // A 410 can also arrive as the response itself rather than an event.
        Err(kube::Error::Api(status)) if status.code == 410 => Mapped::End(WatchEnd::Relist),
        Err(error) => Mapped::End(WatchEnd::Failed(Failure::from_kube(error))),
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

    pub(super) fn extend(&mut self, events: Vec<WatchEvent>) {
        self.pending.extend(events);
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

    fn events(mapped: Mapped) -> Vec<String> {
        match mapped {
            Mapped::Events(events) => summary(&events),
            Mapped::End(_) => panic!("expected events, the watch ended"),
        }
    }

    #[test]
    fn rows_become_events_and_move_the_version() {
        let mut version = "10".to_string();
        let added = map_event(Ok(table_event("ADDED", &[("a", "11")])), &mut version);
        assert_eq!(events(added), ["upsert a"]);
        assert_eq!(version, "11");
        let modified = map_event(
            Ok(table_event("MODIFIED", &[("a", "12"), ("b", "13")])),
            &mut version,
        );
        assert_eq!(events(modified), ["upsert a", "upsert b"]);
        assert_eq!(version, "13");
        let deleted = map_event(Ok(table_event("DELETED", &[("b", "14")])), &mut version);
        assert_eq!(events(deleted), ["delete b"]);
        assert_eq!(version, "14");
    }

    #[test]
    fn a_row_without_a_version_keeps_the_last_one() {
        let mut version = "10".to_string();
        let added = map_event(Ok(table_event("ADDED", &[("a", "")])), &mut version);
        assert_eq!(events(added), ["upsert a"]);
        assert_eq!(version, "10");
    }

    #[test]
    fn a_bookmark_moves_the_version_alone() {
        let mut version = "10".to_string();
        let bookmark = serde_json::from_value(json!({"type": "BOOKMARK", "object": {
            "kind": "Table", "apiVersion": "meta.k8s.io/v1",
            "metadata": {"resourceVersion": "40"}}}))
        .unwrap();
        assert!(events(map_event(Ok(bookmark), &mut version)).is_empty());
        assert_eq!(version, "40");
    }

    #[test]
    fn gone_relists_as_an_event_or_a_response() {
        let mut version = "10".to_string();
        assert!(matches!(
            map_event(Ok(error_event(410)), &mut version),
            Mapped::End(WatchEnd::Relist)
        ));
        assert!(matches!(
            map_event(Err(error_response(410)), &mut version),
            Mapped::End(WatchEnd::Relist)
        ));
        assert_eq!(version, "10", "a relist takes the list's version");
    }

    #[test]
    fn other_errors_fail_the_watch() {
        let mut version = "10".to_string();
        assert!(matches!(
            map_event(Ok(error_event(403)), &mut version),
            Mapped::End(WatchEnd::Failed(failure)) if failure.kind == FailureKind::Forbidden
        ));
        assert!(matches!(
            map_event(Err(error_response(500)), &mut version),
            Mapped::End(WatchEnd::Failed(failure)) if failure.kind == FailureKind::Other
        ));
    }

    #[test]
    fn a_burst_inside_one_window_publishes_once() {
        let start = Instant::now();
        let mut batcher = Batcher::new(WINDOW, 512);
        for (at, name) in [(0, "a"), (5, "b"), (15, "c")] {
            batcher.extend(vec![upsert(name)]);
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
        batcher.extend(vec![upsert("a")]);
        assert!(batcher.flush(start, false).is_none());
        assert_eq!(batcher.deadline(), Some(start + WINDOW));
        // Nothing more arrives: the window elapsing delivers the event.
        let batch = batcher.flush(start + WINDOW, false).unwrap();
        assert_eq!(summary(&batch), ["upsert a"]);
        // After a quiet second, the next event opens a window of its own.
        let later = start + ms(1000);
        batcher.extend(vec![upsert("b")]);
        assert!(batcher.flush(later, false).is_none());
        assert_eq!(batcher.deadline(), Some(later + WINDOW));
        let batch = batcher.flush(later + WINDOW, false).unwrap();
        assert_eq!(summary(&batch), ["upsert b"]);
    }

    #[test]
    fn nothing_pending_opens_no_window() {
        let start = Instant::now();
        let mut batcher = Batcher::new(WINDOW, 512);
        batcher.extend(Vec::new());
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
        batcher.extend(vec![upsert("a"), upsert("b")]);
        assert!(batcher.flush(start, false).is_none());
        batcher.extend(vec![upsert("c")]);
        let batch = batcher.flush(start + ms(1), false).unwrap();
        assert_eq!(summary(&batch), ["upsert a", "upsert b", "upsert c"]);
        assert_eq!(batcher.deadline(), None);
    }

    #[test]
    fn an_ending_watch_delivers_what_is_pending_and_keeps_nothing() {
        let start = Instant::now();
        let mut version = "10".to_string();
        let mut batcher = Batcher::new(WINDOW, 512);
        let Mapped::Events(added) =
            map_event(Ok(table_event("ADDED", &[("a", "11")])), &mut version)
        else {
            panic!("expected events");
        };
        batcher.extend(added);
        assert!(batcher.flush(start, false).is_none());
        // A 410 inside the window: the pending change goes out before the
        // relist, whose complete list then replaces it in a batch of its own.
        let Mapped::End(WatchEnd::Relist) = map_event(Ok(error_event(410)), &mut version) else {
            panic!("expected a relist");
        };
        let batch = batcher.flush(start + ms(2), true).unwrap();
        assert_eq!(summary(&batch), ["upsert a"]);
        assert_eq!(batcher.deadline(), None);
        assert!(batcher.flush(start + ms(40), true).is_none());
    }
}
