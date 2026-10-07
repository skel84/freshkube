use std::cell::{Ref, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ops::Range;

use gpui_kit::SharedString;

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

/// What a row's message column shows: its message, or the whole line when
/// the message is blank.
pub(super) fn shown_message(entry: &LogEntry) -> &str {
    if entry.message.trim().is_empty() {
        entry.selectable_text()
    } else {
        &entry.message
    }
}

/// One match of the search: a line of a row's message, so a stack trace
/// that names the query on three lines holds three. A row whose match lies
/// outside its message's lines, as in its timestamp, holds one, with no
/// line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Hit {
    pub(super) id: u64,
    /// The matching line's index among the lines of the row's
    /// [`shown_message`], split after each newline.
    pub(super) line: Option<usize>,
}

/// The lines of `message`, split after each newline, that hold
/// `lowercase`: each line's index and its text's byte range, without its
/// newline.
fn matching_lines(message: &str, lowercase: &str) -> Vec<(usize, Range<usize>)> {
    if !message.contains('\n') {
        return if message.to_lowercase().contains(lowercase) {
            vec![(0, 0..message.len())]
        } else {
            Vec::new()
        };
    }
    let mut start = 0;
    let mut lines = Vec::new();
    for (ix, line) in message.split_inclusive('\n').enumerate() {
        if line.to_lowercase().contains(lowercase) {
            lines.push((ix, start..start + line.trim_end_matches('\n').len()));
        }
        start += line.len();
    }
    lines
}

/// How a search marks a row or one line of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mark {
    /// It matches.
    Match,
    /// It is the current match.
    Current,
}

/// How a search marks one row: as a whole, or, for a message of several
/// lines that names the search on some of them, line by line, so stepping
/// within the row moves the current line's mark.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct RowMarks {
    pub(super) row: Option<Mark>,
    /// Byte ranges of the row's [`shown_message`] and their marks.
    pub(super) lines: Vec<(Range<usize>, Mark)>,
}

/// The toolbar's search count for one current match, and its tooltip.
#[derive(Clone)]
pub(super) struct SearchCount {
    current: Option<Hit>,
    pub(super) position: Option<usize>,
    pub(super) text: SharedString,
    pub(super) tip: SharedString,
}

/// The matches for one query against one revision of the visible rows.
struct MatchCache {
    revision: u64,
    query: String,
    /// Every matching line, in row order and, within a row, line order.
    hits: Vec<Hit>,
    /// Whether each visible row matches, by row index, so drawing a row
    /// doesn't search its text again.
    rows: Vec<bool>,
    /// The matching lines of each matching row whose message has several
    /// lines, by row index: each line's index and byte range.
    spans: BTreeMap<usize, Vec<(usize, Range<usize>)>>,
    /// The count for the current match, derived when the match or these
    /// hits change.
    count: Option<SearchCount>,
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
    pub(super) current_match: Option<Hit>,
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

