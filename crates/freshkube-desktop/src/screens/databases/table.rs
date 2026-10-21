use super::*;
use display::Row;
use freshkube_ui::table::{
    self as kit, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource,
};
pub(crate) struct Column {
    index: usize,
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
        self.index == 6
    }
    fn pinned(&self) -> bool {
        self.index <= 1
    }
}
pub(super) fn columns(mode: Mode, rows: &[Row]) -> (Vec<Column>, f32) {
    let labels = match mode {
        Mode::Clusters => [
            "",
            "Database",
            "Engine",
            "Namespace",
            "Instances",
            "Primary",
            "Backup coverage",
        ],
        Mode::Backups => [
            "",
            "Backup",
            "Kind",
            "Namespace",
            "Status",
            "Method",
            "Cluster",
        ],
        Mode::Schedules => [
            "",
            "Schedule",
            "Kind",
            "Namespace",
            "Expression",
            "Suspended",
            "Cluster",
        ],
    };
    let columns = labels
        .into_iter()
        .enumerate()
        .map(|(index, label)| Column {
            index,
            label: label.into(),
            width: if index == 0 {
                kit::GLYPH_WIDTH
            } else {
                kit::fit(
                    label,
                    rows.iter().map(|row| &row.cells[index - 1]),
                    if index == 6 {
                        kit::WIDEST_FLEXIBLE
                    } else {
                        kit::WIDEST
                    },
                )
            },
        })
        .collect::<Vec<_>>();
    let width = columns.iter().map(|c| c.width).sum();
    (columns, width)
}
impl TableSource for DatabasesScreen {
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
        "Read-only databases; arrows select, O opens in Resources".into()
    }
    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }
    fn sort(&mut self, _: (), _: &mut Context<Self>) {}
    fn line_count(&self) -> usize {
        self.display.rows.len()
    }
    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, &Row>> {
        let row = self.display.rows.get(line)?;
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
        if column.index == 0 {
            kit::glyph_cell(column)
                .child(ui::status_mark(
                    format!("{}-state", row.data.id),
                    row.data.tone,
                    row.data.cells[match self.mode {
                        Mode::Clusters | Mode::Backups => 3,
                        Mode::Schedules => 5,
                    }]
                    .clone(),
                    cx,
                ))
                .into_any_element()
        } else {
            kit::word_cell(
                column,
                format!("{}-text-{}", row.data.id, column.index),
                row.data.cells[column.index - 1].clone(),
            )
            .into_any_element()
        }
    }
    fn selected_key(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }
    fn line_of(&self, key: &SharedString) -> Option<usize> {
        self.display.rows.iter().position(|row| &row.key == key)
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
            freshkube_ui::menu::MenuAction::new("Open in Resources", OpenResource)
                .enabled(self.selected.is_some()),
        ]
    }
    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
    fn loading(&self) -> Option<&kit::LoadingRows> {
        self.loading.rows()
    }
    fn empty(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.display.rows.is_empty().then(|| {
            ui::empty_state(
                gpui_kit::assets::IconName::Database,
                "Databases",
                self.display.empty.clone(),
                None,
                if self.display.notice.is_some() {
                    vec![self.retry(cx).into_any_element()]
                } else {
                    Vec::new()
                },
                cx,
            )
            .id("databases-empty-state")
            .aria_label(self.display.empty.clone())
            .test_support()
            .into_any_element()
        })
    }
}
