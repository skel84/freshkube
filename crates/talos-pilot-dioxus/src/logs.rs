//! Bounded, caller-owned multi-service logs. Presentation never owns transport state.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use dioxus::prelude::*;
use futures::StreamExt;
use parking_lot::Mutex;
use talos_pilot_core::constants::MAX_LOG_ENTRIES;
use talos_pilot_core::logs::{
    LogEntry, LogEvent, LogFilters, LogStreamState, MultiServiceLogs, ServiceId,
};
use talos_pilot_core::types::LogLevel;
use tokio::sync::mpsc;

use crate::feature::{FeatureContext, LogRequest, copy_text};

const PAGE_SIZE: usize = 100;
const MAX_SELECTED_SERVICES: usize = 32;
const MAX_BATCH_BYTES: usize = 64 * 1024;
const MAX_UI_BATCH_BYTES: usize = 256 * 1024;
const QUEUE_SIZE: usize = 128;
const BATCH_SIZE: usize = 128;
const MAX_HISTORY_BYTES: usize = 8 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const BATCH_INTERVAL: Duration = Duration::from_millis(80);

#[derive(Clone, Debug)]
struct Row {
    id: u64,
    entry: LogEntry,
    // The core parser trims surrounding whitespace. Keep the transport text separately
    // so both DOM selection and clipboard actions preserve the exact received line.
    exact: String,
}

/// Immutable snapshots are merged on the blocking pool, never on the webview thread.
#[derive(Clone)]
struct Retained {
    merged: MultiServiceLogs,
    rows: Vec<Row>,
    next_id: u64,
    evicted: usize,
}

/// Core timestamp merging/retention plus stable row identities for selection and paging.
#[derive(Clone)]
struct History {
    retained: Arc<Retained>,
    filters: LogFilters,
    query: String,
    current_match: Option<u64>,
    stream: LogStreamState,
    page_start: usize,
    anchor: Option<u64>,
    frozen: Option<Vec<Row>>,
    selected: BTreeSet<u64>,
    range_anchor: Option<u64>,
    wrap: bool,
    scroll_revision: u64,
    scroll_destination: Option<ScrollDestination>,
}

/// One bounded current-target session, not a cache of every cluster ever visited.
/// There are deliberately no clients, receivers, or transport handles in this cache.
struct SavedSession {
    key: String,
    history: History,
    selected_services: BTreeSet<String>,
    request_signature: (Vec<String>, i32),
    collecting: bool,
    range_mode: bool,
}

#[derive(Default)]
struct SessionRegistry {
    target: Option<String>,
    saved: Option<SavedSession>,
}

impl SessionRegistry {
    fn retain_target(&mut self, target: Option<&str>) {
        if self.target.as_deref() != target {
            self.saved = None;
            self.target = target.map(str::to_owned);
        }
    }

    fn take(&mut self, key: &str) -> Option<SavedSession> {
        self.retain_target(Some(key));
        self.saved.take().filter(|saved| saved.key == key)
    }

    fn save(&mut self, saved: SavedSession) {
        // A dropping old panel must never resurrect a retired target/config epoch.
        if self.target.as_deref() == Some(saved.key.as_str()) {
            self.saved = Some(saved);
        }
    }
}

fn session_registry() -> &'static Mutex<SessionRegistry> {
    static REGISTRY: LazyLock<Mutex<SessionRegistry>> =
        LazyLock::new(|| Mutex::new(SessionRegistry::default()));
    &REGISTRY
}

/// Shell invokes this on actual identity/target transitions, even off the logs tab.
pub(crate) fn retain_log_target(target: Option<&str>) {
    session_registry().lock().retain_target(target);
}

impl Deref for History {
    type Target = Retained;
    fn deref(&self) -> &Retained {
        &self.retained
    }
}

impl History {
    fn new(address: String) -> Self {
        Self {
            retained: Arc::new(Retained {
                merged: MultiServiceLogs::new(address),
                rows: Vec::new(),
                next_id: 0,
                evicted: 0,
            }),
            filters: LogFilters::default(),
            query: String::new(),
            current_match: None,
            stream: LogStreamState::default(),
            page_start: 0,
            anchor: None,
            frozen: None,
            selected: BTreeSet::new(),
            range_anchor: None,
            wrap: true,
            scroll_revision: 0,
            scroll_destination: None,
        }
    }

    #[cfg(test)]
    fn append(&mut self, events: Vec<LogEvent>) {
        let mut retained = (*self.retained).clone();
        retained.append(events);
        self.commit(Arc::new(retained));
    }

    fn commit(&mut self, retained: Arc<Retained>) {
        self.retained = retained;
        self.prune_selection();
        self.reconcile_page();
    }

    fn prune_selection(&mut self) {
        let retained: BTreeSet<_> = self
            .rows
            .iter()
            .map(|row| row.id)
            .chain(self.frozen.iter().flatten().map(|row| row.id))
            .collect();
        self.selected.retain(|id| retained.contains(id));
        if self.current_match.is_some_and(|id| !retained.contains(&id)) {
            self.current_match = None;
        }
        if self.range_anchor.is_some_and(|id| !retained.contains(&id)) {
            self.range_anchor = None;
        }
    }

    fn visible(&self) -> Vec<&Row> {
        self.rows
            .iter()
            .filter(|row| self.filters.accepts(&row.entry))
            .collect()
    }

    fn matches(&self) -> Vec<u64> {
        if self.query.is_empty() {
            return Vec::new();
        }
        self.visible()
            .into_iter()
            .filter(|row| row.entry.matches_query(&self.query))
            .map(|row| row.id)
            .collect()
    }

    fn reconcile_page(&mut self) {
        let visible = self.visible();
        self.page_start = if self.stream.following && !self.stream.paused {
            visible.len().saturating_sub(PAGE_SIZE)
        } else {
            self.anchor
                .and_then(|id| visible.iter().position(|row| row.id == id))
                .unwrap_or(self.page_start)
                .min(visible.len().saturating_sub(1))
        };
        self.anchor = self.visible().get(self.page_start).map(|row| row.id);
    }

    fn page(&self) -> Vec<Row> {
        if let Some(frozen) = &self.frozen {
            return frozen.clone();
        }
        self.visible()
            .into_iter()
            .skip(self.page_start)
            .take(PAGE_SIZE)
            .cloned()
            .collect()
    }

    fn set_paused(&mut self, paused: bool) {
        if paused == self.stream.paused {
            return;
        }
        if paused {
            self.frozen = Some(self.page());
        } else {
            self.frozen = None;
        }
        self.stream.paused = paused;
        self.prune_selection();
        self.reconcile_page();
    }

    fn set_following(&mut self, following: bool) {
        self.stream.following = following;
        self.reconcile_page();
    }

    fn refresh_frozen(&mut self) {
        if self.stream.paused {
            self.frozen = None;
            self.frozen = Some(self.page());
        }
        self.prune_selection();
    }

    fn move_page(&mut self, action: PageAction) {
        let count = self.visible().len();
        self.stream.following = false;
        self.page_start = match action {
            PageAction::First => 0,
            PageAction::Previous => self.page_start.saturating_sub(PAGE_SIZE),
            PageAction::Next => (self.page_start + PAGE_SIZE).min(count.saturating_sub(PAGE_SIZE)),
            PageAction::Latest => count.saturating_sub(PAGE_SIZE),
        };
        self.anchor = self.visible().get(self.page_start).map(|row| row.id);
        self.refresh_frozen();
        self.request_scroll(match action {
            PageAction::Latest => ScrollDestination::Latest,
            _ => ScrollDestination::Start,
        });
    }

    fn filters_changed(&mut self) {
        self.current_match = None;
        self.page_start = 0;
        self.anchor = None;
        self.reconcile_page();
        self.refresh_frozen();
    }

    fn navigate_match(&mut self, forward: bool) {
        let matches = self.matches();
        let Some(first) = matches.first().copied() else {
            self.current_match = None;
            return;
        };
        let selected = match self
            .current_match
            .and_then(|id| matches.iter().position(|found| *found == id))
        {
            Some(index) if forward => matches[(index + 1) % matches.len()],
            Some(0) => matches[matches.len() - 1],
            Some(index) => matches[index - 1],
            None if forward => first,
            None => matches[matches.len() - 1],
        };
        self.current_match = Some(selected);
        self.stream.following = false;
        let index = self
            .visible()
            .iter()
            .position(|row| row.id == selected)
            .unwrap_or(0);
        self.page_start = index / PAGE_SIZE * PAGE_SIZE;
        self.anchor = self.visible().get(self.page_start).map(|row| row.id);
        self.refresh_frozen();
        self.request_scroll(ScrollDestination::Row(selected));
    }

