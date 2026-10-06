//! The inspector (docs/DESIGN.md#components): the pane that shows a table's
//! selected row. It takes the split's trailing pane without a card, divided
//! from the table by the resize handle's hairline: beside the table on a
//! wide page, under it on a narrow one. The page keeps an
//! [`InspectorSplit`], which remembers how wide the user made it.
use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::resizable::{
    ResizablePanelEvent, ResizableState, h_resizable, resizable_panel, v_resizable,
};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, AppContext, Div, Entity, Pixels, SharedString, Stateful, Subscription,
    TestSupportExt, Window, div,
};

use crate::page::{PANE_PADDING, SHORT_LIST_HEIGHT};
use crate::ui::{BASE_TEXT, dp, dp_px};

/// The least page width, in dp, at which the inspector sits beside the
/// table; on a narrower page it opens under it.
pub const SPLIT_WIDTH: f32 = 900.;
/// The inspector's width beside the table until the user drags it.
pub const WIDTH: f32 = 460.;
/// The least width the inspector keeps beside the table.
pub const MIN_WIDTH: f32 = 320.;
/// The least width the table keeps beside the inspector.
pub const LIST_MIN_WIDTH: f32 = 320.;
/// Stacked, the table and the inspector share the height one to two, so a
/// short window still leaves the inspector room for a few lines.
pub const STACKED_LIST_HEIGHT: f32 = 190.;
pub const STACKED_HEIGHT: f32 = 380.;
/// The least heights the table and the inspector keep when stacked.
pub const LIST_MIN_HEIGHT: f32 = 96.;
pub const MIN_HEIGHT: f32 = 220.;
/// How tall the split is beside the table while a short page scrolls.
pub const SHORT_HEIGHT: f32 = 360.;
/// Between the heading and the body's parts.
const GAP: f32 = 12.;

/// The inspector's frame: a heading, then a body that scrolls on its own,
/// both padded 12, on whatever the split sits on. Its parts' ids are
/// `<id>-heading` and `<id>-body`.
pub struct Inspector {
    id: SharedString,
    heading: Option<AnyElement>,
    body: Vec<AnyElement>,
}

impl Inspector {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            heading: None,
            body: vec![],
        }
    }

    /// What the inspector shows, at the top: a title and its tags.
    pub fn heading(mut self, heading: impl IntoElement) -> Self {
        self.heading = Some(heading.into_any_element());
        self
    }

    /// One part of the body, under the heading and the parts before it.
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.body.push(child.into_any_element());
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.body
            .extend(children.into_iter().map(IntoElement::into_any_element));
        self
    }

    pub fn render(self, _cx: &App) -> Observed<Stateful<Div>> {
        let part = |part: &str| SharedString::from(format!("{}-{part}", self.id));
        let (heading, body) = (part("heading"), part("body"));
        v_flex()
            .id(self.id.clone())
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .children(self.heading.map(|content| {
                h_flex()
                    .id(heading)
                    .test_support()
                    .flex_none()
                    .min_w_0()
                    .items_start()
                    .px(dp(PANE_PADDING))
                    .pt(dp(PANE_PADDING))
                    .child(content)
            }))
            .child(
                v_flex()
                    .id(body)
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_y_scroll()
                    .gap(dp(GAP))
                    .p(dp(PANE_PADDING))
                    .children(self.body),
            )
    }
}

/// What a page keeps for its split: Kit's panel states for either
/// arrangement, and the inspector's width beside the table in dp, so it
/// scales with the text size.
pub struct InspectorSplit {
    beside: Entity<ResizableState>,
    stacked: Entity<ResizableState>,
    width: Rc<Cell<f32>>,
    _resized: Subscription,
}

