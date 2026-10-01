use std::cell::{Ref, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[cfg(test)]
use freshkube_core::constants::MAX_LOG_ENTRIES;
use freshkube_core::{
    logs::{LogEntry, LogEvent, MultiServiceLogs, ServiceId},
    types::LogLevel,
};

pub(super) const MAX_SELECTED_LINES: usize = 200;
const MAX_COPY_BYTES: usize = 1024 * 1024;
const MAX_LINE_BYTES: usize = 64 * 1024;
const MAX_RETAINED_BYTES: usize = 8 * 1024 * 1024;

/// How `visible` changed since the panel last measured its rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VisibleDelta {
    /// Anything may have changed.
    Rebuilt,
    /// Rows left the front and new ones joined the end; the rest are as they were.
    Extended { dropped: usize, added: usize },
}

/// Matching row IDs for one query against one revision of the visible rows.
struct MatchCache {
    revision: u64,
    query: String,
    ids: Vec<u64>,
}

fn level_slot(level: &LogLevel) -> usize {
    match level {
        LogLevel::Error => 0,
        LogLevel::Warning => 1,
        LogLevel::Info => 2,
        LogLevel::Debug => 3,
        LogLevel::Unknown => 4,
    }
}

/// Line identities are the core's arrival sequence numbers, which are unique
/// and mirror its timestamp/sequence ordering. They are deliberately not text
/// hashes: identical lines remain independently selectable.
pub(super) struct LogReview {
    pub(super) logs: MultiServiceLogs,
    pub(super) next_id: u64,
    pub(super) visible: Vec<usize>,
    /// Retained lines per service and level, kept in step with every append
    /// and eviction so rendering never rescans the buffer.
    counts: BTreeMap<ServiceId, [usize; 5]>,
    delta: VisibleDelta,
    matches: RefCell<Option<MatchCache>>,
    pub(super) selected: BTreeSet<u64>,
    cursor: Option<u64>,
    pub(super) selection_anchor: Option<u64>,
    pub(super) query: String,
    pub(super) current_match: Option<u64>,
    pub(super) revision: u64,
    pub(super) omitted: usize,
    pub(super) evicted: usize,
    pub(super) selection_limited: bool,
}

impl LogReview {
    pub(super) fn new(address: &str) -> Self {
        Self {
            logs: MultiServiceLogs::new(address),
            next_id: 0,
            visible: Vec::new(),
            counts: BTreeMap::new(),
            delta: VisibleDelta::Rebuilt,
            matches: RefCell::new(None),
            selected: BTreeSet::new(),
            cursor: None,
            selection_anchor: None,
            query: String::new(),
            current_match: None,
            revision: 0,
            omitted: 0,
            evicted: 0,
            selection_limited: false,
        }
    }

    pub(super) fn append(&mut self, events: impl IntoIterator<Item = LogEvent>) {
        let mut accepted = Vec::new();
        for event in events {
            if event.line.len() > MAX_LINE_BYTES {
                self.omitted += 1;
                continue;
            }
            if event.line.trim().is_empty() {
                continue;
            }
            accepted.push(event);
        }
        // Core parses each line once, merges the batch into timestamp order
        // and applies both retention limits, reporting what it did.
        let outcome = self.logs.append_bounded(accepted, MAX_RETAINED_BYTES);
        self.next_id += outcome.added.len() as u64;
        for (service, level) in &outcome.added {
            self.counts.entry(service.clone()).or_default()[level_slot(level)] += 1;
        }
        for entry in &outcome.evicted {
            if let Some(counts) = self.counts.get_mut(&entry.service) {
                let slot = &mut counts[level_slot(&entry.level)];
                *slot = slot.saturating_sub(1);
            }
        }
        let evicted = outcome.evicted.len();
        self.evicted += evicted;
        if evicted > 0 {
            let gone: HashSet<u64> = outcome.evicted.iter().map(LogEntry::sequence).collect();
            self.selected.retain(|id| !gone.contains(id));
            self.cursor = self.cursor.filter(|id| !gone.contains(id));
            self.selection_anchor = self.selection_anchor.filter(|id| !gone.contains(id));
            self.current_match = self.current_match.filter(|id| !gone.contains(id));
        }
        if outcome.extended_tail() {
            // Entries only left the front and joined the end: shift the
            // visible indexes instead of re-filtering the whole buffer.
            let dropped = self.visible.partition_point(|&ix| ix < evicted);
            self.visible.drain(..dropped);
            for ix in &mut self.visible {
                *ix -= evicted;
            }
            let buffer = self.logs.buffer();
            let before = self.visible.len();
            let first_new = outcome.previous_len.max(evicted);
            for ix in first_new..outcome.previous_len + outcome.added.len() {
                if buffer.accepts(&buffer.entries()[ix - evicted]) {
                    self.visible.push(ix - evicted);
                }
            }
            let added = self.visible.len() - before;
            self.delta = match self.delta {
                VisibleDelta::Extended {
                    dropped: earlier_dropped,
                    added: earlier_added,
                } => VisibleDelta::Extended {
                    dropped: earlier_dropped + dropped,
                    added: earlier_added + added,
                },
                VisibleDelta::Rebuilt => VisibleDelta::Rebuilt,
            };
            self.revision += 1;
        } else {
            self.rebuild_visible();
        }
    }