    fn request_scroll(&mut self, destination: ScrollDestination) {
        self.scroll_revision = self.scroll_revision.wrapping_add(1);
        self.scroll_destination = Some(destination);
    }

    fn select_row(&mut self, id: u64, range: bool) {
        self.stream.following = false;
        if range {
            let ids: Vec<_> = self.visible().iter().map(|row| row.id).collect();
            if let (Some(start), Some(end)) = (
                self.range_anchor
                    .and_then(|anchor| ids.iter().position(|row| *row == anchor)),
                ids.iter().position(|row| *row == id),
            ) {
                self.selected
                    .extend(ids[start.min(end)..=start.max(end)].iter().copied());
                return;
            }
        }
        if !self.selected.insert(id) {
            self.selected.remove(&id);
        }
        self.range_anchor = Some(id);
    }

    fn selected_text(&self) -> String {
        let mut rows: Vec<_> = self
            .rows
            .iter()
            .filter(|row| self.selected.contains(&row.id))
            .collect();
        if let Some(frozen) = &self.frozen {
            for row in frozen {
                if self.selected.contains(&row.id)
                    && !rows.iter().any(|existing| existing.id == row.id)
                {
                    rows.push(row);
                }
            }
        }
        rows.sort_by_key(|row| {
            (
                row.entry
                    .timestamp
                    .as_ref()
                    .map_or(0, |timestamp| timestamp.sort_key),
                row.id,
            )
        });
        rows.iter()
            .map(|row| row.exact.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Retained {
    fn append(&mut self, events: Vec<LogEvent>) {
        if events.is_empty() {
            return;
        }
        let old_count = self.rows.len();
        // Duplicate lines must remain distinct, in arrival order, even across reconnect.
        let mut identities: BTreeMap<(ServiceId, String), VecDeque<(u64, String)>> =
            BTreeMap::new();
        for row in &self.rows {
            identities
                .entry((row.entry.service.clone(), row.entry.raw.clone()))
                .or_default()
                .push_back((row.id, row.exact.clone()));
        }
        let mut accepted = 0;
        for event in &events {
            if event.line.trim().is_empty() {
                continue;
            }
            identities
                .entry((event.service.clone(), event.line.trim().to_owned()))
                .or_default()
                .push_back((self.next_id, event.line.clone()));
            self.next_id = self.next_id.wrapping_add(1);
            accepted += 1;
        }
        self.merged.append_batch(events);
        // Core retention removes the oldest occurrences first. Consume those identities
        // before assigning retained duplicates, otherwise selection could silently move.
        let mut retained_counts = BTreeMap::<(ServiceId, String), usize>::new();
        for entry in self.merged.buffer().entries() {
            *retained_counts
                .entry((entry.service.clone(), entry.raw.clone()))
                .or_default() += 1;
        }
        for (key, queue) in &mut identities {
            let keep = retained_counts.get(key).copied().unwrap_or(0);
            while queue.len() > keep {
                queue.pop_front();
            }
        }
        self.rows = self
            .merged
            .buffer()
            .entries()
            .iter()
            .map(|entry| {
                let (id, exact) = identities
                    .get_mut(&(entry.service.clone(), entry.raw.clone()))
                    .and_then(VecDeque::pop_front)
                    .expect("every retained core entry has a transport identity");
                Row {
                    id,
                    entry: entry.clone(),
                    exact,
                }
            })
            .collect();
        // The core caps row count. Also cap bytes, since a single log line may be 64 KiB.
        let mut bytes = 0;
        let mut keep_from = self.rows.len();
        for (index, row) in self.rows.iter().enumerate().rev() {
            if bytes + row.exact.len() > MAX_HISTORY_BYTES {
                break;
            }
            bytes += row.exact.len();
            keep_from = index;
        }
        if keep_from > 0 {
            self.rows.drain(..keep_from);
            self.merged.replace(
                self.rows
                    .iter()
                    .map(|row| LogEvent::new(row.entry.service.clone(), row.exact.clone())),
            );
        }
        self.evicted += (old_count + accepted).saturating_sub(self.rows.len());
    }
}

#[derive(Clone, Copy)]
enum PageAction {
    First,
    Previous,
    Next,
    Latest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScrollDestination {
    Start,
    Latest,
    Row(u64),
}

/// Render identity is also the DOM callback's guard. Pausing or leaving follow
/// invalidates an already queued follow scroll, even when the page is unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ScrollView {
    target: String,
    rows: Vec<u64>,
    following: bool,
    paused: bool,
    wrap: bool,
    revision: u64,
    explicit: Option<ScrollDestination>,
}

impl ScrollView {
    fn new(target: String, history: &History, page: &[Row]) -> Self {
        Self {
            target,
            rows: page.iter().map(|row| row.id).collect(),
            following: history.stream.following,
            paused: history.stream.paused,
            wrap: history.wrap,
            revision: history.scroll_revision,
            explicit: history.scroll_destination,
        }
    }

    fn stamp(&self) -> String {
        serde_json::json!([
            self.target,
            self.rows,
            self.following,
            self.paused,
            self.wrap,
            self.revision,
            format!("{:?}", self.explicit),
        ])
        .to_string()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ScrollIntent {
    stamp: String,
    destination: ScrollDestination,
}

/// Pure reducer used by the mounted post-render effect, not just test fixtures.
struct ScrollState {
    target: String,
    previous: Option<ScrollView>,
    retired: bool,
}

impl ScrollState {
    fn new(target: String) -> Self {
        Self {
            target,
            previous: None,
            retired: false,
        }
    }

    fn reduce(&mut self, view: ScrollView, mounted: bool) -> Option<ScrollIntent> {
        if self.target != view.target {
            self.retire();
        }
        if self.retired || !mounted {
            return None;
        }
        let explicit_changed = self
            .previous
            .as_ref()
            .is_some_and(|previous| previous.revision != view.revision);
        let changed = self.previous.as_ref() != Some(&view);
        self.previous = Some(view.clone());
        let destination = if explicit_changed {
            view.explicit
        } else if changed && view.following && !view.paused {
            Some(ScrollDestination::Latest)
        } else {
            None
        }?;
        if view.rows.is_empty()
            || matches!(destination, ScrollDestination::Row(id) if !view.rows.contains(&id))
        {
            return None;
        }
        Some(ScrollIntent {
            stamp: view.stamp(),
            destination,
        })
    }

    fn current(&self, intent: &ScrollIntent) -> bool {
        !self.retired
            && self
                .previous
                .as_ref()
                .is_some_and(|view| view.stamp() == intent.stamp)
    }

    fn retire(&mut self) {
        self.retired = true;
        self.previous = None;
    }
}

fn scroll_script(viewport_id: &str, intent: &ScrollIntent) -> String {
    // Only generated numeric row IDs enter selectors. Target/config strings are
    // JSON escaped, never interpolated as JavaScript or CSS selector source.
    let id = serde_json::json!(viewport_id);
    let stamp = serde_json::json!(intent.stamp);
    let action = match intent.destination {
        ScrollDestination::Start => "pane.scrollTop = 0;".to_owned(),
        ScrollDestination::Latest => "pane.scrollTop = pane.scrollHeight;".to_owned(),
        ScrollDestination::Row(row) => format!(
            r#"const row = pane.querySelector('[data-log-row="{row}"]');
            if (!row) return;
            const bounds = pane.getBoundingClientRect();
            const rect = row.getBoundingClientRect();
            const top = bounds.top + pane.clientTop;
            const bottom = top + pane.clientHeight;
            if (rect.top < top || rect.height > pane.clientHeight) {{
                pane.scrollTop += rect.top - top;
            }} else if (rect.bottom > bottom) {{
                pane.scrollTop += rect.bottom - bottom;
            }}"#
        ),
    };
    format!(
        r#"requestAnimationFrame(() => {{
            const pane = document.getElementById({id});
            if (!pane || !pane.isConnected || pane.dataset.logScroll !== {stamp}) return;
            {action}
        }});"#
    )
}

fn log_level_class(level: &LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "log-level-error",
        LogLevel::Warning => "log-level-warn",
        LogLevel::Info => "log-level-info",
        LogLevel::Debug => "log-level-debug",
        LogLevel::Unknown => "log-level-unknown",
    }
}

/// Every spawned worker is owned here; dropping or replacing it aborts idle transport.
struct Worker(tokio::task::JoinHandle<()>);
impl Drop for Worker {
    fn drop(&mut self) {
        self.0.abort();
    }
}

enum Message {
    Services(u64, Result<Vec<String>, String>),
    Lines(u64, String, Vec<String>),
    Status(u64, String, String),
}

fn generation_current(generation: u64, current: u64, collecting: bool) -> bool {
    generation == current && collecting
}

/// Shared by the live controller and fixtures, including independent service EOF.
fn collect_message(
    message: Message,
    current: u64,
    collecting: bool,
    selected: &BTreeSet<String>,
    statuses: &mut BTreeMap<String, String>,
) -> Vec<LogEvent> {
    match message {
        Message::Lines(generation, service, lines)
            if generation_current(generation, current, collecting)
                && selected.contains(&service) =>
        {
            lines
                .into_iter()
                .map(|line| LogEvent::new(service.clone(), line))
                .collect()
        }
        Message::Status(generation, service, status)
            if generation_current(generation, current, collecting)
                && selected.contains(&service) =>
        {
            statuses.insert(service, status);
            Vec::new()
        }
        _ => Vec::new(),
    }
}

struct Controller {
    ctx: FeatureContext,
    history: History,
    tail: i32,
    generation: u64,
    service_generation: u64,
    selected_services: BTreeSet<String>,
    available_services: BTreeSet<String>,
    statuses: BTreeMap<String, String>,
    collecting: bool,
    tx: mpsc::Sender<Message>,
    rx: Option<mpsc::Receiver<Message>>,
    workers: Vec<Worker>,
    discovery: Option<Worker>,
    discovery_error: Option<String>,
    copy_status: Option<String>,
    merge_error: Option<String>,
    copying: bool,
    range_mode: bool,
    request_signature: (Vec<String>, i32),
    session_key: String,
}

impl Controller {
    fn new(ctx: FeatureContext, initial_services: Vec<String>, tail: i32) -> Self {
        let (tx, rx) = mpsc::channel(QUEUE_SIZE);
        let session_key = ctx.key();
        let request_signature = (initial_services.clone(), tail);
        let requested_services = initial_services
            .into_iter()
            .filter(|service| !service.is_empty())
            .take(MAX_SELECTED_SERVICES)
            .collect::<BTreeSet<_>>();
        let saved = session_registry().lock().take(&session_key);
        let (history, selected_services, collecting, range_mode) = match saved {
            Some(saved) if saved.request_signature == request_signature => (
                saved.history,
                saved.selected_services,
                saved.collecting,
                saved.range_mode,
            ),
            Some(saved) => (saved.history, requested_services, true, saved.range_mode),
            None => (
                History::new(ctx.address.clone()),
                requested_services,
                true,
                false,
            ),
        };
        let mut controller = Self {
            history,
            ctx,
            tail,
            generation: 0,
            service_generation: 0,
            available_services: selected_services.clone(),
            selected_services,
            statuses: BTreeMap::new(),
            collecting,
            tx,
            rx: Some(rx),
            workers: Vec::new(),
            discovery: None,
            discovery_error: None,
            copy_status: None,
            merge_error: None,
            copying: false,
            range_mode,
            request_signature,
            session_key,
        };
        controller.discover();
        if controller.collecting {
            controller.reconnect();
        } else {
            controller.stop();
        }
        controller
    }

