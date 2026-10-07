//! The table every list page draws, as Pods does: its cells, group rows,
//! the footer's counts and legend, and the status chips that filter it (docs/DESIGN.md#components).
use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::{
    Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    tooltip::Tooltip,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Div, ElementId, FontWeight, Role, SharedString, Stateful, TestSupportExt, div,
    px,
};

use crate::palette::palette;

mod data;
mod flash;
mod loading;
mod pinned;
use crate::ui::{self, MONO_FONT, Tone, dp};
pub use data::{
    DataTable, Line, RowStyle, SortOrder, TableRow, TableSource, TableState, data_table, reveal,
    step,
};
pub use flash::{FlashLayer, Reduced, RowsAt};
pub use loading::{LOADING_ROWS, LoadingMotion, LoadingRows, Look};
pub use pinned::widest_pinned_run;

/// Every table's row height. Group rows take the same height, so the list
/// stays uniform; the text size scales it for anyone who wants it larger.
pub const ROW_HEIGHT: f32 = 26.;
/// The header row's height.
pub const HEADER_HEIGHT: f32 = 26.;
/// The footer's height: the counts and the legend share one row.
pub const FOOTER_HEIGHT: f32 = 26.;
/// The hover group of a row, for cells that brighten with it.
pub const ROW_GROUP: &str = "table-row";
/// A cell's padding on either side, in dp.
pub const CELL_PAD: f32 = 10.;
/// The width of a row's glyph column, whose glyph sits at its centre.
pub const GLYPH_WIDTH: f32 = 34.;
/// The widest a fixed column grows from its text.
pub const WIDEST: f32 = 280.;
/// The widest the flexible column's least width grows; it takes any room
/// left over beyond that.
pub const WIDEST_FLEXIBLE: f32 = 440.;

/// DESIGN.md's widths, derived when the data changes: 7.5 a character of
/// the longest text or the label, plus 24, at least 64 and at most `most`
/// ([`WIDEST`] or [`WIDEST_FLEXIBLE`]).
pub fn fit<'a>(label: &str, texts: impl Iterator<Item = &'a SharedString>, most: f32) -> f32 {
    let chars = texts
        .map(|text| text.chars().count())
        .chain([label.chars().count()])
        .max()
        .unwrap_or_default();
    (chars as f32 * 7.5 + 24.).clamp(64., most)
}

/// What the table needs to know of a column.
pub trait TableColumn {
    fn label(&self) -> &SharedString;
    /// Its width, or with [`flexible`](Self::flexible) its least width.
    fn width(&self) -> f32;
    /// Takes the room left over; at most one column does.
    fn flexible(&self) -> bool;
    /// Stays at the table's left edge when it scrolls sideways, as a row's
    /// glyph and name do. Only the leading run of pinned columns pins.
    fn pinned(&self) -> bool {
        false
    }
}

/// A cell's frame: padded, truncated, at its column's width.
pub fn cell(column: &impl TableColumn) -> Div {
    let cell = div()
        .px(dp(CELL_PAD))
        .min_w_0()
        .whitespace_nowrap()
        .truncate();
    if column.flexible() {
        cell.flex_1().min_w(dp(column.width()))
    } else {
        cell.flex_none().w(dp(column.width()))
    }
}

/// A glyph column's cell: unpadded, its glyph (or a marked row's box)
/// centred, so every page's glyphs sit on one line with their groups'.
pub fn glyph_cell(column: &impl TableColumn) -> Div {
    debug_assert_eq!(
        column.width(),
        GLYPH_WIDTH,
        "a glyph column is GLYPH_WIDTH wide"
    );
    cell(column)
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .px_0()
}

/// A group's header row: its glyph and label, an optional subject such as
/// a node, the details after them and the group's actions.
pub struct GroupRow {
    id: ElementId,
    tone: Tone,
    label: SharedString,
    subject: Option<SharedString>,
    detail: Vec<String>,
    actions: Vec<AnyElement>,
    after: Vec<String>,
    room: Option<f32>,
    height: f32,
}

impl GroupRow {
    pub fn new(
        id: impl Into<ElementId>,
        tone: Tone,
        label: impl Into<SharedString>,
        height: f32,
    ) -> Self {
        Self {
            id: id.into(),
            tone,
            label: label.into(),
            subject: None,
            detail: Vec::new(),
            actions: Vec::new(),
            after: Vec::new(),
            room: None,
            height,
        }
    }

