//! `DataTable`: the header, the virtualised rows and their selection, for
//! any page whose entity implements [`TableSource`]. Generic rather than
//! dynamic, so a 20,000-row list costs what a hand-written one does.
use std::hash::Hash;
use std::ops::Range;

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, Role, SharedString, TestSupportExt,
    UniformListScrollHandle, Window, div, uniform_list,
};

use super::{COMPACT_ROW_HEIGHT, HEADER_HEIGHT, ROW_GROUP, ROW_HEIGHT, TableColumn, cell};
use crate::page::card;
use crate::palette::palette;
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
    pub label: String,
    pub selected: bool,
    pub marked: bool,
    /// Muted text, as a terminating object's.
    pub muted: bool,
    pub data: R,
}

/// The ids a table gives its parts, from the page's prefix:
/// `<prefix>-list`, `-rows`, `-table-scroll`, `-empty` and `-sort`.
#[derive(Clone, Debug)]
pub struct TableIds {
    list: SharedString,
    rows: SharedString,
    scroll: SharedString,
    empty: SharedString,
    sort: SharedString,
}

impl TableIds {
    pub fn new(prefix: &str) -> Self {
        let id = |part: &str| SharedString::from(format!("{prefix}-{part}"));
        Self {
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
    pub fn new(prefix: &str) -> Self {
        Self {
            scroll: UniformListScrollHandle::new(),
            compact: false,
            ids: TableIds::new(prefix),
        }
    }

    pub fn row_height(&self) -> f32 {
        if self.compact {
            COMPACT_ROW_HEIGHT
        } else {
            ROW_HEIGHT
        }
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
        column: &Self::Column,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement>;
    fn click(&mut self, key: &Self::Key, window: &mut Window, cx: &mut Context<Self>);
    /// The line that replaces the rows when there are none.
    fn empty(&self) -> Option<String>;
    /// Bars above the rows: the selection, folded rows.
    fn notes(&self, _cx: &mut Context<Self>) -> Vec<AnyElement> {
        Vec::new()
    }
    /// The legend below the rows.
    fn footer(&self, _window: &Window, _cx: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
}

/// The table in its card: notes, the header, the rows or the empty line,
/// and the footer. The caller sizes the card.
pub fn data_table<S: TableSource>(source: &S, window: &Window, cx: &mut Context<S>) -> Div {
    let p = palette(cx);
    let state = source.table_state();
    let ids = &state.ids;
    let list = div()
        .id(ids.list.clone())
        .test_support()
        .role(Role::ListBox)
        .aria_label(source.list_label())
        .flex_1()
        .min_h_0()
        .map(|this| match source.empty() {
            Some(text) => this.child(
                div()
                    .id(ids.empty.clone())
                    .test_support()
                    .px_3()
                    .py_3p5()
                    .text_size(dp(12.5))
                    .text_color(p.muted)
                    .child(text),
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
    card(cx)
        .overflow_hidden()
        .children(source.notes(cx))
        .child(
            div()
                .id(ids.scroll.clone())
                .test_support()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_x_scroll()
                .child(
                    v_flex()
                        .h_full()
                        .w_full()
                        .min_w(dp(source.width()))
                        .child(header(source, cx))
                        .child(list),
                ),
        )
        .children(source.footer(window, cx))
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
            let Some((sort, order)) = source.sorting(column) else {
                return cell(column).into_any_element();
            };
            let order = order.map(|order| match order {
                SortOrder::Ascending => ("ascending", IconName::ArrowUp),
                SortOrder::Descending => ("descending", IconName::ArrowDown),
            });
            let label = column.label();
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
    let mut row = match source.line(line, cx)? {
        Line::Group(group) => return source.group(group, cx),
        Line::Row(row) => row,
    };
    let mut p = palette(cx);
    if row.selected || row.marked {
        p.muted = p.ink_2;
    }
    let (selected, marked, muted) = (row.selected, row.marked, row.muted);
    let mut element = h_flex()
        .group(ROW_GROUP)
        .id(row.id.clone())
        .test_support()
        .role(Role::ListBoxOption)
        .aria_selected(selected)
        .aria_label(std::mem::take(&mut row.label))
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
        .cursor_pointer()
        .when(muted, |this| this.text_color(p.muted))
        .when(marked && !selected, |this| this.bg(p.hover))
        .when(selected, |this| this.bg(p.accent_soft))
        .when(!selected, |this| {
            this.hover(|style| {
                let style = style.bg(p.hover);
                if muted {
                    style.text_color(p.ink_2)
                } else {
                    style
                }
            })
        });
    for column in source.columns() {
        element = element.child(source.cell(&row, column, cx));
    }
    let key = row.key;
    Some(
        element
            .on_click(cx.listener(move |view, _, window, cx| view.click(&key, window, cx)))
            .into_any_element(),
    )
}
