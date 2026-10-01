use std::cell::{Ref, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[cfg(test)]
use freshkube_core::constants::MAX_LOG_ENTRIES;
use freshkube_core::{
    logs::{LogEntry, LogEvent, MultiServiceLogs, ServiceId},
    types::LogLevel,
};

const MAX_SELECTED_LINES: usize = 200;
const MAX_COPY_BYTES: usize = 1024 * 1024;
const MAX_LINE_BYTES: usize = 64 * 1024;
const MAX_RETAINED_BYTES: usize = 8 * 1024 * 1024;

/// How `visible` changed since the panel last measured its rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VisibleDelta {
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
struct LogReview {
    logs: MultiServiceLogs,
    next_id: u64,
    visible: Vec<usize>,
    /// Retained lines per service and level, kept in step with every append
    /// and eviction so rendering never rescans the buffer.
    counts: BTreeMap<ServiceId, [usize; 5]>,
    delta: VisibleDelta,
    matches: RefCell<Option<MatchCache>>,
    selected: BTreeSet<u64>,
    cursor: Option<u64>,
    selection_anchor: Option<u64>,
    query: String,
    current_match: Option<u64>,
    revision: u64,
    omitted: usize,
    evicted: usize,
    selection_limited: bool,
}

impl LogReview {
    fn new(address: &str) -> Self {
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

    fn append(&mut self, events: impl IntoIterator<Item = LogEvent>) {
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
    fn take_delta(&mut self) -> VisibleDelta {
        std::mem::replace(
            &mut self.delta,
            VisibleDelta::Extended {
                dropped: 0,
                added: 0,
            },
        )
    }

    /// Retained lines of a service, whatever the filters hide.
    fn service_count(&self, service: &ServiceId) -> usize {
        self.counts
            .get(service)
            .map_or(0, |counts| counts.iter().sum())
    }

    /// Retained lines per level among the given services.
    fn level_counts<'a>(&self, showing: impl IntoIterator<Item = &'a ServiceId>) -> [usize; 5] {
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
    fn matched_ids(&self) -> Ref<'_, Vec<u64>> {
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

    fn entry(&self, row_ix: usize) -> &LogEntry {
        &self.logs.buffer().entries()[self.visible[row_ix]]
    }

    fn id(&self, row_ix: usize) -> u64 {
        self.logs.buffer().entries()[self.visible[row_ix]].sequence()
    }

    fn row_for_id(&self, id: u64) -> Option<usize> {
        let entries = self.logs.buffer().entries();
        self.visible
            .iter()
            .position(|&ix| entries[ix].sequence() == id)
    }

    fn set_service_filter(&mut self, services: BTreeSet<ServiceId>) {
        let mut filters = self.logs.buffer().filters().clone();
        filters.services = Some(services);
        self.logs.buffer_mut().set_filters(filters);
        self.rebuild_visible();
    }

    fn set_level(&mut self, level: &LogLevel, active: bool) {
        let mut filters = self.logs.buffer().filters().clone();
        filters.levels.set(level, active);
        self.logs.buffer_mut().set_filters(filters);
        self.rebuild_visible();
    }

    fn search(&mut self, forward: bool) -> Option<u64> {
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

    fn match_count(&self) -> usize {
        self.matched_ids().len()
    }

    fn select(&mut self, row_ix: usize, extend: bool, toggle: bool) {
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

    fn move_selection(&mut self, delta: isize, extend: bool) -> Option<u64> {
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
    fn copy_text(&self) -> Result<String, &'static str> {
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

pub(crate) use desktop::LogPanel;

mod desktop {
    use super::VisibleDelta;
    use std::{
        cell::Cell,
        collections::BTreeSet,
        rc::Rc,
        time::{Duration, Instant},
    };

    use gpui_kit::assets::IconName;
    use gpui_kit::{
        AppContext, AvailableSpace, Bounds, ClipboardItem, Context, Entity, FocusHandle,
        FontWeight, KeyBinding, ListSizingBehavior, Pixels, Point, Render, Role, ScrollStrategy,
        SharedString, Size, Subscription, Task, TestSupportExt, Toggled, Window, actions,
        component::{
            ActiveTheme, Disableable, ElementExt, Icon, Selectable, Sizable,
            VirtualListScrollHandle,
            button::{Button, ButtonVariants, Toggle, ToggleVariants},
            h_flex,
            input::{Input, InputEvent, InputState},
            scroll::{ScrollableElement, Scrollbar, ScrollbarHandle, ScrollbarMode},
            tooltip::Tooltip,
            v_flex, v_virtual_list,
        },
        div, point,
        prelude::*,
        px, relative, rems, size,
    };
    use talos_rs::{ServiceInfo, TalosClient};
    use tokio::{runtime::Handle, sync::mpsc};

    use super::{LogEvent, LogLevel, LogReview, MAX_SELECTED_LINES, ServiceId};
    use crate::backend::{self, OwnedJob, StreamEvent, Target};
    use crate::palette::palette;
    use crate::ui;

    actions!(
        talos_logs,
        [
            CopySelected,
            NextLine,
            PreviousLine,
            ExtendNext,
            ExtendPrevious,
            FirstLine,
            LastLine,
            PageNext,
            PagePrevious,
            ClearSelection,
            FindNext,
            FindPrevious
        ]
    );

    #[derive(Clone, PartialEq)]
    struct MeasurementKey {
        width: Pixels,
        rem: Pixels,
        font: SharedString,
        wrapped: bool,
        revision: u64,
    }

    #[derive(Clone, Copy)]
    struct ReviewAnchor {
        id: u64,
        within_row: Pixels,
    }

    /// Only the visible scrollbar gets this adapter. VirtualList and application
    /// requests retain the raw handle, so their offsets are not manual review.
    /// This scalar intent flag owns no entity or async task.
    #[derive(Clone)]
    struct ManualReviewScroll {
        base: VirtualListScrollHandle,
        requested: Rc<Cell<bool>>,
    }

    impl ScrollbarHandle for ManualReviewScroll {
        fn viewport_bounds(&self) -> Bounds<Pixels> {
            ScrollbarHandle::viewport_bounds(&self.base)
        }
        fn offset(&self) -> Point<Pixels> {
            ScrollbarHandle::offset(&self.base)
        }
        fn set_offset(&self, offset: Point<Pixels>) {
            self.requested.set(true);
            ScrollbarHandle::set_offset(&self.base, offset);
        }
        fn content_size(&self) -> Size<Pixels> {
            ScrollbarHandle::content_size(&self.base)
        }
        fn start_drag(&self) {
            self.requested.set(true);
        }
    }

    /// How often stream batches are applied while the page is hidden.
    const HIDDEN_APPLY_INTERVAL: Duration = Duration::from_millis(250);

    pub(crate) struct LogPanel {
        runtime: Handle,
        tail: i32,
        target: Option<(Target, TalosClient)>,
        fixture_target: Option<Target>,
        services: Vec<ServiceId>,
        collecting: BTreeSet<ServiceId>,
        /// Whether the default collection was already offered for this target.
        defaults_applied: bool,
        showing: BTreeSet<ServiceId>,
        review: LogReview,
        generation: u64,
        stream_revision: u64,
        job: Option<OwnedJob>,
        delivery: Option<Task<()>>,
        collection_active: bool,
        following: bool,
        wrapped: bool,
        query: Entity<InputState>,
        _query_subscription: Subscription,
        focus: FocusHandle,
        scroll: VirtualListScrollHandle,
        manual_review: Rc<Cell<bool>>,
        width: Option<Pixels>,
        panel_height: Option<Pixels>,
        measured: Option<MeasurementKey>,
        sizes: Rc<Vec<Size<Pixels>>>,
        /// Natural width of each row in `sizes`, for the unwrapped content width.
        row_widths: Vec<Pixels>,
        row_measurements: std::collections::BTreeMap<u64, Size<Pixels>>,
        unwrapped_width: Pixels,
        pending_reveal: Option<u64>,
        review_anchor: Option<ReviewAnchor>,
        anchor_evicted: bool,
        feedback: Option<String>,
        errors: std::collections::BTreeMap<ServiceId, String>,
        /// Whether the Logs page is on screen. Collection continues either way,
        /// but a hidden panel applies batches in coalesced groups and never
        /// asks the window to redraw for them.
        visible: bool,
        backlog: Vec<StreamEvent>,
        last_applied: Instant,
    }

    impl LogPanel {
        pub(crate) fn new(
            runtime: Handle,
            tail: i32,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) -> Self {
            cx.bind_keys([
                KeyBinding::new("secondary-c", CopySelected, Some("TalosLogs")),
                KeyBinding::new("down", NextLine, Some("TalosLogs")),
                KeyBinding::new("up", PreviousLine, Some("TalosLogs")),
                KeyBinding::new("shift-down", ExtendNext, Some("TalosLogs")),
                KeyBinding::new("shift-up", ExtendPrevious, Some("TalosLogs")),
                KeyBinding::new("home", FirstLine, Some("TalosLogs")),
                KeyBinding::new("end", LastLine, Some("TalosLogs")),
                KeyBinding::new("pagedown", PageNext, Some("TalosLogs")),
                KeyBinding::new("pageup", PagePrevious, Some("TalosLogs")),
                KeyBinding::new("escape", ClearSelection, Some("TalosLogs")),
                KeyBinding::new("f3", FindNext, Some("TalosLogs")),
                KeyBinding::new("shift-f3", FindPrevious, Some("TalosLogs")),
            ]);
            let query =
                cx.new(|cx| InputState::new(window, cx).placeholder("Search retained lines"));
            let subscription =
                cx.subscribe_in(&query, window, |this, _, event, _, cx| match event {
                    InputEvent::Change => {
                        this.review.query = this.query.read(cx).value().to_string();
                        this.review.current_match = None;
                        this.feedback = None;
                        cx.notify();
                    }
                    InputEvent::PressEnter { shift, .. } => this.search(!shift, cx),
                    _ => {}
                });
            Self {
                runtime,
                tail,
                target: None,
                fixture_target: None,
                services: Vec::new(),
                collecting: BTreeSet::new(),
                defaults_applied: false,
                showing: BTreeSet::new(),
                review: LogReview::new(""),
                generation: 0,
                stream_revision: 0,
                job: None,
                delivery: None,
                collection_active: false,
                following: true,
                wrapped: true,
                query,
                _query_subscription: subscription,
                focus: cx.focus_handle().tab_stop(true),
                scroll: VirtualListScrollHandle::new(),
                width: None,
                panel_height: None,
                measured: None,
                sizes: Rc::new(Vec::new()),
                row_widths: Vec::new(),
                pending_reveal: None,
                manual_review: Rc::new(Cell::new(false)),
                row_measurements: std::collections::BTreeMap::new(),
                unwrapped_width: px(0.),
                review_anchor: None,
                anchor_evicted: false,
                feedback: None,
                errors: std::collections::BTreeMap::new(),
                visible: true,
                backlog: Vec::new(),
                last_applied: Instant::now(),
            }
        }

        /// Tells the panel whether its page is on screen. Showing it applies
        /// everything received meanwhile.
        pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
            if self.visible == visible {
                return;
            }
            self.visible = visible;
            if visible {
                self.flush_backlog(cx);
                cx.notify();
            }
        }

        fn flush_backlog(&mut self, cx: &mut Context<Self>) {
            if self.backlog.is_empty() {
                return;
            }
            let batch = std::mem::take(&mut self.backlog);
            if let Some(target) = self.active_target().cloned() {
                self.apply_events(&target, batch, cx);
            }
        }

        pub(crate) fn set_target(
            &mut self,
            target: Option<(Target, TalosClient)>,
            services: Vec<ServiceInfo>,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            self.capture_anchor();
            let changed = self.target.as_ref().map(|(target, _)| target)
                != target.as_ref().map(|(target, _)| target)
                || self.fixture_target.is_some();
            if changed {
                self.stop(cx);
                self.backlog.clear();
                self.fixture_target = None;
                self.generation += 1;
                self.review = LogReview::new(
                    target
                        .as_ref()
                        .map_or("", |(target, _)| target.address.as_str()),
                );
                self.collecting.clear();
                self.defaults_applied = false;
                self.showing.clear();
                self.review_anchor = None;
                self.anchor_evicted = false;
                self.feedback = None;
                self.errors.clear();
                self.following = true;
                self.measured = None;
                self.sizes = Rc::new(Vec::new());
                self.row_widths.clear();
                self.scroll = VirtualListScrollHandle::new();
                self.manual_review = Rc::new(Cell::new(false));
                self.query
                    .update(cx, |query, cx| query.set_value("", window, cx));
            }
            self.target = target;
            let mut catalog: Vec<_> = services
                .into_iter()
                .map(|service| ServiceId::new(service.id))
                .collect();
            catalog.sort();
            catalog.dedup();
            if changed {
                self.showing = catalog.iter().cloned().collect();
            } else {
                self.showing.extend(
                    catalog
                        .iter()
                        .filter(|id| !self.services.contains(id))
                        .cloned(),
                );
            }
            self.services = catalog;
            if self.collecting.is_empty() && !self.collection_active && !self.defaults_applied {
                self.collecting = Self::default_collection(&self.services);
                self.defaults_applied = !self.services.is_empty();
            }
            self.review.set_service_filter(self.showing.clone());
            cx.notify();
        }

        pub(crate) fn open_service(
            &mut self,
            service: String,
            _: &mut Window,
            cx: &mut Context<Self>,
        ) {
            let service = ServiceId::new(service);
            self.flush_backlog(cx);
            if !self.services.contains(&service) {
                self.feedback = Some("Service is not in the selected node's catalog".into());
                cx.notify();
                return;
            }
            self.collecting.clear();
            self.collecting.insert(service.clone());
            self.showing.insert(service);
            self.review.set_service_filter(self.showing.clone());
            if self.collection_active {
                self.start_from_now(cx);
            } else {
                self.start(cx);
            }
        }

        pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
            // Lines received before stopping still belong to the review.
            self.flush_backlog(cx);
            self.stream_revision += 1;
            self.job = None;
            self.delivery = None;
            self.collection_active = false;
            cx.notify();
        }

        pub(crate) fn set_fixture(
            &mut self,
            events: Vec<LogEvent>,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            self.stop(cx);
            self.backlog.clear();
            self.target = None;
            self.generation += 1;
            self.fixture_target = Some(Target {
                epoch: self.generation,
                context: "Synthetic fixture".into(),
                node: "fixture-node".into(),
                address: "fixture.invalid".into(),
            });
            self.services = events
                .iter()
                .map(|event| event.service.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            self.collecting = Self::default_collection(&self.services);
            self.showing = self.services.iter().cloned().collect();
            self.review = LogReview::new("fixture.invalid");
            self.review.append(events);
            self.last_applied = Instant::now();
            self.review.set_service_filter(self.showing.clone());
            self.review_anchor = None;
            self.anchor_evicted = false;
            self.feedback = None;
            self.errors.clear();
            self.following = true;
            self.scroll = VirtualListScrollHandle::new();
            self.manual_review = Rc::new(Cell::new(false));
            self.measured = None;
            self.sizes = Rc::new(Vec::new());
            self.row_widths.clear();
            self.pending_reveal = self
                .review
                .visible
                .len()
                .checked_sub(1)
                .map(|ix| self.review.id(ix));
            self.query
                .update(cx, |query, cx| query.set_value("", window, cx));
            cx.notify();
        }

        /// Delivers one stream batch to the fixture target like a collection
        /// job would, from the first service being collected.
        #[cfg(test)]
        pub(crate) fn push_fixture_batch(
            &mut self,
            lines: Vec<String>,
            cx: &mut Context<Self>,
        ) -> bool {
            let (Some(target), Some(service)) = (
                self.fixture_target.clone(),
                self.collecting.iter().next().cloned(),
            ) else {
                return false;
            };
            let batch = lines
                .into_iter()
                .map(|line| StreamEvent {
                    target: target.clone(),
                    service: service.clone(),
                    result: Ok(line),
                })
                .collect();
            let revision = self.stream_revision;
            self.apply_batch(&target, revision, batch, cx)
        }

        /// Lines applied to the review, and lines held back while hidden.
        #[cfg(test)]
        pub(crate) fn applied_and_held(&self) -> (usize, usize) {
            (
                self.review.logs.buffer().entries().len(),
                self.backlog.len(),
            )
        }

        #[cfg(test)]
        pub(crate) fn set_fixture_failures(
            &mut self,
            failures: Vec<(ServiceId, String)>,
            cx: &mut Context<Self>,
        ) {
            if self.fixture_target.is_some() {
                self.errors = failures.into_iter().take(16).collect();
                cx.notify();
            }
        }

        fn start(&mut self, cx: &mut Context<Self>) {
            let may_replay =
                self.fixture_target.is_none() && !self.review.logs.buffer().entries().is_empty();
            self.start_with_tail(self.tail, cx);
            if may_replay && self.collection_active {
                self.feedback = Some("Collection restarted with the configured tail; previously retained lines may appear again".into());
                cx.notify();
            }
        }

        fn start_from_now(&mut self, cx: &mut Context<Self>) {
            self.start_with_tail(0, cx);
            if self.collection_active {
                self.feedback = Some("Service selection changed; collecting new lines only, without replaying retained tails".into());
                cx.notify();
            }
        }

        fn start_with_tail(&mut self, tail: i32, cx: &mut Context<Self>) {
            self.stop(cx);
            if self.collecting.is_empty() {
                self.feedback = Some("Choose up to 16 services to collect".into());
                cx.notify();
                return;
            }
            let (target, job, receiver) = if let Some(target) = self.fixture_target.clone() {
                let (sender, receiver) = mpsc::channel(256);
                let services: Vec<_> = self.collecting.iter().cloned().collect();
                let event_target = target.clone();
                let initial_sequence = self.review.next_id;
                let job = self.runtime.spawn(async move {
                    let mut tick = tokio::time::interval(Duration::from_millis(250));
                    let mut sequence = initial_sequence;
                    loop {
                        tick.tick().await;
                        let service = services[sequence as usize % services.len()].clone();
                        let line = crate::fixture::stream_line(service.as_str(), sequence);
                        if sender
                            .send(StreamEvent {
                                target: event_target.clone(),
                                service,
                                result: Ok(line),
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                        sequence += 1;
                    }
                });
                (target, OwnedJob::new(job), receiver)
            } else if let Some((target, client)) = &self.target {
                let (job, receiver) = backend::stream(
                    self.runtime.clone(),
                    client.clone(),
                    target.clone(),
                    self.collecting.iter().cloned().collect(),
                    tail,
                );
                (target.clone(), job, receiver)
            } else {
                self.feedback = Some("Select a connected node first".into());
                cx.notify();
                return;
            };
            self.feedback = None;
            self.errors.clear();
            self.collection_active = true;
            self.job = Some(job);
            self.receive(target, receiver, cx);
            cx.notify();
        }

        fn receive(
            &mut self,
            target: Target,
            mut receiver: mpsc::Receiver<StreamEvent>,
            cx: &mut Context<Self>,
        ) {
            let revision = self.stream_revision;
            self.delivery = Some(cx.spawn(async move |weak, cx| {
                while let Some(first) = receiver.recv().await {
                    let mut batch = vec![first];
                    // At most 64 lines per turn; yield between turns even when
                    // a busy service keeps the bounded channel continuously full.
                    for _ in 1..64 {
                        let Ok(event) = receiver.try_recv() else {
                            break;
                        };
                        batch.push(event);
                    }
                    let valid = weak
                        .update(cx, |this, cx| {
                            this.apply_batch(&target, revision, batch, cx)
                        })
                        .unwrap_or(false);
                    if !valid {
                        return;
                    }
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                }
                let _ = weak.update(cx, |this, cx| {
                    if this.stream_revision == revision && this.active_target() == Some(&target) {
                        this.flush_backlog(cx);
                        this.collection_active = false;
                        this.job = None;
                        cx.notify();
                    }
                });
            }));
        }

        fn apply_batch(
            &mut self,
            target: &Target,
            revision: u64,
            batch: Vec<StreamEvent>,
            cx: &mut Context<Self>,
        ) -> bool {
            if self.stream_revision != revision || self.active_target() != Some(target) {
                return false;
            }
            if !self.visible {
                // Nothing is drawn, so don't spend main-thread time per
                // batch: hold lines and apply them in coalesced groups.
                self.backlog.extend(batch);
                if self.last_applied.elapsed() < HIDDEN_APPLY_INTERVAL {
                    return true;
                }
                let batch = std::mem::take(&mut self.backlog);
                self.apply_events(target, batch, cx);
                return true;
            }
            self.apply_events(target, batch, cx);
            true
        }

        fn apply_events(
            &mut self,
            target: &Target,
            batch: Vec<StreamEvent>,
            cx: &mut Context<Self>,
        ) {
            self.last_applied = Instant::now();
            self.apply_manual_review(cx);
            self.capture_anchor();
            let mut lines = Vec::new();
            for event in batch {
                if &event.target != target || !self.collecting.contains(&event.service) {
                    continue;
                }
                match event.result {
                    Ok(line) => lines.push(LogEvent::new(event.service, line)),
                    Err(error) => {
                        self.errors.insert(event.service, error);
                    }
                }
            }
            self.review.append(lines);
            if self.following {
                self.pending_reveal = self
                    .review
                    .visible
                    .len()
                    .checked_sub(1)
                    .map(|ix| self.review.id(ix));
            }
            // A hidden panel isn't drawn and the shell shows nothing a batch
            // changes, so only a visible one asks for a redraw.
            if self.visible {
                cx.notify();
            }
        }

        fn active_target(&self) -> Option<&Target> {
            self.fixture_target
                .as_ref()
                .or_else(|| self.target.as_ref().map(|(target, _)| target))
        }

        fn capture_anchor(&mut self) {
            if self.following
                || self.review.visible.is_empty()
                || self.sizes.len() != self.review.visible.len()
            {
                return;
            }
            let mut offset = -self.scroll.offset().y;
            for (ix, row) in self.sizes.iter().enumerate() {
                if offset < row.height || ix + 1 == self.sizes.len() {
                    self.review_anchor = Some(ReviewAnchor {
                        id: self.review.id(ix),
                        within_row: offset.max(px(0.)),
                    });
                    return;
                }
                offset -= row.height;
            }
        }

        fn restore_anchor(&mut self) {
            let Some(anchor) = self.review_anchor else {
                return;
            };
            let row_ix = self.review.row_for_id(anchor.id);
            if let Some(row_ix) = row_ix {
                let before: Pixels = self.sizes[..row_ix].iter().map(|row| row.height).sum();
                let within = anchor.within_row.min(self.sizes[row_ix].height);
                self.scroll
                    .set_offset(point(self.scroll.offset().x, -(before + within)));
            } else {
                self.anchor_evicted = true;
                self.scroll
                    .set_offset(point(self.scroll.offset().x, px(0.)));
                self.review_anchor = self.review.visible.first().map(|_| ReviewAnchor {
                    id: self.review.id(0),
                    within_row: px(0.),
                });
            }
        }

        fn set_following(&mut self, following: bool, cx: &mut Context<Self>) {
            self.following = following;
            self.review.logs.buffer_mut().set_following(following);
            self.review.logs.buffer_mut().set_paused(!following);
            if following {
                self.review_anchor = None;
                self.anchor_evicted = false;
                self.pending_reveal = self
                    .review
                    .visible
                    .len()
                    .checked_sub(1)
                    .map(|ix| self.review.id(ix));
            } else {
                self.capture_anchor();
            }
            cx.notify();
        }

        fn apply_manual_review(&mut self, cx: &mut Context<Self>) {
            if self.manual_review.replace(false) {
                self.set_following(false, cx);
            }
        }

        fn search(&mut self, forward: bool, cx: &mut Context<Self>) {
            self.set_following(false, cx);
            self.pending_reveal = self.review.search(forward);
            self.review_anchor = None;
            self.feedback = if self.pending_reveal.is_none() && !self.review.query.is_empty() {
                Some("No matching retained lines in the current filters".into())
            } else {
                None
            };
            cx.notify();
        }

        fn copy(&mut self, cx: &mut Context<Self>) {
            match self.review.copy_text() {
                Ok(text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    self.feedback = Some("Copied selected complete visible lines".into());
                }
                Err(error) => self.feedback = Some(error.into()),
            }
            cx.notify();
        }

        fn navigate(&mut self, delta: isize, extend: bool, cx: &mut Context<Self>) {
            self.set_following(false, cx);
            self.pending_reveal = self.review.move_selection(delta, extend);
            self.review_anchor = None;
            cx.notify();
        }

        fn toggle_collection(&mut self, service: ServiceId, checked: bool, cx: &mut Context<Self>) {
            self.flush_backlog(cx);
            if checked && self.collecting.len() >= 16 {
                self.feedback = Some("Collect at most 16 services concurrently".into());
            } else {
                if checked {
                    self.collecting.insert(service);
                } else {
                    self.collecting.remove(&service);
                }
                if self.collection_active {
                    self.start_from_now(cx);
                }
            }
            cx.notify();
        }

        /// Collection defaults when a node's catalog first becomes known.
        fn default_collection(services: &[ServiceId]) -> BTreeSet<ServiceId> {
            let preferred: BTreeSet<ServiceId> = ["apid", "kubelet", "etcd"]
                .into_iter()
                .map(ServiceId::from)
                .filter(|service| services.contains(service))
                .collect();
            if preferred.is_empty() {
                services.iter().take(3).cloned().collect()
            } else {
                preferred
            }
        }

        /// Replaces the synthetic backlog with the given node's full catalog.
        pub(crate) fn set_fixture_catalog(
            &mut self,
            events: Vec<LogEvent>,
            catalog: &[ServiceInfo],
            node: &str,
            address: &str,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            self.set_fixture(events, window, cx);
            if let Some(target) = &mut self.fixture_target {
                target.node = node.into();
                target.address = address.into();
            }
            let mut services: BTreeSet<ServiceId> = self.services.iter().cloned().collect();
            services.extend(
                catalog
                    .iter()
                    .map(|service| ServiceId::new(service.id.clone())),
            );
            self.services = services.into_iter().collect();
            self.collecting = Self::default_collection(&self.services);
            self.showing = self.services.iter().cloned().collect();
            self.review.set_service_filter(self.showing.clone());
            cx.notify();
        }

        /// Services currently streaming; zero while collection is stopped.
        pub(crate) fn collecting_count(&self) -> usize {
            if self.collection_active {
                self.collecting.len()
            } else {
                0
            }
        }

        pub(crate) fn is_collecting(&self) -> bool {
            self.collection_active
        }

        /// One-line summary for the window status bar.
        pub(crate) fn status_line(&self) -> String {
            let mut parts = vec![
                if self.collection_active {
                    if self.collecting.len() == 1 {
                        "Collecting 1 service".to_owned()
                    } else {
                        format!("Collecting {} services", self.collecting.len())
                    }
                } else {
                    "Collection stopped".to_owned()
                },
                format!(
                    "{} visible / {} retained",
                    self.review.visible.len(),
                    self.review.logs.buffer().entries().len()
                ),
            ];
            if !self.review.query.is_empty() {
                let count = self.review.match_count();
                parts.push(if count == 1 {
                    "1 match".into()
                } else {
                    format!("{count} matches")
                });
            }
            parts.push(format!("{} selected", self.review.selected.len()));
            parts.push(if self.following {
                "Following".into()
            } else if self.collection_active {
                "Paused, collection continues".into()
            } else {
                "Paused".into()
            });
            if self.review.evicted > 0 || self.review.omitted > 0 {
                parts.push(format!(
                    "{} oldest lines evicted, {} over 64 KiB omitted",
                    self.review.evicted, self.review.omitted
                ));
            }
            if self.anchor_evicted {
                parts.push("Review position was evicted; showing the earliest line".into());
            }
            if self.review.selection_limited {
                parts.push(format!("Selection limited to {MAX_SELECTED_LINES} lines"));
            }
            parts.join(" · ")
        }

        fn render_row(
            &self,
            row_ix: usize,
            measuring: bool,
            cx: &mut Context<Self>,
        ) -> impl IntoElement + use<> {
            let entry = self.review.entry(row_ix);
            let id = self.review.id(row_ix);
            let selected = self.review.selected.contains(&id);
            let matched = !self.review.query.is_empty() && entry.matches_query(&self.review.query);
            let current = self.review.current_match == Some(id);
            let p = palette(cx);
            let (level, level_color, stripe) = match entry.level {
                LogLevel::Error => ("ERROR", p.crit_ink, p.crit),
                LogLevel::Warning => ("WARN", p.warn_ink, p.warn),
                LogLevel::Info => ("INFO", p.muted, ui::transparent()),
                LogLevel::Debug => ("DEBUG", p.faint, ui::transparent()),
                LogLevel::Unknown => ("—", p.faint, ui::transparent()),
            };
            let time = entry
                .timestamp
                .as_ref()
                .map(|time| time.display.clone())
                .unwrap_or_else(|| "—".into());
            let message = if entry.message.trim().is_empty() {
                entry.selectable_text().to_owned()
            } else {
                entry.message.clone()
            };
            let label = format!(
                "{time} {} {level} {}",
                entry.service.as_str(),
                entry.selectable_text()
            );
            let wrapped = self.wrapped;
            h_flex()
                .id(SharedString::from(format!(
                    "log-line-{}-{id}",
                    self.generation
                )))
                .test_support()
                .role(Role::ListBoxOption)
                .aria_label(label)
                .aria_selected(selected)
                .items_start()
                .w_full()
                .gap(rems(0.85))
                .pl(rems(0.7))
                .pr(rems(1.))
                .py(rems(0.14))
                .border_l_2()
                .border_color(stripe)
                .font_family(ui::MONO_FONT)
                // Rem-relative so rows follow the theme's font size.
                .text_size(rems(0.86))
                .line_height(relative(1.5))
                .text_color(p.ink)
                .when(!wrapped && measuring, |element| element.w_auto())
                .when(!wrapped && !measuring, |element| {
                    element.min_w(self.unwrapped_width)
                })
                .when(selected, |element| element.bg(p.accent_soft))
                .when(!selected && current, |element| element.bg(p.mark))
                .when(!selected && matched && !current, |element| {
                    element.bg(p.mark.opacity(0.35))
                })
                .when(!selected && !matched && !current, |element| {
                    element.hover(|style| style.bg(p.hover))
                })
                .child(
                    div()
                        .flex_none()
                        .w(rems(4.7))
                        .text_color(p.muted)
                        .child(time),
                )
                .child(
                    div()
                        .flex_none()
                        .w(rems(6.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_color(p.ink_2)
                        .child(entry.service.as_str().to_owned()),
                )
                .child(
                    div()
                        .flex_none()
                        .w(rems(3.2))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(level_color)
                        .child(level),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .when(!wrapped, |element| element.whitespace_nowrap())
                        .when(wrapped, |element| element.whitespace_normal())
                        .child(message),
                )
                .on_click(
                    cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                        // Resolve the stable ID again: a stream batch may have
                        // reordered or evicted the rendered row before this click.
                        if let Some(row_ix) = this.review.row_for_id(id) {
                            this.set_following(false, cx);
                            this.review.select(
                                row_ix,
                                event.modifiers().shift,
                                event.modifiers().secondary(),
                            );
                            this.focus.focus(window, cx);
                            cx.notify();
                        }
                    }),
                )
        }
        fn measure_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            let key = MeasurementKey {
                width: self.width.unwrap_or_else(|| window.bounds().size.width),
                rem: window.rem_size(),
                font: cx.theme().mono_font_family.clone(),
                wrapped: self.wrapped,
                revision: self.review.revision,
            };
            if self.measured.as_ref() == Some(&key) {
                return;
            }
            let geometry_changed = self.measured.as_ref().is_none_or(|previous| {
                previous.width != key.width
                    || previous.rem != key.rem
                    || previous.font != key.font
                    || previous.wrapped != key.wrapped
            });
            if geometry_changed {
                self.row_measurements.clear();
            }
            let delta = self.review.take_delta();
            // Measurements of evicted lines are dead weight; sweep them only
            // once they outnumber what could be live.
            if self.row_measurements.len() > self.review.logs.buffer().entries().len() {
                let retained: BTreeSet<_> = self
                    .review
                    .logs
                    .buffer()
                    .entries()
                    .iter()
                    .map(|entry| entry.sequence())
                    .collect();
                self.row_measurements.retain(|id, _| retained.contains(id));
            }
            let available = size(
                if self.wrapped {
                    AvailableSpace::Definite(key.width)
                } else {
                    AvailableSpace::MaxContent
                },
                AvailableSpace::MinContent,
            );
            // Rows that left the front and ones that joined the end are the
            // only changes between batches; the sizes in between still hold.
            let rows = self.review.visible.len();
            let kept = match delta {
                VisibleDelta::Extended { dropped, added }
                    if !geometry_changed
                        && dropped <= self.sizes.len()
                        && self.sizes.len() - dropped + added == rows
                        && self.row_widths.len() == self.sizes.len() =>
                {
                    Some((dropped, rows - added))
                }
                _ => None,
            };
            let first_new = match kept {
                Some((dropped, first_new)) => {
                    Rc::make_mut(&mut self.sizes).drain(..dropped);
                    self.row_widths.drain(..dropped);
                    first_new
                }
                None => {
                    self.sizes = Rc::new(Vec::with_capacity(rows));
                    self.row_widths.clear();
                    0
                }
            };
            // VirtualList trusts supplied heights; measure the actual styled
            // row, not character counts. Cache by domain ID and resolved width,
            // font, rem and wrap mode; only newly arrived rows need reshaping.
            for ix in first_new..rows {
                let id = self.review.id(ix);
                let measured = if let Some(measured) = self.row_measurements.get(&id) {
                    *measured
                } else {
                    let mut row = self.render_row(ix, true, cx).into_any_element();
                    let measured = row.layout_as_root(available, window, cx);
                    self.row_measurements.insert(id, measured);
                    measured
                };
                self.row_widths.push(measured.width);
                Rc::make_mut(&mut self.sizes).push(size(key.width, measured.height));
            }
            self.unwrapped_width = self
                .row_widths
                .iter()
                .fold(key.width, |widest, width| widest.max(*width));
            self.measured = Some(key);
            if !self.following && self.pending_reveal.is_none() {
                self.restore_anchor();
            }
            if self.following {
                self.pending_reveal = self
                    .review
                    .visible
                    .len()
                    .checked_sub(1)
                    .map(|ix| self.review.id(ix));
            }
        }

        fn render_catalog_content(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
            let p = palette(cx);
            h_flex()
                .flex_wrap()
                .gap(px(6.))
                .children(self.services.iter().map(|service| {
                    let collect_service = service.clone();
                    let show_service = service.clone();
                    let collecting = self.collecting.contains(service);
                    let showing = self.showing.contains(service);
                    let count = self.review.service_count(service);
                    let full = !collecting && self.collecting.len() >= 16;
                    h_flex()
                        .h(px(26.))
                        .rounded_full()
                        .border_1()
                        .border_color(if collecting {
                            p.accent_line
                        } else {
                            p.line_strong
                        })
                        .bg(if collecting { p.accent_soft } else { p.surface })
                        .overflow_hidden()
                        .child(
                            h_flex()
                                .id(SharedString::from(format!("collect-{}", service.as_str())))
                                .test_support()
                                .role(Role::CheckBox)
                                .aria_toggled(if collecting {
                                    Toggled::True
                                } else {
                                    Toggled::False
                                })
                                .aria_label(format!("Collect {}", service.as_str()))
                                .tab_index(0)
                                .h_full()
                                .pl(px(10.))
                                .pr(px(if collecting || count > 0 { 4. } else { 10. }))
                                .gap(px(5.))
                                .when(!full, |this| this.cursor_pointer())
                                .when(full, |this| this.opacity(0.5))
                                .font_family(ui::MONO_FONT)
                                .text_size(px(12.))
                                .text_color(if collecting { p.ink } else { p.muted })
                                .when(collecting, |this| {
                                    this.child(
                                        Icon::new(IconName::Check)
                                            .with_size(px(13.))
                                            .text_color(p.accent),
                                    )
                                })
                                .child(
                                    div()
                                        .when(!showing, |this| {
                                            this.line_through().text_color(p.faint)
                                        })
                                        .child(service.as_str().to_owned()),
                                )
                                .when(count > 0, |this| {
                                    this.child(
                                        div()
                                            .text_size(px(10.5))
                                            .text_color(p.muted)
                                            .child(count.to_string()),
                                    )
                                })
                                .when(!full, |this| {
                                    this.on_click(cx.listener(move |this, _, _, cx| {
                                        let checked = !this.collecting.contains(&collect_service);
                                        this.toggle_collection(collect_service.clone(), checked, cx)
                                    }))
                                }),
                        )
                        .when(collecting || count > 0, |this| {
                            this.child(
                                h_flex()
                                    .id(SharedString::from(format!("show-{}", service.as_str())))
                                    .test_support()
                                    .role(Role::CheckBox)
                                    .aria_toggled(if showing {
                                        Toggled::True
                                    } else {
                                        Toggled::False
                                    })
                                    .aria_label(format!(
                                        "{} {} lines",
                                        if showing { "Hide" } else { "Show" },
                                        service.as_str()
                                    ))
                                    .tab_index(0)
                                    .h_full()
                                    .pl(px(4.))
                                    .pr(px(9.))
                                    .cursor_pointer()
                                    .child(
                                        Icon::new(if showing {
                                            IconName::Eye
                                        } else {
                                            IconName::EyeOff
                                        })
                                        .with_size(px(13.))
                                        .text_color(p.muted),
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.capture_anchor();
                                        if this.showing.contains(&show_service) {
                                            this.showing.remove(&show_service);
                                        } else {
                                            this.showing.insert(show_service.clone());
                                        }
                                        this.review.set_service_filter(this.showing.clone());
                                        cx.notify();
                                    })),
                            )
                        })
                }))
        }

        fn render_levels(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
            let p = palette(cx);
            let counts = self.review.level_counts(&self.showing);
            h_flex().gap_1().flex_wrap().children(
                [
                    ("error", "Error", LogLevel::Error, Some(p.crit)),
                    ("warning", "Warn", LogLevel::Warning, Some(p.warn)),
                    ("info", "Info", LogLevel::Info, None),
                    ("debug", "Debug", LogLevel::Debug, None),
                    ("unknown", "Unknown", LogLevel::Unknown, None),
                ]
                .into_iter()
                .enumerate()
                .map(|(ix, (id, label, level, dot))| {
                    let active = self.review.logs.buffer().filters().levels.accepts(&level);
                    Toggle::new(SharedString::from(format!("level-{id}")))
                        .outline()
                        .small()
                        .checked(active)
                        .tooltip(format!("Show {label} lines"))
                        .child(
                            h_flex()
                                .gap(px(5.))
                                .px(px(3.))
                                .when_some(dot, |this, color| {
                                    this.child(div().size(px(7.)).rounded_full().bg(color))
                                })
                                .child(label)
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(p.muted)
                                        .child(counts[ix].to_string()),
                                ),
                        )
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            this.capture_anchor();
                            this.review.set_level(&level, *checked);
                            cx.notify();
                        }))
                }),
            )
        }

        fn render_notices(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
            let p = palette(cx);
            v_flex()
                .gap_1()
                .text_size(px(12.))
                .line_height(px(18.))
                .when_some(self.feedback.clone(), |element, feedback| {
                    element.child(
                        div()
                            .id("logs-summary-text")
                            .test_support()
                            .text_color(p.muted)
                            .child(feedback),
                    )
                })
                .children(self.errors.iter().map(|(service, error)| {
                    h_flex()
                        .items_start()
                        .gap_2()
                        .text_color(p.crit_ink)
                        .child(Icon::new(IconName::CircleX).with_size(px(14.)).mt(px(2.)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(format!("{}: {error}", service.as_str())),
                        )
                }))
        }

        fn has_notices(&self) -> bool {
            self.feedback.is_some() || !self.errors.is_empty()
        }

        fn render_toolbar_content(
            &self,
            catalog_height: Pixels,
            cx: &mut Context<Self>,
        ) -> gpui_kit::Div {
            let p = palette(cx);
            let (node, address) = self
                .active_target()
                .map(|target| (target.node.clone(), target.address.clone()))
                .unwrap_or_else(|| ("no node".into(), String::new()));
            let match_count = self.review.match_count();
            let current_position = self.review.current_match.and_then(|id| {
                self.review
                    .matched_ids()
                    .iter()
                    .position(|&matched| matched == id)
            });
            let selected = self.review.selected.len();
            v_flex()
                .gap(px(12.))
                .pb(px(12.))
                .child(
                    h_flex()
                        .items_end()
                        .gap_3()
                        .flex_wrap()
                        .child(
                            v_flex()
                                .gap(px(7.))
                                .child(
                                    h_flex()
                                        .gap_2p5()
                                        .child(
                                            div()
                                                .font_family(ui::DISPLAY_FONT)
                                                .text_size(px(28.))
                                                .line_height(px(32.))
                                                .child("Logs"),
                                        )
                                        .child(if self.collection_active {
                                            ui::tag(ui::Tone::Good, None, "Collecting", cx)
                                        } else {
                                            ui::tag(ui::Tone::Unknown, Some(IconName::Pause), "Stopped", cx)
                                        }),
                                )
                                .child(
                                    h_flex()
                                        .gap_1p5()
                                        .text_size(px(12.5))
                                        .text_color(p.muted)
                                        .child("on")
                                        .child(
                                            div()
                                                .font_family(ui::MONO_FONT)
                                                .text_size(px(12.))
                                                .child(node),
                                        )
                                        .when(!address.is_empty(), |this| {
                                            this.child("·").child(
                                                div()
                                                    .font_family(ui::MONO_FONT)
                                                    .text_size(px(12.))
                                                    .child(address),
                                            )
                                        }),
                                ),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("logs-collection")
                                .small()
                                .map(|button| {
                                    if self.collection_active {
                                        button.outline()
                                    } else {
                                        button.primary()
                                    }
                                })
                                .icon(if self.collection_active {
                                    IconName::Square
                                } else {
                                    IconName::Play
                                })
                                .label(if self.collection_active {
                                    "Stop collecting"
                                } else {
                                    "Start collecting"
                                })
                                .disabled(
                                    self.active_target().is_none()
                                        || (!self.collection_active && self.collecting.is_empty()),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if this.collection_active {
                                        this.stop(cx);
                                    } else {
                                        this.start(cx);
                                    }
                                })),
                        ),
                )
                .child(
                    h_flex()
                        .items_start()
                        .gap_2()
                        .child(
                            div()
                                .id("logs-services-label")
                                .pt(px(6.))
                                .tooltip(|window, cx| {
                                    Tooltip::new("Collect up to 16 services. The eye hides a service's lines without stopping collection.")
                                        .build(window, cx)
                                })
                                .child(ui::caption("Services", cx)),
                        )
                        .child(
                            div()
                                .id("logs-services")
                                .role(Role::Group)
                                .aria_label("Services to collect and show")
                                .flex_1()
                                .min_w_0()
                                .h(catalog_height)
                                .min_h_0()
                                .child(
                                    self.render_catalog_content(cx)
                                        .h_full()
                                        .min_h_0()
                                        .overflow_y_scrollbar()
                                        .id("logs-services-scroll"),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_2()
                        .child(self.render_levels(cx))
                        .child(
                            h_flex()
                                .gap_1()
                                .flex_1()
                                .min_w(px(220.))
                                .child(
                                    div().flex_1().min_w_0().child(
                                        Input::new(&self.query)
                                            .id("logs-search")
                                            .aria_label("Search retained log lines")
                                            .small()
                                            .prefix(Icon::new(IconName::Search).with_size(px(14.))),
                                    ),
                                )
                                .when(!self.review.query.is_empty(), |this| {
                                    this.child(
                                        div()
                                            .flex_none()
                                            .text_size(px(11.))
                                            .text_color(p.muted)
                                            .child(match (match_count, current_position) {
                                                (0, _) => "No matches".to_owned(),
                                                (count, Some(ix)) => format!("{} of {count}", ix + 1),
                                                (count, None) => format!("– of {count}"),
                                            }),
                                    )
                                })
                                .child(
                                    Button::new("logs-search-prev")
                                        .ghost()
                                        .small()
                                        .icon(IconName::ChevronUp)
                                        .accessibility_label("Previous match")
                                        .tooltip("Previous match (Shift Enter)")
                                        .disabled(self.review.query.is_empty())
                                        .on_click(cx.listener(|this, _, _, cx| this.search(false, cx))),
                                )
                                .child(
                                    Button::new("logs-search-next")
                                        .ghost()
                                        .small()
                                        .icon(IconName::ChevronDown)
                                        .accessibility_label("Next match")
                                        .tooltip("Next match (Enter)")
                                        .disabled(self.review.query.is_empty())
                                        .on_click(cx.listener(|this, _, _, cx| this.search(true, cx))),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .child(
                                    Button::new("logs-wrap")
                                        .outline()
                                        .small()
                                        .icon(IconName::TextWrap)
                                        .toggled(self.wrapped)
                                        .selected(self.wrapped)
                                        .accessibility_label("Wrap lines")
                                        .tooltip("Wrap long lines")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.capture_anchor();
                                            this.wrapped = !this.wrapped;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("logs-follow")
                                        .outline()
                                        .small()
                                        .w(px(104.))
                                        .toggled(self.following)
                                        .selected(self.following)
                                        .icon(if self.following {
                                            IconName::ArrowDownToLine
                                        } else {
                                            IconName::Pause
                                        })
                                        .label(if self.following { "Following" } else { "Paused" })
                                        .tooltip(if self.following {
                                            "Pause to review. Collection keeps running."
                                        } else {
                                            "Jump to the newest line and keep following"
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.set_following(!this.following, cx)
                                        })),
                                )
                                .child(
                                    Button::new("logs-copy")
                                        .outline()
                                        .small()
                                        .icon(IconName::Copy)
                                        .label(if selected > 0 {
                                            format!("Copy {selected}")
                                        } else {
                                            "Copy".into()
                                        })
                                        .tooltip("Copy selected lines")
                                        .disabled(self.review.copy_text().is_err())
                                        .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
                                ),
                        ),
                )
        }
    }

    impl Render for LogPanel {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            crate::desktop::probe::hit("logs");
            self.apply_manual_review(cx);
            self.measure_rows(window, cx);
            if self.following {
                self.pending_reveal = None;
                let height: Pixels = self.sizes.iter().map(|row| row.height).sum();
                // Request beyond the tail; VirtualList prepaint clamps against
                // its *inner* viewport. Nearest would expose the top, not the
                // latest text, when the last wrapped row exceeds the viewport.
                self.scroll
                    .set_offset(point(self.scroll.offset().x, -height));
            } else if let Some(id) = self.pending_reveal.take()
                && let Some(ix) = self.review.row_for_id(id)
            {
                self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
            }
            let p = palette(cx);
            let empty = if self.active_target().is_none() {
                "Select a connected node to view its logs."
            } else if self.services.is_empty() {
                "This node didn't report a service catalog."
            } else if self.review.logs.buffer().entries().is_empty() {
                "Choose services above, then start collecting."
            } else {
                "No retained lines pass the service and level filters."
            };
            let entity = cx.entity().downgrade();
            let root_entity = cx.entity().downgrade();
            // Budget the pane's own allocation, not the native window before
            // shell chrome. The root allocation is independent of these caps.
            let panel_height = self.panel_height.unwrap_or(window.bounds().size.height);
            let viewport_min = (window.rem_size() * 6.).min(panel_height * 0.5);
            let chrome_budget = (panel_height - viewport_min).max(px(0.));
            let panel_width = self
                .width
                .map_or(window.bounds().size.width, |width| width + px(2.));
            let notices_cap = (chrome_budget * 0.25).min(chrome_budget);
            let notices_height = if self.has_notices() {
                let mut notices = self.render_notices(cx).into_any_element();
                let measured = notices.layout_as_root(
                    size(
                        AvailableSpace::Definite(panel_width),
                        AvailableSpace::MinContent,
                    ),
                    window,
                    cx,
                );
                (measured.height + px(1.)).min(notices_cap)
            } else {
                px(0.)
            };
            let toolbar_cap = chrome_budget - notices_height;
            let mut catalog_content = self.render_catalog_content(cx).into_any_element();
            let catalog_size = catalog_content.layout_as_root(
                size(
                    AvailableSpace::Definite((panel_width - px(90.)).max(px(0.))),
                    AvailableSpace::MinContent,
                ),
                window,
                cx,
            );
            let catalog_height = catalog_size.height.min(px(26. * 2. + 6.));
            let mut toolbar_content = self
                .render_toolbar_content(catalog_height, cx)
                .into_any_element();
            let toolbar_size = toolbar_content.layout_as_root(
                size(
                    AvailableSpace::Definite(panel_width),
                    AvailableSpace::MinContent,
                ),
                window,
                cx,
            );
            // Every Scrollable area has a definite measured owner. Percentage
            // scroll-area wrappers cannot establish an auto-height ancestor.
            let toolbar_height = (toolbar_size.height + px(1.)).min(toolbar_cap);
            let manual_scroll = ManualReviewScroll {
                base: self.scroll.clone(),
                requested: self.manual_review.clone(),
            };
            let viewport = div().id("logs-viewport").role(Role::ListBox)
                .aria_label("Retained log lines; arrows select, Shift arrows extend, Command or Control C copies selected complete lines")
                .test_support()
                .track_focus(&self.focus).key_context("TalosLogs")
                .relative().flex_1().min_h(viewport_min).min_w_0().overflow_hidden()
                .bg(p.surface)
                .rounded_t(px(10.))
                .border_1().border_color(p.line)
                .focus_visible(|style| style.border_color(cx.theme().ring))
                .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                    this.set_following(false, cx);
                }))
                .on_action(cx.listener(|this, _: &CopySelected, _, cx| this.copy(cx)))
                .on_action(cx.listener(|this, _: &NextLine, _, cx| this.navigate(1, false, cx)))
                .on_action(cx.listener(|this, _: &PreviousLine, _, cx| this.navigate(-1, false, cx)))
                .on_action(cx.listener(|this, _: &ExtendNext, _, cx| this.navigate(1, true, cx)))
                .on_action(cx.listener(|this, _: &ExtendPrevious, _, cx| this.navigate(-1, true, cx)))
                .on_action(cx.listener(|this, _: &PageNext, _, cx| this.navigate(20, false, cx)))
                .on_action(cx.listener(|this, _: &PagePrevious, _, cx| this.navigate(-20, false, cx)))
                .on_action(cx.listener(|this, _: &FirstLine, _, cx| this.navigate(isize::MIN, false, cx)))
                .on_action(cx.listener(|this, _: &LastLine, _, cx| this.navigate(isize::MAX, false, cx)))
                .on_action(cx.listener(|this, _: &FindNext, _, cx| this.search(true, cx)))
                .on_action(cx.listener(|this, _: &FindPrevious, _, cx| this.search(false, cx)))
                .on_action(cx.listener(|this, _: &ClearSelection, _, cx| {
                    this.review.selected.clear(); this.review.selection_anchor = None; cx.notify();
                }))
                .on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(|this, _, window, cx| this.focus.focus(window, cx)))
                .on_prepaint(move |bounds, window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        // The permanent one-pixel border belongs to this
                        // viewport, so rows measure its actual inner width.
                        let width = (bounds.size.width - px(2.)).max(px(0.));
                        if this.width != Some(width) {
                            this.capture_anchor();
                            this.width = Some(width);
                            cx.notify();
                        }
                        if this.measured.as_ref().is_some_and(|key| key.rem != window.rem_size()) {
                            this.capture_anchor();
                            this.measured = None;
                            cx.notify();
                        }
                    });
                })
                .when(self.review.visible.is_empty(), |element| element.child(div().p_4().text_size(px(12.5)).text_color(p.muted).child(empty)))
                .when(!self.review.visible.is_empty(), |element| {
                    element.child(v_virtual_list(cx.entity(), ("log-list", self.generation), self.sizes.clone(), |this, range, _, cx| {
                        range.map(|ix| this.render_row(ix, false, cx)).collect::<Vec<_>>()
                    }).with_sizing_behavior(ListSizingBehavior::Auto).track_scroll(&self.scroll))
                    .child(Scrollbar::vertical(&manual_scroll).id("logs-scrollbar").mode(ScrollbarMode::Always))
                    .when(!self.wrapped, |element| element.child(Scrollbar::horizontal(&manual_scroll)))
                });
            div()
                .id("logs-panel")
                .role(Role::Group)
                .aria_label("Live logs panel")
                .test_support()
                .flex()
                .flex_col()
                .size_full()
                .min_w_0()
                .min_h_0()
                .on_prepaint(move |bounds, _, cx| {
                    let _ = root_entity.update(cx, |this, cx| {
                        if this.panel_height != Some(bounds.size.height) {
                            this.panel_height = Some(bounds.size.height);
                            cx.notify();
                        }
                    });
                })
                .text_color(cx.theme().foreground)
                .child(
                    div()
                        .id("logs-toolbar")
                        .role(Role::Group)
                        .aria_label("Log collection, filters and search")
                        .test_support()
                        .flex()
                        .flex_col()
                        .h(toolbar_height)
                        .min_h_0()
                        .max_h(toolbar_cap)
                        .child(
                            self.render_toolbar_content(catalog_height, cx)
                                .h_full()
                                .min_h_0()
                                .overflow_y_scrollbar()
                                .id("logs-toolbar-scroll"),
                        ),
                )
                .when(self.has_notices(), |this| {
                    this.child(
                        div()
                            .id("logs-notices")
                            .role(Role::Status)
                            .test_support()
                            .flex()
                            .flex_col()
                            .h(notices_height)
                            .mb_2()
                            .child(
                                self.render_notices(cx)
                                    .h_full()
                                    .min_h_0()
                                    .overflow_y_scrollbar()
                                    .id("logs-notices-scroll"),
                            ),
                    )
                })
                .child(viewport)
        }
    }

    #[cfg(test)]
    mod ui_tests {
        use gpui_kit::{
            AppContext, Entity, ScrollDelta, SharedString, TestAppContext, WindowHandle,
            component::{Root, Theme},
            point, px, size,
            test::{TestAppContextExt, TestWindowExt},
        };
        use tokio::runtime::{Builder, Runtime};

        use super::{LogEvent, LogPanel, ServiceId, StreamEvent};

        fn fixture_events() -> Vec<LogEvent> {
            (0..120)
                .map(|ix| {
                    let detail = if ix == 10 {
                        "error needle first 東京".to_owned()
                    } else if ix == 90 {
                        format!(
                            "error needle last {}",
                            "wrapped Unicode Δ 東京 🚀 ".repeat(24)
                        )
                    } else {
                        format!("info ordinary complete line {ix}")
                    };
                    LogEvent::new(if ix % 2 == 0 { "apid" } else { "kubelet" }, detail)
                })
                .collect()
        }

        fn mount(cx: &mut TestAppContext) -> (Runtime, Entity<LogPanel>, WindowHandle<Root>) {
            let runtime = Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .unwrap();
            cx.update(gpui_kit::init);
            let mut panel = None;
            let handle = cx.open_window(size(px(620.), px(760.)), |window, cx| {
                let view = cx.new(|cx| {
                    let mut view = LogPanel::new(runtime.handle().clone(), 100, window, cx);
                    view.set_fixture(fixture_events(), window, cx);
                    view
                });
                panel = Some(view.clone());
                Root::new(view, window, cx)
            });
            (runtime, panel.unwrap(), handle)
        }

        #[gpui_kit::test]
        fn searches_reveal_inner_rows_and_keyboard_copies_complete_selected_lines(
            cx: &mut TestAppContext,
        ) {
            let (_runtime, panel, handle) = mount(cx);
            let row = SharedString::from("log-line-1-10");
            cx.update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                let toolbar = window.find("logs-toolbar").bounds();
                assert!(toolbar.size.height > window.rem_size());
                let viewport = window.find("logs-viewport").bounds();
                let search = window.find("logs-search").bounds();
                assert!(search.bottom() <= viewport.top());
                assert!(viewport.top() - search.bottom() <= window.rem_size() * 2.);
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("unchanged".into()));
                let feedback = panel.read(cx).feedback.clone();
                window.click("logs-copy", cx);
                assert_eq!(
                    cx.read_from_clipboard()
                        .and_then(|item| item.text())
                        .as_deref(),
                    Some("unchanged")
                );
                assert_eq!(panel.read(cx).feedback, feedback);
                assert!(window.try_find(row.clone()).is_none());
                window.click("logs-search", cx);
                window.input("needle", cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, cx| {
                let toolbar = window.find("logs-search").bounds();
                window.click("logs-search-next", cx);
                let viewport = window.find("logs-viewport").bounds();
                let line = window.find(row.clone());
                assert!(line.visible());
                assert!(line.bounds().top() >= viewport.top());
                assert!(line.bounds().bottom() <= viewport.bottom());
                assert_eq!(window.find("logs-search").bounds(), toolbar);
                assert_eq!(panel.read(cx).review.visible.len(), 120);
                assert!(!panel.read(cx).following);
                assert_eq!(panel.read(cx).review.current_match, Some(10));
                window.click(row, cx);
                assert_eq!(panel.read(cx).review.selected.len(), 1);
                assert!(panel.read(cx).focus.is_focused(window));
                window.press("shift-down", cx);
                assert_eq!(panel.read(cx).review.selected.len(), 2);
                window.press("secondary-c", cx);
                let text = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .unwrap();
                assert_eq!(
                    text,
                    "error needle first 東京\ninfo ordinary complete line 11"
                );
                assert!(!text.contains("[apid]"));
                window.press("escape", cx);
                assert!(panel.read(cx).review.selected.is_empty());
                window.click("logs-search-next", cx);
                assert_eq!(panel.read(cx).review.current_match, Some(90));
                assert!(window.find(SharedString::from("log-line-1-90")).visible());
                window.click("logs-search-prev", cx);
                assert_eq!(panel.read(cx).review.current_match, Some(10));
            })
            .unwrap();
        }

        #[gpui_kit::test]
        fn wrap_and_rem_remeasure_real_rows_and_wheel_pauses_follow(cx: &mut TestAppContext) {
            let (_runtime, panel, handle) = mount(cx);
            cx.update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                let original = panel.read(cx).sizes[90].height;
                assert!(original > panel.read(cx).sizes[89].height);
                let toolbar = window.find("logs-follow").bounds();
                let before = panel.read(cx).scroll.offset();
                let viewport = window.find("logs-viewport").bounds();
                // Use the toolkit's visible native thumb, not the model seam.
                window.drag(
                    point(viewport.right() - px(8.), viewport.bottom() - px(24.)),
                    point(viewport.right() - px(8.), viewport.top() + px(40.)),
                    cx,
                );
                assert!(!panel.read(cx).following);
                assert_ne!(panel.read(cx).scroll.offset(), before);
                window.click("logs-follow", cx);
                assert!(panel.read(cx).following);
                window.scroll(
                    "logs-viewport",
                    ScrollDelta::Pixels(point(px(0.), px(900.))),
                    cx,
                );
                assert!(!panel.read(cx).following);
                assert_ne!(panel.read(cx).scroll.offset(), before);
                assert_eq!(window.find("logs-follow").bounds(), toolbar);
                window.click("logs-wrap", cx);
                assert!(!panel.read(cx).wrapped);
                let unwrapped = panel.read(cx).sizes[90].height;
                assert!(unwrapped < original);
                assert_eq!(unwrapped, panel.read(cx).sizes[89].height);
                window.click("logs-wrap", cx);
                assert_eq!(panel.read(cx).sizes[90].height, original);
                Theme::update(cx, |theme| theme.font_size = px(20.));
                window.render_frame(cx);
                window.render_frame(cx);
                assert!(panel.read(cx).sizes[90].height > original);
                window.click("logs-follow", cx);
                assert!(panel.read(cx).following);
                assert!(window.find(SharedString::from("log-line-1-119")).visible());
                assert_eq!(window.find("logs-wrap").checked(), Some(true));
                panel.update(cx, |view, cx| {
                    let target = view.fixture_target.clone().unwrap();
                    assert!(view.apply_batch(
                        &target,
                        view.stream_revision,
                        vec![StreamEvent {
                            target: target.clone(),
                            service: ServiceId::from("apid"),
                            result: Ok(format!(
                                "info tall final row {}",
                                "Unicode 東京 latest text ".repeat(500)
                            )),
                        }],
                        cx
                    ));
                });
                window.render_frame(cx);
                let viewport = window.find("logs-viewport").bounds();
                let last = window.find(SharedString::from("log-line-1-120")).bounds();
                assert!(last.size.height > viewport.size.height);
                assert!(last.bottom() <= viewport.bottom());
                assert!(last.bottom() >= viewport.bottom() - px(2.));
                let total: gpui_kit::Pixels =
                    panel.read(cx).sizes.iter().map(|row| row.height).sum();
                assert_eq!(
                    panel.read(cx).scroll.offset().y,
                    viewport.size.height - px(2.) - total
                );
            })
            .unwrap();
        }

        #[gpui_kit::test]
        fn paused_anchor_survives_arrival_then_labels_eviction_and_rejects_stale_delivery(
            cx: &mut TestAppContext,
        ) {
            let (_runtime, panel, handle) = mount(cx);
            cx.update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                window.scroll(
                    "logs-viewport",
                    ScrollDelta::Pixels(point(px(0.), px(600.))),
                    cx,
                );
                let offset = panel.read(cx).scroll.offset();
                let (target, revision) = {
                    let view = panel.read(cx);
                    (view.fixture_target.clone().unwrap(), view.stream_revision)
                };
                panel.update(cx, |view, cx| {
                    assert!(view.apply_batch(
                        &target,
                        revision,
                        vec![StreamEvent {
                            target: target.clone(),
                            service: ServiceId::from("apid"),
                            result: Ok("info newly received line".into()),
                        }],
                        cx
                    ));
                });
                window.render_frame(cx);
                assert_eq!(panel.read(cx).scroll.offset(), offset);
                assert!(!panel.read(cx).anchor_evicted);
                panel.update(cx, |view, cx| {
                    let batch = (0..freshkube_core::constants::MAX_LOG_ENTRIES)
                        .map(|ix| StreamEvent {
                            target: target.clone(),
                            service: ServiceId::from("apid"),
                            result: Ok(format!("info eviction {ix}")),
                        })
                        .collect();
                    assert!(view.apply_batch(&target, revision, batch, cx));
                });
                window.render_frame(cx);
                assert!(panel.read(cx).anchor_evicted);
                assert!(panel.read(cx).status_line().contains("evicted"));
                panel.update(cx, |view, cx| view.set_target(None, Vec::new(), window, cx));
                window.render_frame(cx);
                assert!(panel.read(cx).review.visible.is_empty());
                panel.update(cx, |view, cx| {
                    assert!(!view.apply_batch(
                        &target,
                        revision,
                        vec![StreamEvent {
                            target: target.clone(),
                            service: ServiceId::from("apid"),
                            result: Ok("error stale".into()),
                        }],
                        cx
                    ));
                });
                assert!(panel.read(cx).review.visible.is_empty());
            })
            .unwrap();
        }

        #[gpui_kit::test]
        async fn synthetic_collection_start_stop_and_filters_use_real_controls(
            cx: &mut TestAppContext,
        ) {
            // Tokio's producer uses real time. Let GPUI park for external wakes
            // rather than racing through its deterministic test-clock timers.
            cx.executor().allow_parking();
            let (_runtime, panel, handle) = mount(cx);
            let (collected, initial_next_id) = cx
                .update_window(handle.into(), |_, window, cx| {
                    window.render_frame(cx);
                    window.render_frame(cx);
                    let view = panel.read(cx);
                    assert!(
                        view.review
                            .logs
                            .buffer()
                            .entries()
                            .iter()
                            .all(|entry| { !entry.raw.contains("seq=") })
                    );
                    let collected = view.collecting.clone();
                    assert_eq!(collected.len(), 2);
                    let initial_next_id = view.review.next_id;
                    window.click("logs-collection", cx);
                    assert!(panel.read(cx).collection_active);
                    assert!(panel.read(cx).job.is_some());
                    assert!(panel.read(cx).delivery.is_some());
                    (collected, initial_next_id)
                })
                .unwrap();
            cx.wait_for(handle.into(), std::time::Duration::from_secs(2), |_, cx| {
                let view = panel.read(cx);
                collected.iter().all(|service| {
                    view.review
                        .logs
                        .buffer()
                        .entries()
                        .iter()
                        .any(|entry| &entry.service == service && entry.raw.contains("seq="))
                })
            })
            .await;

            let paused_counts = cx
                .update_window(handle.into(), |_, window, cx| {
                    window.render_frame(cx);
                    assert!(panel.read(cx).review.next_id > initial_next_id);
                    assert!(panel.read(cx).following);
                    window.click("logs-follow", cx);
                    assert!(!panel.read(cx).following);
                    assert!(panel.read(cx).collection_active);
                    window.click("show-apid", cx);
                    let view = panel.read(cx);
                    assert!(!view.showing.contains(&ServiceId::from("apid")));
                    assert_eq!(view.collecting, collected);
                    assert!(view.showing.contains(&ServiceId::from("kubelet")));
                    assert!(view.review.visible.iter().any(|&ix| {
                        view.review.logs.buffer().entries()[ix].raw.contains("seq=")
                    }));
                    assert!(view.review.visible.iter().all(|&ix| {
                        view.review.logs.buffer().entries()[ix].service.as_str() == "kubelet"
                    }));
                    collected
                        .iter()
                        .map(|service| {
                            let count = view
                                .review
                                .logs
                                .buffer()
                                .entries()
                                .iter()
                                .filter(|entry| {
                                    &entry.service == service && entry.raw.contains("seq=")
                                })
                                .count();
                            (service.clone(), count)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap();
            cx.wait_for(handle.into(), std::time::Duration::from_secs(2), |_, cx| {
                let view = panel.read(cx);
                assert!(!view.following);
                assert!(view.collection_active);
                paused_counts.iter().all(|(service, previous_count)| {
                    view.review
                        .logs
                        .buffer()
                        .entries()
                        .iter()
                        .filter(|entry| &entry.service == service && entry.raw.contains("seq="))
                        .count()
                        > *previous_count
                })
            })
            .await;

            let stopped_next_id = cx
                .update_window(handle.into(), |_, window, cx| {
                    window.click("logs-collection", cx);
                    let view = panel.read(cx);
                    assert!(!view.collection_active);
                    assert!(view.job.is_none());
                    assert!(view.delivery.is_none());
                    assert!(!view.following);
                    view.review.next_id
                })
                .unwrap();
            let stopped_at = std::time::Instant::now();
            cx.wait_for(
                handle.into(),
                std::time::Duration::from_secs(2),
                |window, cx| {
                    let view = panel.read(cx);
                    // Use a monotonic arrival identity, not bounded buffer size,
                    // and observe six real producer ticks with GPUI still driven.
                    assert_eq!(view.review.next_id, stopped_next_id);
                    assert!(!view.collection_active);
                    assert!(view.job.is_none());
                    assert!(view.delivery.is_none());
                    assert_eq!(
                        window.find("logs-collection").label(),
                        Some("Start collecting")
                    );
                    stopped_at.elapsed() >= std::time::Duration::from_millis(120)
                },
            )
            .await;
            cx.update_window(handle.into(), |_, window, cx| {
                window.click("logs-follow", cx);
                assert!(panel.read(cx).following);
                assert!(!panel.read(cx).collection_active);
                assert_eq!(panel.read(cx).review.next_id, stopped_next_id);
            })
            .unwrap();
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