    pub fn subject(mut self, subject: Option<impl Into<SharedString>>) -> Self {
        self.subject = subject.map(Into::into);
        self
    }

    /// The details, joined with `·`.
    pub fn detail(mut self, detail: Vec<String>) -> Self {
        self.detail = detail;
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    /// Details drawn after the actions, joined with `·`, so the actions
    /// stay near the start where a drawer may cover the row's end.
    pub fn after(mut self, after: Vec<String>) -> Self {
        self.after = after;
        self
    }

    /// The width in dp left in sight, when something covers the row's
    /// end, such as a drawer over the list: the row lays its text and
    /// actions out within it, the text truncating so that the glyph and
    /// the actions keep their width.
    pub fn room(mut self, room: Option<f32>) -> Self {
        self.room = room;
        self
    }

    pub fn render(self, cx: &App) -> Observed<Stateful<Div>> {
        let p = palette(cx);
        let color = match self.tone {
            Tone::Crit | Tone::Died => p.crit_ink,
            Tone::Warn => p.warn_ink,
            Tone::Good => p.good_ink,
            _ => p.muted,
        };
        let detail = self.detail.join(" · ");
        let after = self.after.join(" · ");
        let glyph = glyph_slot(self.tone, cx)
            .id((self.id.clone(), "glyph"))
            .test_support()
            .into_any_element();
        let id = self.id.clone();
        let aria = if after.is_empty() {
            format!("{} · {detail}", self.label)
        } else {
            format!("{} · {detail} · {after}", self.label)
        };
        let (height, room) = (self.height, self.room);
        let contents = self.contents(glyph, color, detail, after, p.muted);
        h_flex()
            .id(id)
            .test_support()
            .role(Role::Heading)
            .aria_label(aria)
            .w_full()
            .h(dp(height))
            .bg(p.track.opacity(0.45))
            .border_b_1()
            .border_color(p.line)
            .text_size(dp(12.))
            .child(
                h_flex()
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .when_some(room, |this, room| this.flex_none().w(dp(room)))
                    .pr_3()
                    .gap(dp(CELL_PAD))
                    .overflow_hidden()
                    .children(contents),
            )
    }

    /// The row's parts. What gives way when the room is short goes from
    /// the end: the details after the actions, then the count, and, only
    /// beside a cover (`room`), the subject and then the label. Each part
    /// keeps its whole width until the one before it in that order is
    /// gone: shared shrink factors would cut every part a little at once,
    /// so each run that must outlast the next sits in a box that doesn't
    /// shrink and is capped at its parent's width (`run`).
    fn contents(
        self,
        glyph: AnyElement,
        color: gpui_kit::Hsla,
        detail: String,
        after: String,
        muted: gpui_kit::Hsla,
    ) -> Vec<AnyElement> {
        let (covered, height) = (self.room.is_some(), self.height);
        let part = |what: &'static str| {
            div()
                .id((self.id.clone(), what))
                .test_support()
                .whitespace_nowrap()
        };
        let label = part("label")
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(color)
            .child(self.label);
        let subject = self
            .subject
            .map(|subject| part("subject").font_family(MONO_FONT).child(subject));
        let count = whole_or_gone(
            part("count")
                .truncate()
                .text_color(muted)
                .child(format!("· {detail}")),
            height,
        )
        .min_w_0();
        let actions = h_flex()
            .id((self.id.clone(), "actions"))
            .test_support()
            .flex_none()
            .gap_1()
            .children(self.actions);
        let after = (!after.is_empty()).then(|| {
            whole_or_gone(
                part("after")
                    .truncate()
                    .text_color(muted)
                    .child(format!("· {after}")),
                height,
            )
            .min_w_0()
        });
        let mut parts = vec![glyph];
        // The text before the actions, and the actions after it: the run
        // stays whole while the details after it give way.
        let leading = if covered {
            // Beside a cover, the label and subject give way too, after
            // the count: the subject first, the label last.
            let named = run()
                .child(
                    whole_or_gone(label.truncate(), height)
                        .flex_shrink_0()
                        .max_w_full(),
                )
                .children(
                    subject.map(|subject| whole_or_gone(subject.truncate(), height).min_w_0()),
                );
            h_flex()
                .min_w_0()
                .gap(dp(CELL_PAD))
                .child(named)
                .child(count)
                .into_any_element()
        } else {
            // Otherwise they keep their width, as the table's other pages
            // expect, and the count alone gives way.
            parts.push(label.flex_none().into_any_element());
            parts.extend(subject.map(|subject| subject.flex_none().into_any_element()));
            count.into_any_element()
        };
        parts.push(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(dp(CELL_PAD))
                .child(run().child(leading).child(actions))
                .children(after)
                .into_any_element(),
        );
        parts
    }
}

