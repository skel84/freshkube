//! The destinations table as a `TableSource`.
use super::*;
use freshkube_ui::tooltip::FollowTooltip as _;
use table::{Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, TableState};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Glyph,
    Destination,
    By,
    Entry,
    Found,
}

pub(crate) struct Column {
    field: Field,
    label: SharedString,
    width: f32,
    /// The width the texts ask for; the table's room may take some back.
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

pub(super) fn columns(rows: &[DestinationRow], suggested: &[DestinationRow]) -> (Vec<Column>, f32) {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
        fitted: width,
    };
    let all = || rows.iter().chain(suggested);
    let mut columns = vec![
        column(Field::Glyph, "", table::GLYPH_WIDTH),
        column(
            Field::Destination,
            "Destination",
            fit("Destination", all().map(|row| &row.destination), 360.),
        ),
        column(Field::By, "Match by", 120.),
        column(
            Field::Entry,
            "Workspace cluster",
            fit("Workspace cluster", all().map(|row| &row.entry), 240.),
        ),
    ];
    // Found from only shows while there are suggestions to say it of.
    if !suggested.is_empty() {
        columns.push(column(
            Field::Found,
            "Found from",
            fit(
                "Found from",
                suggested
                    .iter()
                    .filter_map(|row| row.suggested.as_ref().map(|s| &s.found_from)),
                360.,
            ),
        ));
    }
    let width = columns.iter().map(|column| column.width).sum();
    (columns, width)
}

/// The least a column that gives room back may be.
fn least(field: Field) -> f32 {
    match field {
        Field::Found => 140.,
        Field::Destination => 160.,
        _ => 64.,
    }
}

/// Fits the columns to the table's own `room`: each at the width its texts
/// ask for, and when they don't all fit, Found from gives room back first
/// and then Destination, down to a least width each, so the last column ends
/// inside the frame with an ellipsis. Only a narrower window than that
/// scrolls sideways. Returns the table's width.
pub(super) fn fit_columns(columns: &mut [Column], room: f32) -> f32 {
    for column in columns.iter_mut() {
        column.width = column.fitted;
    }
    let mut over = columns.iter().map(|column| column.width).sum::<f32>() - room;
    for field in [Field::Found, Field::Destination] {
        if over <= 0. {
            break;
        }
        if let Some(column) = columns.iter_mut().find(|column| column.field == field) {
            let give = (column.width - least(field)).clamp(0., over);
            column.width -= give;
            over -= give;
        }
    }
    columns.iter().map(|column| column.width).sum()
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
        let suggested = self.finder.rows.len();
        self.rows.len()
            + if self.finder.shows_group() {
                1 + suggested
            } else {
                0
            }
    }

    /// The mapped rows, then the suggestions' group and its rows.
    fn line(&self, line: usize, _: &App) -> Option<Line<Key, &DestinationRow>> {
        let shown = self.finder.shows_group();
        let row = match line.checked_sub(self.rows.len()) {
            None => &self.rows[line],
            Some(0) if shown => return Some(Line::Group(0)),
            Some(past) if shown => self.finder.rows.get(past - 1)?,
            Some(_) => return None,
        };
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
        // A suggestion's cells carry the row's lines (its destination, what
        // it says and where it was found) rather than only their own text.
        let text_cell = |field: &str, text: &SharedString| {
            let id = format!("{}-{field}", row.id);
            let tip = match row.suggested {
                Some(_) => row.tooltip.clone(),
                None => text.clone(),
            };
            table::cell(column)
                .id(SharedString::from(format!("{id}-cell")))
                .follow_tooltip(tip)
                .child(table::word_cut(id, text.clone()))
                .test_support()
        };
        let suggested = row.suggested.as_ref();
        // A suggestion that can't be accepted is marked, and says why.
        let mark = match (suggested, row.listed) {
            (Some(suggested), _) => suggested.tone.map(|tone| (tone, row.mark.clone())),
            (None, false) => Some((ui::Tone::Unknown, row.mark.clone())),
            (None, true) => None,
        };
        let muted = !row.listed || suggested.is_some_and(|s| s.accept.is_none());
        match column.field {
            Field::Glyph => table::glyph_cell(column)
                .id(SharedString::from(format!("{}-glyph", row.id)))
                .test_support()
                .when_some(mark, |this, (tone, words)| {
                    this.child(ui::status_mark(row.mark_id.clone(), tone, words, cx))
                })
                .into_any_element(),
            Field::Destination => text_cell("destination", &row.destination).into_any_element(),
            Field::By => text_cell("by", &row.by).into_any_element(),
            Field::Entry => text_cell("entry", &row.entry)
                .when(muted, |this| this.text_color(style.p.muted))
                .into_any_element(),
            Field::Found => text_cell(
                "found",
                suggested.map_or(&SharedString::default(), |s| &s.found_from),
            )
            .text_color(style.p.muted)
            .into_any_element(),
        }
    }

    fn group(&self, _: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.finder
            .shows_group()
            .then(|| self.render_suggested_group(cx))
    }

    fn selected_key(&self) -> Option<&Key> {
        self.selected.as_ref()
    }

    fn line_of(&self, key: &Key) -> Option<usize> {
        if let Some(at) = self.rows.iter().position(|row| &row.key == key) {
            return Some(at);
        }
        let at = self.finder.rows.iter().position(|row| &row.key == key)?;
        Some(self.rows.len() + 1 + at)
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
        let find = menu::MenuAction::new("Find suggestions", FindDestinations)
            .enabled(!self.finder.is_finding());
        if let Some(suggested) = self
            .selected_suggestion()
            .and_then(|row| row.suggested.as_ref())
        {
            return vec![
                menu::MenuAction::new("Accept", AcceptSuggestion)
                    .enabled(self.editable && suggested.accept.is_some()),
                menu::MenuAction::new("Ignore", IgnoreSuggestion),
                menu::MenuAction::Separator,
                find,
            ];
        }
        vec![
            menu::MenuAction::new("Map", MapDestination).enabled(self.mappable()),
            menu::MenuAction::new("Edit", EditDestination).enabled(self.mappable()),
            menu::MenuAction::new("Remove", RemoveDestination).enabled(self.editable),
            menu::MenuAction::Separator,
            find,
        ]
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        if !self.rows.is_empty() || self.finder.shows_group() {
            return None;
        }
        Some(
            "No destinations are mapped. An Argo CD destination that isn’t mapped stays Unknown \
             on the change page."
                .into_any_element(),
        )
    }
}
