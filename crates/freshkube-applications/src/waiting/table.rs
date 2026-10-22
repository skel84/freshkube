//! The page as a `TableSource`: the glyph, PR, Change, Merged, the Stage it
//! waits in, how long it has waited, and why; the groups, and the footer's
//! count while the caught-up group folds.
use super::*;
use crate::ui::{self, MONO_FONT, dp};
use freshkube_ui::table::{Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource};
use freshkube_ui::tooltip::FollowTooltip as _;
use gpui_kit::component::{
    IconName, Sizable,
    button::{Button, ButtonVariants as _},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Glyph,
    Named,
    Change,
    Merged,
    Stage,
    Waiting,
    Why,
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

/// The columns, and their total width.
pub(super) fn columns() -> (Vec<Column>, f32) {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    let columns = vec![
        column(Field::Glyph, "", kit::GLYPH_WIDTH),
        column(Field::Named, "PR", 140.),
        column(Field::Change, "Change", 140.),
        column(Field::Merged, "Merged", 64.),
        column(Field::Stage, "In", 84.),
        column(Field::Waiting, "Waiting", 72.),
        column(Field::Why, "Why", 250.),
    ];
    let width = columns.iter().map(|column| column.width).sum();
    (columns, width)
}

impl TableSource for WaitingPage {
    type Key = SharedString;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a WaitRow;

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
        "Waiting to ship: each application's Freight that is not in production yet, the one \
         waiting longest first"
            .into()
    }

    /// The groups keep the order that says what waited longest, so no
    /// column sorts them.
    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, &WaitRow>> {
        let ix = match *self.lines.get(line)? {
            Entry::Group(ix) => return Some(Line::Group(ix)),
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
        line: &TableRow<SharedString, &WaitRow>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let p = &style.p;
        let id = |part: &str| SharedString::from(format!("{PREFIX}-row-{}-{part}", row.key));
        let text = |part: &str, value: SharedString| {
            kit::cell(column)
                .id(id(part))
                .test_support()
                .font_family(MONO_FONT)
                .text_color(p.muted)
                .child(value)
                .into_any_element()
        };
        match column.field {
            Field::Glyph => kit::glyph_cell(column)
                .child(ui::status_mark(id("mark"), row.tone, row.state.clone(), cx))
                .into_any_element(),
            Field::Named => kit::cell(column)
                .id(id("pr"))
                .test_support()
                .flex()
                .items_center()
                .gap(dp(4.))
                .overflow_hidden()
                .when_some(row.pull_note.clone(), |cell, note| {
                    cell.follow_tooltip(note)
                })
                .child(
                    div()
                        .when(row.by_commit, |cell| {
                            cell.font_family(MONO_FONT).text_color(p.ink_2)
                        })
                        .child(row.named.clone()),
                )
                // Where the pull request stands, dimmer when it is what was
                // kept and not read now.
                .when_some(row.pull.clone(), |cell, state| {
                    cell.child(
                        div()
                            .text_size(dp(10.))
                            .text_color(if row.pull_stale {
                                p.muted.opacity(0.6)
                            } else {
                                p.muted
                            })
                            .child(state),
                    )
                })
                .into_any_element(),
            Field::Change => kit::cell(column)
                .id(id("title-cell"))
                .flex()
                .items_center()
                .follow_tooltip(row.change.clone())
                .child(div().flex_1().min_w_0().child(
                    kit::word_cut(id("title"), row.change.clone()).dim_from(row.title_len, p.muted),
                ))
                .into_any_element(),
            Field::Merged => text("merged", row.merged.clone()),
            Field::Stage => text("stage", row.stage.clone()),
            Field::Waiting => text("waited", row.waited.clone()),
            Field::Why => kit::cell(column)
                .id(id("why"))
                .test_support()
                .aria_label(format!("{} {}", row.why_word, row.why))
                .flex()
                .items_center()
                .gap(dp(4.))
                .overflow_hidden()
                .follow_tooltip(format!("{} {}", row.why_word, row.why))
                .child(
                    div()
                        .flex_none()
                        .text_color(p.ink_2)
                        .child(row.why_word.clone()),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(p.muted)
                        .child(row.why.clone()),
                )
                .into_any_element(),
        }
    }

    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let info = self.display.groups.get(group)?;
        let mut line = kit::GroupRow::new(
            self.table.id(&format!("group-{}", info.key)),
            info.tone,
            info.title.clone(),
            kit::ROW_HEIGHT,
        )
        .detail(info.detail.clone());
        // The caught-up group keeps its chevron while anything waits, as
        // the healthy group does on Pods.
        if info.folds && self.display.waiting > 0 {
            let what = if self.folded { "Expand" } else { "Collapse" };
            line = line
                .chevron(
                    Button::new(self.table.id("fold-caught-up"))
                        .ghost()
                        .xsmall()
                        .icon(if self.folded {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .tooltip(format!("{what} {}", info.title))
                        .accessibility_label(format!("{what} {}", info.title))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_fold(cx);
                        })),
                )
                .after(if self.folded {
                    vec![format!("{} folded", info.rows)]
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

    fn menu_focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    /// A right-click selects the row without opening the Inspector. Nothing
    /// is written from here, so the menu offers nothing.
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
        Vec::new()
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

    /// `Showing 4 of 6` with Show all, while the caught-up group folds.
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