/// The least width a part that gives way shows: a letter and the
/// ellipsis. Narrower, it goes whole rather than leave a sliver.
const LEAST_PART: f32 = 20.;

/// A part that gives way by truncating, and goes whole once it can't
/// show [`LEAST_PART`]: it then wraps onto a second line, out of sight
/// below a strut as tall as the row, `height` dp.
fn whole_or_gone(part: Observed<Stateful<Div>>, height: f32) -> Div {
    h_flex()
        .flex_wrap()
        .content_start()
        .h(dp(height))
        .overflow_hidden()
        .child(div().w_0().h(dp(height)))
        .child(part.max_w_full().min_w(dp(LEAST_PART)))
}

/// A run of a group row's parts that keeps its width while what follows
/// it gives way, then gives way itself within its parent's width.
fn run() -> Div {
    h_flex().flex_shrink_0().max_w_full().gap(dp(CELL_PAD))
}

/// A line's leading glyph in a slot where a row's glyph column sits:
/// inside the row's 1 px border, [`GLYPH_WIDTH`] wide, the glyph centred.
/// A line with no border of its own and no left padding, as a group row
/// is, starts with it; with a [`CELL_PAD`] gap after it, its text then
/// starts where the next column's does.
pub fn glyph_slot(tone: Tone, cx: &App) -> Div {
    div()
        .flex_none()
        .ml(px(1.))
        .w(dp(GLYPH_WIDTH))
        .flex()
        .items_center()
        .justify_center()
        .children(ui::status_glyph(tone, cx))
}

/// The table's footer: 26 high under a hairline, on the table's `fill`.
/// The counts come first and keep their width; the legend after them
/// truncates at the row's end and never wraps.
pub(crate) fn footer(
    id: impl Into<ElementId>,
    counts: Vec<AnyElement>,
    legend: Option<AnyElement>,
    fill: gpui_kit::Hsla,
    cx: &App,
) -> Observed<Stateful<Div>> {
    let p = palette(cx);
    let mut row = h_flex()
        .id(id)
        .test_support()
        .role(Role::Status)
        .flex_none()
        .h(dp(FOOTER_HEIGHT))
        .px(dp(12.))
        .gap(dp(12.))
        .bg(fill)
        .border_t_1()
        .border_color(p.line)
        .text_size(dp(11.))
        .text_color(p.muted)
        .overflow_hidden();
    let separated = counts.len() > 1;
    for (ix, count) in counts.into_iter().enumerate() {
        if separated && ix > 0 {
            row = row.child(div().flex_none().text_color(p.faint).child("·"));
        }
        row = row.child(div().flex_none().child(count));
    }
    row.children(legend.map(|legend| h_flex().flex_1().min_w_0().overflow_hidden().child(legend)))
}

/// A count in the footer: its text in `ink_2`, then its actions.
fn count(
    id: impl Into<ElementId>,
    text: String,
    weight: FontWeight,
    actions: impl IntoIterator<Item = AnyElement>,
    cx: &App,
) -> Observed<Stateful<Div>> {
    h_flex()
        .id(id)
        .test_support()
        .role(Role::Status)
        .gap(dp(6.))
        .text_color(palette(cx).ink_2)
        .child(div().font_weight(weight).child(text))
        .children(actions)
}

/// How many rows are marked, and what can be done with them, for the
/// table's footer.
pub fn selection(
    id: impl Into<ElementId>,
    count_: usize,
    actions: impl IntoIterator<Item = AnyElement>,
    cx: &App,
) -> Observed<Stateful<Div>> {
    count(
        id,
        format!("{count_} selected"),
        FontWeight::SEMIBOLD,
        actions,
        cx,
    )
}

