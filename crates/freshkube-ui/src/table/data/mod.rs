//! `DataTable`: the header, the virtualised rows and their selection, for
//! any page whose entity implements [`TableSource`]. Generic rather than
//! dynamic, so a 20,000-row list costs what a hand-written one does.
use std::cell::RefCell;
use std::hash::Hash;
use std::ops::Range;

use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::{ActiveTheme, Icon, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, Div, ElementId, FocusHandle, Hsla, Pixels, Role,
    ScrollHandle, ScrollStrategy, SharedString, Stateful, TestSupportExt, UniformListScrollHandle,
    WeakEntity, Window, div, point, px, uniform_list,
};

use super::pinned::{Passing, Pinned, Watch, pins, scrolled_by};
use super::{HEADER_HEIGHT, ROW_GROUP, ROW_HEIGHT, TableColumn, cell};
use crate::page::card;
use crate::palette::{Palette, palette};
use crate::tooltip::FollowTooltip as _;
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
    /// The loading rows the table last drew, stilled when it stops.
    loading: RefCell<Option<super::LoadingRows>>,
}

impl TableState {
    /// The table's ids are `<prefix>-list`, `-rows`, `-table-scroll` and
    /// `-empty`; each labelled header cell is `(<prefix>-sort, column)`.
    pub fn new(prefix: &str) -> Self {
        Self {
            scroll: UniformListScrollHandle::new(),
            sideways: ScrollHandle::new(),
            ids: TableIds::new(prefix),
            loading: RefCell::default(),
        }
    }

    /// `<prefix>-<part>`, for the parts the page draws around the table.
    pub fn id(&self, part: &str) -> SharedString {
        format!("{}-{part}", self.ids.prefix).into()
    }

    /// Where the rows are in the window, for a layer drawn over them, such
    /// as the change flash ([`super::FlashLayer`]).
    pub fn rows_at(&self) -> super::RowsAt {
        super::RowsAt {
            list: self.scroll.clone(),
            sideways: self.sideways.clone(),
        }
    }

    /// Remembers the loading rows the table draws now and stills the ones
    /// it stopped drawing, so their motion stops asking for frames.
    fn track_loading(&self, now: Option<&super::LoadingRows>) {
        let mut drawn = self.loading.borrow_mut();
        if let Some(old) = drawn.as_ref()
            && now.is_none_or(|rows| !rows.same(old))
        {
            old.hide();
        }
        *drawn = now.cloned();
    }

    /// Scrolls a line into view. `Nearest` keeps the rows whole under the
    /// header: a line below the window ends as its last whole row, and a
    /// line above it, or a window left on a part of a row, starts at a row,
    /// so no row is cut in half where it meets the header.
    pub fn reveal(&self, line: usize, strategy: ScrollStrategy) {
        match strategy {
            ScrollStrategy::Nearest => match self.whole_top(line) {
                Some(top) => self.scroll.scroll_to_item_strict(top, ScrollStrategy::Top),
                None => self.scroll.scroll_to_item(line, strategy),
            },
            _ => self.scroll.scroll_to_item(line, strategy),
        }
    }

    /// Scrolls the table sideways so the column spanning `left..left +
    /// width` dp from its left edge shows beside the pinned run of `pinned`
    /// dp, for a selection in a wide table. `false` before the table has
    /// drawn, when nothing can be said yet.
    pub fn reveal_column(&self, left: f32, width: f32, pinned: f32, window: &Window) -> bool {
        let view = self.sideways.bounds().size.width;
        if view <= Pixels::ZERO {
            return false;
        }
        let unit = ui::dp_px(1., window);
        let (from, to, pinned) = (unit * left, unit * (left + width), unit * pinned);
        let at = -self.sideways.offset().x;
        let target = if from < at + pinned {
            from - pinned
        } else if to > at + view {
            to - view
        } else {
            return true;
        };
        self.sideways
            .set_offset(point(-target.max(Pixels::ZERO), self.sideways.offset().y));
        true
    }

