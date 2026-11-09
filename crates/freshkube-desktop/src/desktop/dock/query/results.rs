//! One result tab per statement: what it is waiting on or how it ended, and
//! for rows the table that shows them.
use super::history::Ended;
use crate::ui::{self, dp};
use freshkube_db::{Plan, QueryError, QueryResult, Value};
use freshkube_ui::{
    palette::palette,
    table::{
        self as kit, DataTable, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource,
        TableState,
    },
};
use gpui_kit::*;
use std::time::Duration;

#[derive(Clone)]
pub(super) struct Column {
    /// Its place among the statement's columns. Labels repeat (`select o.id,
    /// c.id`) and are no key.
    ix: usize,
    label: SharedString,
    width: f32,
    flexible: bool,
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
        false
    }
}

#[derive(Clone)]
pub(super) struct Cell {
    text: SharedString,
    null: bool,
}

#[derive(Clone)]
pub(super) struct Row {
    key: SharedString,
    id: SharedString,
    cells: Vec<Cell>,
}

/// The rows of one statement, as drawn.
#[derive(Clone)]
pub(super) struct Rows {
    columns: Vec<Column>,
    rows: Vec<Row>,
    pub(super) capped: bool,
    pub(super) row_cap: usize,
    pub(super) elapsed: Duration,
}

impl Rows {
    pub(super) fn of(result: QueryResult) -> Self {
        let rows: Vec<Vec<Cell>> = result
            .rows
            .iter()
            .map(|row| row.iter().map(cell_of).collect())
            .collect();
        let last = result.columns.len().saturating_sub(1);
        let columns = result
            .columns
            .iter()
            .enumerate()
            .map(|(ix, name)| {
                let flexible = ix == last;
                let width = kit::fit(
                    name,
                    rows.iter().filter_map(|r| r.get(ix).map(|c| &c.text)),
                    if flexible {
                        kit::WIDEST_FLEXIBLE
                    } else {
                        kit::WIDEST
                    },
                );
                Column {
                    ix,
                    label: name.clone().into(),
                    width,
                    flexible,
                }
            })
            .collect();
        let rows = rows
            .into_iter()
            .enumerate()
            .map(|(i, cells)| Row {
                key: format!("row-{i}").into(),
                id: format!("query-row-{}", i + 1).into(),
                cells,
            })
            .collect();
        Self {
            columns,
            rows,
            capped: result.capped,
            row_cap: result.row_cap,
            elapsed: result.elapsed,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.rows.len()
    }

    /// What each row draws under each column, as `cell()` finds it.
    #[cfg(test)]
    pub(super) fn drawn(&self) -> Vec<Vec<String>> {
        self.rows
            .iter()
            .map(|row| {
                self.columns
                    .iter()
                    .map(|column| text_at(row, column).text.to_string())
                    .collect()
            })
            .collect()
    }
}

/// The cell a row draws under `column`: by the column's place, never its label.
fn text_at<'a>(row: &'a Row, column: &Column) -> &'a Cell {
    &row.cells[column.ix]
}

fn cell_of(value: &Value) -> Cell {
    let (text, null): (String, bool) = match value {
        Value::Null => ("NULL".into(), true),
        Value::Text(t) | Value::Json(t) | Value::Decimal(t) => {
            (t.replace(['\n', '\r', '\t'], " "), false)
        }
        Value::Bool(b) => (b.to_string(), false),
        Value::Int(i) => (i.to_string(), false),
        Value::Float(f) => (f.to_string(), false),
        other => (format!("{other:?}"), false),
    };
    Cell {
        text: text.into(),
        null,
    }
}

/// How one statement stands.
pub(super) enum State {
    /// Waiting for the statements before it.
    Pending,
    Running,
    Rows(Rows),
    /// The plan and the index of its longest line.
    Plan(Plan, usize),
    /// The statement's own words, with the kind of failure.
    Over(QueryError),
    /// Not run: an earlier statement failed or was cancelled.
    Skipped,
    /// Not run: the run is cut at `MAX_STATEMENTS`; this many were left out.
    TooMany(usize),
}

impl State {
    pub(super) fn ended(&self) -> Option<Ended> {
        Some(match self {
            Self::Pending | Self::Running | Self::Skipped | Self::TooMany(_) => return None,
            Self::Rows(rows) => Ended::Rows(rows.len()),
            Self::Plan(..) => Ended::Plan,
            Self::Over(QueryError::Refused(_)) => Ended::Refused,
            Self::Over(QueryError::TimedOut) => Ended::TimedOut,
            Self::Over(QueryError::Cancelled) => Ended::Cancelled,
            Self::Over(QueryError::NotSent(_)) => Ended::NotSent,
            Self::Over(_) => Ended::Failed,
        })
    }
}

