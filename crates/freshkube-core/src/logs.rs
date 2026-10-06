//! Framework-independent log parsing, filtering, and retention models.
//!
//! The types in this module deliberately contain no transport or UI concerns. A
//! worker can turn received Talos log lines into [`LogEvent`] values, while a UI
//! owns one of the buffers and applies those immutable events on its own thread.

use chrono::{DateTime, FixedOffset, NaiveDateTime};
use std::collections::BTreeSet;

use crate::{constants::MAX_LOG_ENTRIES, types::LogLevel};

/// Stable Talos service identifier.
///
/// The identifier is retained exactly as supplied by Talos; in particular, it
/// is not lowercased or otherwise normalized, because it is also the request
/// identity used when fetching that service's logs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceId(String);

impl ServiceId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ServiceId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<String> for ServiceId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for ServiceId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

/// Explicit node and service identity for a single-service log request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LogTarget {
    pub node_address: String,
    pub service: ServiceId,
}

impl LogTarget {
    pub fn new(node_address: impl Into<String>, service: impl Into<ServiceId>) -> Self {
        Self {
            node_address: node_address.into(),
            service: service.into(),
        }
    }
}

/// Display and ordering information extracted from a log timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogTimestamp {
    /// The compact timestamp displayed by a log viewer (`HH:MM:SS` where available).
    pub display: String,
    /// A deterministic key used to interleave multi-service logs.
    pub sort_key: i64,
}

impl LogTimestamp {
    /// A time as a row shows it: its clock in the offset it was given in.
    pub fn at(time: &DateTime<FixedOffset>) -> Self {
        Self {
            display: time.format("%H:%M:%S").to_string(),
            sort_key: time.timestamp(),
        }
    }
}

/// An immutable line delivered by a log worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEvent {
    pub service: ServiceId,
    pub line: String,
    /// A note from the viewer, such as "the container restarted", rather
    /// than a line the source wrote. See [`LogEvent::marker`].
    pub marker: bool,
    /// The level the source states, such as a severity Coroot stored with
    /// the line. None guesses it from the line's words.
    pub level: Option<LogLevel>,
    /// The line comes from a Talos service, whose message leaves out the
    /// prefix such a service writes. See [`LogEvent::talos`].
    pub talos: bool,
    /// A marker's time. A line carries its own in its text.
    pub time: Option<DateTime<FixedOffset>>,
}

impl LogEvent {
    pub fn new(service: impl Into<ServiceId>, line: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            line: line.into(),
            marker: false,
            level: None,
            talos: false,
            time: None,
        }
    }

    /// A Talos service's line. Its message leaves out what the service writes
    /// before it: a Go caller, as apid and trustd log with `log.Lshortfile`
    /// (`main.go:94: `), or the service's own name and process id
    /// (`udevd[812]: `). Only at the start, after the timestamp.
    pub fn talos(mut self) -> Self {
        self.talos = true;
        self
    }

    /// The line at the level its source states, instead of one guessed from
    /// its words.
    pub fn with_level(mut self, level: LogLevel) -> Self {
        self.level = Some(level);
        self
    }

    /// A note placed among the lines at `time`, which keeps it in order with
    /// them. Its time shows in `time`'s offset, so a caller gives it in the
    /// offset its lines are written in. It has no level, never matches a
    /// search and is never copied.
    pub fn marker(
        service: impl Into<ServiceId>,
        time: DateTime<FixedOffset>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            service: service.into(),
            line: text.into(),
            marker: true,
            level: None,
            talos: false,
            time: Some(time),
        }
    }
}

/// Parsed, selectable log data.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub service: ServiceId,
    /// The complete original line, with surrounding whitespace removed as in the TUI.
    pub raw: String,
    pub timestamp: Option<LogTimestamp>,
    pub level: LogLevel,
    /// The display-oriented message with common log prefixes removed.
    pub message: String,
    search_text: String,
    sequence: u64,
    /// Where `raw` continues after a leading RFC 3339 timestamp, or 0.
    body: usize,
    marker: bool,
}

impl LogEntry {
    /// Returns the exact retained raw line for copy/select actions.
    pub fn selectable_text(&self) -> &str {
        &self.raw
    }

    /// The line without the RFC 3339 timestamp it starts with, if any, as
    /// Kubernetes writes when asked for timestamps.
    pub fn text_without_timestamp(&self) -> &str {
        &self.raw[self.body..]
    }

    /// Whether this is a viewer's note rather than a logged line.
    pub fn is_marker(&self) -> bool {
        self.marker
    }

    /// Checks a case-insensitive query without rebuilding the line text.
    /// Markers never match.
    pub fn matches_query(&self, query: &str) -> bool {
        self.matches_lowercase_query(&query.to_lowercase())
    }

    /// Like [`Self::matches_query`] for a query the caller already lowercased,
    /// so scanning many entries lowercases the query once.
    pub fn matches_lowercase_query(&self, lowercase_query: &str) -> bool {
        !self.marker && self.search_text.contains(lowercase_query)
    }

    /// Arrival number within the buffer that retained this entry. Unique per
    /// buffer, and the tie-break of the timestamp ordering.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
}

/// Parse one Talos, Kubernetes, klog, JSON, or containerd log line.
pub fn parse_log_line(service: impl Into<ServiceId>, line: impl AsRef<str>) -> LogEntry {
    parse_event(LogEvent::new(service, line.as_ref()), 0)
}

