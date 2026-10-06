//! `DataTable`: the header, the virtualised rows and their selection, for
//! any page whose entity implements [`TableSource`]. Generic rather than
//! dynamic, so a 20,000-row list costs what a hand-written one does.
use std::hash::Hash;
use std::ops::Range;

use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme, Icon, h_flex, tooltip::Tooltip, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, Div, ElementId, Hsla, Role, ScrollHandle, ScrollStrategy,
    SharedString, TestSupportExt, UniformListScrollHandle, Window, div, px, uniform_list,
};

use super::pinned::{Passing, Pinned, Watch, pins, scrolled_by};
use super::{HEADER_HEIGHT, ROW_GROUP, ROW_HEIGHT, TableColumn, cell};
use crate::page::card;
use crate::palette::{Palette, palette};
use crate::ui::{self, MONO_FONT, dp};

/// The order a column is sorted in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

/// One line of the list, as the page's projection yields it.
pub enum Line<K, R> {
    Row(TableRow<K, R>),
    /// A group's header, by the page's own index.
    Group(usize),
}

/// A row: its key, how it shows, and the page's data for its cells.
pub struct TableRow<K, R> {
    /// What selects it; a click hands it back to the page.
    pub key: K,
    /// Derived from what the row shows, so a press that lands after the
    /// rows moved can't complete on another object.
    pub id: ElementId,
    pub label: SharedString,
    /// Shown on hover, as a truncated name's full text.
    pub tooltip: Option<SharedString>,
    pub marked: bool,
    /// Muted text, as a terminating object's.
    pub muted: bool,
    pub data: R,
}

/// How the table draws a row this frame, handed to each of its cells.
pub struct RowStyle {
    /// The row is the page's [`selected_key`](TableSource::selected_key).
    pub selected: bool,
    /// The palette, with muted text brightened on a selected or marked row.
    pub p: Palette,
}

/// The ids a table gives its parts, from the page's prefix.
struct TableIds {
    prefix: SharedString,
    list: SharedString,
    rows: SharedString,
    scroll: SharedString,
    empty: SharedString,
    sort: SharedString,
    pinned: SharedString,
}

impl TableIds {
    fn new(prefix: &str) -> Self {
        let id = |part: &str| SharedString::from(format!("{prefix}-{part}"));
        Self {
            prefix: prefix.to_owned().into(),
            list: id("list"),
            rows: id("rows"),
            scroll: id("table-scroll"),
            empty: id("empty"),
            sort: id("sort"),
            pinned: id("pinned"),
        }
    }
}

/// What a table keeps between frames, owned by the page's entity.
pub struct TableState {
    pub scroll: UniformListScrollHandle,
    /// The sideways scroll, which group labels and pinned columns undo.
    sideways: ScrollHandle,
    ids: TableIds,
}

impl TableState {
    /// The table's ids are `<prefix>-list`, `-rows`, `-table-scroll` and
    /// `-empty`; each labelled header cell is `(<prefix>-sort, column)`.
    pub fn new(prefix: &str) -> Self {
        Self {
            scroll: UniformListScrollHandle::new(),
            sideways: ScrollHandle::new(),
            ids: TableIds::new(prefix),
        }
    }

    /// `<prefix>-<part>`, for the parts the page draws around the table.
    pub fn id(&self, part: &str) -> SharedString {
        format!("{}-{part}", self.ids.prefix).into()
    }

    /// Scrolls a line into view.
    pub fn reveal(&self, line: usize, strategy: ScrollStrategy) {
        self.scroll.scroll_to_item(line, strategy);
    }

    /// Whether the table is scrolled sideways. Until it is, nothing pins
    /// and the rows draw as if pinning didn't exist.
    fn scrolled(&self) -> bool {
        scrolled_by(&self.sideways).is_some()
    }

    /// Whether a pinned run `run` dp wide stays at the left edge, from the
    /// scroll's last frame; `Watch` draws again if this one disagrees.
    fn pins(&self, run: f32, window: &Window) -> bool {
        pins(&self.sideways, run, window)
    }
}

/// How many leading columns pin, and their width.
fn pinned_run<C: TableColumn>(columns: &[C]) -> (usize, f32) {
    let run = columns.iter().take_while(|column| column.pinned());
    run.fold((0, 0.), |(count, width), column| {
        (count + 1, width + column.width())
    })
}