impl InspectorSplit {
    /// A split whose inspector starts `width` dp wide, the width the user
    /// left it at; [`WIDTH`] without one. `remember` hears the new width
    /// once each drag ends, to save it.
    pub fn new(
        width: Option<f32>,
        remember: impl Fn(f32, &mut App) + 'static,
        cx: &mut App,
    ) -> Self {
        let beside = cx.new(|_| ResizableState::default());
        let stacked = cx.new(|_| ResizableState::default());
        let width = Rc::new(Cell::new(start_width(width)));
        // Kit tells the state once a drag ends, not while it moves.
        let _resized = cx.subscribe(&beside, {
            let width = width.clone();
            move |state, _: &ResizablePanelEvent, cx| {
                let Some(&size) = state.read(cx).sizes().get(1) else {
                    return;
                };
                let dp = f32::from(size) * BASE_TEXT / crate::text_size::current(cx);
                width.set(dp);
                remember(dp, cx);
            }
        });
        Self {
            beside,
            stacked,
            width,
            _resized,
        }
    }

    /// The inspector's width beside the table, in dp.
    pub fn width(&self) -> f32 {
        self.width.get()
    }

    /// Kit's state for the split beside the table, for tests that resize it.
    #[cfg(any(test, feature = "testing"))]
    pub fn beside_state(&self) -> &Entity<ResizableState> {
        &self.beside
    }
}

/// A remembered width, or [`WIDTH`]; never narrower than [`MIN_WIDTH`].
fn start_width(width: Option<f32>) -> f32 {
    match width {
        Some(width) if width.is_finite() => width.max(MIN_WIDTH),
        _ => WIDTH,
    }
}

/// How tall a split is on a page that scrolls its frame, where it has no
/// room to fill: the least heights stacked, [`SHORT_HEIGHT`] beside, and a
/// table alone keeps a few rows. A page gives its split this height, or
/// the panes take their contents' heights.
pub fn short_height(beside: bool, open: bool) -> f32 {
    match (open, beside) {
        (true, false) => LIST_MIN_HEIGHT + MIN_HEIGHT,
        (true, true) => SHORT_HEIGHT,
        (false, _) => SHORT_LIST_HEIGHT,
    }
}

/// The table and its inspector, in a split with the id `id`: beside it,
/// resizable, when `beside`, as on a page at least 900 wide; otherwise
/// under it. Without an inspector the table fills the split. Both run edge
/// to edge, and Kit's handle draws the hairline between them. The split
/// fills its parent; on a page that scrolls its frame, the parent gives it
/// [`short_height`].
pub fn split(
    id: impl Into<SharedString>,
    split: &InspectorSplit,
    beside: bool,
    table: AnyElement,
    inspector: Option<AnyElement>,
    window: &Window,
) -> AnyElement {
    let least = short_height(beside, inspector.is_some());
    let frame = div()
        .id(id.into())
        .test_support()
        .flex()
        .flex_col()
        .flex_1()
        .w_full()
        .min_w_0()
        .min_h(dp(least));
    let Some(inspector) = inspector else {
        return frame.child(table).into_any_element();
    };
    let dp = |n: f32| dp_px(n, window);
    let panels = if beside {
        h_resizable("inspector-split")
            .with_state(&split.beside)
            .child(
                resizable_panel()
                    .size_range(dp(LIST_MIN_WIDTH)..Pixels::MAX)
                    .child(table),
            )
            .child(
                resizable_panel()
                    .size(dp(split.width()))
                    .size_range(dp(MIN_WIDTH)..Pixels::MAX)
                    .flex_none()
                    .child(inspector),
            )
    } else {
        v_resizable("inspector-split-stacked")
            .with_state(&split.stacked)
            .child(
                resizable_panel()
                    .size(dp(STACKED_LIST_HEIGHT))
                    .size_range(dp(LIST_MIN_HEIGHT)..Pixels::MAX)
                    .child(table),
            )
            .child(
                resizable_panel()
                    .size(dp(STACKED_HEIGHT))
                    .size_range(dp(MIN_HEIGHT)..Pixels::MAX)
                    .child(inspector),
            )
    };
    frame.child(panels).into_any_element()
}

#[cfg(test)]
mod tests;
