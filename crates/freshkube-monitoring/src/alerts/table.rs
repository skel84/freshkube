//! The three pages as a `TableSource`: a glyph column, the page's cells in
//! the order `display` derived them, the label chips of an alert, and the
//! groups' header rows.
use freshkube_ui::palette::palette;
use freshkube_ui::table::{Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource};
use freshkube_ui::tooltip::FollowTooltip as _;
use freshkube_ui::ui::{self, MONO_FONT, dp};
use gpui_kit::component::h_flex;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::display::{Body, Chips, Entry};
use super::{AlertsPage, Which};
use freshkube_ui::table as kit;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Glyph,
    /// A row's cell by position.
    Cell(usize),
    /// An alert's label chips.
    Labels,
}

pub struct Column {
    field: Field,
    label: SharedString,
    width: f32,
    flexible: bool,
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

    /// The glyph and the name stay in view when the table scrolls
    /// sideways.
    fn pinned(&self) -> bool {
        matches!(self.field, Field::Glyph | Field::Cell(0))
    }
}

/// The columns of a page and their total width. An alerts table of a
/// central store has a cluster column.
pub(super) fn columns(which: Which, central: bool) -> (Vec<Column>, f32) {
    let column = |field, label: &str, width: f32, flexible: bool| Column {
        field,
        label: label.to_owned().into(),
        width,
        flexible,
    };
    let glyph = column(Field::Glyph, "", kit::GLYPH_WIDTH, false);
    let mut columns = match which {
        Which::Alerts => {
            let mut columns = vec![
                glyph,
                column(Field::Cell(0), "Alert", 220., false),
                column(Field::Cell(1), "State", 84., false),
                column(Field::Cell(2), "Since", 78., false),
                column(Field::Labels, "Labels", 280., false),
                column(Field::Cell(3), "Summary", 260., true),
            ];
            if central {
                columns.push(column(Field::Cell(4), "Cluster", 120., false));
            }
            columns
        }
        Which::Rules => vec![
            glyph,
            column(Field::Cell(0), "Rule", 240., false),
            column(Field::Cell(1), "Kind", 90., false),
            column(Field::Cell(2), "Health", 84., false),
            column(Field::Cell(3), "Evaluated", 120., false),
            column(Field::Cell(4), "Alerts", 90., false),
            column(Field::Cell(5), "Last error", 300., true),
        ],
        Which::Silences => vec![
            glyph,
            column(Field::Cell(0), "Matchers", 300., true),
            column(Field::Cell(1), "Created by", 170., false),
            column(Field::Cell(2), "Comment", 240., false),
            column(Field::Cell(3), "Ends", 120., false),
            column(Field::Cell(4), "Muting now", 110., false),
        ],
    };
    let width = columns.iter().map(|column| column.width).sum();
    columns.shrink_to_fit();
    (columns, width)
}

/// An alert's label chips: each `name=value` in a small box, then `+N`.
fn chips(prefix: &str, key: &str, chips: &Chips, cx: &App) -> AnyElement {
    let p = palette(cx);
    h_flex()
        .id(SharedString::from(format!("{prefix}-row-{key}-chips")))
        .test_support()
        .gap(dp(4.))
        .overflow_hidden()
        .children(chips.shown.iter().map(|chip| {
            div()
                .min_w_0()
                .truncate()
                .h(dp(18.))
                .px(dp(5.))
                .flex()
                .items_center()
                .rounded(px(3.))
                .border_1()
                .border_color(p.line)
                .bg(p.surface_2)
                .text_size(dp(11.))
                .font_family(MONO_FONT)
                .text_color(p.ink_2)
                .child(chip.clone())
        }))
        .when(chips.more > 0, |this| {
            this.child(
                div()
                    .flex_none()
                    .text_size(dp(11.))
                    .text_color(p.muted)
                    .child(format!("+{}", chips.more)),
            )
        })
        .into_any_element()
}

impl TableSource for AlertsPage {
    type Key = SharedString;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a super::display::Row;

    fn table_state(&self) -> &kit::TableState {
        &self.table
    }

    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn list_label(&self) -> String {
        match self.which {
            Which::Alerts => {
                "Alerts, grouped by severity and name, with their state, since when and labels"
            }
            Which::Rules => "Rules, grouped by rule group, with their health and last evaluation",
            Which::Silences => "Silences, active, pending and expired, with their matchers",
        }
        .into()
    }

    /// The groups keep the order that says what needs a look, so no column
    /// sorts them.
    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.display.lines.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, &super::display::Row>> {
        let ix = match *self.display.lines.get(line)? {
            Entry::Group(group) => return Some(Line::Group(group)),
            Entry::Row(ix) => ix,
        };
        let row = self.display.rows.get(ix)?;
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: SharedString::from(format!("{}-row-{}", self.which.slug(), row.key)).into(),
            label: row.label.clone(),
            tooltip: Some(row.tooltip.clone()),
            marked: false,
            muted: false,
            data: row,
        }))
    }

    fn cell(
        &self,
        line: &TableRow<SharedString, &super::display::Row>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let prefix = self.which.slug();
        let p = &style.p;
        match column.field {
            Field::Glyph => kit::glyph_cell(column)
                .child(ui::status_mark(
                    SharedString::from(format!("{prefix}-row-{}-mark", row.key)),
                    row.tone,
                    row.state.clone(),
                    cx,
                ))
                .into_any_element(),
            Field::Labels => kit::cell(column)
                .id(SharedString::from(format!(
                    "{prefix}-row-{}-labels-cell",
                    row.key
                )))
                .flex()
                .items_center()
                .children(row.chips.as_ref().map(|c| chips(prefix, &row.key, c, cx)))
                .into_any_element(),
            Field::Cell(ix) => {
                let Some(cell) = row.cells.get(ix) else {
                    return kit::cell(column).into_any_element();
                };
                kit::cell(column)
                    .id(SharedString::from(format!("{prefix}-row-{}-{ix}", row.key)))
                    .test_support()
                    .when(cell.mono, |this| this.font_family(MONO_FONT))
                    .text_color(if cell.muted || ix > 0 && cell.mono {
                        p.muted
                    } else {
                        p.ink
                    })
                    .when(ix == 0 && !cell.mono, |this| {
                        this.font_weight(FontWeight::MEDIUM)
                    })
                    .follow_tooltip(cell.text.clone())
                    .child(cell.text.clone())
                    .into_any_element()
            }
        }
    }

    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let group = self.display.groups.get(group)?;
        let line = kit::GroupRow::new(
            self.table.id(&format!("group-{}", group.key)),
            group.tone,
            group.label.clone(),
            kit::ROW_HEIGHT,
        )
        .subject(group.subject.clone())
        .detail(group.detail.clone());
        Some(line.render(cx).into_any_element())
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

    /// A click selects the row and puts the keyboard on the page.
    fn click(
        &mut self,
        key: &SharedString,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select(&key.clone(), window, cx);
        self.focus(window, cx);
    }

    fn loading(&self) -> Option<&kit::LoadingRows> {
        matches!(self.display.body, Body::Loading).then_some(&self.loading)
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        self.display
            .lines
            .is_empty()
            .then(|| "Nothing read yet.".into_any_element())
    }
}
