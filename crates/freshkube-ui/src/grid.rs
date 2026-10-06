//! `CardGrid`: items laid out `columns` to a row, as a virtualised list of
//! rows, so a long list of cards draws only the rows in view. Every item is
//! the same height (the list measures its first row); the caller draws each
//! item, and the grid only places them
//! ([docs/DESIGN.md](../../docs/DESIGN.md#components)).
use std::ops::Range;

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Context, ElementId, UniformList, UniformListScrollHandle, Window, div, uniform_list,
};

use crate::ui::dp;

/// Items `0..count`, `columns` to a row with `gap` dp between them and below
/// each row, each drawn by `draw` only while its row is in view. Size the
/// grid with `.size_full()` inside a frame of the caller's.
pub fn cards<V: 'static>(
    id: impl Into<ElementId>,
    count: usize,
    columns: usize,
    gap: f32,
    scroll: &UniformListScrollHandle,
    draw: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) -> AnyElement + 'static,
    cx: &mut Context<V>,
) -> UniformList {
    let columns = columns.max(1);
    uniform_list(
        id,
        count.div_ceil(columns),
        cx.processor(move |view: &mut V, rows: Range<usize>, window, cx| {
            rows.map(|row| {
                let start = row * columns;
                let end = (start + columns).min(count);
                div()
                    .grid()
                    .grid_cols(columns as u16)
                    .gap(dp(gap))
                    .pb(dp(gap))
                    .children((start..end).map(|item| draw(view, item, window, cx)))
                    .into_any_element()
            })
            .collect::<Vec<_>>()
        }),
    )
    .track_scroll(scroll)
}

/// The row that holds `item`, to scroll it into view.
pub fn row_of(item: usize, columns: usize) -> usize {
    item / columns.max(1)
}

#[cfg(test)]
mod tests {
    use gpui_kit::base::TestSupportExt as _;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Render, TestAppContext, px, size};

    use super::*;

    #[test]
    fn an_item_is_on_its_row() {
        assert_eq!(row_of(0, 3), 0);
        assert_eq!(row_of(2, 3), 0);
        assert_eq!(row_of(3, 3), 1);
        assert_eq!(row_of(6, 3), 2);
        // One column is a plain list; none is treated as one.
        assert_eq!(row_of(4, 1), 4);
        assert_eq!(row_of(4, 0), 4);
    }

    struct Grid(UniformListScrollHandle);

    impl Render for Grid {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                cards(
                    "grid",
                    7,
                    3,
                    10.,
                    &self.0,
                    |_, item, _, _| {
                        div()
                            .id(("item", item))
                            .test_support()
                            .h(px(40.))
                            .into_any_element()
                    },
                    cx,
                )
                .size_full(),
            )
        }
    }

    #[gpui_kit::test]
    fn seven_items_in_three_columns_fill_two_rows_and_start_a_third(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            cx.set_reduce_motion(true);
        });
        let handle = cx.open_window(size(px(400.), px(600.)), |window, cx| {
            let view = cx.new(|_| Grid(UniformListScrollHandle::new()));
            Root::new(view, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let item = |ix: usize| window.find(("item", ix)).bounds();
            // A row's items share a top, left to right, a gap apart.
            assert_eq!(item(0).top(), item(2).top());
            assert!(item(0).right() < item(1).left());
            assert!(item(1).right() < item(2).left());
            // The next row starts under the first, a gap below.
            assert_eq!(item(3).left(), item(0).left());
            assert!(item(3).top() > item(0).bottom());
            // The last row holds only the seventh item, in the first column.
            assert_eq!(item(6).left(), item(0).left());
            assert!(item(6).top() > item(3).bottom());
            assert!(window.try_find(("item", 7usize)).is_none());
        })
        .unwrap();
    }
}
