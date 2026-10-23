use super::*;
use display::{Entry, Row};
use freshkube_ui::table::{
    self as kit, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource,
};
use gpui_kit::TestSupportExt as _;

/// The page width, in dp, from which the Engine column fits beside the
/// Inspector; narrower, the Inspector's header says the engine instead.
pub(super) const ENGINE_COLUMN_WIDTH: f32 = 1440.;

pub(crate) struct Column {
    /// The logical cell it draws; none for the glyph column.
    cell: Option<usize>,
    label: SharedString,
    width: f32,
    flexible: bool,
    pinned: bool,
}
impl Column {
    #[cfg(test)]
    pub(super) fn label_text(&self) -> &str {
        &self.label
    }
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

/// Logical cells and their labels, by view.
fn labels(mode: Mode) -> &'static [&'static str] {
    match mode {
        Mode::Clusters => &["Database", "Engine", "Lag", "Last good backup", "State"],
        Mode::Backups => &["Backup", "Database", "Status", "Finished"],
        Mode::Schedules => &["Schedule", "Database", "When", "State"],
    }
}

/// The cell whose words name the row's glyph.
fn status_cell(mode: Mode) -> usize {
    match mode {
        Mode::Clusters => 4,
        Mode::Backups => 2,
        Mode::Schedules => 3,
    }
}

pub(super) fn columns(mode: Mode, engine_shown: bool, rows: &[Row]) -> (Vec<Column>, f32) {
    let labels = labels(mode);
    let mut columns = vec![Column {
        cell: None,
        label: "".into(),
        width: kit::GLYPH_WIDTH,
        flexible: false,
        pinned: true,
    }];
    let shown: Vec<usize> = (0..labels.len())
        .filter(|&cell| mode != Mode::Clusters || cell != 1 || engine_shown)
        .collect();
    let last = shown.last().copied();
    for cell in shown {
        let flexible = Some(cell) == last;
        let mut width = kit::fit(
            labels[cell],
            rows.iter().map(|row| &row.cells[cell]),
            if flexible {
                kit::WIDEST_FLEXIBLE
            } else {
                kit::WIDEST
            },
        );
        // The State column also holds its source chip.
        if mode == Mode::Clusters && cell == 4 {
            width = width.max(150.);
        }
        columns.push(Column {
            cell: Some(cell),
            label: labels[cell].into(),
            width,
            flexible,
            pinned: cell == 0,
        });
    }
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
                    data.cells[status_cell(self.mode)].clone(),
                    cx,
                ))
                .into_any_element();
        };
        let id = format!("{}-text-{}", data.id, cell + 1);
        let text = data.cells[cell].clone();
        let tip = data.tips[cell].clone();
        let with_chip = self.mode == Mode::Clusters && cell == 4 && data.chip.is_some();
        if !with_chip {
            return match tip {
                None => kit::word_cell(column, id, text).into_any_element(),
                Some(tip) => {
                    use freshkube_ui::tooltip::FollowTooltip as _;
                    kit::cell(column)
                        .id(SharedString::from(format!("{id}-cell")))
                        .test_support()
                        .follow_tooltip(tip)
                        .child(kit::word_cut(id, text))
                        .into_any_element()
                }
            };
        }
        use freshkube_ui::tooltip::FollowTooltip as _;
        let chip = data.chip.unwrap_or_default();
        let tip = tip.unwrap_or_else(|| text.clone());
        kit::cell(column)
            .id(SharedString::from(format!("{id}-cell")))
            .follow_tooltip(tip)
            .child(
                gpui_kit::component::h_flex()
                    .gap(dp(6.))
                    .child(kit::word_cut(id.clone(), text))
                    .child(
                        ui::tag(ui::Tone::Outline, None, chip, cx)
                            .id(SharedString::from(format!("{id}-chip")))
                            .aria_label(chip)
                            .test_support(),
                    ),
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
        let mut actions = vec![
            freshkube_ui::menu::MenuAction::new("Open in Resources", OpenResource)
                .enabled(self.selected.is_some()),
            freshkube_ui::menu::MenuAction::new("Explore", Explore)
                .enabled(self.explorable().is_some()),
        ];
        actions.push(
            freshkube_ui::menu::MenuAction::new("Open in Observability", OpenObservability)
                .enabled(self.selected_observe().is_some()),
        );
        actions
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
            .h_auto()
            .id("databases-empty-state")
            .aria_label(self.display.empty.clone())
            .test_support()
            .into_any_element()
        })
    }
}