/// `Showing 40 of 212` while rows are folded, and the action that shows
/// them all, for the table's footer.
pub fn showing(
    id: impl Into<ElementId>,
    shown: usize,
    total: usize,
    show_all: impl IntoElement,
    cx: &App,
) -> Observed<Stateful<Div>> {
    count(
        id,
        format!("Showing {shown} of {total}"),
        FontWeight::NORMAL,
        [show_all.into_any_element()],
        cx,
    )
}

/// `Showing 8 of 20` as a strip above a list that isn't a [`DataTable`]:
/// Overview's Attention card, until Overview moves onto the table.
pub fn showing_bar(
    id: impl Into<ElementId>,
    shown: usize,
    total: usize,
    show_all: impl IntoElement,
    cx: &App,
) -> Observed<Stateful<Div>> {
    let p = palette(cx);
    h_flex()
        .bg(p.accent_soft)
        .id(id)
        .test_support()
        .role(Role::Status)
        .px_3()
        .py(dp(5.))
        .gap_2()
        .border_b_1()
        .border_color(p.line)
        .text_size(dp(12.5))
        .text_color(p.ink_2)
        .flex_none()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(format!("Showing {shown} of {total}")),
        )
        .child(show_all)
}

/// What the table's marks mean, for its footer: the items on one line.
pub fn legend(items: impl IntoIterator<Item = AnyElement>, _cx: &App) -> Div {
    h_flex().flex_none().gap(dp(12.)).children(items)
}

/// One entry of a [`legend`]: an example mark and what it means.
pub fn legend_item(mark: impl IntoElement, text: impl Into<SharedString>) -> Div {
    h_flex()
        .flex_none()
        .gap(dp(5.))
        .child(mark)
        .child(text.into())
}

/// The legend in a narrow table: one line that truncates, the whole legend
/// in its tooltip.
pub fn legend_line(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    tooltip: &'static str,
    _cx: &App,
) -> Stateful<Div> {
    div()
        .id(id)
        .min_w_0()
        .truncate()
        .child(text.into())
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
}

/// One status chip: the glyph and how many rows it marks. The caller
/// handles the click, which filters to that status or clears the filter.
pub fn status_chip(
    id: impl Into<ElementId>,
    tone: Tone,
    count: usize,
    what: &str,
    selected: bool,
    cx: &App,
) -> Button {
    let p = palette(cx);
    Button::new(id)
        .ghost()
        .small()
        .px(dp(6.))
        .selected(selected)
        .children(ui::status_glyph(tone, cx))
        .child(div().child(count.to_string()))
        .text_color(p.ink_2)
        .accessibility_label(format!("{count} {what}"))
        .tooltip(format!("{count} {what} · click to filter"))
}

