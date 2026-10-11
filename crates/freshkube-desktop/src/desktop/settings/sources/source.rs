//! The delivery sources table as a `TableSource`.
use super::*;
use table::{Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, TableState};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Glyph,
    Part,
    Entry,
    Why,
    State,
    Reads,
}

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
        self.field == Field::Reads
    }

    fn pinned(&self) -> bool {
        matches!(self.field, Field::Glyph | Field::Part)
    }
}

/// DESIGN.md's widths: 7.5 a character plus 24, between 64 and `most`.
fn fit<'a>(label: &str, texts: impl Iterator<Item = &'a SharedString>, most: f32) -> f32 {
    let chars = texts
        .map(|text| text.chars().count())
        .chain([label.len()])
        .max()
        .unwrap_or_default();
    (chars as f32 * 7.5 + 24.).clamp(64., most)
}

pub(super) fn columns(rows: &[SourceRow]) -> (Vec<Column>, f32) {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    let columns = vec![
        column(Field::Glyph, "", table::GLYPH_WIDTH),
        column(Field::Part, "Part", 96.),
        column(
            Field::Entry,
            "Entry",
            fit("Entry", rows.iter().map(|row| &row.entry), 200.),
        ),
        column(
            Field::Why,
            "Chosen by",
            fit("Chosen by", rows.iter().map(|row| &row.why), 200.),
        ),
        column(Field::State, "State", 160.),
        column(
            Field::Reads,
            "Reads",
            fit("Reads", rows.iter().map(|row| &row.reads), 400.),
        ),
    ];
    let width = columns.iter().map(|column| column.width).sum();
    (columns, width)
}

impl TableSource for SourcesList {
    type Key = Part;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a SourceRow;

    fn table_state(&self) -> &TableState {
        &self.table
    }

    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn list_label(&self) -> String {
        "Where each part of the delivery chain is read".into()
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.rows.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<Part, &SourceRow>> {
        let row = self.rows.get(line)?;
        Some(Line::Row(TableRow {
            key: row.part,
            id: row.id.clone().into(),
            label: format!(
                "{} · {} · {} · {}",
                row.label, row.entry, row.why, row.state
            )
            .into(),
            tooltip: Some(row.tooltip.clone()),
            marked: false,
            muted: false,
            data: row,
        }))
    }

    fn cell(
        &self,
        line: &TableRow<Part, &SourceRow>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let cell = table::cell(column);
        match column.field {
            Field::Glyph => table::glyph_cell(column)
                .when_some(row.tone, |this, tone| {
                    this.child(ui::status_mark(
                        row.mark_id.clone(),
                        tone,
                        row.state.clone(),
                        cx,
                    ))
                })
                .into_any_element(),
            Field::Part => cell.child(row.label.clone()).into_any_element(),
            Field::Entry => cell
                .when(!row.listed, |this| this.text_color(style.p.muted))
                .child(row.entry.clone())
                .into_any_element(),
            Field::Why => cell.child(row.why.clone()).into_any_element(),
            Field::State => cell
                .when(row.tone.is_none(), |this| this.text_color(style.p.muted))
                .child(row.state.clone())
                .into_any_element(),
            Field::Reads => cell
                .text_color(style.p.muted)
                .child(row.reads.clone())
                .into_any_element(),
        }
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    /// The rows are read, not selected.
    fn selected_key(&self) -> Option<&Part> {
        None
    }

    fn line_of(&self, key: &Part) -> Option<usize> {
        self.rows.iter().position(|row| &row.part == key)
    }

    fn click(&mut self, _: &Part, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus(window, cx);
    }

    fn menu_focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    /// A right-click offers the toolbar's one action.
    fn row_menu(
        &mut self,
        _: &Part,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<menu::MenuAction> {
        self.focus(window, cx);
        vec![
            menu::MenuAction::new("Check sources", CheckSources)
                .enabled(self.check != Check::Running),
        ]
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
}