fn parse_line(event: &LogEvent, sequence: u64) -> LogEntry {
    let line = event.line.as_str();
    let raw = line.trim().to_owned();
    // A leading RFC 3339 timestamp, as Kubernetes and Talos write, takes
    // precedence over any time the rest of the line carries.
    let (timestamp, body) = match extract_rfc3339_prefix(&raw) {
        Some((timestamp, body)) => (Some(timestamp), body),
        None => (None, 0),
    };
    let (timestamp, remainder) = match timestamp {
        Some(timestamp) => (Some(timestamp), &raw[body..]),
        None => extract_timestamp(&raw),
    };

    let level = event
        .level
        .clone()
        .unwrap_or_else(|| classify_log_level(remainder));
    let mut message = remainder.trim();
    if event.talos {
        message = without_talos_prefix(message, event.service.as_str());
    }
    LogEntry {
        service: event.service.clone(),
        message: without_level(message, &level).to_owned(),
        level,
        search_text: raw.to_lowercase(),
        raw,
        timestamp,
        sequence,
        body,
        marker: false,
    }
}

fn parse_event(event: LogEvent, sequence: u64) -> LogEntry {
    if event.marker {
        let raw = event.line.trim().to_owned();
        return LogEntry {
            service: event.service,
            message: raw.clone(),
            raw,
            timestamp: event.time.as_ref().map(LogTimestamp::at),
            level: LogLevel::Unknown,
            search_text: String::new(),
            sequence,
            body: 0,
            marker: true,
        };
    }
    parse_line(&event, sequence)
}

/// Classify a line with the same precedence as the existing log viewers.
pub fn classify_log_level(text: &str) -> LogLevel {
    let lower = text.to_lowercase();
    if lower.contains("error") || lower.contains("err") {
        LogLevel::Error
    } else if lower.contains("warn") {
        LogLevel::Warning
    } else if lower.contains("info") {
        LogLevel::Info
    } else if lower.contains("debug") || lower.contains("trace") {
        LogLevel::Debug
    } else {
        LogLevel::Unknown
    }
}

/// Active level switches. All levels are enabled by default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelFilter {
    pub error: bool,
    pub warning: bool,
    pub info: bool,
    pub debug: bool,
    pub unknown: bool,
}

impl Default for LevelFilter {
    fn default() -> Self {
        Self {
            error: true,
            warning: true,
            info: true,
            debug: true,
            unknown: true,
        }
    }
}

impl LevelFilter {
    pub fn accepts(&self, level: &LogLevel) -> bool {
        match level {
            LogLevel::Error => self.error,
            LogLevel::Warning => self.warning,
            LogLevel::Info => self.info,
            LogLevel::Debug => self.debug,
            LogLevel::Unknown => self.unknown,
        }
    }

    pub fn set(&mut self, level: &LogLevel, active: bool) {
        match level {
            LogLevel::Error => self.error = active,
            LogLevel::Warning => self.warning = active,
            LogLevel::Info => self.info = active,
            LogLevel::Debug => self.debug = active,
            LogLevel::Unknown => self.unknown = active,
        }
    }
}

/// Non-visual filtering state for a log view.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LogFilters {
    /// `None` shows every service; `Some(empty)` intentionally shows none.
    pub services: Option<BTreeSet<ServiceId>>,
    pub levels: LevelFilter,
}

impl LogFilters {
    pub fn accepts(&self, entry: &LogEntry) -> bool {
        self.services
            .as_ref()
            .is_none_or(|services| services.contains(&entry.service))
            && (entry.marker || self.levels.accepts(&entry.level))
    }
}

/// Stream presentation state owned by the caller.
///
/// Pausing does not discard received events; it lets the caller stop advancing
/// its viewport while the bounded buffer continues retaining the newest data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogStreamState {
    pub paused: bool,
    pub following: bool,
}

impl Default for LogStreamState {
    fn default() -> Self {
        Self {
            paused: false,
            following: true,
        }
    }
}

