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

pub(super) fn columns(rows: &[SourceRow], suggested: Option<&SourceRow>) -> (Vec<Column>, f32) {
    let rows: Vec<&SourceRow> = rows.iter().chain(suggested).collect();
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
        column(
            Field::State,
            "State",
            fit("State", rows.iter().map(|row| &row.state), table::WIDEST).max(128.),
        ),
        // Fit identity columns before Reads takes the remaining room.
        column(Field::Reads, "Reads", 64.),
    ];
    let width = fit_columns(&mut columns, f32::INFINITY);
    (columns, width)
}

/// Fit the table's own room: shrink Entry and Chosen by together
/// when narrow, then give spare room to their fitted values before Reads.
pub(super) fn fit_columns(columns: &mut [Column], room: f32) -> f32 {
    for column in columns.iter_mut() {
        column.width = match column.field {
            Field::Entry | Field::Why => column.fitted.min(160.),
            Field::State => column.fitted.min(128.),
            _ => column.fitted,
        };
    }
    let base: f32 = columns.iter().map(|column| column.width).sum();
    if base > room {
        let fixed: f32 = columns
            .iter()
            .filter(|column| {
                matches!(
                    column.field,
                    Field::Glyph | Field::Part | Field::State | Field::Reads
                )
            })
            .map(|column| column.width)
            .sum();
        let scale = ((room - fixed).max(0.) / (base - fixed)).min(1.);
        for column in columns.iter_mut() {
            if matches!(column.field, Field::Entry | Field::Why) {
                column.width *= scale;
            }
        }
        return columns.iter().map(|column| column.width).sum();
    }
    let mut spare = (room - base).max(0.);
    for column in columns.iter_mut() {
        if matches!(column.field, Field::Entry | Field::Why | Field::State) {
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
        self.rows.len() + if self.finder.shows_group() { 2 } else { 0 }
    }

    /// The parts, then the suggestion's group and its row.
    fn line(&self, line: usize, _: &App) -> Option<Line<Read, &SourceRow>> {
        let shown = self.finder.shows_group();
        let row = match line.checked_sub(self.rows.len()) {
            None => &self.rows[line],
            Some(0) if shown => return Some(Line::Group(0)),
            Some(1) if shown => self.finder.row.as_ref()?,
            Some(_) => return None,
        };
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

    fn group(&self, _: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.finder
            .shows_group()
            .then(|| self.render_suggested_group(cx))
    }

    fn selected_key(&self) -> Option<&Read> {
        self.selected.as_ref()
    }

    fn line_of(&self, key: &Read) -> Option<usize> {
        if let Some(at) = self.rows.iter().position(|row| &row.read == key) {
            return Some(at);
        }
        (self.finder.row.as_ref()?.read == *key).then_some(self.rows.len() + 1)
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
        // Keep the table in place while its row owns the popup. Moving
        // it into the split would drop the popup's element state.
        let inspecting = self.selected.is_some() && !self.menu_holds_inspector;
        self.menu_had_focus = false;
        self.focus(window, cx);
        self.selected = Some(*key);
        self.menu_holds_inspector = !inspecting;
        cx.notify();
        let mut actions = vec![
            menu::MenuAction::new("Check sources", CheckSources)
                .enabled(self.check != Check::Running),
        ];
        if *key == Read::BuildSuggestion {
            actions.push(menu::MenuAction::Separator);
            actions.push(
                menu::MenuAction::new("Accept", suggest::AcceptBuild)
                    .enabled(self.editable(cx) && self.finder.accept.is_some()),
            );
            actions.push(menu::MenuAction::new("Ignore", suggest::IgnoreBuild));
        }
        if matches!(key, Read::Part(_)) {
            actions.push(menu::MenuAction::Separator);
            actions.push(
                menu::MenuAction::submenu("Pin to entry", self.pin_entries(cx))
                    .enabled(self.editable(cx)),
            );
            actions.push(self.follow_roles(cx));
        }
        actions.push(
            menu::MenuAction::new("Find suggestions", suggest::FindBuilds)
                .enabled(!self.finder.read.is_loading()),
        );
        actions
    }

    fn loading(&self) -> Option<&table::LoadingRows> {
        (self.check == Check::Running).then_some(&self.loading)
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
}
