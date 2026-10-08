//! The audit as a `TableSource`: its columns, sized from the rows when they
//! change, the group rows and the cells of each row.
use super::*;
use audit::{Entry, RowText};
use freshkube_ui::table::{
    self as kit, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Glyph,
    Status,
    Name,
    Summary,
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
        self.field == Field::Summary
    }

    /// The glyph and status stay in view when the table scrolls sideways.
    fn pinned(&self) -> bool {
        matches!(self.field, Field::Glyph | Field::Status)
    }
}

/// DESIGN.md's widths: 7.5 a character plus 24, between 64 and 280.
fn fit<'a>(label: &str, texts: impl Iterator<Item = &'a str>) -> f32 {
    let chars = texts
        .map(|text| text.chars().count())
        .chain([label.chars().count()])
        .max()
        .unwrap_or_default();
    (chars as f32 * 7.5 + 24.).clamp(64., 280.)
}

/// The columns for these rows, and their total width.
pub(super) fn columns(items: &[Item]) -> (Vec<Column>, f32) {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    let columns = vec![
        column(Field::Glyph, "", kit::GLYPH_WIDTH),
        column(
            Field::Status,
            "Status",
            fit("Status", items.iter().map(|item| item.status.as_str())),
        ),
        column(
            Field::Name,
            "Name",
            fit("Name", items.iter().map(|item| item.name.as_str())),
        ),
        column(Field::Summary, "Summary", 240.),
    ];
    let width = columns.iter().map(|column| column.width).sum();
    (columns, width)
}

impl TableSource for SecurityScreen {
    type Key = SharedString;
    type Sort = ();
    type Column = Column;
    type Row<'a> = (&'a Item, &'a RowText);

    fn table_state(&self) -> &TableState {
        &self.table
    }

    fn columns(&self) -> &[Column] {
        &self.columns.0
    }

    fn width(&self) -> f32 {
        self.columns.1
    }

    fn list_label(&self) -> String {
        "Certificates, RBAC role and volume encryption; arrows select an item".into()
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.display.1.lines.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, (&Item, &RowText)>> {
        let ix = match *self.display.1.lines.get(line)? {
            Entry::Group(group) => return Some(Line::Group(group)),
            Entry::Row(ix) => ix,
        };
        let item = self.items().get(ix)?;
        let text = self.display.1.rows.get(ix)?;
        Some(Line::Row(TableRow {
            key: item.key.clone(),
            id: text.id.clone().into(),
            label: text.label.clone(),
            tooltip: None,
            marked: false,
            muted: false,
            data: (item, text),
        }))
    }

    fn cell(
        &self,
        row: &TableRow<SharedString, (&Item, &RowText)>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (item, text) = row.data;
        let cell = kit::cell(column);
        match column.field {
            Field::Glyph => kit::glyph_cell(column)
                .child(ui::status_mark(
                    text.mark.clone(),
                    item.verdict.glyph(),
                    item.status.clone(),
                    cx,
                ))
                .into_any_element(),
            Field::Status => cell.child(item.status.clone()).into_any_element(),
            Field::Name => cell.child(item.name.clone()).into_any_element(),
            Field::Summary => cell
                .text_color(style.p.muted)
                .child(item.summary.clone())
                .into_any_element(),
        }
    }

    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let line = self.display.1.groups.get(group)?;
        Some(
            kit::GroupRow::new(
                self.table.id(&format!("group-{}", line.group.slug())),
                line.verdict.glyph(),
                line.group.label(),
                kit::ROW_HEIGHT,
            )
            .detail(line.detail.clone())
            .render(cx)
            .into_any_element(),
        )
    }

    fn selected_key(&self) -> Option<&SharedString> {
        self.selected.as_ref().map(|selection| &selection.key)
    }

    fn line_of(&self, key: &SharedString) -> Option<usize> {
        let items = self.items();
        (self.display.1.lines.iter())
            .position(|entry| matches!(entry, Entry::Row(ix) if &items[*ix].key == key))
    }

    /// A click selects the row and puts the keyboard on the list.
    fn click(
        &mut self,
        key: &SharedString,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select(key.clone(), cx);
        window.focus(&self.focus, cx);
    }

    fn loading(&self) -> Option<&freshkube_ui::table::LoadingRows> {
        self.loading.rows()
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        self.display
            .1
            .lines
            .is_empty()
            .then(|| "The audit reported nothing.".into_any_element())
    }
}