/// A bounded log collection with deterministic filtering and match navigation.
#[derive(Debug, Clone)]
pub struct LogBuffer {
    entries: Vec<LogEntry>,
    filters: LogFilters,
    query: String,
    query_lower: String,
    stream: LogStreamState,
    current_match: Option<usize>,
    next_sequence: u64,
    /// Total length of every retained raw line, kept in step with `entries`.
    retained_bytes: usize,
    /// Whether `entries` is known to be in timestamp/sequence order, which is
    /// what lets [`MultiServiceLogs`] merge a batch instead of re-sorting.
    ordered: bool,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl LogBuffer {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            filters: LogFilters::default(),
            query: String::new(),
            stream: LogStreamState::default(),
            query_lower: String::new(),
            current_match: None,
            next_sequence: 0,
            retained_bytes: 0,
            ordered: true,
        }
    }

    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    /// Evicts the oldest prefix until complete raw lines fit the supplied byte
    /// budget. Returns the number of removed entries so UI identity/selection
    /// sidecars can apply the same retention operation without copying records.
    pub fn retain_newest_bytes(&mut self, max_bytes: usize) -> usize {
        let removed = self.evict_bytes(max_bytes);
        if !removed.is_empty() {
            self.reset_match();
        }
        removed.len()
    }

    fn evict_bytes(&mut self, max_bytes: usize) -> Vec<LogEntry> {
        let mut retained_bytes = self.retained_bytes;
        let mut removed = 0;
        while retained_bytes > max_bytes && removed < self.entries.len() {
            retained_bytes -= self.entries[removed].raw.len();
            removed += 1;
        }
        self.retained_bytes = retained_bytes;
        self.entries.drain(..removed).collect()
    }

    /// Total length of the retained raw lines.
    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    /// The arrival number the next accepted line receives.
    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    pub fn filters(&self) -> &LogFilters {
        &self.filters
    }

    pub fn set_filters(&mut self, filters: LogFilters) {
        self.filters = filters;
        self.reset_match();
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.query_lower = self.query.to_lowercase();
        self.reset_match();
    }

    pub fn clear_query(&mut self) {
        self.set_query(String::new());
    }

    pub fn stream_state(&self) -> LogStreamState {
        self.stream
    }

    pub fn set_stream_state(&mut self, stream: LogStreamState) {
        self.stream = stream;
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.stream.paused = paused;
    }

    pub fn set_following(&mut self, following: bool) {
        self.stream.following = following;
    }

    /// Appends an event and returns whether it contained a non-empty log line.
    pub fn append(&mut self, event: LogEvent) -> bool {
        let appended = self.append_unbounded(event);
        if appended {
            self.ordered = false;
            self.retain_newest();
            self.reset_match();
        }
        appended
    }

    /// Appends all non-empty immutable stream events in their supplied order.
    pub fn append_batch<I>(&mut self, events: I) -> usize
    where
        I: IntoIterator<Item = LogEvent>,
    {
        let appended = events
            .into_iter()
            .map(|event| usize::from(self.append_unbounded(event)))
            .sum();
        if appended > 0 {
            self.ordered = false;
            self.retain_newest();
            self.reset_match();
        }
        appended
    }

    /// Replaces the collection from an immutable snapshot.
    pub fn replace<I>(&mut self, events: I)
    where
        I: IntoIterator<Item = LogEvent>,
    {
        self.entries.clear();
        self.retained_bytes = 0;
        self.next_sequence = 0;
        self.current_match = None;
        self.append_batch(events);
    }

    /// Whether an entry passes service, level, and query filtering.
    pub fn accepts(&self, entry: &LogEntry) -> bool {
        self.filters.accepts(entry)
            && (self.query_lower.is_empty() || entry.matches_lowercase_query(&self.query_lower))
    }

    /// Entry indices that pass service, level, and query filtering in entry order.
    pub fn visible_indices(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.accepts(entry))
            .map(|(index, _)| index)
            .collect()
    }

    /// Ordered matching entry indices in the currently filtered view.
    pub fn match_indices(&self) -> Vec<usize> {
        if self.query.is_empty() {
            Vec::new()
        } else {
            self.visible_indices()
        }
    }

    pub fn current_match(&self) -> Option<usize> {
        self.current_match
    }

    /// Select the first matching entry, if any.
    pub fn select_first_match(&mut self) -> Option<usize> {
        let first = self.match_indices().into_iter().next();
        self.current_match = first;
        first
    }

    /// Select the next match, wrapping at the end and disabling follow mode.
    pub fn next_match(&mut self) -> Option<usize> {
        self.navigate_match(true)
    }

    /// Select the previous match, wrapping at the beginning and disabling follow mode.
    pub fn previous_match(&mut self) -> Option<usize> {
        self.navigate_match(false)
    }

    pub fn is_current_match(&self, index: usize) -> bool {
        self.current_match == Some(index)
    }

    fn navigate_match(&mut self, forward: bool) -> Option<usize> {
        let matches = self.match_indices();
        let selected = match matches.len() {
            0 => None,
            _ => Some(
                match self
                    .current_match
                    .and_then(|current| matches.iter().position(|&index| index == current))
                {
                    Some(position) if forward => matches[(position + 1) % matches.len()],
                    Some(0) => matches[matches.len() - 1],
                    Some(position) => matches[position - 1],
                    None => matches[0],
                },
            ),
        };
        self.current_match = selected;
        if selected.is_some() {
            self.stream.following = false;
        }
        selected
    }

    fn append_unbounded(&mut self, event: LogEvent) -> bool {
        if event.line.trim().is_empty() {
            return false;
        }
        let entry = parse_event(event, self.next_sequence);
        self.retained_bytes += entry.raw.len();
        self.entries.push(entry);
        self.next_sequence = self.next_sequence.wrapping_add(1);
        true
    }

    fn reset_match(&mut self) {
        self.current_match = self.match_indices().into_iter().next();
    }

    fn retain_newest(&mut self) {
        let excess = self.entries.len().saturating_sub(MAX_LOG_ENTRIES);
        if excess > 0 {
            self.retained_bytes -= self
                .entries
                .drain(..excess)
                .map(|entry| entry.raw.len())
                .sum::<usize>();
        }
    }
}

/// A single service's bounded log model.
#[derive(Debug, Clone)]
pub struct SingleServiceLogs {
    pub target: LogTarget,
    buffer: LogBuffer,
}

impl SingleServiceLogs {
    pub fn new(target: LogTarget) -> Self {
        Self {
            target,
            buffer: LogBuffer::new(),
        }
    }

    pub fn buffer(&self) -> &LogBuffer {
        &self.buffer
    }

    pub fn buffer_mut(&mut self) -> &mut LogBuffer {
        &mut self.buffer
    }

    pub fn append_line(&mut self, line: impl Into<String>) -> bool {
        self.buffer
            .append(LogEvent::new(self.target.service.clone(), line))
    }

    pub fn append_batch<I, S>(&mut self, lines: I) -> usize
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let service = self.target.service.clone();
        self.buffer.append_batch(
            lines
                .into_iter()
                .map(|line| LogEvent::new(service.clone(), line)),
        )
    }

    pub fn replace_content(&mut self, content: &str) {
        self.buffer.replace(
            content
                .lines()
                .map(|line| LogEvent::new(self.target.service.clone(), line)),
        );
    }
}

/// A timestamp-ordered, bounded multi-service log model.
#[derive(Debug, Clone)]
pub struct MultiServiceLogs {
    pub node_address: String,
    buffer: LogBuffer,
}