pub(super) struct Item {
    pub(super) sql: SharedString,
    /// "1 · select * from orders", for the result's tab; made once, with the
    /// item, never while drawing.
    pub(super) label: SharedString,
    pub(super) state: State,
}

impl Item {
    pub(super) fn new(ordinal: usize, sql: String, plan: bool) -> Self {
        let one_line: String = sql.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut short: String = one_line.chars().take(28).collect();
        if one_line.chars().count() > 28 {
            short.push('…');
        }
        let kind = if plan { "Plan · " } else { "" };
        Self {
            label: format!("{ordinal} · {kind}{short}").into(),
            sql: sql.into(),
            state: State::Pending,
        }
    }

    /// The item for the statements a run left out.
    pub(super) fn left_out(ordinal: usize, count: usize) -> Self {
        Self {
            label: format!("{ordinal} · +{count} more").into(),
            sql: SharedString::default(),
            state: State::TooMany(count),
        }
    }
}

/// What a failed statement says, in the words of the state.
pub(super) fn over_words(error: &QueryError) -> (&'static str, String, ui::Tone) {
    match error {
        QueryError::Refused(why) => (
            "Refused: this session is read-only",
            format!(
                "The database refused it: {why}. Nothing was changed, and it was not tried again."
            ),
            ui::Tone::Warn,
        ),
        QueryError::NotSent(why) => ("Not sent", why.clone(), ui::Tone::Warn),
        QueryError::TimedOut => (
            "Timed out",
            "The statement ran past the time limit and the database stopped it.".into(),
            ui::Tone::Warn,
        ),
        QueryError::Cancelled => (
            "Cancelled",
            "The statement was stopped. Nothing was changed.".into(),
            ui::Tone::Outline,
        ),
        QueryError::Failed(why) => ("Failed", why.clone(), ui::Tone::Crit),
        QueryError::Lost(why) => (
            "Connection lost",
            format!(
                "The connection was lost: {why}. Open Query again from the explorer once it is connected."
            ),
            ui::Tone::Crit,
        ),
    }
}

/// The table of the selected result.
pub(super) struct Grid {
    table: TableState,
    rows: Option<Rows>,
    selected: Option<SharedString>,
    focus: FocusHandle,
}

impl Grid {
    pub(super) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            table: TableState::new("query-grid"),
            rows: None,
            selected: None,
            focus: cx.focus_handle(),
        }
    }

    /// Shows `rows`, or nothing.
    pub(super) fn show(&mut self, rows: Option<Rows>, cx: &mut Context<Self>) {
        self.rows = rows;
        self.selected = None;
        cx.notify();
    }
}

impl Render for Grid {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("query-grid")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .child(DataTable::new().render(self, window, cx).flex_1().min_h_0())
    }
}

impl TableSource for Grid {
    type Key = SharedString;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a Row;
    fn table_state(&self) -> &TableState {
        &self.table
    }
    fn columns(&self) -> &[Column] {
        self.rows.as_ref().map_or(&[], |r| &r.columns)
    }
    fn width(&self) -> f32 {
        self.columns().iter().map(|c| c.width).sum()
    }
    fn list_label(&self) -> String {
        "The statement's rows, read-only".into()
    }
    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }
    fn sort(&mut self, _: (), _: &mut Context<Self>) {}
    fn line_count(&self) -> usize {
        self.rows.as_ref().map_or(0, |r| r.rows.len())
    }
    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, &Row>> {
        let row = self.rows.as_ref()?.rows.get(line)?;
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: row.id.clone().into(),
            label: row.key.clone(),
            tooltip: None,
            marked: false,
            muted: false,
            data: row,
        }))
    }
    fn cell(
        &self,
        row: &TableRow<SharedString, &Row>,
        _: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ix = column.ix;
        let cell = text_at(row.data, column);
        let id = format!("{}-text-{}", row.data.id, ix + 1);
        if cell.null {
            return kit::cell(column)
                .id(SharedString::from(format!("{id}-cell")))
                .text_color(palette(cx).muted)
                .child("NULL")
                .into_any_element();
        }
        kit::word_cell(column, id, cell.text.clone()).into_any_element()
    }
    fn selected_key(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }
    fn line_of(&self, key: &SharedString) -> Option<usize> {
        self.rows.as_ref()?.rows.iter().position(|r| &r.key == key)
    }
    fn click(
        &mut self,
        key: &SharedString,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected = Some(key.clone());
        window.focus(&self.focus, cx);
        cx.notify();
    }
    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
    fn empty(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.line_count() > 0 {
            return None;
        }
        Some(
            div()
                .id("query-grid-empty")
                .aria_label("The statement returned no rows")
                .test_support()
                .p(dp(12.))
                .text_size(dp(12.))
                .text_color(palette(cx).muted)
                .child("The statement returned no rows.")
                .into_any_element(),
        )
    }
}