/// The row of [`status_chip`]s, critical first.
pub fn status_chips(
    id: impl Into<ElementId>,
    chips: impl IntoIterator<Item = Button>,
    cx: &App,
) -> Observed<Stateful<Div>> {
    h_flex()
        .id(id)
        .test_support()
        .gap(dp(2.))
        .font_family(MONO_FONT)
        .text_size(dp(12.))
        .text_color(palette(cx).muted)
        .children(chips)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_follows_the_longest_text_between_its_bounds() {
        let texts: Vec<SharedString> = vec!["ab".into(), "abcdefghijkl".into()];
        assert_eq!(fit("Size", texts.iter(), WIDEST), 12. * 7.5 + 24.);
        // A short column keeps the least width; the label counts too.
        assert_eq!(fit("PID", [].iter(), WIDEST), 64.);
        assert_eq!(fit("Transport", [].iter(), WIDEST), 9. * 7.5 + 24.);
        let long: SharedString = "x".repeat(100).into();
        assert_eq!(fit("Model", [&long].into_iter(), WIDEST), WIDEST);
        assert_eq!(
            fit("Model", [&long].into_iter(), WIDEST_FLEXIBLE),
            WIDEST_FLEXIBLE
        );
        // Characters, not bytes.
        let tree: SharedString = "├─ │  ".into();
        assert_eq!(fit("", [&tree].into_iter(), WIDEST), 6. * 7.5 + 24.);
    }

    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AnyWindowHandle, Context, Entity, Render, TestAppContext, Window, size};

    use crate::ui::dp_px;

    /// A group row as Pods draws one, beside a cover `room` dp wide, or
    /// uncovered in a window `width` wide.
    struct Covered {
        room: Option<f32>,
        width: f32,
    }

    impl Render for Covered {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let row = GroupRow::new("g", Tone::Warn, "Node not ready", 26.)
                .subject(Some("talos-wk-fra1-02"))
                .detail(vec!["3 pods".into()])
                .action(div().id("g-act").flex_none().w(dp(60.)).h(dp(18.)))
                .after(vec![
                    "NotReady for 4m".into(),
                    "metrics are last known".into(),
                ])
                .room(self.room)
                .render(cx);
            div().w(dp(self.width)).child(row)
        }
    }

    fn open_row(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Covered>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            cx.set_reduce_motion(true);
        });
        let mut view = None;
        let handle = cx.open_window(size(px(1600.), px(200.)), |window, cx| {
            let row = cx.new(|_| Covered {
                room: None,
                width: 1200.,
            });
            view = Some(row.clone());
            Root::new(row, window, cx)
        });
        (handle.into(), view.unwrap())
    }

    /// The parts' widths, in the order they give way.
    const ORDER: [&str; 4] = ["after", "count", "subject", "label"];

    /// The parts' widths in sight: a part that went whole, wrapped
    /// below the row, counts as 0. A part in sight is never narrower
    /// than [`LEAST_PART`], so none leaves a sliver.
    fn widths(window: &mut Window, cx: &mut App) -> Vec<f32> {
        window.render_frame(cx);
        let row = window.find("g").bounds();
        let least = f32::from(dp_px(LEAST_PART, window)) - 0.5;
        ORDER
            .iter()
            .map(|part| {
                let bounds = window.find((ElementId::from("g"), *part)).bounds();
                let width = f32::from(bounds.size.width);
                // Below the row's text, above its 1 px border.
                if bounds.top() >= row.bottom() - px(1.) {
                    return 0.;
                }
                assert!(width >= least, "{part} shows a sliver {width} wide");
                width
            })
            .collect()
    }

    /// Beside a cover, each part keeps its whole width until the one
    /// before it in the order is gone, and the actions stay in the room.
    #[gpui_kit::test]
    fn a_covered_group_row_gives_way_one_part_at_a_time(cx: &mut TestAppContext) {
        let (handle, view) = open_row(cx);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |v, _| v.room = Some(1200.));
            let whole = widths(window, cx);
            assert!(whole.iter().all(|w| *w > 20.), "{whole:?}");
            let mut room = 1200.;
            let mut seen = [false; 4];
            while room > 150. {
                room -= 4.;
                view.update(cx, |v, _| v.room = Some(room));
                let now = widths(window, cx);
                for (ix, (w, full)) in now.iter().zip(&whole).enumerate() {
                    if *w < full - 0.5 {
                        seen[ix] = true;
                        for (before, gone) in now[..ix].iter().enumerate() {
                            assert!(
                                *gone <= 0.5,
                                "room {room}: {} gives way while {} is {gone} wide",
                                ORDER[ix],
                                ORDER[before]
                            );
                        }
                    }
                }
                let action = window.find((ElementId::from("g"), "actions")).bounds();
                let row = window.find("g").bounds();
                assert!(
                    f32::from(action.right() - row.left()) <= room + 0.5,
                    "room {room}: the action ends at {:?}",
                    action.right()
                );
                assert_eq!(f32::from(action.size.width), f32::from(dp_px(60., window)));
            }
            assert_eq!(seen, [true; 4], "every part gave way by 150 dp");
        })
        .unwrap();
    }

    /// Uncovered, as on every page but Pods beside its drawer, the label
    /// and subject keep their width while the details give way, the
    /// details after the actions first.
    #[gpui_kit::test]
    fn an_uncovered_group_row_keeps_its_label_and_subject(cx: &mut TestAppContext) {
        let (handle, view) = open_row(cx);
        cx.update_window(handle, |_, window, cx| {
            let whole = widths(window, cx);
            let mut width = 1200.;
            while width > 300. {
                width -= 4.;
                view.update(cx, |v, _| v.width = width);
                let now = widths(window, cx);
                assert_eq!(
                    &now[2..],
                    &whole[2..],
                    "width {width}: label or subject cut"
                );
                if now[1] < whole[1] - 0.5 {
                    assert!(now[0] <= 0.5, "width {width}: count cut before the rest");
                }
            }
            assert!(widths(window, cx)[1] < whole[1], "the count gave way");
        })
        .unwrap();
    }
}