/// A page entity that shows a table. Everything it returns was derived
/// when its data changed; these only read it.
pub trait TableSource: Sized + 'static {
    /// Selects a row: `Hash + Eq + Clone`, so it can carry a connection or
    /// a session as well as the object.
    type Key: Clone + Eq + Hash + 'static;
    /// What a header click sorts by.
    type Sort: Clone + 'static;
    type Column: TableColumn;
    /// The page's data for one row's cells, borrowed for the frame.
    type Row<'a>
    where
        Self: 'a;

    fn table_state(&self) -> &TableState;
    fn columns(&self) -> &[Self::Column];
    /// The sum of the columns' widths; the table scrolls sideways below it.
    fn width(&self) -> f32;
    /// The list's accessibility label: what it shows and its keys.
    fn list_label(&self) -> String;
    /// What a sortable column sorts by and whether it is sorted now; `None`
    /// for a column that doesn't sort.
    fn sorting(&self, column: &Self::Column) -> Option<(Self::Sort, Option<SortOrder>)>;
    /// A header click: the page decides the order, as a cycle.
    fn sort(&mut self, sort: Self::Sort, cx: &mut Context<Self>);
    fn line_count(&self) -> usize;
    fn line(&self, line: usize, cx: &App) -> Option<Line<Self::Key, Self::Row<'_>>>;
    fn cell(
        &self,
        row: &TableRow<Self::Key, Self::Row<'_>>,
        style: &RowStyle,
        column: &Self::Column,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement>;
    /// The selected row's key. The table marks the row with it; selection
    /// is by key, never by position.
    fn selected_key(&self) -> Option<&Self::Key> {
        None
    }
    /// The line a key's row is on now, for [`step`] and [`reveal`].
    fn line_of(&self, _key: &Self::Key) -> Option<usize> {
        None
    }
    /// Whether a row click does anything. A table whose rows don't select
    /// returns `false`: no pointer, no hover, no click.
    fn clickable(&self) -> bool {
        true
    }
    /// A row click, with its count, modifiers and button.
    fn click(
        &mut self,
        _key: &Self::Key,
        _event: &ClickEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }
    /// What replaces the rows when there are none, such as a line or a
    /// title and a hint; the table gives it its padding and muted text.
    fn empty(&self, cx: &mut Context<Self>) -> Option<AnyElement>;
    /// The footer's counts, first in its row: the selection
    /// ([`super::selection`]) and folded rows ([`super::showing`]).
    fn counts(&self, _cx: &mut Context<Self>) -> Vec<AnyElement> {
        Vec::new()
    }
    /// What the table's marks mean, after the counts in its footer
    /// ([`super::legend`], or [`super::legend_line`] when narrow).
    fn legend(&self, _window: &Window, _cx: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
}

/// The key of the row `delta` rows from the selected one, skipping group
/// lines and stopping at either end. With nothing selected, a step down
/// lands on the first row and a step up on the last.
pub fn step<S: TableSource>(source: &S, delta: isize, cx: &App) -> Option<S::Key> {
    let from = source.selected_key().and_then(|key| source.line_of(key));
    let is_row = |line| match source.line(line, cx) {
        Some(Line::Row(_)) => Some(true),
        Some(Line::Group(_)) => Some(false),
        None => None,
    };
    let line = step_line(source.line_count(), from, delta, is_row)?;
    match source.line(line, cx)? {
        Line::Row(row) => Some(row.key),
        Line::Group(_) => None,
    }
}

/// Scrolls the selected row into view.
pub fn reveal<S: TableSource>(source: &S, strategy: ScrollStrategy) {
    if let Some(line) = source.selected_key().and_then(|key| source.line_of(key)) {
        source.table_state().reveal(line, strategy);
    }
}

/// The line `delta` rows from `from`, where `is_row` tells a row's line
/// from a group's.
fn step_line(
    count: usize,
    from: Option<usize>,
    delta: isize,
    is_row: impl Fn(usize) -> Option<bool>,
) -> Option<usize> {
    let row = |line: usize| is_row(line) == Some(true);
    let down = delta >= 0;
    let mut at = match from {
        Some(line) if line < count => line,
        _ => {
            let mut lines: Box<dyn Iterator<Item = usize>> = if down {
                Box::new(0..count)
            } else {
                Box::new((0..count).rev())
            };
            return lines.find(|line| row(*line));
        }
    };
    for _ in 0..delta.unsigned_abs() {
        let next = if down {
            (at + 1..count).find(|line| row(*line))
        } else {
            (0..at).rev().find(|line| row(*line))
        };
        match next {
            Some(line) => at = line,
            None => break,
        }
    }
    Some(at)
}

/// How a table sits on its page: bare or in its card, filling its parent
/// or as tall as its rows.
#[derive(Clone, Copy, Default)]
pub struct DataTable {
    carded: bool,
    inset: bool,
    fit: Option<usize>,
}

impl DataTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// In a card of its own, for a table among cards on a scrolling page.
    /// Without it the table is bare: on its pane's background, edge to
    /// edge, with a hairline above and below.
    pub fn carded(mut self) -> Self {
        self.carded = true;
        self
    }

    /// Inside a card its caller draws, such as a dashboard panel's: on the
    /// card's surface, edge to edge, with a hairline above.
    pub fn inset(mut self) -> Self {
        self.inset = true;
        self
    }

    /// As tall as the header and its lines, at most `max_lines` of them,
    /// for a table in a scrolling page rather than one that fills it.
    pub fn fit(mut self, max_lines: usize) -> Self {
        self.fit = Some(max_lines);
        self
    }

    /// The header, the rows or the empty state, and the footer with the
    /// counts and the legend when there are any.
    pub fn render<S: TableSource>(self, source: &S, window: &Window, cx: &mut Context<S>) -> Div {
        let state = source.table_state();
        let ids = &state.ids;
        // What the rows sit on, which pinned cells paint to stay opaque.
        let fill = if self.carded || self.inset {
            palette(cx).surface
        } else {
            cx.theme().background
        };
        let empty = source.empty(cx);
        let is_empty = empty.is_some();
        // A fitted list is as tall as its lines; an empty one as its state.
        let list_height = self
            .fit
            .filter(|_| empty.is_none())
            .map(|max| source.line_count().min(max) as f32 * ROW_HEIGHT);
        let list = div()
            .id(ids.list.clone())
            .test_support()
            .role(Role::ListBox)
            .aria_label(source.list_label())
            .map(|this| match list_height {
                Some(height) => this.flex_none().h(dp(height)),
                None if self.fit.is_some() => this.flex_none(),
                None => this.flex_1().min_h_0(),
            })
            .map(|this| match empty {
                Some(empty) => this.child(
                    div()
                        .id(ids.empty.clone())
                        .test_support()
                        .px_3()
                        .py_3p5()
                        .text_size(dp(12.5))
                        .text_color(palette(cx).muted)
                        .child(empty),
                ),
                None => this.child(
                    uniform_list(
                        ids.rows.clone(),
                        source.line_count(),
                        cx.processor(move |view: &mut S, range: Range<usize>, window, cx| {
                            range
                                .filter_map(|line| render_line(view, line, fill, window, cx))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(&state.scroll)
                    .size_full()
                    // A wheel reaches this list and the sideways scroll around
                    // it; unrestricted, each takes the other axis's delta for
                    // its own, and a sideways swipe also moves the rows (#93).
                    .map(|mut list| {
                        list.style().restrict_scroll_to_axis = Some(true);
                        list
                    }),
                ),
            });
        let frame = if self.carded {
            card(cx)
        } else if self.inset {
            v_flex()
                .min_w_0()
                .bg(fill)
                .border_t_1()
                .border_color(palette(cx).line)
        } else {
            v_flex()
                .min_w_0()
                .bg(fill)
                .border_t_1()
                .border_b_1()
                .border_color(palette(cx).line)
        };
        let fitted = self.fit.is_some();
        // The empty state sits below the sideways scroll, in the table's
        // width: inside it, a table scrolled right would hide it.
        let (inside, below) = if is_empty {
            (None, Some(list))
        } else {
            (Some(list), None)
        };
        let (counts, legend) = (source.counts(cx), source.legend(window, cx));
        let footer = (!counts.is_empty() || legend.is_some())
            .then(|| super::footer(state.id("footer"), counts, legend, fill, cx));
        frame
            .overflow_hidden()
            .child(
                div()
                    .id(ids.scroll.clone())
                    .test_support()
                    .when_else(
                        fitted || is_empty,
                        |this| this.flex_none(),
                        |this| this.flex_1().min_h_0(),
                    )
                    .w_full()
                    .overflow_x_scroll()
                    .restrict_scroll_to_axis()
                    .track_scroll(&state.sideways)
                    .child(
                        v_flex()
                            .when(!fitted && !is_empty, |this| this.h_full())
                            .w_full()
                            .min_w(dp(source.width()))
                            .child(header(source, window, cx))
                            .children(inside),
                    ),
            )
            .children(below)
            .children(footer)
    }
}

/// The bare table, filling the room its caller gives it.
pub fn data_table<S: TableSource>(source: &S, window: &Window, cx: &mut Context<S>) -> Div {
    DataTable::new().render(source, window, cx)
}

fn header<S: TableSource>(source: &S, window: &Window, cx: &mut Context<S>) -> AnyElement {
    let p = palette(cx);
    let state = source.table_state();
    let columns = source.columns();
    let (count, run) = pinned_run(columns);
    let pinned = count > 0 && state.pins(run, window);
    let header = h_flex()
        .w_full()
        .h(dp(HEADER_HEIGHT))
        .flex_none()
        .bg(p.surface_2)
        .border_b_1()
        .border_color(p.line)
        .children(columns.iter().enumerate().map(|(ix, column)| {
            // A pinned cell's place is kept by an empty one of its width,
            // and the cells after it are clipped where they pass under it,
            // their labels kept beside it while their columns show.
            if !pinned {
                header_cell(source, ix, column, cx)
            } else if ix < count {
                cell(column).into_any_element()
            } else {
                let cell = header_cell(source, ix, column, cx);
                Passing::sticky(&state.sideways, run, cell).into_any_element()
            }
        }));
    if count == 0 || !state.scrolled() {
        return header.into_any_element();
    }
    let header = if pinned {
        let cells: Vec<_> = (columns.iter().enumerate().take(count))
            .map(|(ix, column)| header_cell(source, ix, column, cx))
            .collect();
        header.relative().child(Pinned::overlay(
            &state.sideways,
            run,
            h_flex()
                .id(state.id("pinned-header"))
                .test_support()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(dp(run))
                .bg(p.surface_2)
                .children(cells),
            p.line,
        ))
    } else {
        header
    };
    // Built from the scroll's last frame; if this frame's differs, as after
    // a resize, the table draws again to match it.
    Watch::new(&state.sideways, run, pinned, cx.entity_id(), header).into_any_element()
}

fn header_cell<S: TableSource>(
    source: &S,
    ix: usize,
    column: &S::Column,
    cx: &mut Context<S>,
) -> AnyElement {
    let p = palette(cx);
    let sort_id = &source.table_state().ids.sort;
    let label = column.label();
    let Some((sort, order)) = source.sorting(column) else {
        // A column without a label, such as the glyph's, stays bare.
        if label.is_empty() {
            return cell(column).into_any_element();
        }
        return cell(column)
            .id((sort_id.clone(), ix))
            .test_support()
            .role(Role::ColumnHeader)
            .aria_label(label.clone())
            .flex()
            .items_center()
            .child(ui::column_label(label, cx))
            .into_any_element();
    };
    let order = order.map(|order| match order {
        SortOrder::Ascending => ("ascending", IconName::ArrowUp),
        SortOrder::Descending => ("descending", IconName::ArrowDown),
    });
    cell(column)
        .id((sort_id.clone(), ix))
        .test_support()
        .role(Role::ColumnHeader)
        .aria_label(match order {
            Some((order, _)) => format!("{label}, sorted {order}"),
            None => label.to_string(),
        })
        .flex()
        .items_center()
        .gap_1()
        .cursor_pointer()
        .child(ui::column_label(label, cx))
        .children(order.map(|(_, icon)| Icon::new(icon).size(dp(12.)).text_color(p.muted)))
        .on_click(cx.listener(move |view, _, _, cx| view.sort(sort.clone(), cx)))
        .into_any_element()
}

/// One line of the list: a group's header or a row.
fn render_line<S: TableSource>(
    source: &S,
    line: usize,
    fill: Hsla,
    window: &Window,
    cx: &mut Context<S>,
) -> Option<AnyElement> {
    // Rows draw while the list lays out, after the sideways scroll has
    // clamped its offset, so this is the offset the frame paints with.
    let state = source.table_state();
    let scrolled = state.scrolled();
    let row = match source.line(line, cx)? {
        // A group's line stays in view. In a table wider than its view
        // the line is as wide as the view, so its details truncate before
        // its actions; one that fits draws it as wide as the table.
        Line::Group(group) => {
            let group = source.group(group, cx)?;
            let view = state.sideways.bounds().size.width;
            let overflows = view > px(0.) && ui::dp_px(source.width(), window) > view + px(0.5);
            return Some(if overflows {
                Pinned::new(&state.sideways, div().w(view).child(group)).into_any_element()
            } else if scrolled {
                Pinned::new(&state.sideways, group).into_any_element()
            } else {
                group
            });
        }
        Line::Row(row) => row,
    };
    let selected = source.selected_key() == Some(&row.key);
    let (marked, muted) = (row.marked, row.muted);
    let mut p = palette(cx);
    if selected || marked {
        p.muted = p.ink_2;
    }
    let style = RowStyle { selected, p };
    let clickable = source.clickable();
    let mut element = h_flex()
        .group(ROW_GROUP)
        .id(row.id.clone())
        .test_support()
        .role(Role::ListBoxOption)
        .aria_selected(selected)
        .aria_label(row.label.clone())
        .w_full()
        .h(dp(ROW_HEIGHT))
        .border_1()
        .border_color(if selected {
            p.accent
        } else {
            ui::transparent()
        })
        .font_family(MONO_FONT)
        .text_size(dp(12.5))
        .when(muted, |this| this.text_color(p.muted))
        .when(marked && !selected, |this| this.bg(p.hover))
        .when(selected, |this| this.bg(p.accent_soft))
        .when(clickable, |this| this.cursor_pointer())
        .when(clickable && !selected, |this| {
            this.hover(|style| {
                let style = style.bg(p.hover);
                if muted {
                    style.text_color(p.ink_2)
                } else {
                    style
                }
            })
        })
        .when_some(row.tooltip.clone(), |this, tooltip| {
            this.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        });
    let columns = source.columns();
    let (count, run) = pinned_run(columns);
    let pinned = count > 0 && state.pins(run, window);
    for (ix, column) in columns.iter().enumerate() {
        // A pinned cell's place is kept by an empty one of its width; the
        // cell itself draws last, over the cells that pass under it, which
        // are clipped there so they don't take its hover or clicks. Inside
        // the row's border, the run starts a pixel in.
        element = element.child(if !pinned {
            source.cell(&row, &style, column, cx)
        } else if ix < count {
            cell(column).into_any_element()
        } else {
            let cell = source.cell(&row, &style, column, cx);
            Passing::new(&state.sideways, run, px(1.), cell).into_any_element()
        });
    }
    if pinned {
        let cells: Vec<_> = (columns.iter().take(count))
            .map(|column| source.cell(&row, &style, column, cx))
            .collect();
        // Opaque, so the cells passing under them don't show through: the
        // table's fill with the row's own over it, in each of its
        // states. They stay inside the row, so its hover, tooltip and click
        // reach them unchanged.
        let tint = |this: Div| {
            this.when(marked && !selected, |this| this.bg(p.hover))
                .when(selected, |this| this.bg(p.accent_soft))
                .when(clickable && !selected, |this| {
                    this.group_hover(ROW_GROUP, |style| style.bg(p.hover))
                })
        };
        // The row's left border, transparent unless it is selected, would
        // let the passing cells show through; this edge covers it.
        let edge = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(-1.))
            .w(px(1.))
            .bg(if selected { p.accent } else { fill })
            .when(!selected, |this| this.child(tint(div().size_full())));
        element = element.relative().child(Pinned::overlay(
            &state.sideways,
            run,
            div()
                .id((state.ids.pinned.clone(), line))
                .test_support()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(dp(run))
                .bg(fill)
                .child(edge)
                .child(tint(h_flex().size_full()).children(cells)),
            p.line,
        ));
    }
    let key = row.key;
    Some(
        element
            .when(clickable, |this| {
                this.on_click(
                    cx.listener(move |view, event, window, cx| view.click(&key, event, window, cx)),
                )
            })
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AnyView, AnyWindowHandle, AppContext, Background, Entity, InputEvent, IntoElement,
        MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render,
        ScrollDelta, ScrollWheelEvent, StyleRefinement, TestAppContext, TouchPhase, point, px,
        size,
    };

    use super::*;
    use crate::table::{CELL_PAD, FOOTER_HEIGHT, GLYPH_WIDTH, GroupRow, glyph_cell};
    use crate::ui::Tone;

    /// Lines 0 and 3 are group headers; the rest are rows.
    fn lines(line: usize) -> Option<bool> {
        (line < 7).then_some(line != 0 && line != 3)
    }

    #[test]
    fn steps_over_group_lines() {
        assert_eq!(step_line(7, Some(2), 1, lines), Some(4));
        assert_eq!(step_line(7, Some(4), -1, lines), Some(2));
        assert_eq!(step_line(7, Some(1), 3, lines), Some(5));
    }

    #[test]
    fn stops_at_either_end() {
        assert_eq!(step_line(7, Some(6), 1, lines), Some(6));
        assert_eq!(step_line(7, Some(1), -1, lines), Some(1));
        assert_eq!(step_line(7, Some(5), 20, lines), Some(6));
    }

    #[test]
    fn starts_at_the_first_or_last_row() {
        assert_eq!(step_line(7, None, 1, lines), Some(1));
        assert_eq!(step_line(7, None, -1, lines), Some(6));
        assert_eq!(step_line(7, Some(99), 1, lines), Some(1));
        assert_eq!(step_line(0, None, 1, lines), None);
        assert_eq!(step_line(1, None, 1, lines), None);
    }

    /// A label, and whether the column pins.
    struct Column(SharedString, bool);

    impl TableColumn for Column {
        fn label(&self) -> &SharedString {
            &self.0
        }

        fn width(&self) -> f32 {
            if self.0 == "Glyph" { GLYPH_WIDTH } else { 200. }
        }

        fn flexible(&self) -> bool {
            false
        }

        fn pinned(&self) -> bool {
            self.1
        }
    }

    /// Six 200 dp columns, wider than its window, so it scrolls sideways.
    /// With `grouped`, line 0 is a group's header and the rows follow it.
    struct Wide {
        table: TableState,
        columns: Vec<Column>,
        rows: usize,
        grouped: bool,
        selected: Option<usize>,
        /// Cells and headers that take clicks and record them, as
        /// Applications' report buttons do.
        interactive: bool,
        cell_clicks: Vec<SharedString>,
        hovered: Vec<SharedString>,
        sorts: Vec<SharedString>,
        carded: bool,
        /// The first column holds each row's glyph, as a page's does.
        glyphs: bool,
        /// How many rows are marked, for the footer's counts.
        marked: Option<usize>,
        /// The footer's legend line.
        legend: Option<SharedString>,
    }

    impl Wide {
        fn new(pinned: usize, rows: usize, grouped: bool) -> Self {
            Self {
                table: TableState::new("wide"),
                columns: (0..6)
                    .map(|ix| Column(format!("Column {ix}").into(), ix < pinned))
                    .collect(),
                rows,
                grouped,
                selected: None,
                interactive: false,
                cell_clicks: Vec::new(),
                hovered: Vec::new(),
                sorts: Vec::new(),
                carded: false,
                glyphs: false,
                marked: None,
                legend: None,
            }
        }

        /// `marked` rows marked and four of nine showing, then `legend`.
        fn footer(self, marked: Option<usize>, legend: Option<&str>) -> Self {
            Self {
                marked,
                legend: legend.map(|legend| legend.to_owned().into()),
                ..self
            }
        }

        /// The first column becomes a glyph column, as a page's is.
        fn glyphs(mut self) -> Self {
            self.columns[0].0 = "Glyph".into();
            Self {
                glyphs: true,
                ..self
            }
        }

        fn carded(self) -> Self {
            Self {
                carded: true,
                ..self
            }
        }

        fn interactive(self) -> Self {
            Self {
                interactive: true,
                ..self
            }
        }

        fn first_row(&self) -> usize {
            usize::from(self.grouped)
        }
    }

    impl TableSource for Wide {
        type Key = usize;
        type Sort = SharedString;
        type Column = Column;
        type Row<'a> = ();

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
            "Wide rows".into()
        }

        fn sorting(&self, column: &Column) -> Option<(SharedString, Option<SortOrder>)> {
            self.interactive.then(|| (column.0.clone(), None))
        }

        fn sort(&mut self, column: SharedString, _: &mut Context<Self>) {
            self.sorts.push(column);
        }

        fn line_count(&self) -> usize {
            self.rows + self.first_row()
        }

        fn line(&self, line: usize, _: &App) -> Option<Line<usize, ()>> {
            if self.grouped && line == 0 {
                return Some(Line::Group(0));
            }
            (line < self.line_count()).then(|| {
                Line::Row(TableRow {
                    key: line,
                    id: SharedString::from(format!("wide-row-{line}")).into(),
                    label: format!("Row {line}").into(),
                    tooltip: None,
                    marked: false,
                    muted: false,
                    data: (),
                })
            })
        }

        fn cell(
            &self,
            row: &TableRow<usize, ()>,
            _: &RowStyle,
            column: &Column,
            cx: &mut Context<Self>,
        ) -> AnyElement {
            let label = SharedString::from(format!("{} {}", column.0, row.key));
            if self.glyphs && column.0 == "Glyph" {
                return glyph_cell(column)
                    .id(("wide-glyph", row.key))
                    .test_support()
                    .children(ui::status_glyph(Tone::Good, cx))
                    .into_any_element();
            }
            let (click, hover) = (label.clone(), label.clone());
            cell(column)
                .id(label.clone())
                .test_support()
                .child(label)
                .when(self.interactive, |this| {
                    this.on_click(
                        cx.listener(move |wide, _, _, _| wide.cell_clicks.push(click.clone())),
                    )
                    .on_hover(cx.listener(
                        move |wide, hovered: &bool, _, _| {
                            if *hovered {
                                wide.hovered.push(hover.clone())
                            }
                        },
                    ))
                })
                .into_any_element()
        }

        fn group(&self, _: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
            Some(
                GroupRow::new("wide-group", Tone::Crit, "Failing", ROW_HEIGHT)
                    // Longer than the table is wide, as a node's group's can be.
                    .detail(vec![
                        format!("{} rows", self.rows),
                        "a detail that runs on ".repeat(12),
                    ])
                    .action(
                        div()
                            .id("wide-group-action")
                            .test_support()
                            .child("Select all"),
                    )
                    .render(cx)
                    .into_any_element(),
            )
        }

        fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
            (self.rows == 0).then(|| "No rows match this filter.".into_any_element())
        }

        fn selected_key(&self) -> Option<&usize> {
            self.selected.as_ref()
        }

        fn line_of(&self, key: &usize) -> Option<usize> {
            Some(*key)
        }

        fn click(&mut self, key: &usize, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
            self.selected = Some(*key);
            cx.notify();
        }

        fn counts(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
            let Some(marked) = self.marked else {
                return Vec::new();
            };
            vec![
                super::super::selection("wide-marks", marked, [], cx).into_any_element(),
                super::super::showing("wide-showing", 4, 9, div().child("Show all 9"), cx)
                    .into_any_element(),
            ]
        }

        fn legend(&self, _: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
            let legend = self.legend.clone()?;
            Some(
                super::super::legend_line("wide-legend", legend, "The legend", cx)
                    .test_support()
                    .into_any_element(),
            )
        }
    }

    impl Render for Wide {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let table = if self.carded {
                DataTable::new().carded()
            } else {
                DataTable::new()
            };
            v_flex()
                .size_full()
                .child(table.render(self, window, cx).flex_1().min_h_0())
        }
    }

    /// The table in a window `width` wide and 400 tall, at the default
    /// text size.
    fn open(cx: &mut TestAppContext, wide: Wide, width: f32) -> (AnyWindowHandle, Entity<Wide>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            cx.set_reduce_motion(true);
        });
        let mut view = None;
        let handle = cx.open_window(size(px(width), px(400.)), |window, cx| {
            let wide = cx.new(|_| wide);
            view = Some(wide.clone());
            Root::new(wide, window, cx)
        });
        (handle.into(), view.unwrap())
    }

    /// Scrolls the table `by` points to the right.
    fn scroll_right(window: &mut Window, by: f32, cx: &mut App) {
        window.scroll(
            "wide-table-scroll",
            ScrollDelta::Pixels(point(px(-by), px(0.))),
            cx,
        );
    }

    /// How far right of the table's visible left edge an element starts.
    fn inset(window: &Window, id: impl Into<ElementId>) -> f32 {
        let viewport = window.find("wide-table-scroll").bounds();
        f32::from(window.find(id).bounds().left() - viewport.left())
    }

    /// The same for a pinned cell on `line`, or a header cell with `line`
    /// `None`, found inside the pinned part.
    fn pinned_inset(window: &mut Window, line: Option<usize>, id: impl Into<ElementId>) -> f32 {
        let viewport = window.find("wide-table-scroll").bounds();
        let scope: ElementId = match line {
            Some(line) => ("wide-pinned", line).into(),
            None => "wide-pinned-header".into(),
        };
        f32::from(window.within(scope).find(id).bounds().left() - viewport.left())
    }

    /// Sends one wheel event over the middle of the table, as the start of a
    /// gesture: a trackpad's gesture locks to the axis it starts on.
    fn wheel(window: &mut Window, x: f32, y: f32, cx: &mut App) {
        window.render_frame(cx);
        let position = window.find("wide-table-scroll").bounds().center();
        window.dispatch_event(
            MouseMoveEvent {
                position,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(x), px(y))),
                touch_phase: TouchPhase::Started,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    }

    /// How far below the table's top edge an element starts.
    fn top_of(window: &Window, id: impl Into<ElementId>) -> f32 {
        let viewport = window.find("wide-table-scroll").bounds();
        f32::from(window.find(id).bounds().top() - viewport.top())
    }

    /// A sideways wheel scrolls the columns and leaves the rows where they
    /// were: the list inside doesn't take it for a vertical scroll (#93).
    #[gpui_kit::test]
    fn a_sideways_wheel_scrolls_only_the_columns(cx: &mut TestAppContext) {
        let (handle, _) = open(cx, Wide::new(2, 30, true), 900.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let top = top_of(window, "Column 4 1");
            wheel(window, -100., 0., cx);
            let left = inset(window, "Column 4 1");
            assert!((left - 700.).abs() <= 1.5, "no sideways scroll: {left}");
            let moved = top - top_of(window, "Column 4 1");
            assert!(moved.abs() <= 0.5, "the rows moved {moved}");
        })
        .unwrap();
    }

    /// A plain wheel over a table wider than its window scrolls the rows and
    /// leaves the columns at the left edge: the sideways scroll around them
    /// doesn't take it for a sideways one.
    #[gpui_kit::test]
    fn a_vertical_wheel_scrolls_only_the_rows(cx: &mut TestAppContext) {
        let (handle, _) = open(cx, Wide::new(2, 30, true), 900.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let top = top_of(window, "Column 4 8");
            wheel(window, 0., -100., cx);
            let moved = top - top_of(window, "Column 4 8");
            assert!(
                (moved - 100.).abs() <= 1.5,
                "the rows moved {moved}, not 100"
            );
            let left = inset(window, ("wide-sort", 4usize));
            assert!((left - 800.).abs() <= 1.5, "the columns moved: {left}");
            assert!(window.try_find("wide-pinned-header").is_none());
        })
        .unwrap();
    }

    /// A swipe that is mostly sideways keeps to that axis, so a trackpad's
    /// slight drift doesn't move the rows too.
    #[gpui_kit::test]
    fn a_mostly_sideways_swipe_keeps_to_its_axis(cx: &mut TestAppContext) {
        let (handle, _) = open(cx, Wide::new(2, 30, true), 900.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let top = top_of(window, "Column 4 1");
            wheel(window, -100., -10., cx);
            let left = inset(window, "Column 4 1");
            assert!((left - 700.).abs() <= 1.5, "no sideways scroll: {left}");
            let moved = top - top_of(window, "Column 4 1");
            assert!(moved.abs() <= 0.5, "the rows moved {moved}");
        })
        .unwrap();
    }

    /// A filter that leaves no rows after a sideways scroll still shows why,
    /// inside the table's width.
    #[gpui_kit::test]
    fn the_empty_state_shows_after_a_sideways_scroll(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            cx.set_reduce_motion(true);
        });
        let (handle, wide) = open(cx, Wide::new(0, 3, false), 600.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.scroll(
                "wide-table-scroll",
                ScrollDelta::Pixels(point(px(-500.), px(0.))),
                cx,
            );
            let viewport = window.find("wide-table-scroll").bounds();
            let row = window.find("wide-row-0").bounds();
            assert!(
                row.left() < viewport.left() - px(400.),
                "the rows didn't scroll sideways: {row:?} in {viewport:?}"
            );
        })
        .unwrap();
        cx.update(|cx| {
            wide.update(cx, |wide, cx| {
                wide.rows = 0;
                cx.notify();
            })
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let viewport = window.find("wide-table-scroll").bounds();
            let empty = window.find("wide-empty");
            let bounds = empty.bounds();
            assert!(
                bounds.left() >= viewport.left() - px(1.)
                    && bounds.right() <= viewport.right() + px(1.),
                "the empty state at {bounds:?} leaves the table's width {viewport:?}"
            );
            assert!(empty.visible());
        })
        .unwrap();
    }

    /// Scrolled sideways, a group's label and the pinned columns stay at
    /// the table's left edge while the other cells pass under them.
    #[gpui_kit::test]
    fn group_labels_and_pinned_columns_stay_in_view_when_scrolled(cx: &mut TestAppContext) {
        // The two pinned columns' 400 take less than two thirds of the 900.
        let (handle, _) = open(cx, Wide::new(2, 4, true), 900.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(inset(window, "Column 0 1").abs() <= 1.5);
            assert!(window.try_find("wide-pinned-header").is_none());
            scroll_right(window, 100., cx);
            assert!(
                (inset(window, "Column 4 1") - 700.).abs() <= 1.5,
                "no scroll"
            );
            assert!(inset(window, "wide-group").abs() <= 1.5);
            // Inside the row's one-pixel border, as before the scroll.
            assert!(pinned_inset(window, Some(1), "Column 0 1").abs() <= 1.5);
            assert!((pinned_inset(window, Some(1), "Column 1 1") - 200.).abs() <= 1.5);
            assert!(pinned_inset(window, None, ("wide-sort", 0usize)).abs() <= 1.5);
            assert!((pinned_inset(window, None, ("wide-sort", 1usize)) - 200.).abs() <= 1.5);
            // Each cell is drawn once, so its id finds one element.
            assert!(inset(window, "Column 0 1").abs() <= 1.5);
            assert!(inset(window, ("wide-sort", 0usize)).abs() <= 1.5);
            // Back at the left edge, nothing pins.
            scroll_right(window, -100., cx);
            assert!(window.try_find(("wide-pinned", 1usize)).is_none());
            assert!(window.try_find("wide-pinned-header").is_none());
            assert!(inset(window, "Column 0 1").abs() <= 1.5);
        })
        .unwrap();
    }

    /// Bare, the table draws no card: only a hairline above and below, on
    /// the window's background, which its pinned cells paint too so they
    /// stay opaque. Carded, it keeps the rounded card, and its pinned cells
    /// paint the card's surface.
    #[gpui_kit::test]
    fn a_bare_table_draws_hairlines_and_its_pinned_cells_match_it(cx: &mut TestAppContext) {
        for carded in [false, true] {
            let wide = Wide::new(2, 4, true);
            let wide = if carded { wide.carded() } else { wide };
            let (handle, _) = open(cx, wide, 900.);
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                scroll_right(window, 100., cx);
                let scale = window.scale_factor();
                let view = window.find("wide-table-scroll").bounds().scale(scale);
                let quads = window.painted_quads();
                // What frames the table: a bordered quad around its scroll.
                // GPUI splits a border into clipped strips that share its
                // geometry, and paints the fill as a quad of its own.
                let mut frames: Vec<_> = (quads.iter())
                    .filter(|quad| {
                        let (b, w) = (quad.bounds, quad.border_widths);
                        (w.top.0 > 0. || w.left.0 > 0.)
                            && b.left() <= view.left()
                            && b.right() >= view.right()
                            && b.top() <= view.top()
                            && b.bottom() >= view.bottom()
                    })
                    .collect();
                frames.dedup_by(|a, b| a.bounds == b.bounds && a.border_widths == b.border_widths);
                assert_eq!(frames.len(), 1, "carded {carded}: {frames:#?}");
                let frame = frames[0];
                let (sides, corner) = (frame.border_widths, frame.corner_radii.top_left.0);
                let fill = if carded {
                    assert!(sides.left.0 > 0. && sides.right.0 > 0., "{frame:#?}");
                    assert!((corner - 12. * scale).abs() < 0.5, "{frame:#?}");
                    palette(cx).surface
                } else {
                    assert!(sides.top.0 > 0. && sides.bottom.0 > 0., "{frame:#?}");
                    assert!(sides.left.0 == 0. && sides.right.0 == 0., "{frame:#?}");
                    assert_eq!(corner, 0., "{frame:#?}");
                    cx.theme().background
                };
                assert!(
                    quads.iter().any(|quad| {
                        quad.bounds == frame.bounds && quad.background == Background::from(fill)
                    }),
                    "carded {carded}: the table isn't filled with {fill:?}"
                );
                // The pinned run, 400 wide, paints the same fill.
                let run = crate::ui::dp_px(400., window).scale(scale).0;
                assert!(
                    quads.iter().any(|quad| {
                        quad.background == Background::from(fill)
                            && (quad.bounds.size.width.0 - run).abs() < 1.
                    }),
                    "carded {carded}: no pinned run on the table's fill"
                );
            })
            .unwrap();
        }
    }

    /// The counts and the legend share one 26 row under the rows: the
    /// counts first at their width, the legend truncating after them; no
    /// row when there is neither.
    #[gpui_kit::test]
    fn the_footer_holds_the_counts_then_the_legend(cx: &mut TestAppContext) {
        let long = "Meters: band = request · line = use · end = limit ".repeat(8);
        let (handle, _) = open(
            cx,
            Wide::new(0, 4, false).footer(Some(3), Some(&long)),
            600.,
        );
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let footer = window.find("wide-footer");
            assert_eq!(
                footer.bounds().size.height,
                ui::dp_px(FOOTER_HEIGHT, window)
            );
            let rows = window.find("wide-table-scroll").bounds();
            assert!(
                footer.bounds().top() >= rows.bottom(),
                "the footer sits under the rows"
            );
            // Nothing sits between the table's top and its header.
            let header = ui::dp_px(HEADER_HEIGHT, window);
            assert_eq!(rows.top(), window.find("wide-list").bounds().top() - header);
            let (marks, showing, legend) = (
                window.find("wide-marks"),
                window.find("wide-showing"),
                window.find("wide-legend").bounds(),
            );
            assert!(marks.bounds().right() <= showing.bounds().left());
            assert!(showing.bounds().right() <= legend.left());
            // The legend truncates inside the row; the counts keep their width.
            assert!(legend.right() <= footer.bounds().right(), "{legend:?}");
            assert!(showing.bounds().right() < footer.bounds().right());
        })
        .unwrap();
        for wide in [Wide::new(0, 4, false), Wide::new(0, 0, false)] {
            let (handle, _) = open(cx, wide, 600.);
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find("wide-footer").is_none());
            })
            .unwrap();
        }
    }

    /// Pinned columns wider than two thirds of the table's visible width
    /// scroll with the rest, built as if unpinned, so they never cover it;
    /// the group's label still stays.
    #[gpui_kit::test]
    fn pinned_columns_too_wide_for_the_table_scroll(cx: &mut TestAppContext) {
        let (handle, _) = open(cx, Wide::new(2, 4, true), 560.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            scroll_right(window, 500., cx);
            assert!(inset(window, "wide-group").abs() <= 1.5);
            assert!(window.try_find(("wide-pinned", 1usize)).is_none());
            assert!(window.try_find("wide-pinned-header").is_none());
            assert!((inset(window, "Column 0 1") + 499.).abs() <= 1.5);
            assert!((inset(window, ("wide-sort", 0usize)) + 500.).abs() <= 1.5);
        })
        .unwrap();
    }

    /// Over the pinned run, only the row and the pinned cells take the
    /// mouse: the cells passing under it are clipped there, so they neither
    /// hover nor take a click, in the rows and in the header. Right of the
    /// run they take both as before.
    #[gpui_kit::test]
    fn cells_passing_under_the_pinned_run_dont_take_the_mouse(cx: &mut TestAppContext) {
        let (handle, wide) = open(cx, Wide::new(2, 4, false).interactive(), 900.);
        // Scrolled 100, Column 2 runs from 300 to 500, its first 100 under
        // the pinned Column 1, which runs from 200 to 400.
        let under_the_run = point(px(150.), px(5.));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            scroll_right(window, 100., cx);
            assert!(window.find("wide-pinned-header").visible());
            window
                .within(("wide-pinned", 1usize))
                .click_at("Column 1 1", under_the_run, cx);
            window
                .within("wide-pinned-header")
                .click_at(("wide-sort", 1usize), under_the_run, cx);
        })
        .unwrap();
        cx.read(|cx| {
            let wide = wide.read(cx);
            assert_eq!(wide.selected, Some(1));
            assert_eq!(wide.cell_clicks, ["Column 1 1"]);
            assert!(!wide.hovered.iter().any(|label| label == "Column 2 1"));
            assert_eq!(wide.sorts, ["Column 1"]);
        });
        cx.update_window(handle, |_, window, cx| {
            window.click_at("Column 2 1", point(px(150.), px(5.)), cx);
            // The header's cell has moved beside the run, so its visible
            // part starts at its own left.
            window.click_at(("wide-sort", 2usize), point(px(50.), px(5.)), cx);
        })
        .unwrap();
        cx.read(|cx| {
            let wide = wide.read(cx);
            assert_eq!(wide.cell_clicks, ["Column 1 1", "Column 2 1"]);
            assert!(wide.hovered.iter().any(|label| label == "Column 2 1"));
            assert_eq!(wide.sorts, ["Column 1", "Column 2"]);
        });
    }

    /// A column passing under the pinned run keeps its label beside the run
    /// while any of it shows, clipped to its own place, though its rows'
    /// cells pass under as before. A short label would otherwise vanish
    /// while a long cell's tail still shows, leaving the column unnamed.
    #[gpui_kit::test]
    fn a_passing_column_keeps_its_label_beside_the_run(cx: &mut TestAppContext) {
        let (handle, _) = open(cx, Wide::new(2, 4, false).interactive(), 900.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // Column 2 runs from 300 to 500, its first 100 under the run.
            scroll_right(window, 100., cx);
            let label = inset(window, ("wide-sort", 2usize));
            assert!(
                (label - 400.).abs() <= 1.5,
                "label at {label}, under the run"
            );
            assert!((inset(window, "Column 2 1") - 300.).abs() <= 1.5);
            // A column right of the run stays where it is.
            assert!((inset(window, ("wide-sort", 3usize)) - 500.).abs() <= 1.5);
            // Back at the left edge, the label returns to its column.
            scroll_right(window, -100., cx);
            assert!((inset(window, ("wide-sort", 2usize)) - 400.).abs() <= 1.5);
            scroll_right(window, 50., cx);
            let label = inset(window, ("wide-sort", 2usize));
            assert!((label - 400.).abs() <= 1.5, "label at {label}");
        })
        .unwrap();
    }

    /// Moves the pointer to `position` and presses and releases there,
    /// through the window's own dispatch, without drawing a frame.
    fn press(window: &mut Window, position: Point<Pixels>, cx: &mut App) {
        let events = [
            MouseMoveEvent {
                position,
                ..Default::default()
            }
            .to_platform_input(),
            MouseDownEvent {
                position,
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            }
            .to_platform_input(),
            MouseUpEvent {
                position,
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            }
            .to_platform_input(),
        ];
        for event in events {
            window.dispatch_event(event, cx);
        }
    }

    /// A page that keeps its table in a cached view, as the Resources pane
    /// keeps its own.
    struct CachedTable(Entity<Wide>);

    impl Render for CachedTable {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            AnyView::from(self.0.clone()).cached(StyleRefinement::default().size_full())
        }
    }

    /// The first sideways scroll from the left edge pins on the frame it
    /// draws, inside a cached view too: the wheel notifies the table's view,
    /// which marks the cached views around it dirty. The frames here are
    /// the ones the app draws, through the caches, never `render_frame`.
    #[gpui_kit::test]
    fn the_first_scroll_pins_through_a_cached_view(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            cx.set_reduce_motion(true);
        });
        let mut wide = None;
        let handle: AnyWindowHandle = cx
            .open_window(size(px(900.), px(400.)), |window, cx| {
                let table = cx.new(|_| Wide::new(2, 4, true).interactive());
                wide = Some(table.clone());
                let page = cx.new(|_| CachedTable(table));
                Root::new(page, window, cx)
            })
            .into();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(window.try_find(("wide-pinned", 1usize)).is_none());
            let position = window.find("wide-table-scroll").bounds().center();
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                ScrollWheelEvent {
                    position,
                    delta: ScrollDelta::Pixels(point(px(-100.), px(0.))),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        })
        .unwrap();
        // The update's end drew the window the wheel left dirty.
        cx.update_window(handle, |_, window, cx| {
            assert!(
                (inset(window, "Column 4 1") - 700.).abs() <= 1.5,
                "no scroll"
            );
            assert!(inset(window, "wide-group").abs() <= 1.5);
            assert!(pinned_inset(window, Some(1), "Column 0 1").abs() <= 1.5);
            assert!(pinned_inset(window, None, ("wide-sort", 0usize)).abs() <= 1.5);
            // Nothing waits for a next frame, and the next one changes nothing.
            assert_eq!(window.simulate_next_frame(cx), 0);
        })
        .unwrap();
        cx.run_until_parked();
        // The cells passing under the run were clipped on that same frame:
        // a press over Column 1, where Column 2 passes under it, reaches
        // only the pinned cell.
        cx.update_window(handle, |_, window, cx| {
            assert!(pinned_inset(window, Some(1), "Column 0 1").abs() <= 1.5);
            let viewport = window.find("wide-table-scroll").bounds();
            let row = window.find("wide-row-1").bounds();
            press(
                window,
                point(viewport.left() + px(350.), row.center().y),
                cx,
            );
        })
        .unwrap();
        let wide = wide.unwrap();
        cx.read(|cx| assert_eq!(wide.read(cx).cell_clicks, ["Column 1 1"]));
    }

    /// A table without pinned columns still keeps its group labels in view,
    /// and draws its rows as it always has.
    #[gpui_kit::test]
    fn without_pinned_columns_only_group_labels_stay(cx: &mut TestAppContext) {
        let (handle, _) = open(cx, Wide::new(0, 4, true), 600.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            scroll_right(window, 500., cx);
            assert!(inset(window, "wide-group").abs() <= 1.5);
            assert!((inset(window, "Column 0 1") + 500.).abs() <= 1.5);
            assert!(window.try_find(("wide-pinned", 1usize)).is_none());
            assert!(window.try_find("wide-pinned-header").is_none());
        })
        .unwrap();
    }

    /// A click on a pinned cell selects its row, and stepping and revealing
    /// the selection keep the sideways scroll and the pinned cells.
    #[gpui_kit::test]
    fn pinned_cells_click_and_step_like_their_rows(cx: &mut TestAppContext) {
        let (handle, wide) = open(cx, Wide::new(2, 30, false), 900.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            scroll_right(window, 100., cx);
            // The wheel's sideways delta also moves the list down a little,
            // as GPUI scrolls a list without a sideways scroll of its own.
            window
                .within(("wide-pinned", 8usize))
                .click("Column 0 8", cx);
            assert_eq!(wide.read(cx).selected, Some(8));
            for _ in 0..14 {
                let next = step(wide.read(cx), 1, cx);
                wide.update(cx, |wide, cx| {
                    wide.selected = next;
                    reveal(wide, ScrollStrategy::Top);
                    cx.notify();
                });
            }
            window.render_frame(cx);
            assert_eq!(wide.read(cx).selected, Some(22));
            assert!(window.find("wide-row-22").visible());
            assert!(pinned_inset(window, Some(22), "Column 0 22").abs() <= 1.5);
            assert!((inset(window, "Column 4 22") - 700.).abs() <= 1.5);
        })
        .unwrap();
    }

    /// A window grown wide enough for the whole table clamps the scroll to
    /// its left edge without a scroll event; the pinned cells stay in their
    /// place, and the table draws again unpinned without any input.
    #[gpui_kit::test]
    fn a_resize_that_ends_the_scroll_unpins_at_once(cx: &mut TestAppContext) {
        let (handle, _) = open(cx, Wide::new(2, 4, true), 900.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            scroll_right(window, 100., cx);
            assert!(window.find("wide-pinned-header").visible());
        })
        .unwrap();
        cx.simulate_window_resize(handle, size(px(1400.), px(400.)));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(inset(window, ("wide-sort", 0usize)).abs() <= 1.5);
            let mut frames = 0;
            while window.simulate_next_frame(cx) > 0 {
                frames += 1;
                assert!(frames < 4, "the table didn't settle");
                window.render_frame(cx);
            }
            assert_eq!(frames, 1);
            assert!(window.try_find("wide-pinned-header").is_none());
            assert!(window.try_find(("wide-pinned", 1usize)).is_none());
            assert!((inset(window, "Column 4 1") - 800.).abs() <= 1.5);
            assert!(inset(window, "wide-group").abs() <= 1.5);
        })
        .unwrap();
    }

    /// A group's glyph sits on its rows' glyphs, and its label starts where
    /// the next column's text does, at any text size (#129).
    #[gpui_kit::test]
    fn group_glyphs_and_labels_line_up_with_the_rows(cx: &mut TestAppContext) {
        let (handle, _) = open(cx, Wide::new(0, 2, true).glyphs(), 1400.);
        for text in [13., 20.] {
            cx.update_window(handle, |_, window, cx| {
                crate::text_size::set(text, cx);
                window.render_frame(cx);
                let group = ElementId::from("wide-group");
                let slot = window.find((group.clone(), "glyph")).bounds();
                let glyph = window.find(("wide-glyph", 1usize)).bounds();
                assert!(
                    (slot.center().x - glyph.center().x).abs() < px(0.25),
                    "text {text}: group glyph at {:?}, row glyph at {:?}",
                    slot.center().x,
                    glyph.center().x
                );
                let label = window.find((group, "label")).bounds().left();
                let name = window.find("Column 1 1").bounds().left() + ui::dp_px(CELL_PAD, window);
                assert!(
                    (label - name).abs() < px(0.25),
                    "text {text}: group label at {label:?}, row text at {name:?}"
                );
            })
            .unwrap();
        }
    }

    /// In a table wider than its view, a group's actions stay in view,
    /// scrolled or not; in one that fits, its line is as wide as its rows
    /// (#128).
    #[gpui_kit::test]
    fn group_actions_stay_in_view(cx: &mut TestAppContext) {
        let (handle, _) = open(cx, Wide::new(2, 4, true), 900.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            for scroll in [0., 100., 400.] {
                scroll_right(window, scroll, cx);
                window.render_frame(cx);
                let viewport = window.find("wide-table-scroll").bounds();
                let action = window.find("wide-group-action").bounds();
                assert!(
                    action.right() <= viewport.right() + px(0.5),
                    "scrolled {scroll}: action ends at {:?}, view at {:?}",
                    action.right(),
                    viewport.right()
                );
                assert!(action.left() >= viewport.left());
                assert!(inset(window, "wide-group").abs() <= 1.5);
                let group = window.find("wide-group").bounds();
                assert!((group.size.width - viewport.size.width).abs() <= px(1.5));
            }
        })
        .unwrap();

        let (handle, _) = open(cx, Wide::new(2, 4, true), 1400.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let group = window.find("wide-group").bounds();
            let row = window.find("wide-row-1").bounds();
            assert_eq!(group.left(), row.left());
            assert_eq!(group.size.width, row.size.width);
        })
        .unwrap();
    }
}
