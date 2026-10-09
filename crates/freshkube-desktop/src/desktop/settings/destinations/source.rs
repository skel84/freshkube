//! The destinations table as a `TableSource`.
use super::*;
use table::{Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, TableState};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Glyph,
    Destination,
    By,
    Entry,
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
        self.field == Field::Entry
    }

    fn pinned(&self) -> bool {
        matches!(self.field, Field::Glyph | Field::Destination)
    }
}

/// DESIGN.md's widths: 7.5 a character plus 24, between 64 and 360 for a
/// server URL.
fn fit<'a>(label: &str, texts: impl Iterator<Item = &'a SharedString>, most: f32) -> f32 {
    let chars = texts
        .map(|text| text.chars().count())
        .chain([label.len()])
        .max()
        .unwrap_or_default();
    (chars as f32 * 7.5 + 24.).clamp(64., most)
}

pub(super) fn columns(rows: &[DestinationRow]) -> (Vec<Column>, f32) {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    let columns = vec![
        column(Field::Glyph, "", table::GLYPH_WIDTH),
        column(
            Field::Destination,
            "Destination",
            fit("Destination", rows.iter().map(|row| &row.destination), 360.),
        ),
        column(Field::By, "Match by", 120.),
        column(
            Field::Entry,
            "Workspace cluster",
            fit("Workspace cluster", rows.iter().map(|row| &row.entry), 240.),
        ),
    ];
    let width = columns.iter().map(|column| column.width).sum();
    (columns, width)
}

impl TableSource for Destinations {
    type Key = Key;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a DestinationRow;

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
        "Argo CD destinations and the workspace clusters they are".into()
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.rows.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<Key, &DestinationRow>> {
        let row = self.rows.get(line)?;
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: row.id.clone().into(),
            label: format!("{} · {} · {}", row.destination, row.by, row.entry).into(),
            tooltip: Some(row.tooltip.clone()),
            marked: false,
            muted: false,
            data: row,
        }))
    }

    fn cell(
        &self,
        line: &TableRow<Key, &DestinationRow>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let cell = table::cell(column);
        match column.field {
            Field::Glyph => table::glyph_cell(column)
                .when(!row.listed, |this| {
                    this.child(ui::status_mark(
                        row.mark_id.clone(),
                        ui::Tone::Unknown,
                        row.mark.clone(),
                        cx,
                    ))
                })
                .into_any_element(),
            Field::Destination => cell.child(row.destination.clone()).into_any_element(),
            Field::By => cell.child(row.by.clone()).into_any_element(),
            Field::Entry => cell
                .when(!row.listed, |this| this.text_color(style.p.muted))
                .child(row.entry.clone())
                .into_any_element(),
        }
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn selected_key(&self) -> Option<&Key> {
        self.selected.as_ref()
    }

    fn line_of(&self, key: &Key) -> Option<usize> {
        self.rows.iter().position(|row| &row.key == key)
    }

    fn click(&mut self, key: &Key, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.select(key.clone(), cx);
        self.focus(window, cx);
    }

    fn menu_focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    /// A right-click selects the row; its menu is the toolbar's actions on
    /// it, with their keys.
    fn row_menu(
        &mut self,
        key: &Key,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<menu::MenuAction> {
        self.select(key.clone(), cx);
        self.focus(window, cx);
        vec![
            menu::MenuAction::new("Edit", EditDestination).enabled(self.mappable()),
            menu::MenuAction::new("Remove", RemoveDestination).enabled(self.editable),
        ]
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        if !self.rows.is_empty() {
            return None;
        }
        Some(
            "No destinations are mapped. An Argo CD destination that isn’t mapped stays Unknown \
             on the change page."
                .into_any_element(),
        )
    }
}
