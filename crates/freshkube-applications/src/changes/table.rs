//! The list as a `TableSource`: the glyph, PR, Change, the row's own
//! Stages in promotion order, and Age; the groups with their folds, and the
//! footer's count while groups fold.
use super::display::Cell;
use super::*;
use crate::ui::{self, MONO_FONT, dp};
use freshkube_ui::table::{Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource};
use freshkube_ui::tooltip::FollowTooltip as _;
use gpui_kit::component::{
    IconName, Sizable,
    button::{Button, ButtonVariants as _},
    h_flex,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Glyph,
    Named,
    Change,
    Stages,
    Age,
}

pub struct Column {
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
        self.field == Field::Change
    }

    /// The glyph and what names the change stay in view when the table
    /// scrolls sideways.
    fn pinned(&self) -> bool {
        matches!(self.field, Field::Glyph | Field::Named)
    }
}

/// The Stages column's least and widest: wide enough for the widest
/// row's Stages within them, so the table keeps beside the Inspector at
/// 1280 and a row past it ends with `+N`.
const STAGES_MIN: f32 = 100.;
pub(super) const STAGES_MAX: f32 = 226.;

/// The columns, and their total width, with Stages `stages` wide.
pub(super) fn columns(stages: f32) -> (Vec<Column>, f32) {
    let column = |field, label: SharedString, width| Column {
        field,
        label,
        width,
    };
    let columns = vec![
        column(Field::Glyph, SharedString::default(), kit::GLYPH_WIDTH),
        column(Field::Named, "PR".into(), 80.),
        column(Field::Change, "Change".into(), 140.),
        column(Field::Stages, "Stages".into(), stages),
        column(Field::Age, "Age".into(), 64.),
    ];
    let width = columns.iter().map(|column| column.width).sum();
    (columns, width)
}

/// One line's height in the Stages cell.
const STAGE_LINE: f32 = 18.;

/// The gaps in the Stages cell: between the `·`, glyph and name of one
/// Stage, and between Stages.
const STAGE_GAP: f32 = 3.;
const STAGES_GAP: f32 = 4.;

/// The rows' text size and a status glyph's, in dp (`DataTable`'s rows,
/// `ui::status_glyph`).
const ROW_TEXT: f32 = 12.5;
const GLYPH: f32 = 10.;

/// The Stages column's width for `rows`, and how many of each row's
/// Stages it shows, with `+N` for the rest. Measured in the rows' mono
/// font at the default text size: every width here is in dp, so it holds
/// at every text size, and it runs when the rows change, not as they draw.
pub(super) fn fit_stages(rows: &mut [ChangeRow], cx: &App) -> f32 {
    let text = cx.text_system();
    let font = text.resolve_font(&gpui_kit::font(MONO_FONT));
    let size = gpui_kit::px(ROW_TEXT);
    let width = |words: &str| -> f32 {
        words
            .chars()
            .map(|ch| {
                text.advance(font, size, ch)
                    .map_or(0., |advance| f32::from(advance.width))
            })
            .sum()
    };
    let dash = width("–");
    let dot = width("·");
    let stage = |ix: usize, cell: &Cell| {
        let mark = if cell.tone.is_some() { GLYPH } else { dash };
        let separator = if ix > 0 { dot + STAGE_GAP } else { 0. };
        separator + mark + STAGE_GAP + width(&cell.stage)
    };
    let inside = STAGES_MAX - 2. * kit::CELL_PAD;
    let mut widest: f32 = 0.;
    for row in rows.iter_mut() {
        let widths: Vec<f32> = row
            .stages
            .iter()
            .enumerate()
            .map(|(ix, cell)| stage(ix, cell))
            .collect();
        let upto = |n: usize| -> f32 {
            widths[..n].iter().sum::<f32>() + STAGES_GAP * n.saturating_sub(1) as f32
        };
        let all = widths.len();
        let mut shown = all;
        if upto(all) > inside {
            // The Stages that fit beside the `+N` that ends the cell.
            shown -= 1;
            while shown > 0
                && upto(shown) + STAGES_GAP + width(&format!("+{}", all - shown)) > inside
            {
                shown -= 1;
            }
        }
        row.stages_shown = shown;
        row.stages_more = (shown < all).then(|| format!("+{}", all - shown).into());
        widest = widest.max(if shown < all { inside } else { upto(all) });
    }
    (widest + 2. * kit::CELL_PAD)
        .clamp(STAGES_MIN, STAGES_MAX)
        .ceil()
}

