//! The request list: its header, columns and the table's answers. Rows and
//! widths are prepared when an answer or the filter changes; these only
//! read them.
use super::*;
use crate::observability::tables::{ColumnKind, TableCells};
use freshkube_ui::page;
use freshkube_ui::table::{self, DataTable, Line, RowStyle, TableRow};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};

/// A table beside the trace stays bounded in the scrolling page.
const MAX_LINES: usize = 16;
/// Stacked above the trace: about DESIGN.md's 190 dp list.
const STACKED_LINES: usize = 5;
/// The columns Columns can hide; the glyph and name always show.
const OPTIONAL: [ColumnKind; 3] = [
    ColumnKind::Service,
    ColumnKind::Started,
    ColumnKind::Duration,
];
/// Coroot's list stops at its limit; the status bar's tooltip says so.
const LIMIT_NOTE: &str = "Coroot lists up to 100 spans.\nSelect a heatmap cell to narrow them.";

/// The public table contract borrows the page's private row.
pub(crate) struct SpanCells<'a> {
    row: &'a SpanRow,
}

impl ObservabilityPage {
    /// What Traces puts in the status bar.
    pub(in crate::observability) fn traces_read(&self) -> crate::observability::status::Read<'_> {
        let traces = &self.live_traces;
        let count = if self.selected_app.is_none() {
            "No application"
        } else if !self.fixture && self.live.tracing.data().is_none() {
            if self.live.tracing.is_loading() {
                "Loading traces"
            } else {
                "No observation"
            }
        } else {
            &traces.count
        };
        crate::observability::status::Read {
            count,
            stale: self.live.tracing.is_stale(),
            time: self.read_time(self.live.tracing.last_successful()),
            note: traces.limited.then_some(LIMIT_NOTE),
        }
    }

    pub(in crate::observability) fn traces_header(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let header = self.source_header(self.page_header(), cx);
        let traces = &self.live_traces;
        let filter = div().child(
            Input::new(&self.trace_query)
                .id(header.id("filter"))
                .small()
                .h(dp(crate::ui::CONTROL_HEIGHT))
                .cleanable(true)
                .aria_label("Filter requests by name, service or trace id")
                .prefix(Icon::new(IconName::Search).size(dp(14.))),
        );
        let errors =
            matches!(traces.selection, api::TraceSelection::Errors { .. }) && traces.cell.is_none();
        let all = traces.selection == api::TraceSelection::Recent;
        let mut header = header.filter(filter);
        let source_changed = self.trace_source_changed();
        for (kind, name, selected) in &traces.sources {
            let choose = {
                let kind = kind.clone();
                page::handler(cx, move |this: &mut Self, _, cx| {
                    this.choose_trace_source(kind.clone(), cx)
                })
            };
            header = header.foldable(
                action(
                    SharedString::from(format!("obs-trace-source-{kind}")),
                    name.clone(),
                )
                .h(dp(crate::ui::CONTROL_HEIGHT))
                .selected(*selected)
                .on_click({
                    let choose = choose.clone();
                    move |_, window, cx| choose(window, cx)
                }),
                page::Fold::from(page::checked_item(name.clone(), *selected, choose)).changed(
                    (*selected && source_changed).then(|| format!("Source {name}").into()),
                ),
            );
        }
        let show_all = page::handler(cx, |this: &mut Self, _, cx| this.show_all_requests(cx));
        let show_failed = page::handler(cx, |this: &mut Self, _, cx| this.show_failed_requests(cx));
        let columns = self.trace_columns_items(cx);
        self.time_controls(
            header
                .foldable(
                    action("obs-trace-all", "All requests")
                        .h(dp(crate::ui::CONTROL_HEIGHT))
                        .selected(all)
                        .on_click({
                            let show_all = show_all.clone();
                            move |_, window, cx| show_all(window, cx)
                        }),
                    page::checked_item("All requests", all, show_all),
                )
                .foldable(
                    action("obs-trace-failed", "Failed requests")
                        .h(dp(crate::ui::CONTROL_HEIGHT))
                        .selected(errors)
                        .on_click({
                            let show_failed = show_failed.clone();
                            move |_, window, cx| show_failed(window, cx)
                        }),
                    page::Fold::from(page::checked_item("Failed requests", errors, show_failed))
                        .changed(errors.then(|| "Failed requests".into())),
                )
                .foldable(
                    self.trace_columns_menu(columns.clone()),
                    page::columns_fold(
                        columns,
                        self.hidden_trace_columns.len(),
                        self.hidden_trace_columns.is_empty(),
                    ),
                ),
            cx,
        )
        .render(window, cx)
    }

    /// The optional columns as checked items: the Columns menu, and its
    /// folded form.
    fn trace_columns_items(&self, cx: &Context<Self>) -> freshkube_ui::page::MenuItems {
        let owner = cx.entity().downgrade();
        let hidden = self.hidden_trace_columns.clone();
        std::rc::Rc::new(move |mut menu, _, _| {
            for kind in OPTIONAL {
                let owner = owner.clone();
                menu = menu.item(
                    PopupMenuItem::new(label(kind))
                        .checked(!hidden.contains(&kind))
                        .on_click(move |_, _, cx| {
                            _ = owner.update(cx, |this, cx| {
                                let hidden = &mut this.hidden_trace_columns;
                                if !hidden.remove(&kind) {
                                    hidden.insert(kind);
                                }
                                this.prepare_trace_columns();
                                cx.notify();
                            });
                        }),
                );
            }
            menu
        })
    }

    fn trace_columns_menu(&self, items: freshkube_ui::page::MenuItems) -> AnyElement {
        Button::new("obs-columns")
            .outline()
            .small()
            .h(dp(crate::ui::CONTROL_HEIGHT))
            .label("Columns")
            .dropdown_caret(true)
            .dropdown_menu(move |menu, window, cx| items(menu, window, cx))
            .into_any_element()
    }

    /// Typed filter text; the rows are projected again.
    pub(in crate::observability) fn filter_traces(&mut self, query: String) {
        let traces = &mut self.live_traces;
        traces.query = query;
        traces.project();
    }

    fn clear_trace_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.trace_query
            .update(cx, |query, cx| query.set_value("", window, cx));
        self.filter_traces(String::new());
        cx.notify();
    }

    /// Widths fit the answer's values, measured once per answer or Columns.
    pub(in crate::observability) fn prepare_trace_columns(&mut self) {
        let metrics = &self.application_metrics;
        let hidden = &self.hidden_trace_columns;
        let traces = &self.live_traces;
        let columns: Vec<_> = [ColumnKind::Glyph, ColumnKind::Name]
            .into_iter()
            .chain(OPTIONAL)
            .filter(|kind| !hidden.contains(kind))
            .map(|kind| {
                let value = traces
                    .rows
                    .iter()
                    .map(|row| match kind {
                        ColumnKind::Service => metrics.value(row.service.clone(), 12.5),
                        ColumnKind::Started => metrics.value(row.started.clone(), 12.),
                        ColumnKind::Duration => metrics.value(row.duration.clone(), 12.),
                        _ => 0.,
                    })
                    .fold(0., f32::max);
                // Includes both cell insets, with room for fractional shaping.
                let measured = (metrics.caption(label(kind)).max(value) + 24.).ceil();
                PageColumn {
                    kind,
                    label: label(kind).into(),
                    width: match kind {
                        ColumnKind::Glyph => table::GLYPH_WIDTH,
                        ColumnKind::Name => 140.,
                        ColumnKind::Service => measured.clamp(96., 220.),
                        _ => measured.max(56.),
                    },
                }
            })
            .collect();
        let traces = &mut self.live_traces;
        traces.width = columns.iter().map(|c| c.width).sum();
        traces.columns = columns;
    }

    pub(super) fn render_trace_table(
        &self,
        beside: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        DataTable::new()
            .carded()
            .fit(if beside { MAX_LINES } else { STACKED_LINES })
            .render(self, window, cx)
            .id(self.trace_table.id("table"))
            .test_support()
            .w_full()
            .flex_none()
            .into_any_element()
    }

    pub(in crate::observability) fn trace_columns(&self) -> &[PageColumn] {
        &self.live_traces.columns
    }
    pub(in crate::observability) fn trace_width(&self) -> f32 {
        self.live_traces.width
    }
    pub(in crate::observability) fn trace_list_label(&self) -> String {
        "Requests; choose one to see its trace.".into()
    }
    pub(in crate::observability) fn trace_selected_key(&self) -> Option<&TableKey> {
        self.live_traces.selected.as_ref()
    }
    pub(in crate::observability) fn trace_line_of(&self, key: &TableKey) -> Option<usize> {
        let traces = &self.live_traces;
        traces
            .shown
            .iter()
            .position(|&ix| traces.rows[ix].key == *key)
    }
    pub(in crate::observability) fn trace_line_count(&self) -> usize {
        self.live_traces.shown.len()
    }
    pub(in crate::observability) fn trace_line(
        &self,
        line: usize,
    ) -> Option<Line<TableKey, TableCells<'_>>> {
        let traces = &self.live_traces;
        let row = traces.rows.get(*traces.shown.get(line)?)?;
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: row.id.clone().into(),
            label: row.label.clone(),
            tooltip: Some(row.label.clone()),
            marked: false,
            muted: false,
            data: TableCells::Span(SpanCells { row }),
        }))
    }
    pub(in crate::observability) fn span_cell(
        &self,
        cells: &SpanCells<'_>,
        style: &RowStyle,
        column: &PageColumn,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = cells.row;
        let p = style.p;
        let cell = table::cell(column).h_full().flex().items_center();
        let figure = |cell: Div, value: &SharedString| {
            cell.font_family(MONO_FONT)
                .text_size(dp(12.))
                .text_color(p.ink_2)
                .child(text(value.clone()).truncate())
        };
        match column.kind {
            ColumnKind::Glyph => table::glyph_cell(column).children(ui::status_glyph(
                if row.error { Tone::Crit } else { Tone::Good },
                cx,
            )),
            ColumnKind::Name => cell
                .font_family(MONO_FONT)
                .text_size(dp(12.5))
                .child(text(row.name.clone()).truncate()),
            ColumnKind::Service => cell
                .font_family(MONO_FONT)
                .text_size(dp(12.5))
                .text_color(p.ink_2)
                .child(text(row.service.clone()).truncate()),
            ColumnKind::Started => figure(cell, &row.started),
            ColumnKind::Duration => figure(cell, &row.duration),
            _ => cell,
        }
        .into_any_element()
    }
    pub(in crate::observability) fn trace_empty(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let traces = &self.live_traces;
        if !traces.shown.is_empty() {
            return None;
        }
        let empty = if traces.rows.is_empty() {
            ui::empty_state(
                IconName::Inbox,
                "No requests in this selection",
                "Coroot found no spans for this window or cell. Choose All requests, or another cell.",
                None,
                Vec::new(),
                cx,
            )
        } else {
            ui::empty_state(
                IconName::SearchX,
                "No requests match the filter",
                "Clear the filter to see every listed request.",
                None,
                vec![
                    action("obs-traces-clear", "Clear filter")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.clear_trace_filters(window, cx)),
                        )
                        .into_any_element(),
                ],
                cx,
            )
        };
        Some(empty.h_auto().into_any_element())
    }
    pub(in crate::observability) fn trace_notes(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let traces = &self.live_traces;
        let (shown, total) = (traces.shown.len(), traces.rows.len());
        if shown == 0 || shown >= total {
            return Vec::new();
        }
        vec![
            table::showing_bar(
                self.trace_table.id("showing"),
                shown,
                total,
                Button::new("obs-traces-show-all")
                    .ghost()
                    .xsmall()
                    .label(format!("Show all {total}"))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.clear_trace_filters(window, cx)),
                    ),
                cx,
            )
            .into_any_element(),
        ]
    }
}

fn label(kind: ColumnKind) -> &'static str {
    match kind {
        ColumnKind::Name => "Request",
        ColumnKind::Service => "Service",
        ColumnKind::Started => "Started",
        ColumnKind::Duration => "Duration",
        _ => "",
    }
}