    fn remember(&self) {
        session_registry().lock().save(SavedSession {
            key: self.session_key.clone(),
            history: self.history.clone(),
            selected_services: self.selected_services.clone(),
            request_signature: self.request_signature.clone(),
            collecting: self.collecting,
            range_mode: self.range_mode,
        });
    }

    fn request(&mut self, services: Vec<String>, tail: i32) {
        if self.request_signature == (services.clone(), tail) {
            return;
        }
        self.request_signature = (services.clone(), tail);
        self.tail = tail;
        self.selected_services = services
            .into_iter()
            .filter(|service| !service.is_empty())
            .take(MAX_SELECTED_SERVICES)
            .collect();
        self.available_services
            .extend(self.selected_services.iter().cloned());
        self.reconnect();
    }

    fn discover(&mut self) {
        self.service_generation = self.service_generation.wrapping_add(1);
        self.discovery = None;
        self.discovery_error = None;
        let generation = self.service_generation;
        let client = self.ctx.client.clone();
        let tx = self.tx.clone();
        self.discovery = Some(Worker(self.ctx.runtime.spawn(async move {
            let result = match tokio::time::timeout(REQUEST_TIMEOUT, client.services()).await {
                Ok(Ok(nodes)) => Ok(nodes
                    .into_iter()
                    .flat_map(|node| node.services.into_iter().map(|service| service.id))
                    .collect()),
                Ok(Err(error)) => Err(error.to_string()),
                Err(_) => Err(
                    "Service discovery timed out; selected services remain available.".to_owned(),
                ),
            };
            let _ = tx.send(Message::Services(generation, result)).await;
        })));
    }

    fn stop(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.workers.clear();
        self.collecting = false;
        for status in self.statuses.values_mut() {
            *status = "Stopped (history retained)".to_owned();
        }
    }

    fn reconnect(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.workers.clear();
        self.statuses.clear();
        self.collecting = true;
        for service in &self.selected_services {
            self.statuses
                .insert(service.clone(), "Connecting…".to_owned());
            let client = self.ctx.client.clone();
            let tx = self.tx.clone();
            let generation = self.generation;
            let service = service.clone();
            let tail = self.tail;
            self.workers.push(Worker(self.ctx.runtime.spawn(async move {
                let stream = match tokio::time::timeout(REQUEST_TIMEOUT, client.logs_follow(&service, tail)).await {
                    Ok(Ok(stream)) => stream,
                    result => {
                        let message = match result {
                            Ok(Err(error)) => error.to_string(),
                            Err(_) => "Connection timed out".to_owned(),
                            Ok(Ok(_)) => unreachable!(),
                        };
                        let _ = tx.send(Message::Status(generation, service, message)).await;
                        return;
                    }
                };
                let _ = tx.send(Message::Status(generation, service.clone(), "Following".to_owned())).await;
                futures::pin_mut!(stream);
                let mut lines = Vec::with_capacity(BATCH_SIZE);
                let mut bytes = 0;
                let mut tick = tokio::time::interval(BATCH_INTERVAL);
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        line = stream.next() => {
                            match line {
                                Some(Ok(line)) => {
                                    bytes += line.len();
                                    lines.push(line);
                                }
                                terminal => {
                                    if !lines.is_empty() && tx.send(Message::Lines(generation, service.clone(), std::mem::take(&mut lines))).await.is_err() {
                                        return;
                                    }
                                    let status = match terminal {
                                        Some(Err(error)) => format!("Disconnected: {error}; reconnect to retry"),
                                        None => "Stream ended; reconnect to retry".to_owned(),
                                        Some(Ok(_)) => unreachable!(),
                                    };
                                    let _ = tx.send(Message::Status(generation, service, status)).await;
                                    return;
                                }
                            }
                        }
                        _ = tick.tick() => {
                            if !lines.is_empty() && tx.send(Message::Lines(generation, service.clone(), std::mem::take(&mut lines))).await.is_err() {
                                return;
                            }
                            bytes = 0;
                        }
                    }
                    if lines.len() >= BATCH_SIZE || bytes >= MAX_BATCH_BYTES {
                        if tx.send(Message::Lines(generation, service.clone(), std::mem::take(&mut lines))).await.is_err() {
                            return;
                        }
                        bytes = 0;
                    }
                }
            })));
        }
    }

    fn toggle_service(&mut self, service: String) {
        if !self.selected_services.remove(&service) {
            if self.selected_services.len() >= MAX_SELECTED_SERVICES {
                self.copy_status = Some(format!(
                    "Select at most {MAX_SELECTED_SERVICES} simultaneous streams; uncheck another service first."
                ));
                return;
            }
            self.selected_services.insert(service);
        }
        // No event from the old set can commit after a selection change.
        if self.collecting {
            self.reconnect();
        } else {
            self.stop();
        }
    }

    fn apply_batch(&mut self, messages: Vec<Message>) -> Vec<LogEvent> {
        let mut events = Vec::new();
        for message in messages {
            match message {
                Message::Services(generation, result) if generation == self.service_generation => {
                    match result {
                        Ok(services) => {
                            self.available_services = services
                                .into_iter()
                                .chain(self.selected_services.iter().cloned())
                                .collect();
                            self.discovery_error = None;
                        }
                        Err(error) => self.discovery_error = Some(error),
                    }
                }
                message => events.extend(collect_message(
                    message,
                    self.generation,
                    self.collecting,
                    &self.selected_services,
                    &mut self.statuses,
                )),
            }
        }
        events
    }
}

