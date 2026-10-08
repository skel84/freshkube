//! The open dashboard: its panels and rows, its variables and time range,
//! and the reads that answer them. A panel is asked once per generation,
//! when it is in or near the viewport; its answer is applied only while
//! that generation is current.
use std::rc::Rc;
use std::sync::Arc;

use freshkube_core::monitoring::{
    ExampleSource, PanelResult, QueryError, Source,
    catalog::{self, TimeSettings},
    model::{Dashboard, PanelSpec, Variable, data::QueryContext, time::TimeWindow},
    prometheus::Variables,
};
use gpui_kit::{AppContext, Context, Entity, SharedString, Subscription, point, px};

use super::connection::Connection;
use super::layout::{self, Layout, SectionShape};
use super::{EntryId, MonitoringEvent, MonitoringPage, Request};
use crate::panel::{Linked, PanelEvent, PanelView};

/// Samples a range is read at: Prometheus's step is the span over these.
const POINTS: usize = 200;

pub(super) struct Slot {
    pub(super) spec: Arc<PanelSpec>,
    pub(super) view: Entity<PanelView>,
    /// The generation last asked for, and the read in flight.
    pub(super) asked: Option<u64>,
    pub(super) request: Option<Request>,
    /// The last answer was an error.
    pub(super) failed: bool,
    /// The id of the empty card drawn in its place while it is out of reach.
    pub(super) placeholder: SharedString,
}

/// A row of the dashboard, as its header shows it.
pub(super) struct RowHeader {
    pub(super) id: SharedString,
    pub(super) title: SharedString,
    /// "3 panels", shown while folded.
    pub(super) count: SharedString,
}

/// A variable's control: its label, the value shown and the choices.
pub(super) struct Control {
    pub(super) index: usize,
    pub(super) id: SharedString,
    pub(super) label: SharedString,
    pub(super) value: SharedString,
    pub(super) options: Rc<[SharedString]>,
}

pub(super) struct Board {
    /// Tells this board's answers from an earlier one's.
    pub(super) id: u64,
    pub(super) entry: EntryId,
    pub(super) title: SharedString,
    /// Why the dashboard can't be shown at all.
    pub(super) error: Option<SharedString>,
    seed: String,
    definitions: Vec<Variable>,
    selections: Vec<String>,
    pub(super) variables: Option<Variables>,
    pub(super) resolving: Option<Request>,
    pub(super) controls: Vec<Control>,
    pub(super) variable_error: Option<SharedString>,
    pub(super) time: TimeSettings,
    pub(super) span: u64,
    pub(super) range_label: SharedString,
    pub(super) ranges: Rc<[(u64, SharedString)]>,
    /// The window of the generation asked last.
    pub(super) window: Option<TimeWindow>,
    pub(super) slots: Vec<Slot>,
    /// One chart's cursor on the others, in `slots`' order.
    pub(super) linked: Linked,
    pub(super) rows: Vec<Option<RowHeader>>,
    shapes: Vec<SectionShape>,
    /// The wide layout and the stacked one.
    pub(super) layouts: [Layout; 2],
    /// The meta line's count, state and time, derived as answers land.
    pub(super) meta: SharedString,
    /// Unix seconds when the last read in flight answered.
    answered_at: Option<i64>,
    _subscriptions: Vec<Subscription>,
}

impl Board {
    /// Drops every read; what was asked is asked again on showing.
    pub(super) fn hide(&mut self) {
        if self.resolving.take().is_some() {
            self.variables = None;
        }
        for slot in &mut self.slots {
            if slot.request.take().is_some() {
                slot.asked = None;
            }
        }
        self.derive_meta();
    }

    /// "12 panels", then "reading" while any read is in flight or how many
    /// failed, then when the last read answered. Returns whether it changed,
    /// so a page notifies only then, not on every answer.
    pub(super) fn derive_meta(&mut self) -> bool {
        crate::probe::hit("monitoring-derive");
        let mut meta = match self.slots.len() {
            1 => "1 panel".to_owned(),
            count => format!("{count} panels"),
        };
        let failed = self.slots.iter().filter(|slot| slot.failed).count();
        if self.slots.iter().any(|slot| slot.request.is_some()) {
            meta.push_str(" · reading");
        } else if failed > 0 {
            meta.push_str(&format!(" · {failed} failed"));
        }
        if let Some(time) = self.answered_at.and_then(clock) {
            meta.push_str(&format!(" · {time}"));
        }
        let changed = self.meta != meta.as_str();
        if changed {
            self.meta = meta.into();
        }
        changed
    }

    pub(super) fn layout(&self, narrow: bool) -> &Layout {
        &self.layouts[usize::from(narrow)]
    }

    pub(super) fn is_collapsed(&self, section: usize) -> bool {
        self.shapes[section].collapsed
    }

