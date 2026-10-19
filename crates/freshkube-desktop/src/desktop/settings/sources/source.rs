//! The delivery sources table as a `TableSource`.
use super::*;
use table::fit;
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
    fitted: f32,
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

pub(super) fn columns(rows: &[SourceRow]) -> (Vec<Column>, f32) {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
        fitted: width,
    };
    let mut columns = vec![
        column(Field::Glyph, "", table::GLYPH_WIDTH),
        column(Field::Part, "Part", 96.),
        column(
            Field::Entry,
            "Entry",
            fit("Entry", rows.iter().map(|row| &row.entry), table::WIDEST),
        ),
        column(
            Field::Why,
            "Chosen by",
            fit("Chosen by", rows.iter().map(|row| &row.why), table::WIDEST),
        ),
        // Leave room for the glyph column and Reads in the narrow page.
        column(Field::State, "State", 128.),
        // Fit identity columns before Reads takes the remaining room.
        column(Field::Reads, "Reads", 64.),
    ];
    let width = fit_columns(&mut columns, 0.);
    (columns, width)
}

/// Keep the narrow table's widths, then give spare room to the fitted
/// Entry and Chosen by values before Reads takes the remainder.
pub(super) fn fit_columns(columns: &mut [Column], room: f32) -> f32 {
    for column in columns.iter_mut() {
        column.width = match column.field {
            Field::Entry | Field::Why => column.fitted.min(160.),
            _ => column.fitted,
        };
    }
    let base: f32 = columns.iter().map(|column| column.width).sum();
    let mut spare = (room - base).max(0.);
    for column in columns.iter_mut() {
        if matches!(column.field, Field::Entry | Field::Why) {
            let give = (column.fitted - column.width).min(spare);
            column.width += give;
            spare -= give;
        }
    }
    columns.iter().map(|column| column.width).sum()
}

impl TableSource for SourcesList {
    type Key = Read;
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

    fn line(&self, line: usize, _: &App) -> Option<Line<Read, &SourceRow>> {
        let row = self.rows.get(line)?;
        Some(Line::Row(TableRow {
            key: row.read,
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
        line: &TableRow<Read, &SourceRow>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let cell = table::cell(column);
        match column.field {
            Field::Glyph => table::glyph_cell(column)
                .id(SharedString::from(format!("{}-glyph", row.id)))
                .test_support()
                .when_some(row.tone, |this, tone| {
                    this.child(ui::status_mark(
                        row.mark_id.clone(),
                        tone,
                        row.state.clone(),
                        cx,
                    ))
                })
                .into_any_element(),
            Field::Part => cell
                .id(SharedString::from(format!("{}-part", row.id)))
                .test_support()
                .child(
                    div()
                        .id(SharedString::from(format!("{}-label", row.id)))
                        .test_support()
                        .min_w_0()
                        .truncate()
                        .child(row.label.clone()),
                )
                .into_any_element(),
            // Workloads name several entries, cut at a word with all of
            // them in the tooltip.
            Field::Entry => {
                table::word_cell(column, format!("{}-entry", row.id), row.entry.clone())
                    .when(!row.listed, |this| this.text_color(style.p.muted))
                    .into_any_element()
            }
            Field::Why => cell.child(row.why.clone()).into_any_element(),
            Field::State => cell
                .when(row.tone.is_none(), |this| this.text_color(style.p.muted))
                .child(row.state.clone())
                .into_any_element(),
            Field::Reads => {
                table::word_cell(column, format!("{}-reads", row.id), row.reads.clone())
                    .test_support()
                    .flex()
                    .items_center()
                    .text_color(style.p.muted)
                    .into_any_element()
            }
        }
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn selected_key(&self) -> Option<&Read> {
        self.selected.as_ref()
    }

    fn line_of(&self, key: &Read) -> Option<usize> {
        self.rows.iter().position(|row| &row.read == key)
    }

    fn click(&mut self, key: &Read, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus(window, cx);
        self.select(*key, window, cx);
    }

    fn menu_focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    /// Select the row before offering actions for its part.
    fn row_menu(
        &mut self,
        key: &Read,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<menu::MenuAction> {
        self.focus(window, cx);
        self.select(*key, window, cx);
        let mut actions = vec![
            menu::MenuAction::new("Check sources", CheckSources)
                .enabled(self.check != Check::Running),
        ];
        if matches!(key, Read::Part(_)) {
            actions.push(menu::MenuAction::Separator);
            actions.push(menu::MenuAction::submenu(
                "Pin to entry",
                self.pin_entries(cx),
            ));
            actions.push(self.follow_roles(cx));
        }
        actions
    }

    fn loading(&self) -> Option<&table::LoadingRows> {
        (self.check == Check::Running).then_some(&self.loading)
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
}
