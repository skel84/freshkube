//! The page's lists share one `TableSource`, since the page is one entity
//! (#5): the destination on show picks which list it answers for, and each
//! list keeps its own state, columns and ids.
use super::*;
use application_columns::ApplicationColumn;
use applications::ApplicationCells;
use freshkube_ui::table::{Line, RowStyle, SortOrder, TableRow, TableSource, TableState};

/// A row's identity in whichever list it belongs to.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) enum TableKey {
    Application(freshkube_core::coroot::AppId),
}

/// A row's data, borrowed from the page for one frame.
pub(crate) enum TableCells<'a> {
    Application(ApplicationCells<'a>),
}

/// The list the page's table draws now.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shown {
    Applications,
}

impl ObservabilityPage {
    fn shown_table(&self) -> Shown {
        Shown::Applications
    }
}

impl TableSource for ObservabilityPage {
    type Key = TableKey;
    type Sort = ();
    type Column = ApplicationColumn;
    type Row<'a> = TableCells<'a>;

    fn table_state(&self) -> &TableState {
        match self.shown_table() {
            Shown::Applications => &self.application_table,
        }
    }
    fn columns(&self) -> &[ApplicationColumn] {
        match self.shown_table() {
            Shown::Applications => &self.application_columns,
        }
    }
    fn width(&self) -> f32 {
        match self.shown_table() {
            Shown::Applications => self.application_width,
        }
    }
    fn list_label(&self) -> String {
        match self.shown_table() {
            Shown::Applications => self.application_list_label(),
        }
    }
    fn sorting(&self, _: &ApplicationColumn) -> Option<((), Option<SortOrder>)> {
        None
    }
    fn sort(&mut self, _: (), _: &mut Context<Self>) {}
    fn clickable(&self) -> bool {
        match self.shown_table() {
            Shown::Applications => false,
        }
    }
    fn line_count(&self) -> usize {
        match self.shown_table() {
            Shown::Applications => self.matrix.len(),
        }
    }
    fn line(&self, line: usize, _: &App) -> Option<Line<TableKey, TableCells<'_>>> {
        match self.shown_table() {
            Shown::Applications => self.application_line(line),
        }
    }
    fn cell(
        &self,
        row: &TableRow<TableKey, TableCells<'_>>,
        style: &RowStyle,
        column: &ApplicationColumn,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match &row.data {
            TableCells::Application(cells) => self.application_cell(cells, style, column, cx),
        }
    }
    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        match self.shown_table() {
            Shown::Applications => self.application_group(group, cx),
        }
    }
    fn empty(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        match self.shown_table() {
            Shown::Applications => self.application_empty(cx),
        }
    }
    fn notes(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        match self.shown_table() {
            Shown::Applications => self.application_notes(cx),
        }
    }
    fn footer(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        match self.shown_table() {
            Shown::Applications => self.application_footer(window, cx),
        }
    }
}
