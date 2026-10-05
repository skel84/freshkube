//! `DataTable`: the header, the virtualised rows and their selection, for
//! any page whose entity implements [`TableSource`]. Generic rather than
//! dynamic, so a 20,000-row list costs what a hand-written one does.
use std::hash::Hash;
use std::ops::Range;

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, h_flex, tooltip::Tooltip, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, Div, ElementId, Role, ScrollStrategy, SharedString,
    TestSupportExt, UniformListScrollHandle, Window, div, uniform_list,
};

use super::{COMPACT_ROW_HEIGHT, HEADER_HEIGHT, ROW_GROUP, ROW_HEIGHT, TableColumn, cell};
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
        }
    }
}

/// What a table keeps between frames, owned by the page's entity.
pub struct TableState {
    pub scroll: UniformListScrollHandle,
    /// Compact rows, from the density toggle; comfortable by default.
    pub compact: bool,
    ids: TableIds,
}

impl TableState {
    /// The table's ids are `<prefix>-list`, `-rows`, `-table-scroll` and
    /// `-empty`; each labelled header cell is `(<prefix>-sort, column)`.
    pub fn new(prefix: &str) -> Self {
        Self {
            scroll: UniformListScrollHandle::new(),
            compact: false,
            ids: TableIds::new(prefix),
        }
    }

    /// `<prefix>-<part>`, for the parts the page draws around the table.
    pub fn id(&self, part: &str) -> SharedString {
        format!("{}-{part}", self.ids.prefix).into()
    }

    pub fn row_height(&self) -> f32 {
        if self.compact {
            COMPACT_ROW_HEIGHT
        } else {
            ROW_HEIGHT
        }
    }

    /// Scrolls a line into view.
    pub fn reveal(&self, line: usize, strategy: ScrollStrategy) {
        self.scroll.scroll_to_item(line, strategy);
    }
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
    /// Bars above the rows: the selection, folded rows.
    fn notes(&self, _cx: &mut Context<Self>) -> Vec<AnyElement> {
        Vec::new()
    }
    /// The legend below the rows.
    fn footer(&self, _window: &Window, _cx: &mut Context<Self>) -> Option<AnyElement> {
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

/// How a table sits on its page: in its card or bare, filling its parent
/// or as tall as its rows.
#[derive(Clone, Copy, Default)]
pub struct DataTable {
    bare: bool,
    fit: Option<usize>,
}

impl DataTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Without its card, for a table inside a card of its own.
    pub fn bare(mut self) -> Self {
        self.bare = true;
        self
    }

    /// As tall as the header and its lines, at most `max_lines` of them,
    /// for a table in a scrolling page rather than one that fills it.
    pub fn fit(mut self, max_lines: usize) -> Self {
        self.fit = Some(max_lines);
        self
    }

    /// Notes, the header, the rows or the empty state, and the footer.
    pub fn render<S: TableSource>(self, source: &S, window: &Window, cx: &mut Context<S>) -> Div {
        let state = source.table_state();
        let ids = &state.ids;
        let empty = source.empty(cx);
        let is_empty = empty.is_some();
        // A fitted list is as tall as its lines; an empty one as its state.
        let list_height = self
            .fit
            .filter(|_| empty.is_none())
            .map(|max| source.line_count().min(max) as f32 * state.row_height());
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
                        cx.processor(|view: &mut S, range: Range<usize>, _, cx| {
                            range
                                .filter_map(|line| render_line(view, line, cx))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(&state.scroll)
                    .size_full(),
                ),
            });
        let frame = if self.bare {
            v_flex().min_w_0()
        } else {
            card(cx)
        };
        let fitted = self.fit.is_some();
        // The empty state sits below the sideways scroll, in the card's
        // width: inside it, a table scrolled right would hide it.
        let (inside, below) = if is_empty {
            (None, Some(list))
        } else {
            (Some(list), None)
        };
        frame
            .overflow_hidden()
            .children(source.notes(cx))
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
                    .child(
                        v_flex()
                            .when(!fitted && !is_empty, |this| this.h_full())
                            .w_full()
                            .min_w(dp(source.width()))
                            .child(header(source, cx))
                            .children(inside),
                    ),
            )
            .children(below)
            .children(source.footer(window, cx))
    }
}