impl MultiServiceLogs {
    pub fn new(node_address: impl Into<String>) -> Self {
        Self {
            node_address: node_address.into(),
            buffer: LogBuffer::new(),
        }
    }

    pub fn buffer(&self) -> &LogBuffer {
        &self.buffer
    }

    pub fn buffer_mut(&mut self) -> &mut LogBuffer {
        &mut self.buffer
    }

    pub fn append(&mut self, event: LogEvent) -> bool {
        self.append_batch([event]) > 0
    }

    pub fn append_batch<I>(&mut self, events: I) -> usize
    where
        I: IntoIterator<Item = LogEvent>,
    {
        self.append_bounded(events, usize::MAX).added.len()
    }

    /// Appends a batch, keeps timestamp order by merging it into the sorted
    /// buffer, and applies the entry and byte retention limits. The outcome
    /// says what changed so a caller can keep its own indexes in step without
    /// rescanning the buffer.
    pub fn append_bounded<I>(&mut self, events: I, max_bytes: usize) -> AppendOutcome
    where
        I: IntoIterator<Item = LogEvent>,
    {
        let previous_len = self.buffer.entries.len();
        let mut added = Vec::new();
        for event in events {
            if self.buffer.append_unbounded(event) {
                let entry = &self.buffer.entries[self.buffer.entries.len() - 1];
                added.push((
                    entry.service.clone(),
                    (!entry.marker).then(|| entry.level.clone()),
                ));
            }
        }
        let mut outcome = AppendOutcome {
            previous_len,
            first_changed: previous_len,
            added,
            evicted: Vec::new(),
        };
        if !outcome.added.is_empty() {
            outcome.first_changed = self.merge_tail(previous_len);
            let excess = self.buffer.entries.len().saturating_sub(MAX_LOG_ENTRIES);
            if excess > 0 {
                outcome.evicted = self.buffer.entries.drain(..excess).collect();
                self.buffer.retained_bytes -= outcome
                    .evicted
                    .iter()
                    .map(|entry| entry.raw.len())
                    .sum::<usize>();
            }
        }
        outcome.evicted.extend(self.buffer.evict_bytes(max_bytes));
        if !outcome.added.is_empty() || !outcome.evicted.is_empty() {
            self.buffer.reset_match();
        }
        outcome
    }

    pub fn replace<I>(&mut self, events: I)
    where
        I: IntoIterator<Item = LogEvent>,
    {
        self.buffer.entries.clear();
        self.buffer.retained_bytes = 0;
        self.buffer.next_sequence = 0;
        self.buffer.current_match = None;
        self.append_batch(events);
    }

    /// Restores timestamp/sequence order after `entries[from..]` were pushed.
    /// Returns the first index whose entry changed: `from` when the batch
    /// belongs at the end, otherwise where the earliest late line lands.
    fn merge_tail(&mut self, from: usize) -> usize {
        let key = |entry: &LogEntry| {
            (
                entry
                    .timestamp
                    .as_ref()
                    .map_or(0, |timestamp| timestamp.sort_key),
                entry.sequence,
            )
        };
        let entries = &mut self.buffer.entries;
        if !self.buffer.ordered {
            entries.sort_by_key(key);
            self.buffer.ordered = true;
            return 0;
        }
        entries[from..].sort_by_key(key);
        let first = key(&entries[from]);
        let at = entries[..from].partition_point(|entry| key(entry) < first);
        if at < from {
            // Sorting runs, so this merges the two sorted halves.
            entries[at..].sort_by_key(key);
        }
        at
    }
}

/// What [`MultiServiceLogs::append_bounded`] changed.
#[derive(Debug, Clone)]
pub struct AppendOutcome {
    /// Entries held before the call.
    pub previous_len: usize,
    /// Lowest entry index (before evictions) whose entry is new or moved;
    /// equals `previous_len` when the batch only extended the end.
    pub first_changed: usize,
    /// Service and level of each accepted line; a marker has no level.
    pub added: Vec<(ServiceId, Option<LogLevel>)>,
    /// Entries dropped from the front by the entry and byte limits.
    pub evicted: Vec<LogEntry>,
}

impl AppendOutcome {
    /// Whether the batch only extended the end of the buffer.
    pub fn extended_tail(&self) -> bool {
        self.first_changed == self.previous_len
    }
}

fn extract_timestamp(line: &str) -> (Option<LogTimestamp>, &str) {
    if let Some((timestamp, rest)) = extract_klog_timestamp(line) {
        return (Some(timestamp), rest);
    }

    if line.starts_with('{')
        && let Some(timestamp) = extract_json_timestamp(line)
    {
        return (Some(timestamp), line);
    }

    if let Some(timestamp) = extract_quoted_timestamp(line, "time=\"") {
        return (Some(timestamp), line);
    }

    extract_leading_timestamp(line)
}

fn extract_klog_timestamp(line: &str) -> Option<(LogTimestamp, &str)> {
    let bytes = line.as_bytes();
    if bytes.len() < 15 || !matches!(bytes[0], b'I' | b'W' | b'E' | b'F') {
        return None;
    }
    if !bytes[1..5].iter().all(u8::is_ascii_digit) || bytes[5] != b' ' {
        return None;
    }

    let timestamp_end = line[6..]
        .find(|character: char| !character.is_ascii_digit() && character != ':' && character != '.')
        .map(|end| end + 6)?;
    let time = &line[6..timestamp_end];
    let display = extract_time_part(time)?;
    let month = parse_digits(&line[1..3])?;
    let day = parse_digits(&line[3..5])?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let sort_key = month * 2_700_000 + day * 86_400 + time_sort_key(&display);
    Some((
        LogTimestamp { display, sort_key },
        line[timestamp_end..].trim(),
    ))
}

