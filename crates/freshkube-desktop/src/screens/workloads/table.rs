//! The workloads list as a `TableSource`: the rows' display text, derived
//! with the rows when the data or a filter changes, and the table's columns,
//! sized from them.
use super::*;
use freshkube_ui::table::{
    self, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, TableState,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Glyph,
    Name,
    Kind,
    Ready,
    Issue,
}

#[derive(Clone, Debug)]
pub(crate) struct Column {
    field: Field,
    label: SharedString,
    width: f32,
}

impl TableColumn for Column {
    fn label(&self) -> &SharedString {
        &self.label
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn flexible(&self) -> bool {
        self.field == Field::Name
    }

    /// The glyph and name stay in view when the table scrolls sideways.
    fn pinned(&self) -> bool {
        matches!(self.field, Field::Glyph | Field::Name)
    }
}

/// One row as the table draws it, derived with the rows.
#[derive(Clone, Debug)]
pub(crate) struct DisplayRow {
    key: ItemKey,
    pub(super) element_id: SharedString,
    glyph_id: SharedString,
    /// The row's accessibility label.
    label: SharedString,
    health_label: &'static str,
    view: RowView,
}

/// The rows to draw, their columns and the columns' total width.
#[derive(Clone, Debug, Default)]
pub(crate) struct Display {
    pub(super) lines: Vec<DisplayRow>,
    pub(super) columns: Vec<Column>,
    pub(super) width: f32,
}

/// A nested row's indent and a namespace's chevron take about this many
/// characters of the Name column.
const NAME_EXTRA_CHARS: usize = 5;
/// The widest the Name column grows; a longer name truncates and the row's
/// tooltip holds it.
const NAME_WIDTH: f32 = 280.;

/// DESIGN.md's widths: 7.5 a character plus 24, between 64 and 280.
fn fit(label: &str, texts: impl Iterator<Item = usize>) -> f32 {
    let chars = texts
        .chain([label.chars().count()])
        .max()
        .unwrap_or_default();
    (chars as f32 * 7.5 + 24.).clamp(64., 280.)
}

/// An id for the row, from what identifies it, so a press that lands after
/// the rows moved can't complete on another object.
fn element_id(key: &ItemKey) -> String {
    match key {
        ItemKey::Namespace(name) => format!("workload-row-namespace-{name}"),
        ItemKey::Workload {
            namespace,
            name,
            kind,
        } => format!("workload-row-{}-{namespace}-{name}", kind.label()),
        ItemKey::Pod { namespace, name } => format!("workload-row-pod-{namespace}-{name}"),
    }
}

impl WorkloadsScreen {
    /// The rows' display text and the columns that fit them.
    pub(super) fn derive_display(&self, rows: &[RowRef], data: &WorkloadData) -> Display {
        let lines: Vec<DisplayRow> = rows
            .iter()
            .map(|row| {
                let key = row.key(&data.snapshot);
                let view = self.describe(*row, data);
                let element_id = element_id(&key);
                DisplayRow {
                    label: format!(
                        "{} {} · {} · {} · {}",
                        view.kind,
                        view.name,
                        health_label(view.health),
                        view.ready,
                        view.issue
                    )
                    .into(),
                    glyph_id: format!("{element_id}-health").into(),
                    element_id: element_id.into(),
                    health_label: health_label(view.health),
                    key,
                    view,
                }
            })
            .collect();
        let column = |field, label: &str, width| Column {
            field,
            label: label.to_owned().into(),
            width,
        };
        let widest = |label: &str, text: &dyn Fn(&RowView) -> usize| {
            fit(label, lines.iter().map(|line| text(&line.view)))
        };
        let columns = vec![
            column(Field::Glyph, "", table::GLYPH_WIDTH),
            column(
                Field::Name,
                "Name",
                widest("Name", &|view| view.name.chars().count() + NAME_EXTRA_CHARS)
                    .min(NAME_WIDTH),
            ),
            column(
                Field::Kind,
                "Kind",
                widest("Kind", &|view| view.kind.chars().count()),
            ),
            column(
                Field::Ready,
                "Ready / restarts",
                widest("Ready / restarts", &|view| view.ready.chars().count()),
            ),
            column(
                Field::Issue,
                "Issue",
                widest("Issue", &|view| view.issue.chars().count()),
            ),
        ];
        let width = columns.iter().map(|column| column.width).sum();
        Display {
            lines,
            columns,
            width,
        }
    }
}

impl TableSource for WorkloadsScreen {
    type Key = ItemKey;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a DisplayRow;

    fn table_state(&self) -> &TableState {
        &self.table
    }

    fn columns(&self) -> &[Column] {
        &self.display.columns
    }

    fn width(&self) -> f32 {
        self.display.width
    }

    fn list_label(&self) -> String {
        "Namespaces, workloads and pods needing attention; arrows select, Enter opens or closes a namespace, U shows only unhealthy".into()
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.display.lines.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<ItemKey, &DisplayRow>> {
        let row = self.display.lines.get(line)?;
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: row.element_id.clone().into(),
            label: row.label.clone(),
            tooltip: Some(row.view.name.clone().into()),
            marked: false,
            muted: false,
            data: row,
        }))
    }

    fn cell(
        &self,
        line: &TableRow<ItemKey, &DisplayRow>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let view = &row.view;
        let cell = table::cell(column);
        match column.field {
            Field::Glyph => table::glyph_cell(column)
                .child(ui::status_mark(
                    row.glyph_id.clone(),
                    view.tone,
                    row.health_label,
                    cx,
                ))
                .into_any_element(),
            Field::Name => cell
                .flex()
                .items_center()
                .gap_1p5()
                .when(view.nested, |this| this.pl(dp(28.)))
                .when(view.chevron.is_some(), |this| {
                    this.font_weight(FontWeight::SEMIBOLD)
                })
                .children(view.chevron.map(|chevron| Icon::new(chevron).size(dp(13.))))
                // Without `min_w_0` a long pod name widens the column and
                // shifts every later cell in its row.
                .child(div().flex_1().min_w_0().truncate().child(view.name.clone()))
                .into_any_element(),
            Field::Kind => cell
                .text_color(style.p.muted)
                .child(view.kind)
                .into_any_element(),
            Field::Ready => cell.child(view.ready.clone()).into_any_element(),
            Field::Issue => cell
                .when(!style.selected, |this| this.text_color(style.p.muted))
                .child(view.issue.clone())
                .into_any_element(),
        }
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn selected_key(&self) -> Option<&ItemKey> {
        self.selected.as_ref()
    }

    fn line_of(&self, key: &ItemKey) -> Option<usize> {
        self.display.lines.iter().position(|row| &row.key == key)
    }

    /// A click selects; a second click on a namespace opens or closes it.
    fn click(
        &mut self,
        key: &ItemKey,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_selected = self.selected.as_ref() == Some(key);
        self.select(key.clone(), cx);
        if matches!(key, ItemKey::Namespace(_)) && was_selected {
            self.toggle_expanded(cx);
        }
        window.focus(&self.focus, cx);
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        if !self.display.lines.is_empty() {
            return None;
        }
        Some(
            if self
                .loader
                .data()
                .is_some_and(|data| data.snapshot.namespaces.is_empty())
            {
                "No workloads found in this cluster."
            } else {
                "No workloads match these filters."
            }
            .into_any_element(),
        )
    }
}