/// A row's Stages in promotion order, each its glyph and name, `–` where
/// it doesn't run the change; the whole list with each Stage's word in the
/// tooltip.
fn stages_cell(row: &ChangeRow, column: &Column, cx: &App) -> AnyElement {
    let p = freshkube_ui::palette::palette(cx);
    let stage = |ix: usize, cell: &Cell| {
        let mark = match cell.tone {
            Some(tone) => ui::status_glyph(tone, cx).map(IntoElement::into_any_element),
            None => Some(div().text_color(p.muted).child("–").into_any_element()),
        };
        h_flex()
            .flex_none()
            .h(dp(STAGE_LINE))
            .items_center()
            .gap(dp(STAGE_GAP))
            .when(ix > 0, |this| {
                this.child(div().text_color(p.muted).child("·"))
            })
            .children(mark)
            .child(
                div()
                    .text_color(if cell.tone.is_some() { p.ink } else { p.muted })
                    .child(cell.stage.clone()),
            )
    };
    kit::cell(column)
        .id(SharedString::from(format!(
            "{PREFIX}-row-{}-stages",
            row.key
        )))
        .test_support()
        .aria_label(row.stages_text.clone())
        .flex()
        .items_center()
        .follow_tooltip(row.stages_text.clone())
        .child(
            h_flex()
                .min_w_0()
                .overflow_hidden()
                .gap(dp(STAGES_GAP))
                .children(
                    row.stages[..row.stages_shown]
                        .iter()
                        .enumerate()
                        .map(|(ix, cell)| stage(ix, cell)),
                )
                .children(row.stages_more.clone().map(|more| {
                    div()
                        .id(SharedString::from(format!(
                            "{PREFIX}-row-{}-stages-more",
                            row.key
                        )))
                        .test_support()
                        .flex_none()
                        .text_color(p.muted)
                        .child(more)
                })),
        )
        .into_any_element()
}