fn begin_copy(mut state: Signal<Controller>, text: String) {
    if text.is_empty() {
        state.write().copy_status = Some("Nothing selected to copy.".to_owned());
        return;
    }
    if state.read().copying {
        return;
    }
    let ctx = state.read().ctx.clone();
    state.write().copying = true;
    spawn(async move {
        let result = copy_text(ctx, text).await;
        let mut controller = state.write();
        controller.copying = false;
        controller.copy_status = Some(match result {
            Ok(()) => "Copied exact log text.".to_owned(),
            Err(error) => format!("Clipboard failed: {error}"),
        });
    });
}

#[component]
pub(crate) fn LogsPanel(
    ctx: FeatureContext,
    on_logs: EventHandler<LogRequest>,
    initial_services: Vec<String>,
    tail: i32,
) -> Element {
    let _ = on_logs;
    let mut state = use_signal(|| Controller::new(ctx.clone(), initial_services.clone(), tail));
    let viewport_id = use_hook(|| {
        static NEXT_VIEWPORT: AtomicU64 = AtomicU64::new(1);
        format!(
            "talos-log-view-{}",
            NEXT_VIEWPORT.fetch_add(1, Ordering::Relaxed)
        )
    });
    let mut mounted = use_signal(|| false);
    let scroll_state = use_hook(|| Arc::new(Mutex::new(ScrollState::new(ctx.key()))));
    // Prop changes from service/member log links are navigation requests, not a
    // target reset. Keep same-target retained history and exact row identities.
    use_effect(use_reactive(
        (&initial_services, &tail),
        move |(services, tail)| {
            state.write().request(services, tail);
        },
    ));
    // This future is Dioxus-scope-owned. Controller owns every runtime worker;
    // scope destruction drops both receiver and workers, including idle streams.
    use_future(move || {
        let mut rx = state.write().rx.take().expect("one receiver per log panel");
        async move {
            while let Some(message) = rx.recv().await {
                let mut batch = vec![message];
                // Keep one render update bounded by the batch budget, not queue capacity
                // times per-message lines (which could otherwise create huge UI frames).
                let mut count = match &batch[0] {
                    Message::Lines(_, _, lines) => lines.len(),
                    _ => 1,
                };
                let mut bytes = match &batch[0] {
                    Message::Lines(_, _, lines) => lines.iter().map(String::len).sum(),
                    _ => 0,
                };
                while count < BATCH_SIZE && bytes < MAX_UI_BATCH_BYTES {
                    match rx.try_recv() {
                        Ok(message) => {
                            count += match &message {
                                Message::Lines(_, _, lines) => lines.len(),
                                _ => 1,
                            };
                            bytes += match &message {
                                Message::Lines(_, _, lines) => lines.iter().map(String::len).sum(),
                                _ => 0,
                            };
                            batch.push(message);
                        }
                        Err(_) => break,
                    }
                }
                let (events, retained, ctx, generation) = {
                    let mut controller = state.write();
                    let events = controller.apply_batch(batch);
                    (
                        events,
                        controller.history.retained.clone(),
                        controller.ctx.clone(),
                        controller.generation,
                    )
                };
                if events.is_empty() {
                    continue;
                }
                // Clone/parse/sort/prune at most a bounded batch on Tokio's blocking
                // pool. Only the bounded viewport is materialized by the webview.
                let result = ctx
                    .run(async move {
                        tokio::task::spawn_blocking(move || {
                            let mut retained = (*retained).clone();
                            retained.append(events);
                            Arc::new(retained)
                        })
                        .await
                        .map_err(|error| format!("Log merge worker failed: {error}"))
                    })
                    .await
                    .and_then(|result| result);
                let mut controller = state.write();
                if generation_current(generation, controller.generation, controller.collecting) {
                    match result {
                        Ok(retained) => {
                            controller.history.commit(retained);
                            controller.merge_error = None;
                        }
                        Err(error) => controller.merge_error = Some(error),
                    }
                }
            }
        }
    });
    // Drop signal-backed resources explicitly even if a spawned copy future keeps
    // the signal allocation alive until cancellation has propagated.
    let retiring_scroll = scroll_state.clone();
    use_drop(move || {
        retiring_scroll.lock().retire();
        let mut controller = state.write();
        controller.remember();
        controller.stop();
        controller.discovery = None;
    });

    let controller = state.read();
    let page = controller.history.page();
    let page_len = page.len();
    let page_empty = page.is_empty();
    let visible_count = controller.history.visible().len();
    let retained_count = controller.history.rows.len();
    let match_ids = controller.history.matches();
    let match_position = controller
        .history
        .current_match
        .and_then(|id| match_ids.iter().position(|found| *found == id))
        .map(|index| index + 1)
        .unwrap_or(0);
    let page_start = controller.history.page_start;
    let paused = controller.history.stream.paused;
    let following = controller.history.stream.following;
    let collecting = controller.collecting;
    let wrap = controller.history.wrap;
    let selected_count = controller.history.selected.len();
    let range_mode = controller.range_mode;
    let copying = controller.copying;
    let query = controller.history.query.clone();
    let levels = controller.history.filters.levels.clone();
    let service_options: BTreeSet<_> = controller
        .available_services
        .iter()
        .cloned()
        .chain(
            controller
                .history
                .rows
                .iter()
                .map(|row| row.entry.service.as_str().to_owned()),
        )
        .collect();
    let services: Vec<_> = service_options
        .into_iter()
        .map(|service| {
            let collect = controller.selected_services.contains(&service);
            let shown = controller
                .history
                .filters
                .services
                .as_ref()
                .is_none_or(|set| set.contains(&ServiceId::from(service.clone())));
            let status = controller
                .statuses
                .get(&service)
                .cloned()
                .unwrap_or_else(|| "Not collecting".to_owned());
            (service, collect, shown, status)
        })
        .collect();
    let discovery_error = controller.discovery_error.clone();
    let merge_error = controller.merge_error.clone();
    let copy_status = controller.copy_status.clone();
    let evicted = controller.history.evicted;
    let selected = controller.history.selected.clone();
    let current_match = controller.history.current_match;
    let scroll_view = ScrollView::new(ctx.key(), &controller.history, &page);
    drop(controller);
    let scroll_stamp = scroll_view.stamp();
    use_effect(use_reactive(
        (&scroll_view, &viewport_id),
        move |(view, viewport_id)| {
            let ready = mounted();
            let mut reducer = scroll_state.lock();
            if let Some(intent) = reducer.reduce(view, ready)
                && reducer.current(&intent)
            {
                // Effects run after render; mounting gates the first effect.
                // The next-frame callback checks this exact render and unique
                // panel ID, so it cannot scroll a replacement target/unmount.
                // DesktopDocument dispatches immediately in new_query; Eval is
                // Copy and its evaluator is owned by the query slot, not this
                // handle. JS channel cleanup does not cancel animation frames.
                document::eval(&scroll_script(&viewport_id, &intent));
            }
        },
    ));

    rsx! {
        section { class: "panel logs-panel",
            h2 { "Multi-service logs" }
            p { class: "muted", "Target: {ctx.context} / {ctx.node} ({ctx.address}). Each checked service has an independent pull-based stream." }
            div { class: "toolbar",
                button { onclick: move |_| state.write().reconnect(), "Reconnect (retain history)" }
                button { disabled: !collecting, onclick: move |_| state.write().stop(), "Stop collection" }
                button { onclick: move |_| state.write().history.set_paused(!paused), if paused { "Resume presentation" } else { "Pause presentation" } }
                label { input { r#type: "checkbox", checked: following, onchange: move |event| state.write().history.set_following(event.checked()) } "Follow latest" }
                label { input { r#type: "checkbox", checked: wrap, onchange: move |event| state.write().history.wrap = event.checked() } "Wrap lines" }
                button { onclick: move |_| state.write().discover(), "Refresh services" }
            }
            p { class: "muted", "Collection enabled: {collecting} · Presentation paused: {paused} · Follow: {following} · Retained: {retained_count} · Evicted: {evicted} ({MAX_LOG_ENTRIES} rows / 8 MiB limit; at most 32 simultaneous services). Reconnect may replay the requested tail; duplicate records remain selectable." }
            if let Some(error) = discovery_error { p { class: "error", "Services unavailable: {error}" } }
            if let Some(error) = merge_error { p { class: "feature-status error", "{error}. Retained history is still available." } }
            details { open: true,
                summary { "Services: collect and presentation filters are independent" }
                div { style: "max-height: 220px; overflow: auto;",
                    for (service, collect, shown, status) in services {
                        div { key: "{service}", class: "toolbar",
                            label {
                                input { r#type: "checkbox", checked: collect,
                                    onchange: {
                                        let service = service.clone();
                                        move |_| state.write().toggle_service(service.clone())
                                    }
                                }
                                "Collect {service}"
                            }
                            label {
                                input { r#type: "checkbox", checked: shown,
                                    onchange: {
                                        let service = service.clone();
                                        move |event: Event<FormData>| {
                                            let mut controller = state.write();
                                            let mut shown = controller.history.filters.services.clone().unwrap_or_else(|| {
                                                controller.available_services.iter().cloned().map(ServiceId::from)
                                                    .chain(controller.history.rows.iter().map(|row| row.entry.service.clone())).collect()
                                            });
                                            if event.checked() { shown.insert(ServiceId::from(service.clone())); }
                                            else { shown.remove(&ServiceId::from(service.clone())); }
                                            controller.history.filters.services = Some(shown);
                                            controller.history.filters_changed();
                                        }
                                    }
                                }
                                "Show"
                            }
                            span { class: "muted", "{status}" }
                        }
                    }
                }
                button { onclick: move |_| {
                    let mut controller = state.write();
                    controller.history.filters.services = None;
                    controller.history.filters_changed();
                }, "Show all retained services" }
            }
            div { class: "toolbar",
                for (label, level, enabled) in [
                    ("Error", LogLevel::Error, levels.error),
                    ("Warning", LogLevel::Warning, levels.warning),
                    ("Info", LogLevel::Info, levels.info),
                    ("Debug", LogLevel::Debug, levels.debug),
                    ("Unknown", LogLevel::Unknown, levels.unknown),
                ] {
                    label { key: "{label}",
                        input { r#type: "checkbox", checked: enabled, onchange: move |event| {
                            let mut controller = state.write();
                            controller.history.filters.levels.set(&level, event.checked());
                            controller.history.filters_changed();
                        } }
                        "{label}"
                    }
                }
            }
            div { class: "toolbar",
                input { r#type: "search", aria_label: "Case-insensitive log search", placeholder: "Search retained filtered history…", value: "{query}", oninput: move |event| {
                    let mut controller = state.write();
                    controller.history.query = event.value();
                    controller.history.current_match = None;
                } }
                button { disabled: match_ids.is_empty(), onclick: move |_| state.write().history.navigate_match(false), "Previous match" }
                button { disabled: match_ids.is_empty(), onclick: move |_| state.write().history.navigate_match(true), "Next match" }
                span { "{match_position} / {match_ids.len()} matches (cyclic)" }
            }
            div { class: "toolbar",
                button { onclick: move |_| state.write().history.move_page(PageAction::First), "First retained" }
                button { disabled: page_start == 0, onclick: move |_| state.write().history.move_page(PageAction::Previous), "Previous page" }
                button { disabled: page_start + PAGE_SIZE >= visible_count, onclick: move |_| state.write().history.move_page(PageAction::Next), "Next page" }
                button { onclick: move |_| state.write().history.move_page(PageAction::Latest), "Latest retained" }
                span { "Filtered rows {page_start.saturating_add(usize::from(!page_empty))}–{(page_start + page_len).min(visible_count)} / {visible_count}" }
            }
            div { class: "toolbar",
                label { input { r#type: "checkbox", checked: range_mode, onchange: move |event| state.write().range_mode = event.checked() } "Extend range from last selected row (across pages)" }
                button { onclick: move |_| {
                    let mut controller = state.write();
                    let ids: Vec<_> = controller.history.page().iter().map(|row| row.id).collect();
                    controller.history.selected.extend(ids);
                }, "Select page" }
                button { onclick: move |_| {
                    let mut controller = state.write();
                    controller.history.selected.clear();
                    controller.history.range_anchor = None;
                }, "Clear selection" }
                button { disabled: copying || selected_count == 0, onclick: move |_| {
                    let text = state.read().history.selected_text();
                    begin_copy(state, text);
                }, "Copy selected ({selected_count})" }
                button { disabled: copying || page_empty, onclick: move |_| {
                    let text = state.read().history.page().iter().map(|row| row.exact.as_str()).collect::<Vec<_>>().join("\n");
                    begin_copy(state, text);
                }, "Copy page" }
            }
            if let Some(message) = copy_status { p { class: "muted", "{message}" } }
            div { id: "{viewport_id}", class: "log-lines text-review-region", role: "region", tabindex: "0",
                aria_label: "Retained log rows; scroll to review history", "data-log-scroll": "{scroll_stamp}",
                onmounted: move |_| mounted.set(true),
                style: "max-height: 65vh; overflow: auto; min-width: 0;",
                if page_empty { p { class: "muted", "No retained rows match the service/severity filters. Check services to collect logs, or reconnect." } }
                for row in page {
                    div { key: "{row.id}", class: log_level_class(&row.entry.level), "data-log-row": "{row.id}",
                        style: if current_match == Some(row.id) { "background: #423c18; padding: 4px;" } else { "padding: 4px;" },
                        div { class: "toolbar",
                            input { r#type: "checkbox", checked: selected.contains(&row.id), aria_label: "Select retained log row", onchange: move |_| state.write().history.select_row(row.id, range_mode) }
                            small { "{row.entry.service.as_str()} · {row.entry.level:?}" }
                            button { disabled: copying, onclick: {
                                let text = row.exact.clone();
                                move |_| begin_copy(state, text.clone())
                            }, "Copy row" }
                        }
                        pre { style: if wrap { "white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; margin: 0;" } else { "white-space: pre; user-select: text; margin: 0;" }, "{row.exact}" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(service: &str, lines: &[&str]) -> Vec<LogEvent> {
        lines
            .iter()
            .map(|line| LogEvent::new(service, *line))
            .collect()
    }

    fn scroll_view(history: &History) -> ScrollView {
        ScrollView::new("target".to_owned(), history, &history.page())
    }

    #[test]
    fn scroll_intent_search_reveals_page_end_and_older_matches() {
        let mut history = History::new("node".to_owned());
        history.append(
            (0..350)
                .map(|index| {
                    let message = if index == 99 || index == 299 {
                        format!("INFO needle {index}")
                    } else {
                        format!("INFO context {index}")
                    };
                    LogEvent::new("a", message)
                })
                .collect(),
        );
        let mut reducer = ScrollState::new("target".to_owned());
        reducer.reduce(scroll_view(&history), true);
        history.query = "needle".to_owned();
        history.navigate_match(false);
        let newer = history.current_match.unwrap();
        assert_eq!(history.page().last().unwrap().id, newer);
        let intent = reducer.reduce(scroll_view(&history), true).unwrap();
        assert_eq!(intent.destination, ScrollDestination::Row(newer));
        assert!(!history.stream.following);
        history.navigate_match(false);
        let older = history.current_match.unwrap();
        assert_eq!(history.page().last().unwrap().id, older);
        assert_eq!(history.page_start, 0);
        assert_eq!(
            reducer
                .reduce(scroll_view(&history), true)
                .unwrap()
                .destination,
            ScrollDestination::Row(older)
        );
        assert!(!reducer.current(&intent));
        // A single-match cyclic jump is still an explicit request after manual
        // scrolling, even though the destination and page do not change.
        history.query = "needle 99".to_owned();
        history.navigate_match(true);
        let first = reducer.reduce(scroll_view(&history), true).unwrap();
        history.navigate_match(true);
        let repeated = reducer.reduce(scroll_view(&history), true).unwrap();
        assert_eq!(first.destination, repeated.destination);
        assert_ne!(first.stamp, repeated.stamp);
    }

    #[test]
    fn scroll_intent_follow_updates_to_newest_tail_after_mount() {
        let mut history = History::new("node".to_owned());
        history.append(
            (0..150)
                .map(|index| LogEvent::new("a", format!("INFO row {index}")))
                .collect(),
        );
        let mut reducer = ScrollState::new("target".to_owned());
        assert!(reducer.reduce(scroll_view(&history), false).is_none());
        let first = reducer.reduce(scroll_view(&history), true).unwrap();
        assert_eq!(first.destination, ScrollDestination::Latest);
        history.append(events("a", &["INFO newest tail"]));
        assert_eq!(history.page().last().unwrap().exact, "INFO newest tail");
        let latest = reducer.reduce(scroll_view(&history), true).unwrap();
        assert_eq!(latest.destination, ScrollDestination::Latest);
        assert!(!reducer.current(&first));
        assert!(reducer.current(&latest));
        assert!(reducer.reduce(scroll_view(&history), true).is_none());
        let script = scroll_script("talos-log-view-1", &latest);
        assert!(script.contains("pane.scrollTop = pane.scrollHeight"));
        assert!(!script.contains("scrollIntoView"));
    }

    #[test]
    fn scroll_intent_preserves_paused_and_manual_viewports() {
        let mut history = History::new("node".to_owned());
        history.append(events("a", &["INFO needle", "INFO context"]));
        let mut reducer = ScrollState::new("target".to_owned());
        let pending = reducer.reduce(scroll_view(&history), true).unwrap();
        history.set_paused(true);
        assert!(reducer.reduce(scroll_view(&history), true).is_none());
        assert!(!reducer.current(&pending));
        history.append(events("a", &["INFO while paused"]));
        assert!(reducer.reduce(scroll_view(&history), true).is_none());
        // Explicit search navigation remains available while presentation is
        // paused, including matches outside the frozen viewport.
        history.query = "while paused".to_owned();
        history.navigate_match(true);
        assert_eq!(
            reducer
                .reduce(scroll_view(&history), true)
                .unwrap()
                .destination,
            ScrollDestination::Row(history.current_match.unwrap())
        );
        history.set_paused(false);
        assert!(reducer.reduce(scroll_view(&history), true).is_none());
        history.append(events("a", &["INFO manual remains"]));
        assert!(reducer.reduce(scroll_view(&history), true).is_none());
        history.wrap = false;
        assert!(reducer.reduce(scroll_view(&history), true).is_none());
        // Restoring a non-following session is not a fresh search/page jump.
        let mut restored = ScrollState::new("target".to_owned());
        assert!(restored.reduce(scroll_view(&history), true).is_none());
        history.move_page(PageAction::Latest);
        assert_eq!(
            reducer
                .reduce(scroll_view(&history), true)
                .unwrap()
                .destination,
            ScrollDestination::Latest
        );
        history.move_page(PageAction::First);
        assert_eq!(
            reducer
                .reduce(scroll_view(&history), true)
                .unwrap()
                .destination,
            ScrollDestination::Start
        );
        history.set_following(true);
        assert_eq!(
            reducer
                .reduce(scroll_view(&history), true)
                .unwrap()
                .destination,
            ScrollDestination::Latest
        );
    }

    #[test]
    fn scroll_intent_retirement_and_dom_guard_ignore_old_callbacks() {
        let mut history = History::new("node".to_owned());
        history.append(events("a", &["INFO line"]));
        let mut reducer = ScrollState::new("target".to_owned());
        let pending = reducer.reduce(scroll_view(&history), true).unwrap();
        let mut changed_target = scroll_view(&history);
        changed_target.target = "replacement".to_owned();
        assert!(reducer.reduce(changed_target, true).is_none());
        assert!(!reducer.current(&pending));
        assert!(reducer.reduce(scroll_view(&history), true).is_none());
        let mut unmounted = ScrollState::new("target".to_owned());
        let pending = unmounted.reduce(scroll_view(&history), true).unwrap();
        unmounted.retire();
        assert!(!unmounted.current(&pending));
        assert!(unmounted.reduce(scroll_view(&history), true).is_none());
        let script = scroll_script("talos-log-view-17", &pending);
        assert!(script.contains("requestAnimationFrame"));
        assert!(script.contains("document.getElementById(\"talos-log-view-17\")"));
        assert!(script.contains("!pane.isConnected"));
        assert!(script.contains("pane.dataset.logScroll !=="));
        assert_ne!(script, scroll_script("talos-log-view-18", &pending));
        let row_script = scroll_script(
            "talos-log-view-17",
            &ScrollIntent {
                destination: ScrollDestination::Row(99),
                ..pending
            },
        );
        assert!(row_script.contains("[data-log-row=\"99\"]"));
        assert!(row_script.contains("pane.scrollTop += rect.bottom - bottom"));
    }

    #[test]
    fn export_production_scroll_scripts_for_browser_smoke() {
        fn fixture_history() -> History {
            let mut history = History::new("node".to_owned());
            history.append(
                (0..200)
                    .map(|index| {
                        let message = if index == 199 {
                            "INFO needle at page end".to_owned()
                        } else {
                            format!("INFO context {index}")
                        };
                        LogEvent::new("a", message)
                    })
                    .collect(),
            );
            history
        }

        fn escape_attribute(value: &str) -> String {
            value
                .replace('&', "&amp;")
                .replace('"', "&quot;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        }

        // The JS file is the exact generator called by LogsPanel's mounted
        // effect. HTML contains only bounded retained rows, never a live client.
        fn fixture(
            name: &str,
            history: &History,
            view: &ScrollView,
            intent: &ScrollIntent,
            pane_id: &str,
            script_id: &str,
            expected: &str,
        ) -> (String, String, serde_json::Value) {
            let page = history.page();
            assert_eq!(view.rows, page.iter().map(|row| row.id).collect::<Vec<_>>());
            assert_eq!(page.len(), PAGE_SIZE);
            let script = scroll_script(script_id, intent);
            assert!(script.contains(&format!(
                "document.getElementById({})",
                serde_json::json!(script_id)
            )));
            assert!(script.contains(&format!(
                "pane.dataset.logScroll !== {}",
                serde_json::json!(intent.stamp)
            )));
            assert!(script.contains("!pane.isConnected"));
            assert!(script.starts_with("requestAnimationFrame"));
            assert!(!script.contains("scrollIntoView"));
            let row_id = match intent.destination {
                ScrollDestination::Row(id) => {
                    assert!(script.contains(&format!("[data-log-row=\"{id}\"]")));
                    assert!(script.contains("pane.scrollTop += rect.bottom - bottom"));
                    Some(id)
                }
                ScrollDestination::Latest => {
                    assert!(script.contains("pane.scrollTop = pane.scrollHeight"));
                    None
                }
                ScrollDestination::Start => {
                    assert!(script.contains("pane.scrollTop = 0"));
                    None
                }
            };
            let rows = page
                .iter()
                .map(|row| {
                    format!(
                        "<div data-log-row=\"{}\" class=\"{}\" style=\"height:32px;box-sizing:border-box\">row {}</div>",
                        row.id,
                        log_level_class(&row.entry.level),
                        row.id,
                    )
                })
                .collect::<String>();
            let html = format!(
                r#"<!doctype html><meta charset="utf-8"><title>Log scroll {name}</title>
<body style="margin:0"><div style="height:600px">Outer scroll must remain unchanged</div>
<div id="{pane_id}" class="log-lines text-review-region" role="region" tabindex="0"
data-log-scroll="{stamp}" style="height:160px;overflow:auto;border:2px solid;min-width:0">{rows}</div>
<div style="height:1200px"></div>
<script>document.getElementById({encoded_id}).scrollTop = 64; window.scrollTo(0, 240);</script>
</body>"#,
                stamp = escape_attribute(&view.stamp()),
                encoded_id = serde_json::json!(pane_id),
            );
            let manifest = serde_json::json!({
                "name": name,
                "html": format!("log-scroll-{name}.html"),
                "script": format!("log-scroll-{name}.js"),
                "pane_id": pane_id,
                "script_pane_id": script_id,
                "row_id": row_id,
                "initial_scroll_top": 64,
                "initial_outer_scroll_y": 240,
                "expected": expected,
                "render_stamp": view.stamp(),
                "intent_stamp": intent.stamp,
            });
            (html, script, manifest)
        }

        let mut cases = Vec::new();
        let mut history = fixture_history();
        let mut reducer = ScrollState::new("target".to_owned());
        reducer.reduce(scroll_view(&history), true).unwrap();
        history.query = "needle".to_owned();
        history.navigate_match(true);
        let view = scroll_view(&history);
        let matched = reducer.reduce(view.clone(), true).unwrap();
        let match_id = history.current_match.unwrap();
        assert_eq!(history.page().last().unwrap().id, match_id);
        assert_eq!(matched.destination, ScrollDestination::Row(match_id));
        assert!(reducer.current(&matched));
        cases.push(fixture(
            "match-page-end",
            &history,
            &view,
            &matched,
            "talos-log-view-1001",
            "talos-log-view-1001",
            "row-visible",
        ));

        let mut history = fixture_history();
        let mut reducer = ScrollState::new("target".to_owned());
        let old = reducer.reduce(scroll_view(&history), true).unwrap();
        history.append(events("a", &["INFO newest tail"]));
        let view = scroll_view(&history);
        let newest = reducer.reduce(view.clone(), true).unwrap();
        assert_eq!(newest.destination, ScrollDestination::Latest);
        assert_eq!(history.page().last().unwrap().exact, "INFO newest tail");
        assert!(reducer.current(&newest));
        assert!(!reducer.current(&old));
        cases.push(fixture(
            "follow-newest",
            &history,
            &view,
            &newest,
            "talos-log-view-1002",
            "talos-log-view-1002",
            "bottom",
        ));
        cases.push(fixture(
            "old-render-stamp",
            &history,
            &view,
            &old,
            "talos-log-view-1002",
            "talos-log-view-1002",
            "unchanged",
        ));

        history.set_paused(true);
        history.append(events("a", &["INFO collected while paused"]));
        let paused = scroll_view(&history);
        assert!(reducer.reduce(paused.clone(), true).is_none());
        assert!(!reducer.current(&newest));
        assert_ne!(paused.stamp(), newest.stamp);
        cases.push(fixture(
            "paused",
            &history,
            &paused,
            &newest,
            "talos-log-view-1002",
            "talos-log-view-1002",
            "unchanged",
        ));

        history.set_paused(false);
        history.set_following(false);
        history.append(events("a", &["INFO collected during manual review"]));
        let manual = scroll_view(&history);
        assert!(reducer.reduce(manual.clone(), true).is_none());
        assert!(!reducer.current(&newest));
        assert_ne!(manual.stamp(), newest.stamp);
        cases.push(fixture(
            "manual-review",
            &history,
            &manual,
            &newest,
            "talos-log-view-1002",
            "talos-log-view-1002",
            "unchanged",
        ));

        reducer.retire();
        assert!(!reducer.current(&newest));
        assert!(reducer.reduce(manual, true).is_none());
        let mut replacement = scroll_view(&history);
        replacement.target = "replacement".to_owned();
        // A retired panel has been removed; a new mount has a unique viewport
        // ID. Replaying the old script must not move the replacement pane.
        cases.push(fixture(
            "retired-panel",
            &history,
            &replacement,
            &newest,
            "talos-log-view-1003",
            "talos-log-view-1002",
            "unchanged",
        ));

        if let Some(directory) = std::env::var_os("TALOS_PILOT_UI_SMOKE_DIR") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            let mut manifest = Vec::new();
            for (html, script, case) in cases {
                std::fs::write(directory.join(case["html"].as_str().unwrap()), html).unwrap();
                std::fs::write(directory.join(case["script"].as_str().unwrap()), script).unwrap();
                manifest.push(case);
            }
            std::fs::write(
                directory.join("log-scroll-cases.json"),
                serde_json::to_string_pretty(&manifest).unwrap(),
            )
            .unwrap();
        }
    }

    #[test]
    fn log_rows_use_semantic_severity_classes() {
        for (level, class) in [
            (LogLevel::Error, "log-level-error"),
            (LogLevel::Warning, "log-level-warn"),
            (LogLevel::Info, "log-level-info"),
            (LogLevel::Debug, "log-level-debug"),
            (LogLevel::Unknown, "log-level-unknown"),
        ] {
            assert_eq!(log_level_class(&level), class);
        }
    }

    #[test]
    fn timestamp_merge_filters_and_exact_copy() {
        let mut history = History::new("node".to_owned());
        history.append(events("kubelet", &[" 2026-09-30T12:00:02Z ERROR second  "]));
        history.append(events("apid", &["2026-09-30T12:00:01Z INFO first"]));
        assert_eq!(history.rows[0].entry.service.as_str(), "apid");
        history.filters.levels.info = false;
        assert_eq!(history.visible().len(), 1);
        let id = history.visible()[0].id;
        history.select_row(id, false);
        assert_eq!(
            history.selected_text(),
            " 2026-09-30T12:00:02Z ERROR second  "
        );
        history.filters.services = Some([ServiceId::from("apid")].into_iter().collect());
        assert!(history.visible().is_empty());
    }

    #[test]
    fn search_is_case_insensitive_cyclic_and_does_not_hide_context() {
        let mut history = History::new("node".to_owned());
        history.append(events("a", &["INFO needle", "INFO context", "WARN NEEDLE"]));
        history.query = "NeEdLe".to_owned();
        let matches = history.matches();
        assert_eq!(history.visible().len(), 3);
        assert_eq!(matches.len(), 2);
        history.navigate_match(true);
        assert_eq!(history.current_match, Some(matches[0]));
        history.navigate_match(true);
        assert_eq!(history.current_match, Some(matches[1]));
        history.navigate_match(true);
        assert_eq!(history.current_match, Some(matches[0]));
        history.navigate_match(false);
        assert_eq!(history.current_match, Some(matches[1]));
        assert!(!history.stream.following);
    }

    #[test]
    fn all_retained_history_is_reachable_and_range_spans_pages() {
        let mut history = History::new("node".to_owned());
        history.append(
            (0..750)
                .map(|index| LogEvent::new("a", format!("INFO row {index}")))
                .collect(),
        );
        assert_eq!(history.page().len(), PAGE_SIZE);
        history.move_page(PageAction::First);
        assert_eq!(history.page()[0].exact, "INFO row 0");
        let first = history.page()[0].id;
        history.select_row(first, false);
        history.move_page(PageAction::Next);
        assert_eq!(history.page()[0].exact, "INFO row 100");
        history.select_row(history.page()[0].id, true);
        assert_eq!(history.selected.len(), 101);
        history.move_page(PageAction::Latest);
        assert_eq!(history.page().last().unwrap().exact, "INFO row 749");
        history.move_page(PageAction::Previous);
        assert_eq!(history.page()[0].exact, "INFO row 550");
    }

    #[test]
    fn pause_freezes_presentation_not_collection_and_follow_is_independent() {
        let mut history = History::new("node".to_owned());
        history.append(events("a", &["INFO before"]));
        history.set_paused(true);
        history.append(events("a", &["INFO during pause"]));
        assert_eq!(history.rows.len(), 2);
        assert_eq!(history.page().len(), 1);
        assert!(history.stream.following);
        history.set_paused(false);
        assert_eq!(history.page().len(), 2);
        history.move_page(PageAction::First);
        history.append(events("a", &["INFO after"]));
        assert!(!history.stream.following);
        assert_eq!(history.page_start, 0);
    }

    #[test]
    fn retained_duplicates_keep_stable_identity_and_reconnect_history() {
        let mut history = History::new("node".to_owned());
        history.append(events("a", &["INFO duplicate"]));
        let original = history.rows[0].id;
        history.select_row(original, false);
        // Reconnect does not replace the model; replayed tail is a new record.
        history.append(events("a", &["INFO duplicate", "INFO new"]));
        assert_eq!(history.rows.len(), 3);
        assert_eq!(history.rows[0].id, original);
        assert_ne!(history.rows[1].id, original);
        assert_eq!(history.selected_text(), "INFO duplicate");
    }

    #[test]
    fn row_and_byte_limits_prune_selection_without_latest_only_trap() {
        let mut history = History::new("node".to_owned());
        history.append(events("a", &["INFO selected"]));
        history.select_row(history.rows[0].id, false);
        history.append(
            (0..10_100)
                .map(|index| LogEvent::new("a", format!("INFO {index}")))
                .collect(),
        );
        assert_eq!(history.rows.len(), MAX_LOG_ENTRIES);
        assert!(history.selected.is_empty());
        assert!(history.evicted > 0);
        let large = "x".repeat(64 * 1024);
        history.append(
            (0..200)
                .map(|_| LogEvent::new("a", large.clone()))
                .collect(),
        );
        assert!(
            history
                .rows
                .iter()
                .map(|row| row.exact.len())
                .sum::<usize>()
                <= MAX_HISTORY_BYTES
        );
        history.move_page(PageAction::First);
        assert!(!history.page().is_empty());
    }

    #[test]
    fn stale_batches_and_service_eof_cannot_affect_other_generations() {
        let selected = ["a".to_owned(), "b".to_owned()].into_iter().collect();
        let mut statuses = BTreeMap::from([
            ("a".to_owned(), "Following".to_owned()),
            ("b".to_owned(), "Following".to_owned()),
        ]);
        let stale = Message::Lines(
            1,
            "a".to_owned(),
            vec!["INFO stale target/service set".to_owned()],
        );
        assert!(collect_message(stale, 2, true, &selected, &mut statuses).is_empty());
        let stopped = Message::Lines(2, "a".to_owned(), vec!["INFO after stop".to_owned()]);
        assert!(collect_message(stopped, 2, false, &selected, &mut statuses).is_empty());
        let removed = Message::Lines(2, "removed".to_owned(), vec!["INFO removed".to_owned()]);
        assert!(collect_message(removed, 2, true, &selected, &mut statuses).is_empty());
        let eof = Message::Status(2, "a".to_owned(), "Stream ended".to_owned());
        assert!(collect_message(eof, 2, true, &selected, &mut statuses).is_empty());
        assert_eq!(statuses["b"], "Following");
        let mut history = History::new("node".to_owned());
        let live = Message::Lines(2, "b".to_owned(), vec!["INFO still collecting".to_owned()]);
        history.append(collect_message(live, 2, true, &selected, &mut statuses));
        assert_eq!(history.rows[0].exact, "INFO still collecting");
        // A merge that was already in progress also has to pass the generation guard.
        assert!(!generation_current(2, 3, true));
        assert!(!generation_current(3, 3, false));
        assert!(generation_current(3, 3, true));
    }

    #[test]
    fn paging_visits_every_retained_row_not_only_latest_three_hundred() {
        let mut history = History::new("node".to_owned());
        history.append(
            (0..750)
                .map(|index| LogEvent::new("a", format!("INFO row {index}")))
                .collect(),
        );
        history.move_page(PageAction::First);
        let mut visited = BTreeSet::new();
        loop {
            visited.extend(history.page().iter().map(|row| row.id));
            if history.page_start + PAGE_SIZE >= history.visible().len() {
                break;
            }
            history.move_page(PageAction::Next);
        }
        assert_eq!(visited.len(), 750);
        history.query = "ROW 1".to_owned();
        history.navigate_match(true);
        assert_eq!(history.current_match, Some(history.rows[1].id));
        assert_eq!(history.page_start, 0);
    }

    #[test]
    fn historical_view_and_selection_anchor_survive_out_of_order_merge() {
        let mut history = History::new("node".to_owned());
        history.append(events("a", &["2026-09-30T12:00:02Z INFO later"]));
        history.move_page(PageAction::First);
        let anchor = history.page()[0].id;
        history.select_row(anchor, false);
        history.append(events("b", &["2026-09-30T12:00:01Z INFO earlier"]));
        assert_eq!(history.page()[0].id, anchor);
        assert_eq!(history.selected_text(), "2026-09-30T12:00:02Z INFO later");
    }

    #[test]
    fn duplicate_eviction_does_not_transfer_selection_to_identical_new_line() {
        let mut history = History::new("node".to_owned());
        history.append(events("a", &["INFO identical"]));
        let old = history.rows[0].id;
        history.select_row(old, false);
        history.append(
            (0..MAX_LOG_ENTRIES)
                .map(|_| LogEvent::new("a", "INFO identical"))
                .collect(),
        );
        assert!(history.rows.iter().all(|row| row.id != old));
        assert!(history.selected.is_empty());
    }

    #[test]
    fn paused_page_keeps_exact_evicted_selection_until_presentation_resumes() {
        let mut history = History::new("node".to_owned());
        history.append(events("a", &["  INFO frozen exact text  "]));
        history.select_row(history.rows[0].id, false);
        history.set_paused(true);
        history.append(
            (0..MAX_LOG_ENTRIES)
                .map(|index| LogEvent::new("a", format!("INFO {index}")))
                .collect(),
        );
        assert_eq!(history.page()[0].exact, "  INFO frozen exact text  ");
        assert_eq!(history.selected_text(), "  INFO frozen exact text  ");
        history.set_paused(false);
        assert!(history.selected.is_empty());
        assert!(history.range_anchor.is_none());
    }

    fn saved_fixture(key: &str) -> SavedSession {
        let mut history = History::new("node".to_owned());
        history.append(
            (0..750)
                .map(|index| LogEvent::new("a", format!("INFO row {index}")))
                .collect(),
        );
        history.filters.services = Some([ServiceId::from("a")].into_iter().collect());
        history.filters.levels.debug = false;
        history.query = "ROW 100".to_owned();
        history.navigate_match(true);
        history.select_row(history.rows[100].id, false);
        history.wrap = false;
        history.set_paused(true);
        SavedSession {
            key: key.to_owned(),
            history,
            selected_services: ["a".to_owned()].into_iter().collect(),
            request_signature: (vec!["a".to_owned()], 200),
            collecting: false,
            range_mode: true,
        }
    }

    #[test]
    fn ordinary_tab_unmount_remount_retains_bounded_history_and_presentation_state() {
        let mut registry = SessionRegistry::default();
        registry.retain_target(Some("epoch1/context/node/address"));
        registry.save(saved_fixture("epoch1/context/node/address"));
        // Overview / Services have no log transports. The cache is data-only and
        // holds one bounded same-target session until the next Logs mount.
        registry.retain_target(Some("epoch1/context/node/address"));
        let restored = registry.take("epoch1/context/node/address").unwrap();
        assert_eq!(restored.history.rows.len(), 750);
        assert_eq!(restored.history.page_start, 100);
        assert_eq!(restored.history.query, "ROW 100");
        assert_eq!(restored.history.selected_text(), "INFO row 100");
        assert!(!restored.history.filters.levels.debug);
        assert!(!restored.history.wrap);
        assert!(restored.history.stream.paused);
        assert!(!restored.history.stream.following);
        assert!(!restored.collecting);
        assert!(restored.range_mode);
        assert!(restored.selected_services.contains("a"));
        assert!(registry.saved.is_none());
    }

    #[test]
    fn target_or_configuration_retirement_rejects_old_panel_save() {
        let mut registry = SessionRegistry::default();
        registry.retain_target(Some("epoch1/node-a"));
        registry.save(saved_fixture("epoch1/node-a"));
        registry.retain_target(Some("epoch1/node-b"));
        assert!(registry.saved.is_none());
        // An old component cleanup can race the shell's new target selection.
        registry.save(saved_fixture("epoch1/node-a"));
        assert!(registry.saved.is_none());
        assert!(registry.take("epoch1/node-a").is_none());
        registry.save(saved_fixture("epoch1/node-a"));
        registry.retain_target(None);
        registry.save(saved_fixture("epoch1/node-a"));
        assert!(registry.saved.is_none());
        assert!(registry.take("epoch2/node-a").is_none());
    }

    #[tokio::test]
    async fn dropping_worker_cancels_idle_transport_ownership() {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
        struct DropProbe(Option<tokio::sync::oneshot::Sender<()>>);
        impl Drop for DropProbe {
            fn drop(&mut self) {
                if let Some(tx) = self.0.take() {
                    let _ = tx.send(());
                }
            }
        }
        let worker = Worker(tokio::spawn(async move {
            let _probe = DropProbe(Some(dropped_tx));
            let _ = started_tx.send(());
            futures::future::pending::<()>().await;
        }));
        started_rx.await.unwrap();
        drop(worker);
        tokio::time::timeout(Duration::from_secs(1), dropped_rx)
            .await
            .unwrap()
            .unwrap();
    }
}
