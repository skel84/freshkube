use super::*;
use display::{Entry, Row};
use freshkube_core::access::COLUMNS;
use freshkube_ui::table::{
    self as kit, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource,
};

pub(crate) struct Column {
    /// The logical cell it draws: 0 is the name, 1.. the access columns;
    /// none for the glyph column.
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

pub(super) fn columns(rows: &[Row]) -> (Vec<Column>, f32) {
    let mut columns = vec![Column {
        cell: None,
        label: "".into(),
        width: kit::GLYPH_WIDTH,
        flexible: false,
        pinned: true,
    }];
    columns.push(Column {
        cell: Some(0),
        label: "Namespace".into(),
        width: kit::fit("Namespace", rows.iter().map(|row| &row.label), kit::WIDEST),
        flexible: false,
        pinned: true,
    });
    let last = COLUMNS.len() - 1;
    for (ix, column) in COLUMNS.iter().enumerate() {
        let label: SharedString = column.label().into();
        let flexible = ix == last;
        // The words are short; the label sets the width, with room to read.
        let width = kit::fit(
            &label,
            std::iter::empty(),
            if flexible {
                kit::WIDEST_FLEXIBLE
            } else {
                kit::WIDEST
            },
        )
        .max(76.);
        columns.push(Column {
            cell: Some(ix + 1),
            label,
            width,
            flexible,
            pinned: false,
        });
    }
    let width = columns.iter().map(|c| c.width).sum();
    (columns, width)
}

impl TableSource for AccessScreen {
    type Key = SharedString;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a Row;

    fn table_state(&self) -> &TableState {
        &self.table
    }
    fn columns(&self) -> &[Column] {
        &self.display.columns.0
    }
    fn width(&self) -> f32 {
        self.display.columns.1
    }
    fn list_label(&self) -> String {
        "What you may do in each namespace; arrows select, C copies an access request, R rechecks"
            .into()
    }
    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }
    fn sort(&mut self, _: (), _: &mut Context<Self>) {}
    fn line_count(&self) -> usize {
        self.display.lines.len()
    }
    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, &Row>> {
        let ix = match *self.display.lines.get(line)? {
            Entry::Group(group) => return Some(Line::Group(group)),
            Entry::Row(ix) => ix,
        };
        let row = self.display.rows.get(ix)?;
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: row.id.clone().into(),
            label: row.label.clone(),
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
        let data = row.data;
        let Some(cell) = column.cell else {
            // A row the frame draws is a row to ask about.
            self.note_shown(&data.row_key, cx);
            return kit::glyph_cell(column)
                .child(ui::status_mark(
                    format!("{}-state", data.id),
                    data.tone,
                    data.state.clone(),
                    cx,
                ))
                .into_any_element();
        };
        let id = format!("{}-text-{}", data.id, cell + 1);
        if cell == 0 {
            return kit::word_cell(column, id, data.label.clone()).into_any_element();
        }
        let view = &data.cells[cell - 1];
        use freshkube_ui::tooltip::FollowTooltip as _;
        kit::cell(column)
            .id(SharedString::from(format!("{id}-cell")))
            .follow_tooltip(view.tip.clone())
            .child(kit::word_cut(id, view.word))
            .into_any_element()
    }
    fn selected_key(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }
    fn line_of(&self, key: &SharedString) -> Option<usize> {
        self.display
            .lines
            .iter()
            .position(|entry| matches!(entry, Entry::Row(ix) if &self.display.rows[*ix].key == key))
    }
    fn click(
        &mut self,
        key: &SharedString,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose(key.clone(), cx);
        window.focus(&self.focus, cx)
    }
    fn menu_focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }
    fn row_menu(
        &mut self,
        key: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<freshkube_ui::menu::MenuAction> {
        self.choose(key.clone(), cx);
        window.focus(&self.focus, cx);
        vec![
            freshkube_ui::menu::MenuAction::new("Copy as access request", CopyRequest)
                .enabled(self.requestable().is_some()),
            freshkube_ui::menu::MenuAction::new("Recheck", Recheck)
                .enabled(self.selected.is_some()),
        ]
    }
    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let line = self.display.groups.get(group)?;
        Some(
            kit::GroupRow::new(
                self.table.id(&format!("group-{}", line.slug)),
                ui::Tone::Outline,
                line.label,
                kit::ROW_HEIGHT,
            )
            .detail(vec![line.count.to_string()])
            .render(cx)
            .into_any_element(),
        )
    }
    fn loading(&self) -> Option<&kit::LoadingRows> {
        self.loading.rows()
    }
    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
}
