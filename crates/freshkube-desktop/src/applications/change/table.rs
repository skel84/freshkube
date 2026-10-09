//! The trail as a `TableSource`: its columns, the cells, the group rows
//! with their folds, the footer's count while Stages fold, and the Link
//! legend. Detail cuts at a word, with its whole text in the tooltip.
use super::*;
use crate::ui::{self, MONO_FONT, dp};
use freshkube_ui::palette::palette;
use freshkube_ui::table::{Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource};
use gpui_kit::component::{
    IconName, Sizable,
    button::{Button, ButtonVariants as _},
    h_flex,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Glyph,
    Hop,
    Link,
    Time,
    Detail,
    From,
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
        self.field == Field::Detail
    }

    /// The glyph and the hop stay in view when the table scrolls sideways,
    /// so beside the Inspector the link and the time stay whole and
    /// Detail's tail and Read from scroll away.
    fn pinned(&self) -> bool {
        matches!(self.field, Field::Glyph | Field::Hop)
    }
}

/// DESIGN.md's columns: the glyph, Hop (pinned), Link, Time, Detail (the
/// flexible one) and Read from.
pub(super) fn columns() -> Vec<Column> {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    vec![
        column(Field::Glyph, "", kit::GLYPH_WIDTH),
        column(Field::Hop, "Hop", 200.),
        column(Field::Link, "Link", 112.),
        column(Field::Time, "Time", 64.),
        column(Field::Detail, "Detail", 240.),
        column(Field::From, "Read from", 136.),
    ]
}

/// A link's glyph, if any, and its word: Confirmed muted, the other two
/// in ink.
pub(super) fn link_mark(confidence: Confidence, cx: &App) -> Div {
    let p = palette(cx);
    h_flex()
        .items_center()
        .gap(dp(5.))
        .children(link_glyph(confidence).and_then(|tone| ui::status_glyph(tone, cx)))
        .child(
            div()
                .flex_none()
                .text_color(match confidence {
                    Confidence::Confirmed => p.muted,
                    _ => p.ink,
                })
                .child(link_word(confidence)),
        )
}

impl TableSource for ChangePage {
    type Key = SharedString;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a HopRow;

    fn table_state(&self) -> &TableState {
        &self.table
    }

    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn width(&self) -> f32 {
        self.columns.iter().map(TableColumn::width).sum()
    }

    fn list_label(&self) -> String {
        format!(
            "The hops of Freight {}, in the order the change travels",
            self.change.freight
        )
    }