impl TableSource for ChangesPage {
    type Key = SharedString;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a ChangeRow;

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
        "Changes: each Freight, with how sure it is that each Stage's pods run it, those not \
         fully confirmed first"
            .into()
    }

    /// The groups keep the order that says what needs a look, so no
    /// column sorts them.
    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, &ChangeRow>> {
        let ix = match *self.lines.get(line)? {
            Entry::Group(fold) => {
                let at = Fold::ALL.iter().position(|f| *f == fold)?;
                return Some(Line::Group(at));
            }
            Entry::Row(ix) => ix,
        };
        let row = self.display.rows.get(ix)?;
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: SharedString::from(format!("{PREFIX}-row-{}", row.key)).into(),
            label: row.label.clone(),
            tooltip: Some(row.tooltip.clone()),
            marked: false,
            muted: false,
            data: row,
        }))
    }

    fn cell(
        &self,
        line: &TableRow<SharedString, &ChangeRow>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let p = &style.p;
        match column.field {
            Field::Glyph => kit::glyph_cell(column)
                .child(ui::status_mark(
                    SharedString::from(format!("{PREFIX}-row-{}-mark", row.key)),
                    row.tone,
                    row.state.clone(),
                    cx,
                ))
                .into_any_element(),
            Field::Named => kit::cell(column)
                .when(row.by_commit, |cell| {
                    cell.font_family(MONO_FONT).text_color(p.ink_2)
                })
                .child(row.named.clone())
                .into_any_element(),
            // The title, then its application, dimmer, in one line cut at a
            // word: the application gives way before the title.
            Field::Change => kit::cell(column)
                .id(SharedString::from(format!(
                    "{PREFIX}-row-{}-title-cell",
                    row.key
                )))
                .flex()
                .items_center()
                .follow_tooltip(row.change.clone())
                .child(
                    div().flex_1().min_w_0().child(
                        kit::word_cut(
                            SharedString::from(format!("{PREFIX}-row-{}-title", row.key)),
                            row.change.clone(),
                        )
                        .dim_from(row.title.len(), p.muted),
                    ),
                )
                .into_any_element(),
            Field::Stages => stages_cell(row, column, cx),
            Field::Age => kit::cell(column)
                .id(SharedString::from(format!("{PREFIX}-row-{}-age", row.key)))
                .test_support()
                .font_family(MONO_FONT)
                .text_color(p.muted)
                .child(row.age.clone())
                .into_any_element(),
        }
    }

    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let fold = *Fold::ALL.get(group)?;
        let rows = self.group_rows[group];
        let tone = match fold {
            Fold::NotConfirmed => Tone::Unknown,
            Fold::Confirmed => Tone::Good,
            Fold::NotRunning => Tone::Outline,
        };
        let count = format!("{rows} {}", if rows == 1 { "change" } else { "changes" });
        let mut line = kit::GroupRow::new(
            self.table.id(&format!("group-{}", slug(fold))),
            tone,
            fold.label(),
            kit::ROW_HEIGHT,
        )
        .detail(vec![count]);
        // The groups that fold keep their chevron while anything needs a
        // look, as the healthy group does on Pods.
        if fold.folds() && self.display.needs_a_look {
            let folded = self.folded.contains(&fold);
            let what = if folded { "Expand" } else { "Collapse" };
            line = line
                .chevron(
                    Button::new(self.table.id(&format!("fold-{}", slug(fold))))
                        .ghost()
                        .xsmall()
                        .icon(if folded {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .tooltip(format!("{what} {}", fold.label()))
                        .accessibility_label(format!("{what} {}", fold.label()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_fold(fold, cx);
                        })),
                )
                .after(if folded {
                    vec![format!("{rows} folded")]
                } else {
                    Vec::new()
                });
        }
        Some(line.render(cx).into_any_element())
    }

    fn selected_key(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }

    fn line_of(&self, key: &SharedString) -> Option<usize> {
        self.lines
            .iter()
            .position(|entry| matches!(entry, Entry::Row(ix) if &self.display.rows[*ix].key == key))
    }

    /// A click selects the change and puts the keyboard on the list; a
    /// double-click opens its page.
    fn click(
        &mut self,
        key: &SharedString,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select(&key.clone(), window, cx);
        self.focus(window, cx);
        if event.click_count() >= 2 {
            self.open_selected(window, cx);
        }
    }

    fn menu_focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    /// A right-click selects the change without opening the Inspector;
    /// Open shows its page, as Enter does.
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
        let mut actions = vec![freshkube_ui::menu::MenuAction::new("Open", OpenChange)];
        if let Some(session) = &self.results.session {
            let session = session.read(cx);
            actions.push(
                freshkube_ui::menu::MenuAction::new(session.label.clone(), ForwardResults)
                    .enabled(session.enabled),
            );
        }
        actions
    }

    fn loading(&self) -> Option<&kit::LoadingRows> {
        (self.loader.is_loading() && self.loader.data().is_none() && self.loader.error().is_none())
            .then_some(&self.loading)
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        self.lines
            .is_empty()
            .then(|| "Nothing read yet.".into_any_element())
    }

    /// `Showing 4 of 6` with Show all, while groups fold.
    fn counts(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let total = self.display.rows.len();
        let shown = self.shown_rows;
        if shown == total {
            return Vec::new();
        }
        vec![
            kit::showing(
                self.table.id("showing"),
                shown,
                total,
                Button::new(self.table.id("show-all"))
                    .ghost()
                    .xsmall()
                    .label(format!("Show all {total}"))
                    .on_click(cx.listener(|this, _, _, cx| this.unfold_all(cx))),
                cx,
            )
            .into_any_element(),
        ]
    }
}

pub(super) fn slug(fold: Fold) -> &'static str {
    match fold {
        Fold::NotConfirmed => "not-confirmed",
        Fold::Confirmed => "confirmed",
        Fold::NotRunning => "not-running",
    }
}
