//! The page's lists share one `TableSource`, since the page is one entity
//! (#5): the destination on show picks which list it answers for, and each
//! list keeps its own state, columns and ids.
use super::*;
use applications::ApplicationCells;
use freshkube_ui::table::{
    Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, TableState,
};
use gpui_kit::ClickEvent;
use incidents::IncidentCells;

/// A row's identity in whichever list it belongs to.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) enum TableKey {
    Application(freshkube_core::coroot::AppId),
    /// An incident's key and its application, as Coroot names it.
    Incident(String, freshkube_core::coroot::AppId),
}

/// A row's data, borrowed from the page for one frame.
pub(crate) enum TableCells<'a> {
    Application(ApplicationCells<'a>),
    Incident(IncidentCells<'a>),
}

/// What a column shows. Each list uses its own kinds; Glyph is shared.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::observability) enum ColumnKind {
    Glyph,
    Name,
    Type,
    Report(Report),
    Key,
    Title,
    App,
    Opened,
    Duration,
    Impact,
}

/// One of a list's columns, measured when its data or Columns change.
#[derive(Clone)]
pub(crate) struct PageColumn {
    pub(in crate::observability) kind: ColumnKind,
    pub(in crate::observability) label: SharedString,
    pub(in crate::observability) width: f32,
}

impl TableColumn for PageColumn {
    fn label(&self) -> &SharedString {
        &self.label
    }
    fn width(&self) -> f32 {
        self.width
    }
    fn flexible(&self) -> bool {
        matches!(self.kind, ColumnKind::Name | ColumnKind::Title)
    }
}

/// The list the page's table draws now.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shown {
    Applications,
    Incidents,
}

impl ObservabilityPage {
    fn shown_table(&self) -> Shown {
        match self.destination {
            Destination::Incidents => Shown::Incidents,
            _ => Shown::Applications,
        }
    }
}

impl TableSource for ObservabilityPage {
    type Key = TableKey;
    type Sort = ();
    type Column = PageColumn;
    type Row<'a> = TableCells<'a>;

    fn table_state(&self) -> &TableState {
        match self.shown_table() {
            Shown::Applications => &self.application_table,
            Shown::Incidents => &self.incident_table,
        }
    }
    fn columns(&self) -> &[PageColumn] {
        match self.shown_table() {
            Shown::Applications => &self.application_columns,
            Shown::Incidents => self.incident_columns(),
        }
    }
    fn width(&self) -> f32 {
        match self.shown_table() {
            Shown::Applications => self.application_width,
            Shown::Incidents => self.incident_width(),
        }
    }
    fn list_label(&self) -> String {
        match self.shown_table() {
            Shown::Applications => self.application_list_label(),
            Shown::Incidents => self.incident_list_label(),
        }
    }
    fn sorting(&self, _: &PageColumn) -> Option<((), Option<SortOrder>)> {
        None
    }
    fn sort(&mut self, _: (), _: &mut Context<Self>) {}
    fn selected_key(&self) -> Option<&TableKey> {
        match self.shown_table() {
            Shown::Applications => None,
            Shown::Incidents => self.incident_selected_key(),
        }
    }
    fn line_of(&self, key: &TableKey) -> Option<usize> {
        match self.shown_table() {
            Shown::Applications => None,
            Shown::Incidents => self.incident_line_of(key),
        }
    }
    fn clickable(&self) -> bool {
        match self.shown_table() {
            Shown::Applications => false,
            Shown::Incidents => true,
        }
    }
    fn click(&mut self, key: &TableKey, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let TableKey::Incident(key, app) = key {
            self.select_incident(key.clone(), app.clone(), cx);
        }
    }
    fn line_count(&self) -> usize {
        match self.shown_table() {
            Shown::Applications => self.matrix.len(),
            Shown::Incidents => self.incident_line_count(),
        }
    }
    fn line(&self, line: usize, _: &App) -> Option<Line<TableKey, TableCells<'_>>> {
        match self.shown_table() {
            Shown::Applications => self.application_line(line),
            Shown::Incidents => self.incident_line(line),
        }
    }
    fn cell(
        &self,
        row: &TableRow<TableKey, TableCells<'_>>,
        style: &RowStyle,
        column: &PageColumn,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match &row.data {
            TableCells::Application(cells) => self.application_cell(cells, style, column, cx),
            TableCells::Incident(cells) => self.incident_cell(cells, style, column, cx),
        }
    }
    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        match self.shown_table() {
            Shown::Applications => self.application_group(group, cx),
            Shown::Incidents => None,
        }
    }
    fn empty(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        match self.shown_table() {
            Shown::Applications => self.application_empty(cx),
            Shown::Incidents => self.incident_empty(cx),
        }
    }
    fn notes(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        match self.shown_table() {
            Shown::Applications => self.application_notes(cx),
            Shown::Incidents => self.incident_notes(cx),
        }
    }
    fn footer(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        match self.shown_table() {
            Shown::Applications => self.application_footer(window, cx),
            Shown::Incidents => None,
        }
    }
}