fn extract_json_timestamp(line: &str) -> Option<LogTimestamp> {
    for key in ["ts", "time"] {
        if let Some(value) = json_field_value(line, key)
            && let Some(timestamp) = timestamp_from_text(value)
        {
            return Some(timestamp);
        }
    }
    None
}

fn json_field_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let marker = match key {
        "ts" => "\"ts\"",
        "time" => "\"time\"",
        _ => return None,
    };
    let start = line.find(marker)? + marker.len();
    let rest = line[start..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    if let Some(quoted) = rest.strip_prefix('"') {
        return quoted.find('"').map(|end| &quoted[..end]);
    }
    let end = rest.find([',', '}', ' ']).unwrap_or(rest.len());
    Some(&rest[..end])
}

fn extract_quoted_timestamp(line: &str, marker: &str) -> Option<LogTimestamp> {
    let start = line.find(marker)? + marker.len();
    let end = line[start..].find('"')? + start;
    timestamp_from_text(&line[start..end])
}

/// A line that starts with a complete RFC 3339 timestamp and a space, as
/// Kubernetes writes with `timestamps=true`: the timestamp, kept as written
/// for display and to the second for ordering, and where the rest begins.
fn extract_rfc3339_prefix(line: &str) -> Option<(LogTimestamp, usize)> {
    let token = line.split(' ').next()?;
    let time = DateTime::parse_from_rfc3339(token).ok()?;
    let body = (token.len() + 1).min(line.len());
    Some((LogTimestamp::at(&time), body))
}

fn extract_leading_timestamp(line: &str) -> (Option<LogTimestamp>, &str) {
    let mut end = 0;
    let mut has_colon = false;
    let bytes = line.as_bytes();

    while end < bytes.len() {
        let byte = bytes[end];
        if byte == b':' {
            has_colon = true;
        }
        if byte.is_ascii_digit()
            || matches!(byte, b'/' | b'-' | b':' | b'.' | b'T' | b'Z' | b'+' | b' ')
        {
            end += 1;
        } else {
            break;
        }
    }

    if end < 8 || !has_colon {
        return (None, line);
    }
    let candidate = line[..end].trim();
    let Some(timestamp) = timestamp_from_text(candidate) else {
        return (None, line);
    };
    (Some(timestamp), line[end..].trim())
}

fn timestamp_from_text(text: &str) -> Option<LogTimestamp> {
    if let Some(display) = extract_time_part(text) {
        let sort_key = absolute_sort_key(text).unwrap_or_else(|| time_sort_key(&display));
        return Some(LogTimestamp { display, sort_key });
    }

    let sort_key = absolute_sort_key(text)?;
    let seconds_in_day = sort_key.rem_euclid(86_400);
    Some(LogTimestamp {
        display: format!(
            "{:02}:{:02}:{:02}",
            seconds_in_day / 3600,
            (seconds_in_day % 3600) / 60,
            seconds_in_day % 60
        ),
        sort_key,
    })
}

fn extract_time_part(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let search_length = bytes.len().min(30);
    for index in 0..search_length.saturating_sub(7) {
        if valid_time_at(bytes, index, true) {
            let display = &text[index..index + 8];
            if display != "00:00:00" {
                return Some(display.to_owned());
            }
        }
    }
    for index in 0..bytes.len().min(20).saturating_sub(4) {
        if valid_time_at(bytes, index, false) {
            let display = &text[index..index + 5];
            if display != "00:00" {
                return Some(display.to_owned());
            }
        }
    }
    None
}

fn valid_time_at(bytes: &[u8], index: usize, seconds: bool) -> bool {
    let required = if seconds { 8 } else { 5 };
    if index + required > bytes.len()
        || !bytes[index].is_ascii_digit()
        || !bytes[index + 1].is_ascii_digit()
        || bytes[index + 2] != b':'
        || !bytes[index + 3].is_ascii_digit()
        || !bytes[index + 4].is_ascii_digit()
    {
        return false;
    }
    let hour = (bytes[index] - b'0') * 10 + bytes[index + 1] - b'0';
    let minute = (bytes[index + 3] - b'0') * 10 + bytes[index + 4] - b'0';
    if hour >= 24 || minute >= 60 {
        return false;
    }
    !seconds
        || (bytes[index + 5] == b':'
            && bytes[index + 6].is_ascii_digit()
            && bytes[index + 7].is_ascii_digit()
            && (bytes[index + 6] - b'0') * 10 + bytes[index + 7] - b'0' < 60)
}

fn absolute_sort_key(text: &str) -> Option<i64> {
    if let Ok(timestamp) = DateTime::parse_from_rfc3339(text) {
        return Some(timestamp.timestamp());
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y/%m/%d %H:%M:%S%.f",
    ] {
        if let Ok(timestamp) = NaiveDateTime::parse_from_str(text, format) {
            return Some(timestamp.and_utc().timestamp());
        }
    }
    let value = text.parse::<f64>().ok()?;
    if !value.is_finite() {
        return None;
    }
    Some(if value.abs() >= 1_000_000_000_000.0 {
        (value / 1000.0).trunc() as i64
    } else {
        value.trunc() as i64
    })
}

fn parse_digits(value: &str) -> Option<i64> {
    value.parse().ok()
}

fn time_sort_key(display: &str) -> i64 {
    let bytes = display.as_bytes();
    let hour = parse_digits(&display[0..2]).unwrap_or(0);
    let minute = parse_digits(&display[3..5]).unwrap_or(0);
    let seconds = if bytes.len() >= 8 {
        parse_digits(&display[6..8]).unwrap_or(0)
    } else {
        0
    };
    hour * 3600 + minute * 60 + seconds
}