    /// The line the window should start at to show `line` with whole rows
    /// from the top, from the list's last frame; `None` before it has
    /// drawn, or when the window holds less than a row.
    fn whole_top(&self, line: usize) -> Option<usize> {
        let state = self.scroll.0.borrow();
        let row = f32::from(state.last_item_size?.item.height);
        let view = f32::from(state.base_handle.bounds().size.height);
        if row < 1. || view < row {
            return None;
        }
        // A fraction of a pixel over a whole row count is the hairlines'
        // rounding, not room for another row.
        let whole = ((view + 1.5) / row).floor() as usize;
        let scrolled = (-f32::from(state.base_handle.offset().y)).max(0.);
        let first = (scrolled / row - 0.01).ceil().max(0.) as usize;
        Some(if line < first {
            line
        } else if line < first + whole {
            first
        } else {
            line + 1 - whole
        })
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
    /// The height of a row, in dp. A table declares a taller row only when
    /// its cells hold two lines (DESIGN.md: the Overview matrix's 40 dp);
    /// the header stays at [`HEADER_HEIGHT`] and group lines at
    /// [`ROW_HEIGHT`], so a table with groups keeps the default.
    fn row_height(&self) -> f32 {
        ROW_HEIGHT
    }
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
    /// The focus a row's context menu dispatches its actions to: the
    /// page's list, where their keys are bound. `None`, the default, gives
    /// the rows no menu.
    fn menu_focus(&self, _cx: &App) -> Option<FocusHandle> {
        None
    }
    /// A row's context menu, asked as it opens (DESIGN.md change 10). The
    /// page selects the row first, through its usual selection, so every
    /// item acts on the selection as its key does. Empty, it shows nothing.
    fn row_menu(
        &mut self,
        _key: &Self::Key,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Vec<crate::menu::MenuAction> {
        Vec::new()
    }
    /// What replaces the rows until the first answer: the
    /// [`super::LoadingRows`] the page keeps, drawn still under the real
    /// header; the page draws their [`super::LoadingMotion`] over the table.
    /// While they show, [`empty`](Self::empty) isn't asked.
    fn loading(&self) -> Option<&super::LoadingRows> {
        None
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

/// How many frames [`reveal_when_settled`] waits at most for the table's
/// height to change.
const SETTLE_FRAMES: usize = 6;

/// Reveals the selected row on each of the next frames until the table's
/// height has changed and then held: a stacked inspector's split learns its
/// heights while drawing and applies them on a later frame, and a row
/// revealed in the taller table would end up under it. Each reveal asks for
/// the next frame, so the split's change is drawn; the wait is bounded when
/// the height never changes. `showing` says whether the row still has a
/// table to be revealed in; once it doesn't, the wait ends.
pub fn reveal_when_settled<S: TableSource>(
    view: WeakEntity<S>,
    showing: fn(&S) -> bool,
    window: &Window,
) {
    settle(view, showing, None, false, SETTLE_FRAMES, window);
}

fn settle<S: TableSource>(
    view: WeakEntity<S>,
    showing: fn(&S) -> bool,
    last: Option<Pixels>,
    changed: bool,
    frames: usize,
    window: &Window,
) {
    window.on_next_frame(move |window, cx| {
        let Some(height) = view
            .update(cx, |source, cx| {
                if !showing(source) {
                    return None;
                }
                reveal(source, ScrollStrategy::Nearest);
                cx.notify();
                Some(
                    (source.table_state().scroll.0.borrow().last_item_size)
                        .map(|size| size.item.height),
                )
            })
            .ok()
            .flatten()
        else {
            return;
        };
        let settled = changed && height == last;
        if frames > 1 && !settled {
            let changed = changed || (last.is_some() && height != last);
            settle(view, showing, height, changed, frames - 1, window);
        }
    });
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

/// A card's corner inside its hairline: `page::card`'s 12 px less 1.
const CARD_INNER_RADIUS: f32 = 11.;

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
    /// for a table in a scrolling page rather than one that fills it. While
    /// it loads, its loading rows take `max_lines`, at most
    /// [`LOADING_ROWS`](super::LOADING_ROWS).
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
        let loading = source.loading();
        state.track_loading(loading);
        let empty = if loading.is_some() {
            None
        } else {
            source.empty(cx)
        };
        let is_empty = empty.is_some();
        // A fitted list is as tall as its lines, or while loading as its
        // most lines; an empty one as its state.
        let list_height = self.fit.and_then(|max| match (&loading, &empty) {
            (Some(_), _) => Some(max.min(super::LOADING_ROWS) as f32 * source.row_height()),
            (None, None) => Some(source.line_count().min(max) as f32 * source.row_height()),
            (None, Some(_)) => None,
        });
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
            .map(|this| match (loading, empty) {
                (Some(rows), _) => {
                    this.child(rows.render(source.columns(), fill, source.row_height(), window, cx))
                }
                (None, Some(empty)) => this.child(
                    div()
                        .id(ids.empty.clone())
                        .test_support()
                        .px_3()
                        .py_3p5()
                        .text_size(dp(12.5))
                        .text_color(palette(cx).muted)
                        .child(empty),
                ),
                (None, None) => this.child(
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
            // GPUI doesn't clip a fill to its parent's corners, so a table
            // that reaches its card's bottom rounds its own.
            v_flex()
                .min_w_0()
                .bg(fill)
                .rounded_b(px(CARD_INNER_RADIUS))
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
            .then(|| super::footer(state.id("footer"), counts, legend, fill, cx))
            .map(|footer| {
                footer.when(self.carded || self.inset, |this| {
                    this.rounded_b(px(CARD_INNER_RADIUS))
                })
            });
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
    let _span = freshkube_probe::perf::span("table.data");
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
    let sorting = source.sorting(column);
    // A column without a label, such as the glyph's, stays bare.
    if sorting.is_none() && label.is_empty() {
        return cell(column).into_any_element();
    }
    let order = sorting
        .as_ref()
        .and_then(|(_, order)| *order)
        .map(|order| match order {
            SortOrder::Ascending => ("ascending", IconName::ArrowUp),
            SortOrder::Descending => ("descending", IconName::ArrowDown),
        });
    cell(column)
        .id((sort_id.clone(), ix))
        .test_support()
        .role(Role::ColumnHeader)
        .aria_label(match order {
            Some((order, _)) => SharedString::from(format!("{label}, sorted {order}")),
            None => label.clone(),
        })
        .flex()
        .items_center()
        .gap_1()
        // Sorted and unsorted headers share the same visible-edge cut
        // and full-text tooltip, as the table's prose cells do.
        .follow_tooltip(label.clone())
        .child(ui::column_label_style(cx).min_w_0().child(super::word_cut(
            SharedString::from(format!("{sort_id}-{ix}-label")),
            label.clone(),
        )))
        .children(order.map(|(_, icon)| Icon::new(icon).size(dp(12.)).text_color(p.muted)))
        .when_some(sorting, |header, (sort, _)| {
            header
                .cursor_pointer()
                .on_click(cx.listener(move |view, _, _, cx| {
                    // A sort is no click beside the rows for a page around it.
                    cx.stop_propagation();
                    view.sort(sort.clone(), cx)
                }))
        })
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
    let row = match source.line(line, cx)? {
        Line::Group(group) => return render_group_line(source, group, window, cx),
        Line::Row(row) => row,
    };
    let selected = source.selected_key() == Some(&row.key);
    let mut p = palette(cx);
    if selected || row.marked {
        p.muted = p.ink_2;
    }
    let style = RowStyle { selected, p };
    let clickable = source.clickable();
    let mut element = row_frame(&row, &style, clickable, source.row_height());
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
        element = element.relative().child(pinned_overlay(
            source, &row, &style, line, fill, count, run, clickable, cx,
        ));
    }
    let menu_id = ElementId::from((row.id.clone(), "menu"));
    let key = row.key;
    let menu = source
        .menu_focus(cx)
        .map(|focus| (focus, key.clone(), cx.entity().downgrade()));
    let element = element.when(clickable, |this| {
        this.on_click(
            cx.listener(move |view, event, window, cx| view.click(&key, event, window, cx)),
        )
    });
    Some(match menu {
        // The menu draws inside its trigger and takes its text style, so
        // the trigger is a wrapper in the UI font around the monospace row.
        Some((focus, key, view)) => div()
            .id(menu_id)
            .w_full()
            .font_family(cx.theme().font_family.clone())
            .child(element)
            .context_menu(move |menu, window, cx| {
                let actions = view
                    .update(cx, |view, cx| view.row_menu(&key, window, cx))
                    .unwrap_or_default();
                let (view, key) = (view.clone(), key.clone());
                let live = move |cx: &App| {
                    view.upgrade().is_some_and(|view| {
                        let view = view.read(cx);
                        view.selected_key() == Some(&key) && view.line_of(&key).is_some()
                    })
                };
                crate::menu::actions(menu, actions, &focus, live, window, cx)
            })
            .into_any_element(),
        None => element.into_any_element(),
    })
}

/// A group's line, which stays in view. In a table wider than its view
/// the line is as wide as the view, so its details truncate before its
/// actions; one that fits draws it as wide as the table.
fn render_group_line<S: TableSource>(
    source: &S,
    group: usize,
    window: &Window,
    cx: &mut Context<S>,
) -> Option<AnyElement> {
    let state = source.table_state();
    let scrolled = state.scrolled();
    let group = source.group(group, cx)?;
    let view = state.sideways.bounds().size.width;
    let overflows = view > px(0.) && ui::dp_px(source.width(), window) > view + px(0.5);
    Some(if overflows {
        Pinned::new(&state.sideways, div().w(view).child(group)).into_any_element()
    } else if scrolled {
        Pinned::new(&state.sideways, group).into_any_element()
    } else {
        group
    })
}

/// A row's frame, before its cells: its id and role, its border and its
/// fill in each of its states, its hover and its tooltip.
fn row_frame<K, R>(
    row: &TableRow<K, R>,
    style: &RowStyle,
    clickable: bool,
    height: f32,
) -> Observed<Stateful<Div>> {
    let (selected, p) = (style.selected, style.p);
    let (marked, muted) = (row.marked, row.muted);
    h_flex()
        .group(ROW_GROUP)
        .id(row.id.clone())
        .test_support()
        .role(Role::ListBoxOption)
        .aria_selected(selected)
        .aria_label(row.label.clone())
        .w_full()
        .h(dp(height))
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
            this.follow_tooltip(tooltip)
        })
}

/// The row's pinned run, drawn over the cells that pass under it.
#[allow(clippy::too_many_arguments)]
fn pinned_overlay<S: TableSource>(
    source: &S,
    row: &TableRow<S::Key, S::Row<'_>>,
    style: &RowStyle,
    line: usize,
    fill: Hsla,
    count: usize,
    run: f32,
    clickable: bool,
    cx: &mut Context<S>,
) -> Pinned {
    let state = source.table_state();
    let (selected, p) = (style.selected, style.p);
    let marked = row.marked;
    let cells: Vec<_> = (source.columns().iter().take(count))
        .map(|column| source.cell(row, style, column, cx))
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
    Pinned::overlay(
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
    )
}

#[cfg(test)]
mod tests;
