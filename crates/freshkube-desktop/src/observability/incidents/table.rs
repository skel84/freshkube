//! The incident list: its header, columns and the table's answers. Rows,
//! widths and the chips' counts are prepared when the list or a filter
//! changes; these only read them.
use super::*;
use crate::observability::tables::TableCells;
use freshkube_ui::table::{self, DataTable, Line, RowStyle, TableRow};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};

/// A table beside the detail stays bounded in the scrolling page.
const MAX_LINES: usize = 16;
/// Stacked above the detail: about DESIGN.md's 190 dp list.
const STACKED_LINES: usize = 5;
/// The columns Columns can hide; the glyph, key and title always show.
const OPTIONAL: [ColumnKind; 4] = [
    ColumnKind::App,
    ColumnKind::Opened,
    ColumnKind::Duration,
    ColumnKind::Impact,
];
/// The list ignores the time range; the meta line's tooltip says so.
const NOTE: &str =
    "Up to 100 recent incidents, in every state.\nThe time range doesn't filter them.";

/// The public table contract borrows the page's private row.
pub(crate) struct IncidentCells<'a> {
    row: &'a Row,
}

impl ObservabilityPage {
    pub(in crate::observability) fn incidents_header(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let header = self.source_header(self.page_header(window), cx);
        let state = &self.incident_observations;
        let chips = table::status_chips(
            "obs-incident-tallies",
            [
                (api::IncidentState::Open, Tone::Crit, "open"),
                (api::IncidentState::Resolved, Tone::Good, "resolved"),
            ]
            .into_iter()
            .enumerate()
            .map(|(ix, (choice, tone, what))| {
                table::status_chip(
                    SharedString::from(format!("obs-incidents-{what}")),
                    tone,
                    state.counts[ix],
                    what,
                    state.state_filter == Some(choice),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let state = &mut this.incident_observations;
                    state.state_filter = (state.state_filter != Some(choice)).then_some(choice);
                    state.project();
                    cx.notify();
                }))
            }),
            cx,
        );
        let filter = div().child(
            Input::new(&self.incident_query)
                .id(header.id("filter"))
                .small()
                .cleanable(true)
                .aria_label("Filter incidents by key, title or application")
                .prefix(Icon::new(IconName::Search).size(dp(14.))),
        );
        let sample = if !self.fixture && self.live.incidents.data().is_none() {
            if self.live.incidents.is_loading() {
                "Loading incidents".into()
            } else {
                "No observation".into()
            }
        } else {
            state.sample.clone()
        };
        let mut meta = vec![
            " · ".into_any_element(),
            div()
                .id("obs-incidents-sample")
                .child(sample)
                .tooltip(|window, cx| Tooltip::new(NOTE).build(window, cx))
                .into_any_element(),
        ];
        if self.live.incidents.is_stale() {
            meta.push(" · stale".into_any_element());
        }
        if let Some(time) = self
            .live
            .incidents
            .last_successful()
            .or_else(|| self.live.range.to.filter(|_| self.fixture).map(Into::into))
        {
            meta.extend([" · ".into_any_element(), ui::clock(time).into_any_element()]);
        }
        self.time_controls(
            header
                .filter(filter)
                .chips(Some(chips))
                .meta(meta)
                .control(self.incident_columns_menu(cx)),
            cx,
        )
        .render(cx)
    }

    fn incident_columns_menu(&self, cx: &Context<Self>) -> AnyElement {
        let owner = cx.entity().downgrade();
        let hidden = self.hidden_incident_columns.clone();
        Button::new("obs-columns")
            .outline()
            .small()
            .label("Columns")
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for kind in OPTIONAL {
                    let owner = owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label(kind))
                            .checked(!hidden.contains(&kind))
                            .on_click(move |_, _, cx| {
                                _ = owner.update(cx, |this, cx| {
                                    let hidden = &mut this.hidden_incident_columns;
                                    if !hidden.remove(&kind) {
                                        hidden.insert(kind);
                                    }
                                    this.prepare_incident_columns();
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    /// Typed filter text; the rows are projected again.
    pub(in crate::observability) fn filter_incidents(&mut self, query: String) {
        let state = &mut self.incident_observations;
        state.query = query;
        state.project();
    }

    fn clear_incident_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.incident_observations.state_filter = None;
        self.incident_query
            .update(cx, |query, cx| query.set_value("", window, cx));
        self.filter_incidents(String::new());
        cx.notify();
    }

    /// Widths fit the sample's values, measured once per list or Columns.
    pub(in crate::observability) fn prepare_incident_columns(&mut self) {
        let metrics = &self.application_metrics;
        let hidden = &self.hidden_incident_columns;
        let state = &self.incident_observations;
        let columns: Vec<_> = [ColumnKind::Glyph, ColumnKind::Key, ColumnKind::Title]
            .into_iter()
            .chain(OPTIONAL)
            .filter(|kind| !hidden.contains(kind))
            .map(|kind| {
                let value = state
                    .rows
                    .iter()
                    .map(|row| match kind {
                        ColumnKind::Key => metrics.value(row.incident.clone(), 12.5),
                        ColumnKind::App => metrics.value(row.app_label.clone(), 12.5),
                        ColumnKind::Opened => metrics.value(row.opened.clone(), 12.),
                        ColumnKind::Duration => metrics.value(row.duration.clone(), 12.),
                        ColumnKind::Impact => metrics.value(row.impact.clone(), 12.),
                        _ => 0.,
                    })
                    .fold(0., f32::max);
                // Includes both cell insets, with room for fractional shaping.
                let measured = (metrics.caption(label(kind)).max(value) + 24.).ceil();
                PageColumn {
                    kind,
                    label: label(kind).into(),
                    width: match kind {
                        ColumnKind::Glyph => 34.,
                        ColumnKind::Title => 120.,
                        ColumnKind::App => measured.clamp(96., 240.),
                        _ => measured.max(56.),
                    },
                }
            })
            .collect();
        let state = &mut self.incident_observations;
        state.width = columns.iter().map(|c| c.width).sum();
        state.columns = columns;
    }

    pub(super) fn render_incident_table(
        &self,
        beside: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        DataTable::new()
            .carded()
            .fit(if beside { MAX_LINES } else { STACKED_LINES })
            .render(self, window, cx)
            .id(self.incident_table.id("table"))
            .test_support()
            .w_full()
            .flex_none()
            .into_any_element()
    }

    pub(in crate::observability) fn incident_columns(&self) -> &[PageColumn] {
        &self.incident_observations.columns
    }
    pub(in crate::observability) fn incident_width(&self) -> f32 {
        self.incident_observations.width
    }
    pub(in crate::observability) fn incident_list_label(&self) -> String {
        format!("Incidents; choose one to read its details. {NOTE}")
    }
    pub(in crate::observability) fn incident_selected_key(&self) -> Option<&TableKey> {
        self.incident_observations.selected.as_ref()
    }
    pub(in crate::observability) fn incident_line_of(&self, key: &TableKey) -> Option<usize> {
        let state = &self.incident_observations;
        state
            .shown
            .iter()
            .position(|&ix| state.rows[ix].key == *key)
    }
    pub(in crate::observability) fn incident_line_count(&self) -> usize {
        self.incident_observations.shown.len()
    }
    pub(in crate::observability) fn incident_line(
        &self,
        line: usize,
    ) -> Option<Line<TableKey, TableCells<'_>>> {
        let state = &self.incident_observations;
        let row = state.rows.get(*state.shown.get(line)?)?;
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: row.id.clone().into(),
            label: row.label.clone(),
            tooltip: Some(row.label.clone()),
            marked: false,
            muted: row.state == api::IncidentState::Resolved,
            data: TableCells::Incident(IncidentCells { row }),
        }))
    }
    pub(in crate::observability) fn incident_cell(
        &self,
        cells: &IncidentCells<'_>,
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
            ColumnKind::Glyph => cell.children(ui::status_glyph(row.severity.tone(), cx)),
            ColumnKind::Key => cell
                .font_family(MONO_FONT)
                .text_size(dp(12.5))
                .child(text(row.incident.clone()).truncate()),
            // Prose, in the interface face; ids and times stay mono.
            ColumnKind::Title => cell
                .font_family(gpui_kit::component::Theme::global(cx).font_family.clone())
                .child(text(row.title.clone()).truncate()),
            ColumnKind::App => cell
                .font_family(MONO_FONT)
                .text_size(dp(12.5))
                .text_color(p.ink_2)
                .child(text(row.app_label.clone()).truncate()),
            ColumnKind::Opened => figure(cell, &row.opened),
            ColumnKind::Duration => figure(cell, &row.duration),
            ColumnKind::Impact => figure(cell, &row.impact),
            _ => cell,
        }
        .into_any_element()
    }
    pub(in crate::observability) fn incident_empty(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = &self.incident_observations;
        if !state.shown.is_empty() {
            return None;
        }
        let empty = if state.rows.is_empty() {
            ui::empty_state(
                IconName::ShieldCheck,
                "No incidents in the latest sample",
                "Coroot can also return no data before its project world is ready. Refresh to check again.",
                None,
                Vec::new(),
                cx,
            )
        } else {
            ui::empty_state(
                IconName::SearchX,
                "No incidents match these filters",
                "Clear the search and the state chip to see every incident in the sample.",
                None,
                vec![
                    action("obs-incidents-clear", "Clear filters")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.clear_incident_filters(window, cx)
                        }))
                        .into_any_element(),
                ],
                cx,
            )
        };
        Some(empty.h_auto().into_any_element())
    }
    pub(in crate::observability) fn incident_notes(
        &self,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let state = &self.incident_observations;
        let (shown, total) = (state.shown.len(), state.rows.len());
        if shown == 0 || shown >= total {
            return Vec::new();
        }
        vec![
            table::showing_bar(
                self.incident_table.id("showing"),
                shown,
                total,
                Button::new("obs-incidents-show-all")
                    .ghost()
                    .xsmall()
                    .label(format!("Show all {total}"))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.clear_incident_filters(window, cx)),
                    ),
                cx,
            )
            .into_any_element(),
        ]
    }
}

fn label(kind: ColumnKind) -> &'static str {
    match kind {
        ColumnKind::Key => "Incident",
        ColumnKind::Title => "Title",
        ColumnKind::App => "Application",
        ColumnKind::Opened => "Opened",
        ColumnKind::Duration => "Duration",
        ColumnKind::Impact => "Impact",
        _ => "",
    }
}
