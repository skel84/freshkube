//! The revision list: its header, columns and the table's answers. Rows and
//! widths are prepared when the application's page arrives; these only
//! read them.
use super::*;
use crate::tables::TableCells;
use freshkube_ui::table::{self, DataTable, Line, RowStyle, TableRow};
use freshkube_ui::tooltip::FollowTooltip as _;

/// Coroot keeps the history; the status bar's tooltip says how much.
const NOTE: &str = "Coroot keeps an application's last 100 deployments.";

/// The list's accessibility label.
const LIST_LABEL: &str = "Deployments of the chosen application; choose one to read around its start. Coroot keeps an application's last 100 deployments.";

const COLUMNS: [ColumnKind; 5] = [
    ColumnKind::Glyph,
    ColumnKind::Revision,
    ColumnKind::Image,
    ColumnKind::Started,
    ColumnKind::Finding,
];

/// The public table contract borrows the page's private row.
pub struct RevisionCells<'a> {
    row: &'a Row,
}

impl ObservabilityPage {
    /// What Deployments puts in the status bar.
    pub(crate) fn deployments_read(&self) -> crate::status::Read<'_> {
        let state = &self.revision_observations;
        let count = if self.selected_app.is_none() {
            "No application chosen"
        } else if (!self.fixture && self.live.view.data().is_none()) || state.app.is_none() {
            if self.live.view.is_loading() {
                "Loading deployments"
            } else {
                "No observation"
            }
        } else {
            &state.count
        };
        crate::status::Read {
            count,
            stale: self.live.view.is_stale(),
            time: self.read_time(self.live.view.last_successful()),
            note: Some(NOTE),
        }
    }

    pub(crate) fn deployments_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = self.source_header(self.page_header(cx), cx);
        // Kit's Select fills its parent, so a box sets its size in the row.
        let picker = div().w(dp(260.)).flex_none().child(
            Select::new(&self.app_select)
                .id("obs-deploy-app")
                .small()
                .menu_width(dp(420.))
                .placeholder("Choose an application")
                .search_placeholder("Find an application")
                .accessibility_label("Application")
                .disabled(self.app_choices.is_empty()),
        );
        self.time_controls(header.filter(picker), cx)
            .render(window, cx)
    }

    /// Widths fit the revisions' values, measured once per list.
    pub(crate) fn prepare_revision_columns(&mut self) {
        let metrics = &self.application_metrics;
        let state = &self.revision_observations;
        let columns: Vec<_> = COLUMNS
            .into_iter()
            .map(|kind| {
                let value = state
                    .rows
                    .iter()
                    .map(|row| match kind {
                        ColumnKind::Revision => metrics.value(row.hash.clone(), 12.5),
                        ColumnKind::Image => metrics.value(row.short_image.clone(), 12.5),
                        ColumnKind::Started => metrics.value(row.started.clone(), 12.),
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
                        ColumnKind::Finding => 160.,
                        // Narrow enough beside the inspector for Coroot's
                        // finding to show; the tooltip has every image.
                        ColumnKind::Image => measured.clamp(96., 200.),
                        _ => measured.max(56.),
                    },
                }
            })
            .collect();
        let state = &mut self.revision_observations;
        state.width = columns.iter().map(|c| c.width).sum();
        state.columns = columns;
    }

    /// The table, bare, filling the split's pane beside its inspector.
    pub(super) fn render_revision_table(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        DataTable::new()
            .render(self, window, cx)
            .id(self.revision_table.id("table"))
            .test_support()
            .size_full()
            .into_any_element()
    }

    pub(crate) fn revision_columns(&self) -> &[PageColumn] {
        &self.revision_observations.columns
    }
    pub(crate) fn revision_width(&self) -> f32 {
        self.revision_observations.width
    }
    pub(crate) fn revision_list_label(&self) -> String {
        LIST_LABEL.to_owned()
    }
    pub(crate) fn revision_selected_key(&self) -> Option<&TableKey> {
        self.revision_observations.selected.as_ref()
    }
    /// The rows show only for the chosen application.
    pub(super) fn revision_rows(&self) -> &[Row] {
        let state = &self.revision_observations;
        if state.app.is_some() && state.app == self.selected_app {
            &state.rows
        } else {
            &[]
        }
    }
    pub(crate) fn revision_line_of(&self, key: &TableKey) -> Option<usize> {
        self.revision_rows().iter().position(|row| row.key == *key)
    }
    pub(crate) fn revision_line_count(&self) -> usize {
        self.revision_rows().len()
    }
    pub(crate) fn revision_line(&self, line: usize) -> Option<Line<TableKey, TableCells<'_>>> {
        let row = self.revision_rows().get(line)?;
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: row.id.clone().into(),
            label: row.label.clone(),
            tooltip: Some(row.label.clone()),
            marked: false,
            muted: false,
            data: TableCells::Revision(RevisionCells { row }),
        }))
    }
    pub(crate) fn revision_cell(
        &self,
        cells: &RevisionCells<'_>,
        style: &RowStyle,
        column: &PageColumn,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = cells.row;
        let p = style.p;
        let cell = table::cell(column).h_full().flex().items_center();
        let mono = |cell: Div, value: &SharedString, size: f32| {
            cell.font_family(MONO_FONT)
                .text_size(dp(size))
                .child(text(value.clone()).truncate())
        };
        match column.kind {
            ColumnKind::Glyph => table::glyph_cell(column)
                .children(ui::status_glyph(row.status.tone(), cx))
                .into_any_element(),
            ColumnKind::Revision => mono(cell, &row.hash, 12.5).into_any_element(),
            // Each image's name and tag, as Coroot already sends them;
            // example data's whole references show in the tooltip and the
            // inspector.
            ColumnKind::Image => table::cell(column)
                .id(SharedString::from(format!("{}-image-cell", row.id)))
                .follow_tooltip(row.image.clone())
                .child(table::word_cut(
                    SharedString::from(format!("{}-image", row.id)),
                    row.short_image.clone(),
                ))
                .h_full()
                .flex()
                .items_center()
                .font_family(MONO_FONT)
                .text_size(dp(12.5))
                .text_color(p.ink_2)
                .into_any_element(),
            ColumnKind::Started => {
                mono(cell.text_color(p.ink_2), &row.started, 12.).into_any_element()
            }
            // Coroot's words, in the interface face.
            ColumnKind::Finding => {
                table::word_cell(column, format!("{}-finding", row.id), row.finding.clone())
                    .h_full()
                    .flex()
                    .items_center()
                    .font_family(gpui_kit::component::Theme::global(cx).font_family.clone())
                    .into_any_element()
            }
            _ => cell.into_any_element(),
        }
    }
    pub(crate) fn revision_empty(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.revision_rows().is_empty() {
            return None;
        }
        let state = &self.revision_observations;
        // Until the application's page answers, having no revisions isn't
        // known: its loading rows or its failure show instead.
        let known = state.answered && state.app == self.selected_app;
        if self.selected_app.is_some() && !known {
            return None;
        }
        let empty = match (&self.selected_app, &state.list_error) {
            (None, _) => ui::empty_state(
                IconName::LayoutGrid,
                "Choose an application",
                "Its deployments are read from Coroot's Deployments report.",
                None,
                Vec::new(),
                cx,
            ),
            (Some(_), Some(error)) => ui::empty_state(
                IconName::TriangleAlert,
                "Couldn't read this application's deployments",
                "Coroot answered with the application's page, but its Deployments report couldn't be read. Nothing is shown as missing.",
                Some(error.clone()),
                vec![
                    action("obs-deployments-retry", "Retry")
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx)))
                        .into_any_element(),
                ],
                cx,
            ),
            (Some(app), None) if app.kind() != "Deployment" => ui::empty_state(
                IconName::Info,
                "Coroot records rollouts of Deployments only",
                format!(
                    "This application is a {}, so Coroot keeps no revisions of it.",
                    if app.kind().is_empty() {
                        "workload of another kind"
                    } else {
                        app.kind()
                    }
                ),
                None,
                Vec::new(),
                cx,
            ),
            (Some(_), None) => ui::empty_state(
                IconName::Info,
                "Coroot keeps no deployment of this application",
                "Coroot records a revision when a Deployment's pods roll out.",
                None,
                Vec::new(),
                cx,
            ),
        };
        // The table names its empty state `obs-deployments-empty`.
        Some(empty.h_auto().into_any_element())
    }
}

fn label(kind: ColumnKind) -> &'static str {
    match kind {
        ColumnKind::Revision => "Revision",
        ColumnKind::Image => "Images",
        ColumnKind::Started => "Started",
        ColumnKind::Finding => "Coroot's finding",
        _ => "",
    }
}