    fn relayout(&mut self) {
        self.layouts = [
            layout::layout(&self.shapes, false),
            layout::layout(&self.shapes, true),
        ];
    }

    /// Each shown variable's control, from the resolved values.
    fn derive_controls(&mut self) {
        crate::probe::hit("monitoring-derive");
        let Some(variables) = &self.variables else {
            return;
        };
        self.controls = self
            .definitions
            .iter()
            .enumerate()
            .filter(|(_, definition)| !definition.hidden)
            .map(|(index, definition)| {
                // Resolved options start with "All" where the dashboard
                // allows it.
                let options: Vec<SharedString> = variables
                    .options(index)
                    .iter()
                    .map(|option| SharedString::from(option.clone()))
                    .collect();
                let label = if definition.label.is_empty() {
                    definition.name.clone()
                } else {
                    definition.label.clone()
                };
                let value = variables.selection(index);
                Control {
                    index,
                    id: format!("monitoring-variable-{}", definition.name).into(),
                    label: label.into(),
                    value: if value.is_empty() {
                        "None".into()
                    } else {
                        value.into()
                    },
                    options: options.into(),
                }
            })
            .collect();
    }
}

/// "Last 6 hours" and the like.
pub(super) fn range_label(span: u64) -> SharedString {
    freshkube_core::monitoring::model::time::describe(span).into()
}

impl MonitoringPage {
    /// Opens the chosen dashboard, or the first built-in when it is gone.
    pub(super) fn load_board(&mut self, cx: &mut Context<Self>) {
        if self.catalog.find(&self.chosen).is_none() && matches!(self.chosen, EntryId::Builtin(_)) {
            self.chosen = self.catalog.builtins[0].id.clone();
        }
        let Some(entry) = self.catalog.find(&self.chosen) else {
            // A file of a folder still being read opens once it is.
            return;
        };
        self.generation += 1;
        let id = self.generation;
        let (entry_id, title) = (entry.id.clone(), entry.title.clone());
        let json = entry.json.clone();
        let parsed = json.as_ref().map_err(Clone::clone).and_then(|json| {
            Dashboard::parse_unexpanded(json)
                .map_err(|error| SharedString::from(format!("Not a dashboard: {error}")))
        });
        let time = json
            .as_deref()
            .map(catalog::time_settings)
            .unwrap_or_else(|_| catalog::time_settings("{}"));
        let mut board = Board {
            id,
            entry: entry_id,
            title,
            error: None,
            seed: String::new(),
            definitions: Vec::new(),
            selections: Vec::new(),
            variables: None,
            resolving: None,
            controls: Vec::new(),
            variable_error: None,
            span: time.start,
            range_label: range_label(time.start),
            ranges: time
                .ranges
                .iter()
                .map(|span| (*span, range_label(*span)))
                .collect(),
            time,
            window: None,
            slots: Vec::new(),
            linked: Linked::default(),
            rows: Vec::new(),
            shapes: Vec::new(),
            layouts: Default::default(),
            meta: SharedString::default(),
            answered_at: None,
            _subscriptions: Vec::new(),
        };
        match parsed {
            Ok(dashboard) => self.fill_board(&mut board, dashboard, cx),
            Err(error) => board.error = Some(error),
        }
        board.derive_meta();
        self.refresh_every = board.time.refresh;
        self.board = Some(board);
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.start_refresh(cx);
        self.connect(cx);
    }

    fn fill_board(&mut self, board: &mut Board, dashboard: Dashboard, cx: &mut Context<Self>) {
        board.title = dashboard.title.clone().into();
        board.seed = dashboard
            .uid
            .clone()
            .unwrap_or_else(|| dashboard.title.clone());
        board.definitions = dashboard.variables.clone();
        for (index, section) in dashboard.sections.iter().enumerate() {
            let mut panels = Vec::new();
            for placed in &section.panels {
                let spec = Arc::new(dashboard.panels[placed.key].clone());
                let view = cx.new(|_| {
                    PanelView::new(placed.key.to_string(), Rc::new(spec.as_ref().clone()))
                });
                board
                    ._subscriptions
                    .push(cx.subscribe(&view, Self::on_panel));
                panels.push((board.slots.len(), placed.pos));
                board.slots.push(Slot {
                    spec,
                    view,
                    asked: None,
                    request: None,
                    failed: false,
                    placeholder: format!("monitoring-placeholder-{}", placed.key).into(),
                });
            }
            board.rows.push(section.row.as_ref().map(|row| RowHeader {
                id: format!("monitoring-row-{index}").into(),
                title: row.title.clone().into(),
                count: match panels.len() {
                    1 => "1 panel".into(),
                    count => format!("{count} panels").into(),
                },
            }));
            board.shapes.push(SectionShape {
                has_header: section.row.is_some(),
                collapsed: section.row.as_ref().is_some_and(|row| row.collapsed),
                height: section.height,
                panels,
            });
        }
        board.relayout();
    }

