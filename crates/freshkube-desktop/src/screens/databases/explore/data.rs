//! A table's first rows, capped, with the selected row's record in the
//! shared Inspector. Cells are the server's text, shown as it sent it.
use super::reading::read;
use super::{Shared, session_of};
use crate::ui::{self, dp};
use freshkube_db::{ColumnEntry, TablePage, Value};
use freshkube_reads::Loader;
use freshkube_ui::{
    inspector::{self, Inspector, InspectorSplit},
    palette::palette,
    table::{
        self as kit, DataTable, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource,
        TableState,
    },
};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;
use tokio::runtime::Handle;

pub(super) const PREFIX: &str = "explore-data";

actions!(explore_data, [Next, Previous, Close]);

pub(super) struct Column {
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

pub(super) struct Row {
    key: SharedString,
    id: SharedString,
    cells: Vec<Cell>,
}

#[derive(Clone)]
struct Cell {
    text: SharedString,
    null: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Target {
    epoch: u64,
    schema: String,
    table: String,
}

pub(super) struct DataView {
    runtime: Handle,
    session: Option<(u64, Shared)>,
    epoch: u64,
    target: Option<(String, String)>,
    info: Loader<Vec<ColumnEntry>, Target>,
    page: Loader<TablePage, Target>,
    columns: Vec<Column>,
    rows: Vec<Row>,
    prepared: (u64, u64),
    selected: Option<SharedString>,
    table: TableState,
    split: InspectorSplit,
    focus: FocusHandle,
}

impl DataView {
    pub(super) fn new(runtime: Handle, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", Next, Some("ExploreData")),
            KeyBinding::new("up", Previous, Some("ExploreData")),
            KeyBinding::new("escape", Close, Some("ExploreData")),
        ]);
        Self {
            runtime,
            session: None,
            epoch: 0,
            target: None,
            info: Loader::default(),
            page: Loader::default(),
            columns: Vec::new(),
            rows: Vec::new(),
            prepared: (u64::MAX, u64::MAX),
            selected: None,
            table: TableState::new(PREFIX),
            split: InspectorSplit::new(PREFIX, cx),
            focus: cx.focus_handle(),
        }
    }

    pub(super) fn set_session(&mut self, session: Option<Shared>, cx: &mut Context<Self>) {
        self.epoch += 1;
        self.session = session.map(|s| (self.epoch, s));
        self.target = None;
        self.info.reset();
        self.page.reset();
        self.selected = None;
        self.prepared = (u64::MAX, u64::MAX);
        cx.notify();
    }

    pub(super) fn open(&mut self, schema: String, table: String, cx: &mut Context<Self>) {
        self.target = Some((schema, table));
        self.selected = None;
        self.reload(cx);
    }

    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        let (Some((epoch, session)), Some((schema, table))) =
            (self.session.clone(), self.target.clone())
        else {
            return;
        };
        let runtime = self.runtime.clone();
        let id = Target {
            epoch,
            schema: schema.clone(),
            table: table.clone(),
        };
        let (s, t, sess) = (schema.clone(), table.clone(), session.clone());
        read(
            self,
            |this: &mut Self| &mut this.info,
            id.clone(),
            &runtime,
            "the columns",
            async move {
                session_of(&sess)
                    .await
                    .columns(&s, &t)
                    .await
                    .map_err(|e| e.to_string())
            },
            |_, _| {},
            cx,
        );
        read(
            self,
            |this: &mut Self| &mut this.page,
            id,
            &runtime,
            "the rows",
            async move {
                session_of(&session)
                    .await
                    .table_rows(&schema, &table)
                    .await
                    .map_err(|e| e.to_string())
            },
            |this, _| this.selected = None,
            cx,
        );
    }

    pub(super) fn is_loading(&self) -> bool {
        self.page.is_loading()
    }

    pub(super) fn error(&self) -> Option<String> {
        self.page.error().map(ToString::to_string)
    }

    /// "public.orders · 200 rows, the first of more", for the header.
    pub(super) fn summary(&self) -> Option<String> {
        let (schema, table) = self.target.as_ref()?;
        let page = self.page.data()?;
        let words = if page.capped {
            format!("first {} rows (row cap)", page.row_cap)
        } else {
            format!(
                "{} row{}",
                page.rows.len(),
                if page.rows.len() == 1 { "" } else { "s" }
            )
        };
        Some(format!("{schema}.{table} · {words}"))
    }

    pub(super) fn capped(&self) -> bool {
        self.page.data().is_some_and(|p| p.capped)
    }

    fn prepare(&mut self) {
        let now = (self.page.revision(), self.info.revision());
        if self.prepared == now {
            return;
        }
        self.prepared = now;
        self.columns.clear();
        self.rows.clear();
        let Some(page) = self.page.data() else {
            return;
        };
        let rows: Vec<Vec<Cell>> = page
            .rows
            .iter()
            .map(|row| row.iter().map(cell_of).collect())
            .collect();
        let last = page.columns.len().saturating_sub(1);
        for (ix, name) in page.columns.iter().enumerate() {
            let label: SharedString = name.clone().into();
            let texts: Vec<SharedString> = rows
                .iter()
                .filter_map(|r| r.get(ix).map(|c| c.text.clone()))
                .collect();
            let flexible = ix == last;
            let width = kit::fit(
                name,
                texts.iter(),
                if flexible {
                    kit::WIDEST_FLEXIBLE
                } else {
                    kit::WIDEST
                },
            );
            self.columns.push(Column {
                label,
                width,
                flexible,
            });
        }
        self.rows = rows
            .into_iter()
            .enumerate()
            .map(|(i, cells)| Row {
                key: format!("row-{i}").into(),
                id: format!("explore-row-{}", i + 1).into(),
                cells,
            })
            .collect();
    }

    fn choose(&mut self, key: SharedString, cx: &mut Context<Self>) {
        self.selected = Some(key);
        kit::reveal(self, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.prepare();
        if let Some(key) = kit::step(self, delta, cx) {
            self.choose(key, cx);
        }
    }

    fn details(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let key = self.selected.as_ref()?;
        let row = self.rows.iter().find(|r| &r.key == key)?;
        let p = palette(cx);
        let info = self.info.data();
        let fields = self
            .columns
            .iter()
            .zip(&row.cells)
            .enumerate()
            .map(|(i, (column, cell))| {
                let meta = info.and_then(|cols| cols.iter().find(|c| c.name == *column.label));
                let kind = meta.map(|c| {
                    let key = if c.primary_key { " · primary key" } else { "" };
                    let null = if c.nullable { "" } else { " · not null" };
                    format!("{}{key}{null}", c.type_name)
                });
                let value = if cell.null {
                    div().text_color(p.muted).child("NULL")
                } else {
                    ui::mono(cell.text.clone()).whitespace_normal()
                };
                v_flex()
                    .id(SharedString::from(format!("explore-field-{}", i + 1)))
                    .test_support()
                    .gap(dp(2.))
                    .text_size(dp(12.))
                    .child(
                        h_flex()
                            .gap(dp(6.))
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(column.label.clone()),
                            )
                            .children(kind.map(|k| div().text_color(p.muted).child(k))),
                    )
                    .child(value)
            });
        Some(
            Inspector::new("explore-record")
                .selection(key)
                .heading(
                    div()
                        .truncate()
                        .child(format!("Row {}", row.id.trim_start_matches("explore-row-"))),
                )
                .children(fields)
                .render(cx)
                .into_any_element(),
        )
    }

    pub(super) fn render_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.prepare();
        let table = div()
            .id("explore-data-table")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .child(DataTable::new().render(self, window, cx).flex_1().min_h_0())
            .into_any_element();
        inspector::split(
            "explore-data-split",
            &self.split,
            crate::screens::page_width(window) >= inspector::SPLIT_WIDTH + 360.,
            table,
            self.details(cx),
            window,
        )
    }
}

