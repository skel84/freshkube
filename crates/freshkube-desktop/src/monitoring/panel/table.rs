//! A table panel's rows in the shared `DataTable`. It is a child view of its
//! panel, as the plot is, so each table keeps its own scroll and Show all.
//! The page builds its panels for one dashboard on one source, so another
//! dashboard or source starts every table again; a new answer keeps them.
use std::rc::Rc;

use freshkube_ui::table::{
    self, DataTable, GLYPH_WIDTH, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource,
    TableState, WIDEST, WIDEST_FLEXIBLE,
};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, IntoElement, Render, SharedString, Window, div};

use super::summary::tone;
use crate::monitoring::derive::{FOLDED_ROWS, RowKey, TableData, TableRow as Row};
use crate::ui;

pub(crate) struct TableView {
    /// The panel's title, for the list's label.
    title: SharedString,
    data: Rc<TableData>,
    columns: Vec<Column>,
    width: f32,
    /// Each row's element id, from its key.
    ids: Vec<SharedString>,
    state: TableState,
    /// Show all was pressed: every kept row shows, not just the first
    /// [`FOLDED_ROWS`].
    all: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// The severity's status glyph.
    Glyph,
    /// The answer's column at this index.
    Cell { index: usize, numeric: bool },
}

pub(crate) struct Column {
    label: SharedString,
    width: f32,
    flexible: bool,
    kind: Kind,
}

impl TableColumn for Column {
    fn label(&self) -> &SharedString {
        &self.label
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn flexible(&self) -> bool {
        self.flexible
    }

    fn pinned(&self) -> bool {
        self.kind == Kind::Glyph
    }
}

impl TableView {
    /// A table under `prefix`, its panel's id, for `title`'s rows.
    pub(crate) fn new(prefix: &str, title: SharedString, data: Rc<TableData>) -> Self {
        let state = TableState::new(&format!("{prefix}-table"));
        let mut view = Self {
            title,
            data: Rc::default(),
            columns: Vec::new(),
            width: 0.,
            ids: Vec::new(),
            state,
            all: false,
        };
        view.set_data(data);
        view
    }

    /// A new answer: its columns' widths and its rows' ids, derived once.
    /// The scroll and Show all stay, as a refresh shouldn't move the rows.
    pub(crate) fn set_data(&mut self, data: Rc<TableData>) {
        self.columns = columns(&data);
        self.width = self.columns.iter().map(TableColumn::width).sum();
        self.ids = data
            .rows
            .iter()
            .map(|row| self.state.id(&row_id(&row.key)))
            .collect();
        self.data = data;
    }

    /// Whether Show all was pressed, for the page's tests.
    #[cfg(test)]
    pub(crate) fn shows_all(&self) -> bool {
        self.all
    }

    /// The list's id and the first row's, for the page's tests.
    #[cfg(test)]
    pub(crate) fn test_ids(&self) -> (SharedString, SharedString) {
        (self.state.id("list"), self.ids[0].clone())
    }

    /// Whether the rows lead with a severity glyph, for the page's tests.
    #[cfg(test)]
    pub(crate) fn leads_with_glyph(&self) -> bool {
        self.columns
            .first()
            .is_some_and(|column| column.kind == Kind::Glyph)
    }

    pub(crate) fn show_all(&mut self, cx: &mut Context<Self>) {
        self.all = true;
        cx.notify();
    }
}

/// A leading glyph column when the rows carry a severity, then the
/// answer's columns, each as wide as its widest cell or name by DESIGN.md's
/// rule ([`table::fit`]). The widest text column takes the room left over.
fn columns(data: &TableData) -> Vec<Column> {
    let flexible = data
        .columns
        .iter()
        .enumerate()
        .filter(|(_, column)| !column.numeric)
        .max_by_key(|(_, column)| column.chars)
        .map(|(index, _)| index);
    let glyph = data.severity.then(|| Column {
        label: SharedString::default(),
        width: GLYPH_WIDTH,
        flexible: false,
        kind: Kind::Glyph,
    });
    let cells = data.columns.iter().enumerate().map(|(index, column)| {
        let flexible = flexible == Some(index);
        let most = if flexible { WIDEST_FLEXIBLE } else { WIDEST };
        Column {
            label: column.name.clone(),
            width: table::fit(
                &column.name,
                data.rows.iter().map(|row| &row.cells[index]),
                most,
            ),
            flexible,
            kind: Kind::Cell {
                index,
                numeric: column.numeric,
            },
        }
    });
    glyph.into_iter().chain(cells).collect()
}

/// `row-<labels>-<n>`: the labels joined, and which of the rows with
/// those labels it is.
fn row_id(key: &RowKey) -> String {
    format!("row-{}-{}", key.labels.join("·"), key.occurrence)
}

impl TableSource for TableView {
    type Key = RowKey;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a Row;

    fn table_state(&self) -> &TableState {
        &self.state
    }

    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn list_label(&self) -> String {
        format!("{} rows", self.title)
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        if self.all {
            self.data.rows.len()
        } else {
            self.data.rows.len().min(FOLDED_ROWS)
        }
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<RowKey, &Row>> {
        if line >= self.line_count() {
            return None;
        }
        let row = self.data.rows.get(line)?;
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: self.ids[line].clone().into(),
            label: row.cells.first().cloned().unwrap_or_default(),
            tooltip: None,
            marked: false,
            muted: false,
            data: row,
        }))
    }

    fn cell(
        &self,
        row: &TableRow<RowKey, &Row>,
        _: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match column.kind {
            Kind::Glyph => table::glyph_cell(column)
                .children(
                    row.data
                        .tier
                        .and_then(|tier| ui::status_glyph(tone(tier), cx)),
                )
                .into_any_element(),
            Kind::Cell { index, numeric } => table::cell(column)
                .when(numeric, |this| this.text_right().font_family(ui::MONO_FONT))
                .child(row.data.cells[index].clone())
                .into_any_element(),
        }
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn clickable(&self) -> bool {
        false
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    /// `Showing 100 of 240` while rows are folded or past the cap, and
    /// Show all while any kept row is folded.
    fn counts(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let (shown, kept, total) = (self.line_count(), self.data.rows.len(), self.data.total);
        if shown >= total {
            return Vec::new();
        }
        let show_all = (shown < kept).then(|| {
            Button::new(self.state.id("show-all"))
                .ghost()
                .xsmall()
                .label(format!("Show all {kept}"))
                .on_click(cx.listener(|view, _, _, cx| view.show_all(cx)))
        });
        vec![
            table::showing(
                self.state.id("showing"),
                shown,
                total,
                div().children(show_all),
                cx,
            )
            .into_any_element(),
        ]
    }
}

impl Render for TableView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        DataTable::new()
            .inset()
            .render(self, window, cx)
            .size_full()
    }
}