/// `text` without a Talos service's prefix: a Go caller (`main.go:94: `), or
/// the service's own name and process id (`udevd[812]: `).
fn without_talos_prefix<'a>(text: &'a str, service: &str) -> &'a str {
    let Some((head, rest)) = text.split_once(": ") else {
        return text;
    };
    let caller = head.split_once(".go:").is_some_and(|(file, line)| {
        !file.is_empty()
            && file
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'/'))
            && !line.is_empty()
            && line.bytes().all(|b| b.is_ascii_digit())
    });
    let own_name = head
        .strip_prefix(service)
        .and_then(|pid| pid.strip_prefix('['))
        .and_then(|pid| pid.strip_suffix(']'))
        .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()));
    if caller || own_name {
        rest.trim_start()
    } else {
        text
    }
}

/// `text` without a leading word that names `level`, which the row's level
/// column already shows: `INFO` or `[INFO]`, as a whole word.
fn without_level<'a>(text: &'a str, level: &LogLevel) -> &'a str {
    let (word, bracketed) = match level {
        LogLevel::Error => ("ERROR", "[ERROR]"),
        LogLevel::Warning => ("WARN", "[WARN]"),
        LogLevel::Info => ("INFO", "[INFO]"),
        LogLevel::Debug => ("DEBUG", "[DEBUG]"),
        LogLevel::Unknown => return text,
    };
    for token in [bracketed, word] {
        if let Some(rest) = text.strip_prefix(token)
            && (rest.is_empty() || rest.starts_with(char::is_whitespace))
        {
            return rest.trim_start();
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn parses_talos_klog_json_and_containerd_timestamps() {
        let talos = parse_log_line("kubelet", "2026-01-09T16:40:59.776940Z INFO started");
        assert_eq!(talos.timestamp.unwrap().display, "16:40:59");
        assert_eq!(talos.level, LogLevel::Info);

        let klog = parse_log_line("kubelet", "I0109 16:42:01.123456 1 server.go:94] ready");
        assert_eq!(klog.timestamp.unwrap().display, "16:42:01");

        let json = parse_log_line("etcd", r#"{"ts":1767972610784.5803,"msg":"ready"}"#);
        assert_eq!(json.timestamp.unwrap().sort_key, 1_767_972_610);

        let containerd = parse_log_line(
            "containerd",
            r#"time="2026-01-09T16:42:01.123456789Z" level=warning msg="slow""#,
        );
        assert_eq!(containerd.timestamp.unwrap().display, "16:42:01");
        assert_eq!(containerd.level, LogLevel::Warning);
    }

    fn message(event: LogEvent) -> String {
        parse_event(event, 0).message
    }

    #[test]
    fn a_talos_message_leaves_out_its_service_s_own_prefix() {
        // apid and trustd log with Go's `log.Lshortfile`.
        let apid = LogEvent::new(
            "apid",
            "2026/01/09 16:40:59.123456 main.go:94: cart-db: connection refused",
        );
        assert_eq!(message(apid.talos()), "cart-db: connection refused");
        let udevd = LogEvent::new("udevd", "udevd[812]: rules file changed, reloading");
        assert_eq!(message(udevd.talos()), "rules file changed, reloading");
        let zap = LogEvent::new("machined", "2026-01-09T16:40:59.776940Z INFO started");
        assert_eq!(message(zap.talos()), "started");
    }

    #[test]
    fn a_name_before_a_colon_stays_in_the_message() {
        for line in [
            "cart-db: connection refused",
            "kubelet: node not ready",
            "Error: ENOENT: no such file",
            "panic: runtime error: index out of range",
            "[talos] task setupLogger: done",
            // Another service's name, or a caller anywhere but the start.
            "udevd[812]: rules file changed",
            "retrying main.go:94: cart-db",
        ] {
            let stamped = format!("2026-01-09T16:40:59.776940Z {line}");
            for event in [
                LogEvent::new("kubelet", stamped.clone()).talos(),
                LogEvent::new("app", stamped.clone()),
                LogEvent::new("app", stamped.clone()).with_level(LogLevel::Error),
            ] {
                assert_eq!(message(event), line);
            }
        }
        // A pod's or Coroot's caller is the app's, and it stays.
        let pod = LogEvent::new("app", "2026-01-09T16:40:59Z main.go:94: cart-db: down");
        assert_eq!(message(pod), "main.go:94: cart-db: down");
    }

    #[test]
    fn only_a_whole_word_naming_the_row_s_level_is_left_out() {
        let at =
            |line: &str, level: LogLevel| message(LogEvent::new("app", line).with_level(level));
        assert_eq!(
            at("[WARN] disk 91% full", LogLevel::Warning),
            "disk 91% full"
        );
        assert_eq!(
            at("ERROR\tcart-db unreachable", LogLevel::Error),
            "cart-db unreachable"
        );
        for (line, level) in [
            ("OKTA login refused", LogLevel::Error),
            ("OK", LogLevel::Unknown),
            ("INFORMATION_SCHEMA ready", LogLevel::Info),
            ("DEBUGGING on", LogLevel::Debug),
            ("ERROR: disk full", LogLevel::Error),
            // Coroot's severity says otherwise, so the word is news.
            ("INFO retrying", LogLevel::Error),
        ] {
            assert_eq!(at(line, level), line);
        }
    }

    #[test]
    fn kubernetes_lines_take_their_rfc3339_prefix() {
        // The text after the timestamp starts with digits, dots and spaces,
        // which a looser scan would take for more of the time.
        let access = parse_log_line(
            "nginx",
            r#"2026-10-01T12:04:51.902118344Z 10.0.4.17 - - "GET /healthz HTTP/1.1" 200"#,
        );
        let time = access.timestamp.clone().unwrap();
        assert_eq!(time.display, "12:04:51");
        assert_eq!(time.sort_key, 1_790_856_291);
        assert_eq!(
            access.text_without_timestamp(),
            r#"10.0.4.17 - - "GET /healthz HTTP/1.1" 200"#
        );
        assert!(access.message.starts_with("10.0.4.17 - -"));
        assert_eq!(access.selectable_text(), access.raw);

        // The prefix wins over a time the application wrote itself, and its
        // level comes from the rest of the line.
        let own_time = parse_log_line(
            "app",
            "2026-10-01T12:00:00Z 2026/10/01 11:59:58 [error] upstream timed out",
        );
        assert_eq!(own_time.timestamp.as_ref().unwrap().display, "12:00:00");
        assert_eq!(own_time.level, LogLevel::Error);
        assert_eq!(
            own_time.text_without_timestamp(),
            "2026/10/01 11:59:58 [error] upstream timed out"
        );

        // An offset is shown as written and ordered as the instant it is.
        let offset = parse_log_line("app", "2026-10-01T14:00:00.5+02:00 WARN slow");
        let time = offset.timestamp.unwrap();
        assert_eq!(time.display, "14:00:00");
        assert_eq!(time.sort_key, 1_790_856_000);
        assert_eq!(offset.level, LogLevel::Warning);

        // An empty line keeps its timestamp; a line without one is unchanged.
        let empty = parse_log_line("app", "2026-10-01T12:00:00.000000001Z");
        assert_eq!(empty.text_without_timestamp(), "");
        assert!(empty.timestamp.is_some());
        let plain = parse_log_line("app", "listening on :8080");
        assert!(plain.timestamp.is_none());
        assert_eq!(plain.text_without_timestamp(), "listening on :8080");
        // Not a complete RFC 3339 time: the older scan still reads it.
        let spaced = parse_log_line("app", "2026-10-01 12:00:00 INFO up");
        assert_eq!(spaced.timestamp.as_ref().unwrap().display, "12:00:00");
        assert_eq!(spaced.text_without_timestamp(), spaced.raw);
    }

    /// A marker reads in the offset it's given, as the lines around it do,
    /// and keeps its place by the instant.
    #[test]
    fn a_marker_reads_in_the_offset_of_the_lines_around_it() {
        let east = FixedOffset::east_opt(2 * 3600).unwrap();
        let gap = DateTime::parse_from_rfc3339("2026-10-01T05:55:30Z")
            .unwrap()
            .with_timezone(&east);
        let mut logs = MultiServiceLogs::new("app");
        logs.append_batch([
            LogEvent::new("app", "2026-10-01T07:56:00+02:00 after"),
            LogEvent::marker("app", gap, "Some messages may be missing"),
            LogEvent::new("app", "2026-10-01T07:55:00+02:00 before"),
        ]);
        let rows: Vec<_> = logs
            .buffer()
            .entries()
            .iter()
            .map(|entry| {
                (
                    entry.timestamp.as_ref().unwrap().display.as_str(),
                    entry.message.as_str(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("07:55:00", "before"),
                ("07:55:30", "Some messages may be missing"),
                ("07:56:00", "after"),
            ]
        );
    }

    #[test]
    fn markers_keep_their_place_but_never_match_count_or_filter() {
        let time = DateTime::parse_from_rfc3339("2026-10-01T12:00:01Z").unwrap();
        let mut logs = MultiServiceLogs::new("pod");
        let outcome = logs.append_bounded(
            [
                LogEvent::new("app", "2026-10-01T12:00:02Z ERROR after"),
                LogEvent::marker("app", time, "app restarted (exit 1 Error)"),
                LogEvent::new("app", "2026-10-01T12:00:00Z INFO before"),
            ],
            usize::MAX,
        );
        let marker_levels: Vec<_> = outcome
            .added
            .iter()
            .map(|(_, level)| level.clone())
            .collect();
        assert_eq!(
            marker_levels,
            [Some(LogLevel::Error), None, Some(LogLevel::Info)]
        );
        let entries = logs.buffer().entries();
        let order: Vec<_> = entries
            .iter()
            .map(LogEntry::text_without_timestamp)
            .collect();
        assert_eq!(
            order,
            ["INFO before", "app restarted (exit 1 Error)", "ERROR after"]
        );
        let marker = &entries[1];
        assert!(marker.is_marker());
        assert_eq!(marker.message, "app restarted (exit 1 Error)");
        assert!(!marker.matches_query("restarted"));
        let filters = LogFilters {
            levels: LevelFilter {
                unknown: false,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(filters.accepts(marker));
    }

    #[test]
    fn filters_queries_and_navigates_in_entry_order() {
        let mut logs = MultiServiceLogs::new("10.0.0.1");
        logs.append_batch([
            LogEvent::new("kubelet", "2026-01-09T16:40:01Z INFO alpha"),
            LogEvent::new("etcd", "2026-01-09T16:40:02Z ERROR beta alpha"),
            LogEvent::new("kubelet", "2026-01-09T16:40:03Z WARN gamma"),
        ]);
        let buffer = logs.buffer_mut();
        buffer.set_query("ALPHA");
        assert_eq!(buffer.match_indices(), vec![0, 1]);
        assert_eq!(buffer.current_match(), Some(0));
        assert_eq!(buffer.next_match(), Some(1));
        assert_eq!(buffer.next_match(), Some(0));
        assert!(!buffer.stream_state().following);

        let filters = LogFilters {
            services: Some(BTreeSet::from([ServiceId::from("etcd")])),
            ..Default::default()
        };
        buffer.set_filters(filters);
        assert_eq!(buffer.visible_indices(), vec![1]);
    }

    #[test]
    fn interleaving_is_timestamp_ordered_and_stable_for_ties() {
        let mut logs = MultiServiceLogs::new("10.0.0.1");
        logs.append_batch([
            LogEvent::new("z", "2026-01-09T16:40:02Z INFO second"),
            LogEvent::new("a", "2026-01-09T16:40:01Z INFO first-a"),
            LogEvent::new("b", "2026-01-09T16:40:01Z INFO first-b"),
        ]);
        let messages: Vec<_> = logs
            .buffer()
            .entries()
            .iter()
            .map(|entry| entry.message.as_str())
            .collect();
        assert_eq!(messages, ["first-a", "first-b", "second"]);
    }

    #[test]
    fn bounded_retention_keeps_the_newest_arrival_order() {
        let mut buffer = LogBuffer::new();
        buffer.append_batch(
            (0..=MAX_LOG_ENTRIES)
                .map(|index| LogEvent::new("kubelet", format!("INFO line-{index}"))),
        );
        assert_eq!(buffer.entries().len(), MAX_LOG_ENTRIES);
        assert_eq!(buffer.entries()[0].message, "line-1");
        assert_eq!(
            buffer.entries().last().unwrap().message,
            format!("line-{MAX_LOG_ENTRIES}")
        );
    }

    #[test]
    fn byte_budget_evicts_complete_oldest_lines_and_resets_matches() {
        let mut buffer = LogBuffer::new();
        buffer.append_batch([
            LogEvent::new("apid", "INFO oldest"),
            LogEvent::new("apid", "ERROR newest"),
        ]);
        buffer.set_query("newest");
        assert_eq!(buffer.current_match(), Some(1));
        assert_eq!(buffer.retain_newest_bytes("ERROR newest".len()), 1);
        assert_eq!(buffer.entries()[0].raw, "ERROR newest");
        assert_eq!(buffer.current_match(), Some(0));
        assert_eq!(buffer.retain_newest_bytes("ERROR newest".len()), 0);
        assert_eq!(buffer.retain_newest_bytes(0), 1);
        assert!(buffer.entries().is_empty());
        assert_eq!(buffer.current_match(), None);
    }

    #[test]
    fn a_stated_level_wins_and_lines_without_one_are_guessed_as_before() {
        let mut buffer = LogBuffer::new();
        buffer.append_batch([
            // Existing sources state no level: the words still decide.
            LogEvent::new("apid", "retrying after error"),
            LogEvent::new("apid", "INFO ready"),
            // A severity Coroot stored outranks the words in the line.
            LogEvent::new("app", "retrying after error").with_level(LogLevel::Info),
            LogEvent::new("app", "a plain line").with_level(LogLevel::Unknown),
            LogEvent::marker("app", Utc::now().fixed_offset(), "a note")
                .with_level(LogLevel::Error),
        ]);
        let levels: Vec<_> = buffer.entries().iter().map(|e| e.level.clone()).collect();
        assert_eq!(
            levels,
            [
                LogLevel::Error,
                LogLevel::Info,
                LogLevel::Info,
                LogLevel::Unknown,
                // A marker never has a level.
                LogLevel::Unknown,
            ]
        );
    }

    fn stamped(second: u32, text: &str) -> LogEvent {
        LogEvent::new("apid", format!("2026-01-09T16:40:{second:02}Z INFO {text}"))
    }

    #[test]
    fn late_lines_merge_into_order_and_report_where_they_landed() {
        let mut logs = MultiServiceLogs::new("10.0.0.1");
        let first = logs.append_bounded([stamped(10, "a"), stamped(30, "c")], usize::MAX);
        assert!(first.extended_tail());
        let tail = logs.append_bounded([stamped(40, "d"), stamped(35, "e")], usize::MAX);
        assert!(tail.extended_tail());
        let late = logs.append_bounded([stamped(20, "b"), stamped(50, "f")], usize::MAX);
        assert!(!late.extended_tail());
        assert_eq!(late.first_changed, 1);
        let messages: Vec<_> = logs
            .buffer()
            .entries()
            .iter()
            .map(|entry| entry.message.as_str())
            .collect();
        assert_eq!(messages, ["a", "b", "c", "e", "d", "f"]);
        let sequences: Vec<_> = logs
            .buffer()
            .entries()
            .iter()
            .map(LogEntry::sequence)
            .collect();
        assert_eq!(sequences, [0, 4, 1, 3, 2, 5]);
    }

    #[test]
    fn retained_bytes_follow_every_append_and_eviction() {
        let mut logs = MultiServiceLogs::new("10.0.0.1");
        let outcome = logs.append_bounded(
            (0..=MAX_LOG_ENTRIES).map(|ix| LogEvent::new("apid", format!("INFO l{ix}"))),
            usize::MAX,
        );
        assert_eq!(outcome.evicted.len(), 1);
        let actual: usize = logs.buffer().entries().iter().map(|e| e.raw.len()).sum();
        assert_eq!(logs.buffer().retained_bytes(), actual);
        let outcome = logs.append_bounded([LogEvent::new("apid", "INFO tail")], actual);
        assert!(outcome.evicted.len() > 1);
        let actual: usize = logs.buffer().entries().iter().map(|e| e.raw.len()).sum();
        assert_eq!(logs.buffer().retained_bytes(), actual);
    }
}
