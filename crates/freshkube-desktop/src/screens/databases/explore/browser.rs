//! The schema browser: schemas as groups of the shared table, their tables
//! as rows. A schema's tables are read when it is first opened, never
//! before; the sizes and row counts are the server's estimates.
use super::reading::read;
use super::{Shared, session_of};
use crate::ui::{self, dp};
use freshkube_db::{Listed, TableEntry, TableKind};
use freshkube_reads::Loader;
use freshkube_ui::table::{
    self as kit, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, TableState,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::*;
use std::collections::{BTreeSet, HashMap};
use tokio::runtime::Handle;

pub(super) const PREFIX: &str = "explore-browser";

actions!(explore_browser, [Next, Previous]);

/// A table was chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Open {
    pub(super) schema: String,
    pub(super) table: String,
}

pub(super) struct Column {
    /// The cell it draws; none for the glyph column that lines the rows up
    /// under the schemas' group rows.
    cell: Option<usize>,
    label: SharedString,
    width: f32,
    flexible: bool,
    pinned: bool,
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
        self.pinned
    }
}

pub(super) struct Row {
    key: SharedString,
    id: SharedString,
    schema: String,
    name: String,
    cells: [SharedString; 3],
}

enum Entry {
    Group(usize),
    Row(usize),
}

struct Group {
    schema: String,
    slug: String,
    open: bool,
    /// "5 tables", or why there are none to show yet.
    detail: String,
}

pub(super) struct Browser {
    runtime: Handle,
    session: Option<(u64, Shared)>,
    epoch: u64,
    schemas: Loader<Listed<String>, u64>,
    tables: HashMap<String, Loader<Listed<TableEntry>, u64>>,
    expanded: BTreeSet<String>,
    groups: Vec<Group>,
    rows: Vec<Row>,
    lines: Vec<Entry>,
    /// The revisions the lines were prepared from.
    prepared: u64,
    revision: u64,
    columns: Vec<Column>,
    selected: Option<SharedString>,
    table: TableState,
    focus: FocusHandle,
}
impl EventEmitter<Open> for Browser {}