fn cell_of(value: &Value) -> Cell {
    match value {
        Value::Null => Cell {
            text: "NULL".into(),
            null: true,
        },
        Value::Text(t) | Value::Json(t) | Value::Decimal(t) => Cell {
            text: one_line(t).into(),
            null: false,
        },
        Value::Bool(b) => Cell {
            text: b.to_string().into(),
            null: false,
        },
        Value::Int(i) => Cell {
            text: i.to_string().into(),
            null: false,
        },
        Value::Float(f) => Cell {
            text: f.to_string().into(),
            null: false,
        },
        other => Cell {
            text: format!("{other:?}").into(),
            null: false,
        },
    }
}

fn one_line(text: &str) -> String {
    text.replace(['\n', '\r', '\t'], " ")
}

impl Render for DataView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.render_body(window, cx);
        div()
            .key_context("ExploreData")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Next, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &Previous, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &Close, _, cx| {
                if this.selected.take().is_some() {
                    cx.notify()
                } else {
                    cx.propagate()
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .child(body)
    }
}

impl TableSource for DataView {
    type Key = SharedString;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a Row;
    fn table_state(&self) -> &TableState {
        &self.table
    }
    fn columns(&self) -> &[Column] {
        &self.columns
    }
    fn width(&self) -> f32 {
        self.columns.iter().map(|c| c.width).sum()
    }
    fn list_label(&self) -> String {
        "The table's first rows, read-only; arrows select a row".into()
    }
    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }
    fn sort(&mut self, _: (), _: &mut Context<Self>) {}
    fn line_count(&self) -> usize {
        self.rows.len()
    }
    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, &Row>> {
        let row = self.rows.get(line)?;
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
        let ix = self
            .columns
            .iter()
            .position(|c| c.label == column.label)
            .unwrap_or_default();
        let cell = &row.data.cells[ix];
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
        self.rows.iter().position(|r| &r.key == key)
    }
    fn click(
        &mut self,
        key: &SharedString,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose(key.clone(), cx);
        window.focus(&self.focus, cx);
    }
    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
    fn empty(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.rows.is_empty() {
            return None;
        }
        let words = if self.target.is_none() {
            "Choose a table to read its rows.".to_owned()
        } else if let Some(error) = self.error() {
            error
        } else if self.page.is_loading() {
            "Reading rows…".to_owned()
        } else {
            "This table has no rows.".to_owned()
        };
        Some(
            div()
                .id("explore-data-note")
                .aria_label(words.clone())
                .test_support()
                .p(dp(12.))
                .text_size(dp(12.))
                .text_color(palette(cx).muted)
                .child(words)
                .into_any_element(),
        )
    }
}

impl DataView {
    pub(super) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let on_show = freshkube_reads::OnShow::Never;
        self.info.set_visible(visible, on_show, cx);
        self.page.set_visible(visible, on_show, cx);
        if visible && self.target.is_some() && self.page.data().is_none() && !self.page.is_loading()
        {
            self.reload(cx);
        }
    }
}
