use super::*;
use display::{Entry, Row};
use freshkube_ui::table::{
    self as kit, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource,
};

pub(crate) struct Column {
    /// The logical cell it draws; none for the glyph column.
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

const LABELS: [&str; 4] = ["Route", "Name", "Gateway", "Status"];

pub(super) fn columns(rows: &[Row]) -> (Vec<Column>, f32) {
    let mut columns = vec![Column {
        cell: None,
        label: "".into(),
        width: kit::GLYPH_WIDTH,
        flexible: false,
        pinned: true,
    }];
    for (cell, label) in LABELS.iter().enumerate() {
        let flexible = cell == LABELS.len() - 1;
        columns.push(Column {
            cell: Some(cell),
            label: (*label).into(),
            width: kit::fit(
                label,
                rows.iter().map(|row| &row.cells[cell]),
                if flexible {
                    kit::WIDEST_FLEXIBLE
                } else {
                    kit::WIDEST
                },
            ),
            flexible,
            pinned: cell == 0,
        });
    }
    let width = columns.iter().map(|c| c.width).sum();
    (columns, width)
}

impl TableSource for RoutesScreen {
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
        "HTTPRoutes by verdict; arrows select, Enter opens the trace".into()
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
            return kit::glyph_cell(column)
                .child(ui::status_mark(
                    format!("{}-state", data.id),
                    data.tone,
                    data.verdict.label(),
                    cx,
                ))
                .into_any_element();
        };
        kit::word_cell(
            column,
            format!("{}-text-{}", data.id, cell + 1),
            data.cells[cell].clone(),
        )
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
            freshkube_ui::menu::MenuAction::new("Open trace", OpenTrace)
                .enabled(self.selected.is_some()),
            freshkube_ui::menu::MenuAction::new("Open route", OpenRoute)
                .enabled(self.selected.is_some()),
        ]
    }
    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let line = self.display.groups.get(group)?;
        Some(
            kit::GroupRow::new(
                self.table.id(&format!("group-{}", line.slug)),
                line.tone,
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
    fn empty(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.display.lines.is_empty().then(|| {
            let words = if self.display.filtered_out() {
                SharedString::from("No route matches the filter.")
            } else {
                self.display.empty.clone()
            };
            ui::empty_state(
                gpui_kit::assets::IconName::Network,
                "Routes",
                words.clone(),
                None,
                Vec::new(),
                cx,
            )
            .h_auto()
            .id("routes-empty-state")
            .aria_label(words)
            .test_support()
            .into_any_element()
        })
    }
}
