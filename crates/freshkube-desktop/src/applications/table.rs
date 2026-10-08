//! The applications table as a `TableSource`: its columns, sized from the
//! rows when they change, and the cells of each row.
use super::display::{AppRow, Mark, RULES, rule_label};
use super::*;
use crate::ui;
use freshkube_ui::table::{Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Glyph,
    Name,
    FoundBy,
    Parts,
    Clusters,
    Notes,
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
        self.field == Field::Notes
    }

    /// The glyph and name stay in view when the table scrolls sideways.
    fn pinned(&self) -> bool {
        matches!(self.field, Field::Glyph | Field::Name)
    }
}

/// DESIGN.md's widths: 7.5 a character plus 24, between 64 and 280.
fn fit<'a>(label: &str, texts: impl Iterator<Item = &'a SharedString>) -> f32 {
    let chars = texts
        .map(|text| text.chars().count())
        .chain([label.len()])
        .max()
        .unwrap_or_default();
    (chars as f32 * 7.5 + 24.).clamp(64., 280.)
}

/// The columns for these rows, and their total width.
pub(super) fn columns(rows: &[AppRow]) -> (Vec<Column>, f32) {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    let columns = vec![
        column(Field::Glyph, "", kit::GLYPH_WIDTH),
        column(
            Field::Name,
            "Application",
            fit("Application", rows.iter().map(|row| &row.name)).min(200.),
        ),
        column(
            Field::FoundBy,
            "Found by",
            fit("Found by", rows.iter().map(|row| &row.found_by)),
        ),
        column(
            Field::Parts,
            "Parts",
            fit("Parts", rows.iter().map(|row| &row.parts)),
        ),
        column(
            Field::Clusters,
            "Clusters",
            fit("Clusters", rows.iter().map(|row| &row.clusters)),
        ),
        column(Field::Notes, "Notes", 200.),
    ];
    let width = columns.iter().map(|column| column.width).sum();
    (columns, width)
}

pub(super) fn tone(mark: Mark) -> ui::Tone {
    match mark {
        Mark::Incomplete => ui::Tone::Unknown,
        Mark::Scoped | Mark::Notes => ui::Tone::Info,
        Mark::Read => ui::Tone::Good,
    }
}

impl TableSource for ApplicationsPage {
    type Key = SharedString;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a AppRow;

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
        "Applications, grouped by the rule that found them: Kargo Projects, Argo CD, then the \
         part-of label"
            .into()
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, &AppRow>> {
        let ix = match *self.lines.get(line)? {
            Entry::Group(rule) => return Some(Line::Group(rule)),
            Entry::Row(ix) => ix,
        };
        let row = self.display.rows.get(ix)?;
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: SharedString::from(format!("application-{}", row.key)).into(),
            label: format!(
                "{} · {} · {} · {}",
                row.name, row.mark_words, row.found_by, row.parts
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
        line: &TableRow<SharedString, &AppRow>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let cell = kit::cell(column);
        match column.field {
            Field::Glyph => kit::glyph_cell(column)
                .child(ui::status_mark(
                    SharedString::from(format!("application-{}-mark", row.key)),
                    tone(row.mark),
                    row.mark_words.clone(),
                    cx,
                ))
                .into_any_element(),
            Field::Name => cell.child(row.name.clone()).into_any_element(),
            Field::FoundBy => cell.child(row.found_by.clone()).into_any_element(),
            Field::Parts => cell.child(row.parts.clone()).into_any_element(),
            Field::Clusters => cell.child(row.clusters.clone()).into_any_element(),
            Field::Notes => cell
                .text_color(style.p.muted)
                .child(row.note.clone())
                .into_any_element(),
        }
    }

    fn group(&self, rule: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let label = rule_label(*RULES.get(rule)?);
        let count = self.groups[rule];
        Some(
            kit::GroupRow::new(
                self.table
                    .id(&format!("group-{}", label.to_lowercase().replace(' ', "-"))),
                ui::Tone::Outline,
                label,
                kit::ROW_HEIGHT,
            )
            .detail(vec![format!(
                "{count} {}",
                if count == 1 {
                    "application"
                } else {
                    "applications"
                }
            )])
            .render(cx)
            .into_any_element(),
        )
    }

    fn selected_key(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }

    fn line_of(&self, key: &SharedString) -> Option<usize> {
        self.lines
            .iter()
            .position(|entry| matches!(entry, Entry::Row(ix) if &self.display.rows[*ix].key == key))
    }

    /// A click selects the row, which opens the Inspector, and puts the
    /// keyboard on the list; a double-click opens the application's page.
    fn click(
        &mut self,
        key: &SharedString,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select(key.clone(), cx);
        self.focus(window, cx);
        if event.click_count() >= 2 {
            self.open_application(key, window, cx);
        }
    }

    fn menu_focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    /// A right-click selects the row as an arrow does, but an Inspector
    /// that was closed stays closed, so the table keeps its place under
    /// the pointer and the menu opens where it was asked for. Open shows
    /// the application's page, as Enter does.
    fn row_menu(
        &mut self,
        key: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<freshkube_ui::menu::MenuAction> {
        if self.selected.as_ref() != Some(key) {
            self.selected = Some(key.clone());
            cx.notify();
        }
        self.focus(window, cx);
        vec![freshkube_ui::menu::MenuAction::new("Open", OpenApplication)]
    }

    fn loading(&self) -> Option<&kit::LoadingRows> {
        (self.pending && self.snapshot.data().is_none() && self.snapshot.error().is_none())
            .then_some(&self.loading)
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        if !self.lines.is_empty() {
            return None;
        }
        Some(if self.display.rows.is_empty() {
            "Nothing read yet.".into_any_element()
        } else {
            "No applications match this filter.".into_any_element()
        })
    }
}