    /// Appends a batch and returns the identities of the lines retention
    /// evicted to make room.
    pub(super) fn append(&mut self, events: impl IntoIterator<Item = LogEvent>) -> Vec<u64> {
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
        // Markers are notes, not lines, so no level counts them.
        for (service, level) in &outcome.added {
            let Some(level) = level else { continue };
            self.counts.entry(service.clone()).or_default()[level_slot(level)] += 1;
        }
        for entry in outcome.evicted.iter().filter(|entry| !entry.is_marker()) {
            if let Some(counts) = self.counts.get_mut(&entry.service) {
                let slot = &mut counts[level_slot(&entry.level)];
                *slot = slot.saturating_sub(1);
            }
        }
        let evicted = outcome.evicted.len();
        self.evicted += evicted;
        let gone: Vec<u64> = outcome.evicted.iter().map(LogEntry::sequence).collect();
        if evicted > 0 {
            let gone: HashSet<u64> = gone.iter().copied().collect();
            self.selected.retain(|id| !gone.contains(id));
            self.cursor = self.cursor.filter(|id| !gone.contains(id));
            self.selection_anchor = self.selection_anchor.filter(|id| !gone.contains(id));
            self.current_match = self.current_match.filter(|hit| !gone.contains(&hit.id));
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
        gone
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

    /// Retained lines per level among the services the filter shows: all of
    /// them when there is no service filter, as with a pod's one stream.
    pub(super) fn level_counts(&self) -> [usize; 5] {
        let shown = self.logs.buffer().filters().services.as_ref();
        let mut totals = [0; 5];
        for (service, counts) in &self.counts {
            if shown.is_none_or(|shown| shown.contains(service)) {
                for (total, count) in totals.iter_mut().zip(counts) {
                    *total += count;
                }
            }
        }
        totals
    }

    /// Every matching line of the visible rows, in order. Computed once per
    /// revision and query rather than once per caller per frame.
    pub(super) fn hits(&self) -> Ref<'_, Vec<Hit>> {
        Ref::map(self.match_cache(), |cache| &cache.hits)
    }

    /// Where the current match stands among [`Self::hits`].
    pub(super) fn current_position(&self) -> Option<usize> {
        self.search_count().position
    }

    /// The search count and its tooltip, derived again only when the
    /// current match or the hits have changed, not every frame.
    pub(super) fn search_count(&self) -> SearchCount {
        drop(self.match_cache());
        let mut cache = self.matches.borrow_mut();
        let cache = cache.as_mut().expect("filled by match_cache");
        if let Some(count) = &cache.count
            && count.current == self.current_match
        {
            return count.clone();
        }
        let total = cache.hits.len();
        let position = (self.current_match)
            .and_then(|current| cache.hits.iter().position(|&hit| hit == current));
        let text = match (total, position) {
            (0, _) => "No matches".to_owned(),
            (total, Some(ix)) => format!("{} of {total}", ix + 1),
            (total, None) => format!("– of {total}"),
        };
        // The count is of lines: a stack trace that names the search on
        // three lines counts three.
        let tip = match total {
            0 => "No matching lines".to_owned(),
            1 => "1 matching line".to_owned(),
            total => format!(
                "{total} matching lines. Each matching line of a multi-line message \
                 counts, and Next visits each."
            ),
        };
        let count = SearchCount {
            current: self.current_match,
            position,
            text: text.into(),
            tip: tip.into(),
        };
        cache.count = Some(count.clone());
        count
    }

    /// How the search marks the visible row at `row_ix`: line by line when
    /// its message has several lines and some of them match, otherwise as
    /// a whole.
    pub(super) fn marks(&self, row_ix: usize) -> RowMarks {
        if !self.is_match(row_ix) {
            return RowMarks::default();
        }
        let id = self.id(row_ix);
        let current = self.current_match.filter(|hit| hit.id == id);
        let mark = |current: bool| if current { Mark::Current } else { Mark::Match };
        let cache = self.match_cache();
        match cache.spans.get(&row_ix) {
            Some(spans) => RowMarks {
                row: None,
                lines: (spans.iter())
                    .map(|(line, range)| {
                        let here = current.is_some_and(|hit| hit.line == Some(*line));
                        (range.clone(), mark(here))
                    })
                    .collect(),
            },
            None => RowMarks {
                row: Some(mark(current.is_some())),
                lines: Vec::new(),
            },
        }
    }

    /// Whether the visible row at `row_ix` matches the query, from the same
    /// cache as [`Self::hits`].
    pub(super) fn is_match(&self, row_ix: usize) -> bool {
        !self.query.is_empty()
            && self
                .match_cache()
                .rows
                .get(row_ix)
                .copied()
                .unwrap_or(false)
    }

    /// The matches for the current revision and query, searched again only
    /// when either has changed.
    fn match_cache(&self) -> Ref<'_, MatchCache> {
        {
            let mut cache = self.matches.borrow_mut();
            if cache
                .as_ref()
                .is_none_or(|cache| cache.revision != self.revision || cache.query != self.query)
            {
                let lowercase = self.query.to_lowercase();
                let rows: Vec<bool> = if self.query.is_empty() {
                    Vec::new()
                } else {
                    (0..self.visible.len())
                        .map(|ix| self.entry(ix).matches_lowercase_query(&lowercase))
                        .collect()
                };
                let mut hits = Vec::new();
                let mut spans = BTreeMap::new();
                for (ix, _) in rows.iter().enumerate().filter(|(_, matched)| **matched) {
                    let id = self.id(ix);
                    let message = shown_message(self.entry(ix));
                    let lines = matching_lines(message, &lowercase);
                    if lines.is_empty() {
                        hits.push(Hit { id, line: None });
                    }
                    hits.extend(lines.iter().map(|(line, _)| Hit {
                        id,
                        line: Some(*line),
                    }));
                    if message.contains('\n') && !lines.is_empty() {
                        spans.insert(ix, lines);
                    }
                }
                *cache = Some(MatchCache {
                    revision: self.revision,
                    query: self.query.clone(),
                    hits,
                    rows,
                    spans,
                    count: None,
                });
            }
        }
        Ref::map(self.matches.borrow(), |cache| {
            cache.as_ref().expect("filled above")
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

    /// Steps to the next or previous matching line: through a row's
    /// matching lines first, then on to the next row's.
    pub(super) fn search(&mut self, forward: bool) -> Option<Hit> {
        if self.query.is_empty() {
            self.current_match = None;
            return None;
        }
        let position = self.current_position();
        let hit = {
            let hits = self.hits();
            let count = hits.len();
            let ix = match position {
                _ if count == 0 => None,
                Some(ix) if forward => Some((ix + 1) % count),
                Some(0) => Some(count - 1),
                Some(ix) => Some(ix - 1),
                None if forward => Some(0),
                None => Some(count - 1),
            };
            ix.map(|ix| hits[ix])
        };
        self.current_match = hit;
        if let Some(hit) = hit {
            self.cursor = Some(hit.id);
        }
        hit
    }

    /// Matching lines, as the search steps through them.
    pub(super) fn match_count(&self) -> usize {
        self.hits().len()
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

    /// Selects every visible line, or the newest `MAX_SELECTED_LINES` of
    /// them. Returns how many lines are visible.
    pub(super) fn select_all(&mut self) -> usize {
        let count = self.visible.len();
        if count == 0 {
            return 0;
        }
        let start = count.saturating_sub(MAX_SELECTED_LINES);
        self.selected = (start..count).map(|ix| self.id(ix)).collect();
        self.selection_limited = start > 0;
        self.selection_anchor = Some(self.id(start));
        self.cursor = Some(self.id(count - 1));
        count
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

    /// Copy only selected complete, retained, currently visible original lines,
    /// as `copied` writes them. Markers are notes, not lines, so they are never
    /// copied. Never silently truncate a line or turn Copy into a whole-buffer
    /// export.
    pub(super) fn copy_text(&self, as_: CopyAs) -> Result<String, &'static str> {
        let mut output = String::new();
        let mut line = String::new();
        for row_ix in 0..self.visible.len() {
            let entry = self.entry(row_ix);
            if entry.is_marker() || !self.selected.contains(&self.id(row_ix)) {
                continue;
            }
            line.clear();
            copied(entry, as_, &mut line);
            if output.len() + line.len() + usize::from(!output.is_empty()) > MAX_COPY_BYTES {
                return Err("Selection exceeds 1 MiB; select fewer complete lines");
            }
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&line);
        }
        if output.is_empty() {
            Err("Select visible lines to copy")
        } else {
            Ok(output)
        }
    }
}

/// How Copy and Download write a line.
#[derive(Clone, Copy, Default)]
pub(super) struct CopyAs {
    /// With the line's leading timestamp.
    pub(super) time: bool,
    /// Led by its source's full name, as where several sources interleave.
    pub(super) tagged: bool,
}

/// One original line as Copy and Download write it: its source's full name
/// first when `tagged`, then the line, without its leading timestamp unless
/// `time`.
pub(super) fn copied(entry: &LogEntry, as_: CopyAs, out: &mut String) {
    if as_.tagged {
        out.push_str(entry.service.as_str());
        out.push(' ');
    }
    out.push_str(if as_.time {
        entry.selectable_text()
    } else {
        entry.text_without_timestamp()
    });
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
            review
                .copy_text(CopyAs {
                    time: true,
                    tagged: false
                })
                .unwrap(),
            "2026-09-30T10:00:02Z info repeated"
        );
    }

    #[test]
    fn copy_skips_markers_and_can_leave_out_timestamps() {
        let mut review = LogReview::new("pod");
        let restarted = "2026-10-01T12:00:01Z".parse().unwrap();
        review.append([
            LogEvent::new("web", "2026-10-01T12:00:00Z GET / 200"),
            LogEvent::marker("web", restarted, "web restarted"),
            LogEvent::new("web", "2026-10-01T12:00:02Z GET /health 200"),
        ]);
        review.select(0, false, false);
        review.select(2, true, false);
        assert_eq!(review.selected.len(), 3);
        assert_eq!(
            review
                .copy_text(CopyAs {
                    time: true,
                    tagged: false
                })
                .unwrap(),
            "2026-10-01T12:00:00Z GET / 200\n2026-10-01T12:00:02Z GET /health 200"
        );
        assert_eq!(
            review.copy_text(CopyAs::default()).unwrap(),
            "GET / 200\nGET /health 200"
        );
        assert_eq!(review.level_counts().iter().sum::<usize>(), 2);
    }

    /// Without a service filter every service counts, as a pod's one stream
    /// needs; with one, only the services it shows.
    #[test]
    fn level_counts_follow_the_service_filter() {
        let mut review = LogReview::new("node");
        review.append([
            LogEvent::new("apid", "error first"),
            LogEvent::new("apid", "info second"),
            LogEvent::new("kubelet", "warning third"),
        ]);
        let [error, warning, info, ..] = review.level_counts();
        assert_eq!((error, warning, info), (1, 1, 1));
        review.set_service_filter(BTreeSet::from([ServiceId::from("kubelet")]));
        assert_eq!(review.level_counts(), [0, 1, 0, 0, 0]);
        review.set_service_filter(BTreeSet::new());
        assert_eq!(review.level_counts(), [0; 5]);
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
        let mut step = |forward| review.search(forward).map(|hit| hit.id);
        assert_eq!(step(true), Some(1));
        assert_eq!(step(true), Some(2));
        assert_eq!(step(true), Some(1));
        assert_eq!(step(false), Some(2));
        review.set_service_filter(BTreeSet::from([ServiceId::from("kubelet")]));
        assert_eq!(review.match_count(), 1);
        review.set_level(&LogLevel::Error, false);
        assert_eq!(review.match_count(), 0);
    }

    /// A row that names the query on several of its lines holds a match for
    /// each: Next visits every one before the next row, Previous mirrors it,
    /// and the count counts lines (#281).
    #[test]
    fn search_steps_through_every_matching_line_of_a_row() {
        let mut review = LogReview::new("pod");
        review.append([
            LogEvent::new("web", "2026-10-01T12:00:00Z info starting"),
            LogEvent::new(
                "web",
                "2026-10-01T12:00:01Z error panic: queue closed\n    at drain()\n    \
                 at flush(queue)\n    at main()\n    at queue_loop()",
            ),
            LogEvent::new("web", "2026-10-01T12:00:02Z warning queue slow"),
        ]);
        review.query = "QUEUE".into();
        let trace = review.id(1);
        let last = review.id(2);
        let hit = |id, line| Some(Hit { id, line });
        let lines = shown_message(review.entry(1)).split_inclusive('\n').count();
        assert_eq!(lines, 5);
        assert_eq!(review.match_count(), 4);
        assert_eq!(review.current_position(), None);
        assert_eq!(review.search(true), hit(trace, Some(0)));
        assert_eq!(review.search(true), hit(trace, Some(2)));
        assert_eq!(review.current_position(), Some(1));
        assert_eq!(review.search(true), hit(trace, Some(4)));
        assert_eq!(review.search(true), hit(last, Some(0)));
        assert_eq!(review.current_position(), Some(3));
        assert_eq!(review.search(true), hit(trace, Some(0)));
        assert_eq!(review.search(false), hit(last, Some(0)));
        assert_eq!(review.search(false), hit(trace, Some(4)));
        assert_eq!(review.search(false), hit(trace, Some(2)));
        assert_eq!(review.search_count().text, "2 of 4");
        assert!(review.search_count().tip.starts_with("4 matching lines"));
        // The trace marks its matching lines, the current one apart; the
        // one-line row is marked whole.
        let message = shown_message(review.entry(1)).to_owned();
        let marks = review.marks(1);
        assert_eq!(marks.row, None);
        let lines: Vec<&str> = message.lines().collect();
        let expected = [
            (0, "queue closed", Mark::Match),
            (2, "at flush(queue)", Mark::Current),
            (4, "at queue_loop()", Mark::Match),
        ];
        assert_eq!(marks.lines.len(), expected.len());
        for ((range, mark), (line, end, want)) in marks.lines.iter().zip(expected) {
            // Each range is its whole line, without the newline.
            assert_eq!(&message[range.clone()], lines[line]);
            assert!(lines[line].ends_with(end), "{:?}", lines[line]);
            assert_eq!(*mark, want, "line {line}");
        }
        assert_eq!(review.marks(2).row, Some(Mark::Match));
        assert_eq!(review.marks(0), RowMarks::default());
        // The selection's cursor follows the match's row.
        assert_eq!(review.move_selection(1, false), Some(last));
        // A match outside the message's lines, here in the timestamp, is one
        // match for its row, with no line of its own.
        review.query = "12:00:00".into();
        assert_eq!(review.match_count(), 1);
        assert_eq!(review.search(true), hit(review.id(0), None));
    }

    #[test]
    fn row_match_flags_follow_the_query_and_new_lines() {
        let mut review = LogReview::new("node");
        review.append([
            LogEvent::new("apid", "info first"),
            LogEvent::new("apid", "error second"),
        ]);
        assert!(!review.is_match(1), "no query matches nothing");
        review.query = "ERROR".into();
        let flags = |review: &LogReview| -> Vec<bool> {
            (0..review.visible.len())
                .map(|ix| review.is_match(ix))
                .collect()
        };
        assert_eq!(flags(&review), [false, true]);
        review.append([LogEvent::new("apid", "error third")]);
        assert_eq!(flags(&review), [false, true, true]);
        let ids: Vec<u64> = (0..3)
            .filter(|&ix| review.is_match(ix))
            .map(|ix| review.id(ix))
            .collect();
        let hit_ids: Vec<u64> = review.hits().iter().map(|hit| hit.id).collect();
        assert_eq!(hit_ids, ids);
        review.query = "first".into();
        assert_eq!(flags(&review), [true, false, false]);
        assert!(!review.is_match(7), "a row past the end");
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
        assert!(
            review
                .copy_text(CopyAs {
                    time: true,
                    tagged: false
                })
                .is_err()
        );
    }

    #[test]
    fn copy_size_limit_rejects_instead_of_truncating_and_large_lines_are_not_retained() {
        let mut review = LogReview::new("node");
        review.append((0..20).map(|_| LogEvent::new("apid", "x".repeat(MAX_LINE_BYTES))));
        review.select(0, false, false);
        review.select(19, true, false);
        assert!(
            review
                .copy_text(CopyAs {
                    time: true,
                    tagged: false
                })
                .is_err()
        );
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
        let gone =
            review.append((0..72).map(|_| LogEvent::new("kubelet", "y".repeat(MAX_LINE_BYTES))));
        assert_eq!(gone, (0..72).collect::<Vec<u64>>());
        assert_eq!(review.logs.buffer().entries().len(), 128);
        assert_eq!(review.evicted, 72);
        assert_eq!(review.row_for_id(0), None);
        assert_eq!(review.row_for_id(survivor), Some(55));
        assert_eq!(review.selected, BTreeSet::from([survivor]));
        assert_eq!(
            review
                .copy_text(CopyAs {
                    time: true,
                    tagged: false
                })
                .unwrap()
                .len(),
            MAX_LINE_BYTES
        );
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

    #[test]
    fn select_all_takes_every_visible_line_or_the_newest_up_to_the_limit() {
        let mut review = LogReview::new("node");
        assert_eq!(review.select_all(), 0);
        assert!(review.selected.is_empty());
        review.append((0..5).map(|ix| LogEvent::new("apid", format!("info line {ix}"))));
        assert_eq!(review.select_all(), 5);
        assert_eq!(review.selected.len(), 5);
        assert!(!review.selection_limited);

        review.append((5..250).map(|ix| LogEvent::new("apid", format!("info line {ix}"))));
        assert_eq!(review.select_all(), 250);
        assert_eq!(review.selected.len(), MAX_SELECTED_LINES);
        assert!(review.selection_limited);
        assert!(!review.selected.contains(&review.id(49)));
        assert!(review.selected.contains(&review.id(50)));
        assert!(review.selected.contains(&review.id(249)));
        // Shift and an arrow carry on from the newest line.
        assert_eq!(review.move_selection(-1, false), Some(review.id(248)));
    }
}