    /// The trail keeps the order the change travels in, so no column
    /// sorts it.
    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, &HopRow>> {
        let ix = match *self.lines.get(line)? {
            Entry::Group(group) => return Some(Line::Group(group)),
            Entry::Row(ix) => ix,
        };
        let row = self.rows.get(ix)?;
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: SharedString::from(format!("{PREFIX}-hop-{}", row.key)).into(),
            label: row.label.clone(),
            tooltip: None,
            marked: false,
            muted: false,
            data: row,
        }))
    }

    fn cell(
        &self,
        line: &TableRow<SharedString, &HopRow>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let p = &style.p;
        let text = |text: &SharedString| kit::cell(column).child(text.clone());
        match column.field {
            Field::Glyph => kit::glyph_cell(column)
                .children(ui::status_glyph(row.tone, cx))
                .into_any_element(),
            Field::Hop => text(&row.name).into_any_element(),
            Field::Detail => kit::word_cell(
                column,
                format!("{PREFIX}-hop-{}-detail", row.key),
                row.detail.clone(),
            )
            .text_color(p.ink_2)
            .into_any_element(),
            Field::From => text(&row.from)
                .font_family(MONO_FONT)
                .text_color(p.ink_2)
                .into_any_element(),
            Field::Link => match row.link {
                None => kit::cell(column).into_any_element(),
                Some(confidence) => kit::cell(column)
                    .child(
                        link_mark(confidence, cx)
                            .id(SharedString::from(format!("{PREFIX}-hop-{}-link", row.key)))
                            .test_support()
                            .aria_label(link_word(confidence)),
                    )
                    .into_any_element(),
            },
            Field::Time => text(&row.time)
                .font_family(MONO_FONT)
                .text_color(p.muted)
                .into_any_element(),
        }
    }

    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let line = self.groups.get(group)?;
        let mut row = kit::GroupRow::new(
            SharedString::from(format!("{PREFIX}-group-{group}")),
            line.tone,
            line.label.clone(),
            kit::ROW_HEIGHT,
        )
        .detail(line.detail.clone());
        if line.folds && self.filter.is_none() {
            let folded = self.folded.contains(&group);
            let what = if folded { "Expand" } else { "Collapse" };
            row = row
                .chevron(
                    Button::new(SharedString::from(format!("{PREFIX}-fold-{group}")))
                        .ghost()
                        .xsmall()
                        .icon(if folded {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .tooltip(format!("{what} {}", line.label))
                        .accessibility_label(format!("{what} {}", line.label))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_fold(group, cx);
                        })),
                )
                .after(if folded {
                    vec![format!("{} rows folded", line.rows)]
                } else {
                    Vec::new()
                });
        }
        Some(row.render(cx).into_any_element())
    }

    fn loading(&self) -> Option<&kit::LoadingRows> {
        self.loading_rows()
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        if !self.lines.is_empty() {
            return None;
        }
        Some(if self.rows.is_empty() {
            "Nothing read yet.".into_any_element()
        } else {
            "No hops match this filter.".into_any_element()
        })
    }

    fn selected_key(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }

    fn line_of(&self, key: &SharedString) -> Option<usize> {
        self.lines
            .iter()
            .position(|entry| matches!(entry, Entry::Row(ix) if &self.rows[*ix].key == key))
    }

    /// A click selects the hop and puts the keyboard on the table.
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

    /// A right-click selects the hop; Open in Resources acts as O does,
    /// greyed out for a hop with nothing to open or in a cluster that
    /// isn't open.
    fn row_menu(
        &mut self,
        key: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<freshkube_ui::menu::MenuAction> {
        self.select(&key.clone(), window, cx);
        self.focus(window, cx);
        let opens = self
            .selected_object()
            .is_some_and(|object| self.object_link(object).1.is_none());
        vec![freshkube_ui::menu::MenuAction::new("Open in Resources", OpenHop).enabled(opens)]
    }

    /// `Showing 17 of 27` with Show all, while Stages fold.
    fn counts(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let total = self.rows.len();
        let shown = self.shown_rows();
        if shown == total || self.filter.is_some() {
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

    /// What the Link column's three words mean: the whole legend when the
    /// table is wide, one line with it in a tooltip when narrow.
    fn legend(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        const WHOLE: &str = "Link: how a hop joins the one before it. Confirmed: the key \
                             matches on both sides. Claimed: only a label, annotation or \
                             attestation says so. Unknown: a side can't be read.";
        if self.table_width(window) < 900. {
            return Some(
                kit::legend_line(
                    self.table.id("legend"),
                    "Confirmed · Claimed · Unknown ⓘ",
                    WHOLE,
                    cx,
                )
                .into_any_element(),
            );
        }
        let item = |confidence: Confidence, text: &str| {
            kit::legend_item(link_mark(confidence, cx), text.to_owned()).into_any_element()
        };
        Some(
            kit::legend(
                [
                    item(Confidence::Confirmed, "the key matches on both sides"),
                    item(Confidence::Claimed, "only a label or attestation says so"),
                    item(Confidence::Unknown, "a side can't be read"),
                ],
                cx,
            )
            .id(self.table.id("legend"))
            .test_support()
            .aria_label(WHOLE)
            .into_any_element(),
        )
    }
}

impl ChangePage {
    /// The table's width: the page's, less the Inspector's beside it.
    fn table_width(&self, window: &Window) -> f32 {
        let page = crate::screens::page_width(window);
        if self.selected.is_some() && page >= inspector::SPLIT_WIDTH {
            page - self.split.width()
        } else {
            page
        }
    }
}