    fn rebuild_visible(&mut self) {
        // Search navigates the filtered view, rather than hiding nonmatches.
        // Core owns the authoritative service/severity filtering and retention.
        self.visible = self.logs.buffer().visible_indices();
        self.delta = VisibleDelta::Rebuilt;
        self.revision += 1;
    }

    /// What `measure_rows` should apply to its row sizes, resetting the record.
    pub(super) fn take_delta(&mut self) -> VisibleDelta {
        std::mem::replace(
            &mut self.delta,
            VisibleDelta::Extended {
                dropped: 0,
                added: 0,
            },
        )
    }

    /// Retained lines of a service, whatever the filters hide.
    pub(super) fn service_count(&self, service: &ServiceId) -> usize {
        self.counts
            .get(service)
            .map_or(0, |counts| counts.iter().sum())
    }

    /// Retained lines per level among the given services.
    pub(super) fn level_counts<'a>(
        &self,
        showing: impl IntoIterator<Item = &'a ServiceId>,
    ) -> [usize; 5] {
        let mut totals = [0; 5];
        for service in showing {
            if let Some(counts) = self.counts.get(service) {
                for (total, count) in totals.iter_mut().zip(counts) {
                    *total += count;
                }
            }
        }
        totals
    }

    /// IDs of the visible rows matching the query, in row order. Computed once
    /// per revision and query rather than once per caller per frame.
    pub(super) fn matched_ids(&self) -> Ref<'_, Vec<u64>> {
        {
            let mut cache = self.matches.borrow_mut();
            if cache
                .as_ref()
                .is_none_or(|cache| cache.revision != self.revision || cache.query != self.query)
            {
                let lowercase = self.query.to_lowercase();
                let ids = if self.query.is_empty() {
                    Vec::new()
                } else {
                    (0..self.visible.len())
                        .filter(|&ix| self.entry(ix).matches_lowercase_query(&lowercase))
                        .map(|ix| self.id(ix))
                        .collect()
                };
                *cache = Some(MatchCache {
                    revision: self.revision,
                    query: self.query.clone(),
                    ids,
                });
            }
        }
        Ref::map(self.matches.borrow(), |cache| {
            &cache.as_ref().expect("filled above").ids
        })
    }

    pub(super) fn entry(&self, row_ix: usize) -> &LogEntry {
        &self.logs.buffer().entries()[self.visible[row_ix]]
    }

    pub(super) fn id(&self, row_ix: usize) -> u64 {
        self.logs.buffer().entries()[self.visible[row_ix]].sequence()
    }

    pub(super) fn row_for_id(&self, id: u64) -> Option<usize> {
        let entries = self.logs.buffer().entries();
        self.visible
            .iter()
            .position(|&ix| entries[ix].sequence() == id)
    }

    pub(super) fn set_service_filter(&mut self, services: BTreeSet<ServiceId>) {
        let mut filters = self.logs.buffer().filters().clone();
        filters.services = Some(services);
        self.logs.buffer_mut().set_filters(filters);
        self.rebuild_visible();
    }

    pub(super) fn set_level(&mut self, level: &LogLevel, active: bool) {
        let mut filters = self.logs.buffer().filters().clone();
        filters.levels.set(level, active);
        self.logs.buffer_mut().set_filters(filters);
        self.rebuild_visible();
    }

    pub(super) fn search(&mut self, forward: bool) -> Option<u64> {
        if self.query.is_empty() {
            self.current_match = None;
            return None;
        }
        let matches = self.matched_ids().clone();
        if matches.is_empty() {
            self.current_match = None;
            return None;
        }
        let position = self
            .current_match
            .and_then(|id| matches.iter().position(|&v| v == id));
        let ix = match position {
            Some(ix) if forward => (ix + 1) % matches.len(),
            Some(0) => matches.len() - 1,
            Some(ix) => ix - 1,
            None if forward => 0,
            None => matches.len() - 1,
        };
        self.current_match = Some(matches[ix]);
        self.cursor = self.current_match;
        self.current_match
    }

    pub(super) fn match_count(&self) -> usize {
        self.matched_ids().len()
    }

    pub(super) fn select(&mut self, row_ix: usize, extend: bool, toggle: bool) {
        if row_ix >= self.visible.len() {
            return;
        }
        let id = self.id(row_ix);
        self.selection_limited = false;
        if extend {
            let start = self
                .selection_anchor
                .and_then(|id| self.row_for_id(id))
                .unwrap_or(row_ix);
            self.selected.clear();
            for ix in start.min(row_ix)..=start.max(row_ix) {
                if self.selected.len() == MAX_SELECTED_LINES {
                    self.selection_limited = true;
                    break;
                }
                self.selected.insert(self.id(ix));
            }
        } else {
            if !toggle {
                self.selected.clear();
            }
            if toggle && self.selected.remove(&id) {
                // Toggle removed the line.
            } else if self.selected.len() < MAX_SELECTED_LINES {
                self.selected.insert(id);
            } else {
                self.selection_limited = true;
            }
            self.selection_anchor = Some(id);
        }
        self.cursor = Some(id);
    }

    pub(super) fn move_selection(&mut self, delta: isize, extend: bool) -> Option<u64> {
        if self.visible.is_empty() {
            return None;
        }
        let current = self.cursor.and_then(|id| self.row_for_id(id));
        let row_ix = if delta == isize::MIN {
            0
        } else if delta == isize::MAX {
            self.visible.len() - 1
        } else {
            current.map_or_else(
                || if delta < 0 { self.visible.len() - 1 } else { 0 },
                |ix| ix.saturating_add_signed(delta).min(self.visible.len() - 1),
            )
        };
        self.select(row_ix, extend, false);
        self.cursor
    }

    /// Copy only selected complete, retained, currently visible original lines.
    /// Never silently truncate a line or turn Copy into a whole-buffer export.
    pub(super) fn copy_text(&self) -> Result<String, &'static str> {
        let mut output = String::new();
        for row_ix in 0..self.visible.len() {
            if !self.selected.contains(&self.id(row_ix)) {
                continue;
            }
            let line = self.entry(row_ix).selectable_text();
            if output.len() + line.len() + usize::from(!output.is_empty()) > MAX_COPY_BYTES {
                return Err("Selection exceeds 1 MiB; select fewer complete lines");
            }
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(line);
        }
        if output.is_empty() {
            Err("Select visible lines to copy")
        } else {
            Ok(output)
        }
    }
}