impl Browser {
    pub(super) fn new(runtime: Handle, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", Next, Some("ExploreBrowser")),
            KeyBinding::new("up", Previous, Some("ExploreBrowser")),
        ]);
        Self {
            runtime,
            session: None,
            epoch: 0,
            schemas: Loader::default(),
            tables: HashMap::new(),
            expanded: BTreeSet::new(),
            groups: Vec::new(),
            rows: Vec::new(),
            lines: Vec::new(),
            prepared: u64::MAX,
            revision: 0,
            columns: vec![
                Column {
                    cell: None,
                    label: "".into(),
                    width: kit::GLYPH_WIDTH,
                    flexible: false,
                    pinned: true,
                },
                Column {
                    cell: Some(0),
                    label: "Table".into(),
                    width: 140.,
                    flexible: true,
                    pinned: false,
                },
                Column {
                    cell: Some(1),
                    label: "Rows ≈".into(),
                    width: 84.,
                    flexible: false,
                    pinned: false,
                },
                Column {
                    cell: Some(2),
                    label: "Size".into(),
                    width: 88.,
                    flexible: false,
                    pinned: false,
                },
            ],
            selected: None,
            table: TableState::new(PREFIX),
            focus: cx.focus_handle(),
        }
    }

    /// A new connection, or none: everything read before is another
    /// connection's and goes.
    pub(super) fn set_session(&mut self, session: Option<Shared>, cx: &mut Context<Self>) {
        self.epoch += 1;
        self.schemas.reset();
        self.tables.clear();
        self.expanded.clear();
        self.selected = None;
        self.revision += 1;
        self.session = session.map(|s| (self.epoch, s));
        if self.session.is_some() {
            self.read_schemas(cx);
        }
        cx.notify();
    }

    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        let session = self.session.as_ref().map(|(_, s)| s.clone());
        self.set_session(session, cx);
    }

    #[cfg(test)]
    /// Whether the schema's tables were asked for, and whether they landed.
    pub(super) fn tables_of(&self, schema: &str) -> Option<bool> {
        self.tables.get(schema).map(|l| l.data().is_some())
    }

    pub(super) fn is_loading(&self) -> bool {
        self.schemas.is_loading() || self.tables.values().any(|t| t.is_loading())
    }

    fn read_schemas(&mut self, cx: &mut Context<Self>) {
        let Some((epoch, session)) = self.session.clone() else {
            return;
        };
        let runtime = self.runtime.clone();
        read(
            self,
            |this: &mut Self| &mut this.schemas,
            epoch,
            &runtime,
            "the schemas",
            async move {
                session_of(&session)
                    .await
                    .schemas()
                    .await
                    .map_err(|e| e.to_string())
            },
            |this, cx| {
                // Open the schema people mean first; the rest wait for a click.
                let first = this.schemas.data().and_then(|s| {
                    s.items
                        .iter()
                        .find(|s| *s == "public")
                        .or_else(|| s.items.first())
                        .cloned()
                });
                if let Some(first) = first {
                    this.expand(first, cx);
                }
            },
            cx,
        );
    }

    fn expand(&mut self, schema: String, cx: &mut Context<Self>) {
        self.expanded.insert(schema.clone());
        self.revision += 1;
        if !self.tables.contains_key(&schema) {
            self.read_tables(schema, cx);
        }
        cx.notify();
    }

    fn read_tables(&mut self, schema: String, cx: &mut Context<Self>) {
        let Some((epoch, session)) = self.session.clone() else {
            return;
        };
        self.tables.entry(schema.clone()).or_default();
        let runtime = self.runtime.clone();
        let (key, name) = (schema.clone(), schema);
        read(
            self,
            move |this: &mut Self| this.tables.entry(key.clone()).or_default(),
            epoch,
            &runtime,
            "the tables",
            async move {
                session_of(&session)
                    .await
                    .tables(&name)
                    .await
                    .map_err(|e| e.to_string())
            },
            |this, _| this.revision += 1,
            cx,
        );
    }

    fn toggle(&mut self, schema: &str, cx: &mut Context<Self>) {
        if self.expanded.remove(schema) {
            self.revision += 1;
            cx.notify();
        } else {
            self.expand(schema.to_owned(), cx);
        }
    }

    /// Opens the table under the selection, when one is chosen.
    pub(super) fn choose(&mut self, key: SharedString, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter().find(|r| r.key == key) else {
            return;
        };
        cx.emit(Open {
            schema: row.schema.clone(),
            table: row.name.clone(),
        });
        self.selected = Some(key);
        cx.notify();
    }

    pub(super) fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.prepare();
        if let Some(key) = kit::step(self, delta, cx) {
            self.choose(key, cx);
            kit::reveal(self, ScrollStrategy::Nearest);
        }
    }

    /// Prepares lines when what they come from changed; not while drawing.
    pub(super) fn prepare(&mut self) {
        let now = self.revision + self.schemas.revision();
        let tables: u64 = self.tables.values().map(|t| t.revision()).sum();
        let now = now + tables;
        if now == self.prepared {
            return;
        }
        self.prepared = now;
        self.groups.clear();
        self.rows.clear();
        self.lines.clear();
        let schemas = self
            .schemas
            .data()
            .map(|l| l.items.clone())
            .unwrap_or_default();
        for schema in schemas {
            let open = self.expanded.contains(&schema);
            let loader = self.tables.get(&schema);
            let detail = match loader {
                _ if !open => "closed".to_owned(),
                Some(l) if l.error().is_some() => "couldn't read".to_owned(),
                Some(l) => match l.data() {
                    Some(t) => format!(
                        "{}{} table{}{}",
                        t.items.len(),
                        if t.capped { "+" } else { "" },
                        if t.items.len() == 1 { "" } else { "s" },
                        if t.capped { " (capped)" } else { "" }
                    ),
                    None => "reading…".to_owned(),
                },
                None => "reading…".to_owned(),
            };
            let group = self.groups.len();
            self.lines.push(Entry::Group(group));
            self.groups.push(Group {
                slug: schema.to_lowercase(),
                schema: schema.clone(),
                open,
                detail,
            });
            if !open {
                continue;
            }
            for table in loader
                .and_then(|l| l.data())
                .into_iter()
                .flat_map(|l| &l.items)
            {
                let ix = self.rows.len();
                self.lines.push(Entry::Row(ix));
                self.rows.push(Row {
                    key: format!("{schema}/{}", table.name).into(),
                    id: format!("explore-table-{}-{}", schema, table.name).into(),
                    schema: schema.clone(),
                    name: table.name.clone(),
                    cells: [
                        match table.kind {
                            TableKind::Table => table.name.clone(),
                            other => format!("{} ({})", table.name, kind_word(other)),
                        }
                        .into(),
                        table.rows_estimate.map_or("–".to_owned(), count).into(),
                        table.size_bytes.map_or("–".to_owned(), size).into(),
                    ],
                });
            }
        }
    }

    fn failure(&self) -> Option<String> {
        self.schemas.error().map(ToString::to_string)
    }
}

