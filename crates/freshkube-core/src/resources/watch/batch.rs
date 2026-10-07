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