#[cfg(test)]
mod model_tests {
    use super::*;

    #[test]
    fn duplicate_lines_keep_identity_after_timestamp_reordering() {
        let mut review = LogReview::new("node");
        review.append([
            LogEvent::new("kubelet", "2026-09-30T10:00:02Z info repeated"),
            LogEvent::new("kubelet", "2026-09-30T10:00:02Z info repeated"),
        ]);
        review.select(1, false, false);
        let selected = review.id(1);
        review.append([LogEvent::new("apid", "2026-09-30T10:00:01Z error earlier")]);
        assert_eq!(review.row_for_id(selected), Some(2));
        assert_eq!(review.selected, BTreeSet::from([selected]));
        assert_eq!(
            review.copy_text().unwrap(),
            "2026-09-30T10:00:02Z info repeated"
        );
    }

    #[test]
    fn search_navigates_filtered_lines_without_filtering_nonmatches() {
        let mut review = LogReview::new("node");
        review.append([
            LogEvent::new("apid", "info first"),
            LogEvent::new("kubelet", "error UTF-8 東京"),
            LogEvent::new("apid", "error last"),
        ]);
        review.query = "error".into();
        assert_eq!(review.visible.len(), 3);
        assert_eq!(review.search(true), Some(1));
        assert_eq!(review.search(true), Some(2));
        assert_eq!(review.search(true), Some(1));
        assert_eq!(review.search(false), Some(2));
        review.set_service_filter(BTreeSet::from([ServiceId::from("kubelet")]));
        assert_eq!(review.match_count(), 1);
        review.set_level(&LogLevel::Error, false);
        assert_eq!(review.match_count(), 0);
    }