/// The table in its card, filling the room its caller gives it.
pub fn data_table<S: TableSource>(source: &S, window: &Window, cx: &mut Context<S>) -> Div {
    DataTable::new().render(source, window, cx)
}

fn header<S: TableSource>(source: &S, cx: &mut Context<S>) -> Div {
    let p = palette(cx);
    let sort_id = &source.table_state().ids.sort;
    h_flex()
        .w_full()
        .h(dp(HEADER_HEIGHT))
        .flex_none()
        .bg(p.surface_2)
        .border_b_1()
        .border_color(p.line)
        .children(source.columns().iter().enumerate().map(|(ix, column)| {
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
                    .child(ui::caption(label, cx))
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
                .child(ui::caption(label, cx))
                .children(order.map(|(_, icon)| Icon::new(icon).size(dp(12.)).text_color(p.muted)))
                .on_click(cx.listener(move |view, _, _, cx| view.sort(sort.clone(), cx)))
                .into_any_element()
        }))
}

/// One line of the list: a group's header or a row.
fn render_line<S: TableSource>(source: &S, line: usize, cx: &mut Context<S>) -> Option<AnyElement> {
    let row = match source.line(line, cx)? {
        Line::Group(group) => return source.group(group, cx),
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
        .h(dp(source.table_state().row_height()))
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
    for column in source.columns() {
        element = element.child(source.cell(&row, &style, column, cx));
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
    use gpui_kit::{AppContext, IntoElement, Render, ScrollDelta, TestAppContext, point, px, size};

    use super::*;

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

    struct Column(SharedString);

    impl TableColumn for Column {
        fn label(&self) -> &SharedString {
            &self.0
        }

        fn width(&self) -> f32 {
            200.
        }

        fn flexible(&self) -> bool {
            false
        }
    }

    /// Six 200 dp columns, wider than its window, so it scrolls sideways.
    struct Wide {
        table: TableState,
        columns: Vec<Column>,
        rows: usize,
    }

    impl TableSource for Wide {
        type Key = usize;
        type Sort = ();
        type Column = Column;
        type Row<'a> = ();

        fn table_state(&self) -> &TableState {
            &self.table
        }

        fn columns(&self) -> &[Column] {
            &self.columns
        }

        fn width(&self) -> f32 {
            self.columns.len() as f32 * 200.
        }

        fn list_label(&self) -> String {
            "Wide rows".into()
        }

        fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
            None
        }

        fn sort(&mut self, _: (), _: &mut Context<Self>) {}

        fn line_count(&self) -> usize {
            self.rows
        }

        fn line(&self, line: usize, _: &App) -> Option<Line<usize, ()>> {
            (line < self.rows).then(|| {
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
            _: &mut Context<Self>,
        ) -> AnyElement {
            cell(column)
                .child(format!("{} {}", column.0, row.key))
                .into_any_element()
        }

        fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
            None
        }

        fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
            (self.rows == 0).then(|| "No rows match this filter.".into_any_element())
        }
    }

    impl Render for Wide {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .size_full()
                .child(data_table(self, window, cx).flex_1().min_h_0())
        }
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
        let mut wide = None;
        let handle = cx.open_window(size(px(600.), px(400.)), |window, cx| {
            let view = cx.new(|_| Wide {
                table: TableState::new("wide"),
                columns: (0..6)
                    .map(|ix| Column(format!("Column {ix}").into()))
                    .collect(),
                rows: 3,
            });
            wide = Some(view.clone());
            Root::new(view, window, cx)
        });
        let wide = wide.unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
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
        cx.update_window(handle.into(), |_, window, cx| {
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
}
