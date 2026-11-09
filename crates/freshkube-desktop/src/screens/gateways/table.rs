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

/// The columns as they are drawn: the name first, then why it reads as it
/// does, so a narrow window shows both.
pub(super) fn columns(mode: Mode, rows: &[Row]) -> (Vec<Column>, f32) {
    let labels = mode.labels();
    let mut columns = vec![Column {
        cell: None,
        label: "".into(),
        width: kit::GLYPH_WIDTH,
        flexible: false,
        pinned: true,
    }];
    for (cell, label) in labels.iter().enumerate() {
        let flexible = cell == STATUS;
        columns.push(Column {
            cell: Some(cell),
            label: (*label).into(),
            width: kit::fit(
                label,
                rows.iter().filter_map(|row| row.cells.get(cell)),
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

impl TableSource for GatewaysScreen {
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
        match self.mode {
            Mode::Gateways => "Gateways by state; arrows select, Enter opens the Gateway",
            Mode::Classes => "GatewayClasses by state; arrows select, Enter opens the class",
        }
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
            return kit::glyph_cell(column)
                .child(ui::status_mark(
                    format!("{}-state", data.id),
                    data.tone,
                    data.state,
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
        let openable = self.selected_row().is_some_and(|row| row.openable);
        vec![
            freshkube_ui::menu::MenuAction::new(self.mode.open_label(), OpenObject)
                .enabled(openable),
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
                SharedString::from(match self.mode {
                    Mode::Gateways => "No Gateway matches the filter.",
                    Mode::Classes => "No class matches the filter.",
                })
            } else {
                self.display.empty.clone()
            };
            ui::empty_state(
                gpui_kit::assets::IconName::Network,
                self.mode.title(),
                words.clone(),
                None,
                Vec::new(),
                cx,
            )
            .h_auto()
            .id(SharedString::from(format!(
                "{}-empty-state",
                self.mode.prefix()
            )))
            .aria_label(words)
            .test_support()
            .into_any_element()
        })
    }
}