    #[test]
    fn retention_and_selection_are_bounded_and_copy_is_visible_complete_scope() {
        let mut review = LogReview::new("node");
        review.append(
            (0..MAX_LOG_ENTRIES).map(|ix| LogEvent::new("apid", format!("info line {ix}"))),
        );
        review.select(0, false, false);
        let old = review.id(0);
        review.select(250, true, false);
        assert_eq!(review.selected.len(), MAX_SELECTED_LINES);
        assert!(review.selection_limited);
        review.append([LogEvent::new("apid", "info new")]);
        assert_eq!(review.logs.buffer().entries().len(), MAX_LOG_ENTRIES);
        assert_eq!(review.row_for_id(old), None);
        assert!(!review.selected.contains(&old));
        review.set_service_filter(BTreeSet::new());
        assert!(review.copy_text().is_err());
    }

    #[test]
    fn copy_size_limit_rejects_instead_of_truncating_and_large_lines_are_not_retained() {
        let mut review = LogReview::new("node");
        review.append((0..20).map(|_| LogEvent::new("apid", "x".repeat(MAX_LINE_BYTES))));
        review.select(0, false, false);
        review.select(19, true, false);
        assert!(review.copy_text().is_err());
        review.append([LogEvent::new("apid", "x".repeat(MAX_LINE_BYTES + 1))]);
        assert_eq!(review.omitted, 1);
        assert_eq!(review.visible.len(), 20);
        review.query = "x".into();
        assert_eq!(review.match_count(), 20);
        assert_eq!(review.move_selection(1, false), Some(19));
    }

    #[test]
    fn byte_budget_evicts_matching_identity_prefix_without_losing_survivor_selection() {
        let mut review = LogReview::new("node");
        review.append((0..128).map(|_| LogEvent::new("apid", "x".repeat(MAX_LINE_BYTES))));
        review.select(127, false, false);
        let survivor = review.id(127);
        review.append((0..72).map(|_| LogEvent::new("kubelet", "y".repeat(MAX_LINE_BYTES))));
        assert_eq!(review.logs.buffer().entries().len(), 128);
        assert_eq!(review.evicted, 72);
        assert_eq!(review.row_for_id(0), None);
        assert_eq!(review.row_for_id(survivor), Some(55));
        assert_eq!(review.selected, BTreeSet::from([survivor]));
        assert_eq!(review.copy_text().unwrap().len(), MAX_LINE_BYTES);
        let retained_bytes: usize = review
            .logs
            .buffer()
            .entries()
            .iter()
            .map(|entry| entry.raw.len())
            .sum();
        assert_eq!(retained_bytes, MAX_RETAINED_BYTES);
    }

    #[test]
    fn home_and_end_choose_explicit_endpoints_without_an_existing_cursor() {
        let mut review = LogReview::new("node");
        review.append([
            LogEvent::new("apid", "first"),
            LogEvent::new("apid", "middle"),
            LogEvent::new("apid", "last"),
        ]);
        assert_eq!(review.move_selection(isize::MIN, false), Some(0));
        review.cursor = None;
        assert_eq!(review.move_selection(isize::MAX, false), Some(2));
        assert_eq!(review.move_selection(isize::MIN, false), Some(0));
        assert_eq!(review.move_selection(isize::MAX, false), Some(2));
    }
}