    /// Opens a dashboard from the column.
    pub fn open(&mut self, entry: EntryId, cx: &mut Context<Self>) {
        if self.chosen == entry
            && self
                .board
                .as_ref()
                .is_some_and(|board| board.entry == entry)
        {
            return;
        }
        self.chosen = entry;
        self.board = None;
        if self.visible {
            self.load_board(cx);
        }
        cx.emit(MonitoringEvent::Catalog);
        cx.notify();
    }

    /// Where the page's data comes from now, seeded per dashboard for
    /// example data.
    fn query_source(&self) -> Option<Source> {
        match &self.connection {
            Connection::Example => {
                let seed = self.board.as_ref().map_or("", |board| &board.seed);
                Some(Source::Example(ExampleSource::new(seed)))
            }
            Connection::Ready { prometheus, .. } => Some(Source::Prometheus(prometheus.clone())),
            _ => None,
        }
    }

    fn time_window(&self, span: u64) -> TimeWindow {
        TimeWindow::new((self.now)(), span, POINTS)
    }

    /// With a usable connection: the variables when they aren't resolved,
    /// else what the viewport needs.
    pub(super) fn resume(&mut self, cx: &mut Context<Self>) {
        let Some(board) = &self.board else {
            return;
        };
        if board.error.is_some() || !self.visible {
            return;
        }
        if board.variables.is_none() {
            if board.resolving.is_none() {
                self.resolve_variables(cx);
            }
        } else {
            self.ask_visible(cx);
        }
    }

    /// Resolves the variables in order, each seeing the ones before, then
    /// asks every panel again.
    pub(super) fn resolve_variables(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.query_source() else {
            return;
        };
        let Some(board) = &self.board else {
            return;
        };
        if board.error.is_some() || !self.visible {
            return;
        }
        let (id, definitions, selections) = (
            board.id,
            board.definitions.clone(),
            board.selections.clone(),
        );
        let window = self.time_window(board.span);
        let request = self.run(
            async move {
                source
                    .resolve_variables(&definitions, &selections, window)
                    .await
            },
            cx,
            move |this, result, cx| this.variables_resolved(id, result, cx),
        );
        if let Some(board) = &mut self.board {
            board.resolving = Some(request);
        }
    }

    fn variables_resolved(
        &mut self,
        id: u64,
        result: Result<(Variables, Vec<String>), QueryError>,
        cx: &mut Context<Self>,
    ) {
        let Some(board) = self.board.as_mut().filter(|board| board.id == id) else {
            return;
        };
        board.resolving = None;
        match result {
            Ok((variables, _)) => {
                board.selections = (0..board.definitions.len())
                    .map(|index| variables.selection(index))
                    .collect();
                board.variables = Some(variables);
                board.variable_error = None;
                board.derive_controls();
                self.ask_again(cx);
            }
            Err(error) => {
                board.variable_error = Some(error.message.into());
            }
        }
        cx.notify();
    }

    /// A new generation over a window ending now: every panel is asked
    /// again as it comes into view, keeping its answer until then.
    pub(super) fn ask_again(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let window = match &self.board {
            Some(board) => self.time_window(board.span),
            None => return,
        };
        if let Some(board) = &mut self.board {
            board.window = Some(window);
            for slot in &mut board.slots {
                slot.request = None;
            }
        }
        self.read_markers(window, cx);
        self.ask_visible(cx);
    }

    /// Asks the panels in or near the viewport that this generation hasn't.
    pub(super) fn ask_visible(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        let Some(source) = self.query_source() else {
            return;
        };
        let viewport = self.viewport.get();
        let generation = self.generation;
        let Some(board) = &self.board else {
            return;
        };
        let (Some(variables), Some(window)) = (board.variables.clone(), board.window) else {
            return;
        };
        let (top, bottom) = viewport.reach();
        let wanted: Vec<usize> = board
            .layout(viewport.narrow)
            .within(top, bottom)
            .filter(|slot| board.slots[*slot].asked != Some(generation))
            .collect();
        let context = QueryContext {
            window,
            variables: variables.context_values(),
        };
        for index in wanted {
            let spec = self.board.as_ref().unwrap().slots[index].spec.clone();
            let (source, context, variables) = (source.clone(), context.clone(), variables.clone());
            let request = self.run(
                async move { source.query_panel(&spec, &context, &variables).await },
                cx,
                move |this, result, cx| this.answered(index, generation, window, result, cx),
            );
            let slot = &mut self.board.as_mut().unwrap().slots[index];
            slot.asked = Some(generation);
            slot.request = Some(request);
        }
        if self.board.as_mut().is_some_and(Board::derive_meta) {
            cx.notify();
        }
    }

