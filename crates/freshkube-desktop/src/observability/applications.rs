//! Applications' shared table; its projection is prepared when observations or filters change.
use super::*;
use application_columns::{ApplicationColumn, ColumnKind};
use freshkube_ui::table::{
    self, DataTable, Line, RowStyle, SortOrder, TableRow, TableSource, TableState,
};

/// A fitted table stays bounded even when its outer page scrolls.
const MAX_APPLICATION_LINES: usize = 16;

/// The public table contract borrows the page's private presentation model.
pub(crate) struct ApplicationCells<'a> {
    app: &'a Application,
}

impl ObservabilityPage {
    pub(super) fn render_applications(
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
        self.active_categories = Rc::new(
            self.categories
                .iter()
                .map(|choice| choice.name.clone())
                .collect(),
        );
        self.query_text.clear();
        self.query
            .update(cx, |query, cx| query.set_value("", window, cx));
        self.project();
        cx.notify();
    }
}

impl TableSource for ObservabilityPage {
    type Key = freshkube_core::coroot::AppId;
    type Sort = ();
    type Column = ApplicationColumn;
    type Row<'a> = ApplicationCells<'a>;

    fn table_state(&self) -> &TableState {
        &self.application_table
    }
    fn columns(&self) -> &[ApplicationColumn] {
        &self.application_columns
    }
    fn width(&self) -> f32 {
        self.application_width
    }
    fn list_label(&self) -> String {
        "Applications grouped by namespace; choose a name or reported value to open its report"
            .into()
    }
    fn sorting(&self, _: &ApplicationColumn) -> Option<((), Option<SortOrder>)> {
        None
    }
    fn sort(&mut self, _: (), _: &mut Context<Self>) {}
    fn clickable(&self) -> bool {
        false
    }
    fn line_count(&self) -> usize {
        self.matrix.len()
    }
    fn line(&self, line: usize, _: &App) -> Option<Line<Self::Key, ApplicationCells<'_>>> {
        let index = match self.matrix.get(line)? {
            MatrixRow::Group { .. } => return Some(Line::Group(line)),
            MatrixRow::App(index) => *index,
        };
        let app = self.applications.get(index)?;
        Some(Line::Row(TableRow {
            key: app.id.clone(),
            id: app.row_id.clone().into(),
            label: app.label.clone(),
            tooltip: Some(app.label.clone()),
            marked: false,
            muted: false,
            data: ApplicationCells { app },
        }))
    }
    fn cell(
        &self,
        row: &TableRow<Self::Key, ApplicationCells<'_>>,
        style: &RowStyle,
        column: &ApplicationColumn,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = row.data.app;
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
                let problem = check.status.report_tone();
                let color = match problem {
                    Some(Tone::Crit) => p.crit_ink,
                    Some(Tone::Warn) => p.warn_ink,
                    _ if check.value.as_ref() == "—" => p.muted,
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
                        .children(problem.and_then(|tone| ui::status_glyph(tone, cx)))
                        .child(text(check.value.clone()).flex_1().text_size(dp(12.)))
                        .tooltip(check.tooltip.clone())
                        .accessibility_label(check.label.clone())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_app(app_id.clone(), report, cx)
                        })),
                )
                .into_any_element()
            }
        }
    }
    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
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
    fn empty(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
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
    fn notes(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
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
                    .small()
                    .label("Show all")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.filter = Filter::All;
                        this.project();
                        cx.notify();
                    })),
                cx,
            )
            .into_any_element(),
        ]
    }
    fn footer(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        const HINT: &str = "Glyphs show healthy, warning, critical, unknown or integration required. Healthy report values are plain; warning and critical reports carry a glyph. An em dash means no report.";
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
            (Tone::Integration, "Integration required"),
        ]
        .into_iter()
        .map(|(tone, label)| {
            table::legend_item(ui::status_glyph(tone, cx).unwrap(), label).into_any_element()
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
