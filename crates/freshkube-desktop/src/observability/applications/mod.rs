//! Applications' shared table; its projection is prepared when observations or filters change.
use super::*;
pub(super) mod columns;
pub(super) mod header;
use super::tables::{TableCells, TableKey};
use application_columns::{ColumnKind, PageColumn};
use freshkube_ui::table::{self, DataTable, Line, RowStyle, TableRow};

/// A fitted table stays bounded even when its outer page scrolls.
const MAX_APPLICATION_LINES: usize = 16;

/// The public table contract borrows the page's private presentation model.
pub(crate) struct ApplicationCells<'a> {
    app: &'a Application,
}

impl ObservabilityPage {
    pub(in crate::observability) fn render_applications(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        DataTable::new()
            .fit(MAX_APPLICATION_LINES)
            .render(self, window, cx)
            .id(self.application_table.id("table"))
            .test_support()
            .w_full()
            .flex_none()
            .into_any_element()
    }

    fn clear_application_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter = Filter::All;
        self.namespace = None;
        self.all_categories = true;
        self.query_text.clear();
        self.query
            .update(cx, |query, cx| query.set_value("", window, cx));
        self.project_filters();
        cx.notify();
    }
}

/// Applications' answers to the page's `TableSource` (`tables.rs`).
impl ObservabilityPage {
    pub(in crate::observability) fn application_list_label(&self) -> String {
        "Applications grouped by namespace; choose a name or reported value to open its report"
            .into()
    }
    pub(in crate::observability) fn application_line(
        &self,
        line: usize,
    ) -> Option<Line<TableKey, TableCells<'_>>> {
        let index = match self.matrix.get(line)? {
            MatrixRow::Group { .. } => return Some(Line::Group(line)),
            MatrixRow::App(index) => *index,
        };
        let app = self.applications.get(index)?;
        Some(Line::Row(TableRow {
            key: TableKey::Application(app.id.clone()),
            id: app.row_id.clone().into(),
            label: app.label.clone(),
            tooltip: None,
            marked: false,
            muted: false,
            data: TableCells::Application(ApplicationCells { app }),
        }))
    }
    pub(in crate::observability) fn application_cell(
        &self,
        cells: &ApplicationCells<'_>,
        style: &RowStyle,
        column: &PageColumn,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = cells.app;
        let p = style.p;
        let cell = table::cell(column).h_full().flex().items_center();
        match column.kind {
            ColumnKind::Glyph => cell
                .children(ui::status_glyph(app.status.tone(), cx))
                .into_any_element(),
            ColumnKind::Name => {
                let app_id = app.id.clone();
                cell.child(
                    Button::new(app.name_id.clone())
                        .ghost()
                        .small()
                        .h_full()
                        .w_full()
                        .min_w_0()
                        .justify_start()
                        .px_0()
                        .font_family(MONO_FONT)
                        .text_size(dp(12.5))
                        .text_color(p.ink)
                        .child(mono(app.name.clone()).w_full().truncate())
                        .tooltip(app.label.clone())
                        .accessibility_label(app.label.clone())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_app(app_id.clone(), Report::Errors, cx)
                        })),
                )
                .into_any_element()
            }
            ColumnKind::Type => cell
                .child(text(app.language.clone()).truncate().text_color(p.ink_2))
                .into_any_element(),
            ColumnKind::Report(report) => {
                let check = app.check(report);
                let app_id = app.id.clone();
                let glyph = check.status.report_tone();
                let color = match check.status {
                    Status::Critical => p.crit_ink,
                    Status::Warning | Status::LogError => p.warn_ink,
                    Status::Absent | Status::Unknown => p.muted,
                    _ => p.ink_2,
                };
                cell.child(
                    Button::new(check.element_id.clone())
                        .ghost()
                        .small()
                        .h_full()
                        .w_full()
                        .min_w_0()
                        .px_0()
                        .gap(dp(4.))
                        .justify_start()
                        .font_family(MONO_FONT)
                        .text_size(dp(12.))
                        .text_color(color)
                        .children(glyph.and_then(|tone| ui::status_glyph(tone, cx)))
                        .child(text(check.value.clone()).flex_1().text_size(dp(12.)))
                        .tooltip(check.tooltip.clone())
                        .accessibility_label(check.label.clone())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_app(app_id.clone(), report, cx)
                        })),
                )
                .into_any_element()
            }
            _ => cell.into_any_element(),
        }
    }
    pub(in crate::observability) fn application_group(
        &self,
        group: usize,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let MatrixRow::Group {
            id,
            label,
            status,
            summary,
        } = self.matrix.get(group)?
        else {
            return None;
        };
        Some(
            table::GroupRow::new(
                id.clone(),
                status.tone(),
                label.clone(),
                self.application_table.row_height(),
            )
            .detail(vec![summary.clone()])
            .render(cx)
            .into_any_element(),
        )
    }
    pub(in crate::observability) fn application_empty(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.matrix.is_empty() {
            return None;
        }
        let no_apps = self.applications.is_empty();
        let actions =
            if no_apps {
                Vec::new()
            } else {
                vec![
                    action("obs-clear-filters", "Clear filters")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.clear_application_filters(window, cx)
                        }))
                        .into_any_element(),
                ]
            };
        Some(
            ui::empty_state(
                IconName::LayoutGrid,
                if no_apps {
                    "No applications were returned"
                } else {
                    "No applications match these filters"
                },
                if no_apps {
                    "Coroot has no applications in this observation window."
                } else {
                    "Clear the search, namespace and category filters to see every application."
                },
                None,
                actions,
                cx,
            )
            .h_auto()
            .into_any_element(),
        )
    }
    pub(in crate::observability) fn application_notes(
        &self,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if self.shown_apps >= self.counts[1] {
            return Vec::new();
        }
        vec![
            table::showing_bar(
                self.application_table.id("collapsed"),
                self.shown_apps,
                self.counts[1],
                Button::new("obs-show-all")
                    .ghost()
                    .xsmall()
                    .label(format!("Show all {}", self.counts[1]))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.filter = Filter::All;
                        this.project_filters();
                        cx.notify();
                    })),
                cx,
            )
            .into_any_element(),
        ]
    }
    pub(in crate::observability) fn application_footer(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        const HINT: &str = "Glyphs show healthy, warning, critical or unknown. Healthy report values are plain; healthy checks without figures read ok; unknown, warning and critical reports carry a glyph. An em dash means no report.";
        if crate::screens::content_width(window) < 600. {
            return Some(
                table::legend_line(
                    self.application_table.id("legend"),
                    "Health glyphs · Plain values · — no report",
                    HINT,
                    cx,
                )
                .test_support()
                .into_any_element(),
            );
        }
        let mut items: Vec<_> = [
            (Tone::Good, "Healthy"),
            (Tone::Warn, "Warning"),
            (Tone::Crit, "Critical"),
            (Tone::Unknown, "Unknown"),
        ]
        .into_iter()
        .map(|(tone, label)| {
            table::legend_item(div().children(ui::status_glyph(tone, cx)), label).into_any_element()
        })
        .collect();
        items.push(table::legend_item(text("12%"), "Reported value").into_any_element());
        items.push(table::legend_item(text("—"), "No report").into_any_element());
        Some(
            table::legend(items, cx)
                .id(self.application_table.id("legend"))
                .test_support()
                .into_any_element(),
        )
    }
}
