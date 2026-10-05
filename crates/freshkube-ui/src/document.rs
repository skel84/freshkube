//! `Document`: a text document's lines, such as a configuration under review,
//! as a virtualised list of fixed-height monospace lines. The caller draws
//! each line from [`line`], so a viewer can add a gutter, highlights or a
//! selection. The list is as wide as its widest line and scrolls sideways
//! rather than clipping it ([docs/DESIGN.md](../../docs/DESIGN.md#components)).
use std::ops::Range;

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Context, Div, ElementId, ListHorizontalSizingBehavior, UniformList,
    UniformListScrollHandle, Window, div, uniform_list,
};

use crate::ui::{MONO_FONT, dp};

/// A line's height, as `dp`.
pub const LINE_HEIGHT: f32 = 20.;

/// One line: the line height, monospace 12, never wrapped. The caller adds
/// its text and anything around it.
pub fn line() -> Div {
    div()
        .h(dp(LINE_HEIGHT))
        .font_family(MONO_FONT)
        .text_size(dp(12.))
        .whitespace_nowrap()
}

/// Lines `0..count`, each drawn by `draw` only while it is in view. `widest`
/// is the index of the widest line: it sets how far the list scrolls
/// sideways. Size the list with `.size_full()` inside a frame of the
/// caller's.
pub fn lines<V: 'static>(
    id: impl Into<ElementId>,
    count: usize,
    widest: Option<usize>,
    scroll: &UniformListScrollHandle,
    draw: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) -> Option<AnyElement> + 'static,
    cx: &mut Context<V>,
) -> UniformList {
    uniform_list(
        id,
        count,
        cx.processor(move |view: &mut V, range: Range<usize>, window, cx| {
            range
                .filter_map(|ix| draw(view, ix, window, cx))
                .collect::<Vec<_>>()
        }),
    )
    .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
    .with_width_from_item(widest)
    .track_scroll(scroll)
    // Inside a page that scrolls, a wheel reaches both; unrestricted, each
    // takes the other axis's delta too, as the table's rows did (#93).
    .map(|mut list| {
        list.style().restrict_scroll_to_axis = Some(true);
        list
    })
}