fn kind_word(kind: TableKind) -> &'static str {
    match kind {
        TableKind::Table => "table",
        TableKind::View => "view",
        TableKind::Materialized => "mat. view",
        TableKind::Foreign => "foreign",
        TableKind::Partitioned => "partitioned",
    }
}

fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024. && unit < UNITS.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

impl TableSource for Browser {
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
        "Schemas and tables; arrows select a table, Enter opens a schema".into()
    }
    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }
    fn sort(&mut self, _: (), _: &mut Context<Self>) {}
    fn line_count(&self) -> usize {
        self.lines.len()
    }
    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, &Row>> {
        match *self.lines.get(line)? {
            Entry::Group(group) => Some(Line::Group(group)),
            Entry::Row(ix) => {
                let row = self.rows.get(ix)?;
                Some(Line::Row(TableRow {
                    key: row.key.clone(),
                    id: row.id.clone().into(),
                    label: row.name.clone().into(),
                    tooltip: None,
                    marked: false,
                    muted: false,
                    data: row,
                }))
            }
        }
    }
    fn cell(
        &self,
        row: &TableRow<SharedString, &Row>,
        _: &RowStyle,
        column: &Column,
        _: &mut Context<Self>,
    ) -> AnyElement {
        let Some(ix) = column.cell else {
            return kit::glyph_cell(column).into_any_element();
        };
        kit::word_cell(
            column,
            format!("{}-text-{}", row.data.id, ix + 1),
            row.data.cells[ix].clone(),
        )
        .into_any_element()
    }
    fn selected_key(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }
    fn line_of(&self, key: &SharedString) -> Option<usize> {
        self.lines
            .iter()
            .position(|entry| matches!(entry, Entry::Row(ix) if &self.rows[*ix].key == key))
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
    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let line = self.groups.get(group)?;
        let (open, toggled) = (line.schema.clone(), line.schema.clone());
        let chevron = Button::new(SharedString::from(format!(
            "explore-schema-{}-toggle",
            line.slug
        )))
        .ghost()
        .xsmall()
        .icon(if line.open {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        })
        .tooltip(if line.open {
            "Close schema"
        } else {
            "Open schema"
        })
        .accessibility_label(format!(
            "{} schema {}",
            if line.open { "Close" } else { "Open" },
            line.schema
        ))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.toggle(&toggled, cx)
        }));
        Some(
            div()
                .id(SharedString::from(format!("explore-schema-{}", line.slug)))
                .test_support()
                .aria_label(format!("Schema {} · {}", line.schema, line.detail))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.toggle(&open, cx)))
                .child(
                    kit::GroupRow::new(
                        self.table.id(&format!("group-{}", line.slug)),
                        ui::Tone::Outline,
                        line.schema.clone(),
                        kit::ROW_HEIGHT,
                    )
                    .detail(vec![line.detail.clone()])
                    .chevron(chevron)
                    .render(cx),
                )
                .into_any_element(),
        )
    }
    fn empty(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.lines.is_empty() {
            return None;
        }
        let words = match self.failure() {
            Some(error) => error,
            None if self.schemas.is_loading() => "Reading schemas…".into(),
            None => "This database has no schemas you can see.".into(),
        };
        Some(
            div()
                .id("explore-browser-note")
                .aria_label(words.clone())
                .test_support()
                .p(dp(12.))
                .text_size(dp(12.))
                .text_color(freshkube_ui::palette::palette(cx).muted)
                .child(words)
                .into_any_element(),
        )
    }
}

impl Render for Browser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.prepare();
        div()
            .key_context("ExploreBrowser")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Next, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &Previous, _, cx| this.step(-1, cx)))
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .child(
                div()
                    .id("explore-browser")
                    .test_support()
                    .size_full()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .min_h_0()
                    .child(
                        freshkube_ui::table::DataTable::new()
                            .render(self, window, cx)
                            .flex_1()
                            .min_h_0(),
                    ),
            )
    }
}

impl Browser {
    /// Shown again: what hiding cut short is read again.
    pub(super) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let on_show = freshkube_reads::OnShow::Never;
        self.schemas.set_visible(visible, on_show, cx);
        for loader in self.tables.values_mut() {
            loader.set_visible(visible, on_show, cx);
        }
        if !visible || self.session.is_none() {
            return;
        }
        if self.schemas.data().is_none() && !self.schemas.is_loading() {
            self.read_schemas(cx);
        }
        let again: Vec<String> = self
            .expanded
            .iter()
            .filter(|s| {
                self.tables
                    .get(*s)
                    .is_none_or(|l| l.data().is_none() && !l.is_loading())
            })
            .cloned()
            .collect();
        for schema in again {
            self.read_tables(schema, cx);
        }
    }
}