    fn answered(
        &mut self,
        index: usize,
        generation: u64,
        window: TimeWindow,
        result: Result<PanelResult, QueryError>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        let Some(slot) = self
            .board
            .as_mut()
            .and_then(|board| board.slots.get_mut(index))
        else {
            return;
        };
        slot.request = None;
        slot.failed = result.is_err();
        slot.view.update(cx, |panel, cx| match result {
            Ok(result) => panel.set_result(result, window, cx),
            Err(error) => panel.set_error(&error, cx),
        });
        let now = (self.now)();
        let board = self.board.as_mut().unwrap();
        let moved = board
            .linked
            .refresh(board.slots.iter().map(|slot| &slot.view), cx);
        if board.slots.iter().all(|slot| slot.request.is_none()) {
            board.answered_at = Some(now);
        }
        if board.derive_meta() | moved {
            cx.notify();
        }
    }

    /// Debug fixture checks: answers the variables and every panel from
    /// example data now, so a screenshot taken before the executor runs,
    /// or with GPUI not drawing, shows them.
    #[cfg(any(debug_assertions, feature = "stress"))]
    pub fn answer_example_now(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.connection, Connection::Example) {
            return;
        }
        let Some(board) = &self.board else {
            return;
        };
        let source = ExampleSource::new(&board.seed);
        let window = self.time_window(board.span);
        let Ok((variables, _)) =
            source.resolve_variables(&board.definitions, &board.selections, window)
        else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        let board = self.board.as_mut().unwrap();
        board.resolving = None;
        board.selections = (0..board.definitions.len())
            .map(|index| variables.selection(index))
            .collect();
        board.variables = Some(variables.clone());
        board.window = Some(window);
        board.derive_controls();
        let context = QueryContext {
            window,
            variables: variables.context_values(),
        };
        for slot in &mut board.slots {
            slot.request = None;
            slot.asked = Some(generation);
            let result = source.query_panel(&slot.spec, &context, &variables);
            slot.failed = result.is_err();
            slot.view.update(cx, |panel, cx| match result {
                Ok(result) => panel.set_result(result, window, cx),
                Err(error) => panel.set_error(&error, cx),
            });
        }
        board
            .linked
            .refresh(board.slots.iter().map(|slot| &slot.view), cx);
        board.answered_at = Some((self.now)());
        board.derive_meta();
        self.read_markers(window, cx);
        cx.notify();
    }

    /// One chart's cursor shows on every other timeseries, drawn by the
    /// page so that no other panel redraws.
    fn on_panel(&mut self, from: Entity<PanelView>, event: &PanelEvent, cx: &mut Context<Self>) {
        let PanelEvent::Cursor(time) = event;
        let Some(board) = &mut self.board else {
            return;
        };
        let panels = board.slots.iter().map(|slot| &slot.view);
        board.linked.show(from.entity_id(), *time, panels, cx);
        cx.notify();
    }

    pub(super) fn set_variable(
        &mut self,
        index: usize,
        value: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(board) = &mut self.board else {
            return;
        };
        let Some(selection) = board.selections.get_mut(index) else {
            return;
        };
        if *selection == value.as_ref() {
            return;
        }
        *selection = value.to_string();
        self.cap_again(cx);
        // Variables after this one may depend on it.
        self.resolve_variables(cx);
        cx.notify();
    }

    pub(super) fn set_range(&mut self, span: u64, cx: &mut Context<Self>) {
        let Some(board) = &mut self.board else {
            return;
        };
        if board.span == span {
            return;
        }
        board.span = span;
        board.range_label = range_label(span);
        self.cap_again(cx);
        let Some(board) = &self.board else {
            return;
        };
        if board.variables.is_some() {
            self.ask_again(cx);
        }
        cx.notify();
    }

    /// Charts showing all their series go back to the highest peaks once
    /// the page asks for something else; a refresh keeps them.
    fn cap_again(&self, cx: &mut Context<Self>) {
        let Some(board) = &self.board else {
            return;
        };
        for slot in &board.slots {
            slot.view.update(cx, |panel, _| panel.cap_again());
        }
    }

    pub(super) fn toggle_row(&mut self, section: usize, cx: &mut Context<Self>) {
        let Some(board) = &mut self.board else {
            return;
        };
        let Some(shape) = board.shapes.get_mut(section) else {
            return;
        };
        shape.collapsed = !shape.collapsed;
        board.relayout();
        self.ask_visible(cx);
        cx.notify();
    }
}

/// Unix seconds as the meta line's local time.
pub(super) fn clock(time: i64) -> Option<String> {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(time, 0)
        .single()
        .map(|time| time.format("%H:%M:%S").to_string())
}
