//! Tooltips that follow their element when it moves under a still pointer.
//!
//! GPUI decides whether a tooltip stays up from the bounds its element had
//! when the pointer first rested on it, and a wheel scroll doesn't cancel a
//! tooltip still waiting to show. In a scrolling list, a still pointer can
//! then see one row's tooltip over another row (#192). [`FollowTooltip`]
//! draws the tooltip only while the pointer is over its element as this
//! frame placed it.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{AnyView, Bounds, Context, ElementId, Pixels, SharedString, Window, canvas, div};

/// Where an element was drawn this frame, clipped to what shows of it.
type Placed = Rc<Cell<Option<Bounds<Pixels>>>>;

/// A text tooltip that hides once its element is no longer under the
/// pointer, even when a scroll moved it there rather than the pointer.
pub trait FollowTooltip: ParentElement + StatefulInteractiveElement + Sized {
    fn follow_tooltip(self, text: impl Into<SharedString>) -> Self {
        let text = text.into();
        // Filled while this frame draws, with the element's lasting record
        // of where it is; a tooltip shown later reads that record.
        let slot: Rc<RefCell<Option<Placed>>> = Rc::default();
        let anchor = {
            let slot = slot.clone();
            canvas(
                move |bounds, window, _| {
                    let shown = bounds.intersect(&window.content_mask().bounds);
                    let id = ElementId::from("follow-tooltip");
                    let placed = window.with_global_id(id, |id, window| {
                        window.with_element_state(id, |placed: Option<Placed>, _| {
                            let placed = placed.unwrap_or_default();
                            (placed.clone(), placed)
                        })
                    });
                    placed.set(Some(shown));
                    *slot.borrow_mut() = Some(placed);
                },
                |_, _, _, _| {},
            )
            // At the element's corner in a block parent too, where an
            // absolute child without an inset sits after its siblings.
            .absolute()
            .top_0()
            .left_0()
            .size_full()
        };
        self.child(anchor).tooltip(move |window, cx| {
            let tip = Tooltip::new(text.clone()).build(window, cx);
            let placed = slot.borrow().clone();
            let text = text.clone();
            cx.new(|_| Follow { tip, placed, text }).into()
        })
    }
}

impl<E: ParentElement + StatefulInteractiveElement> FollowTooltip for E {}

struct Follow {
    tip: AnyView,
    placed: Option<Placed>,
    /// Counts the tooltip's draws in UI tests.
    #[cfg_attr(not(any(test, feature = "testing")), allow(dead_code))]
    text: SharedString,
}

impl Render for Follow {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let over = self.placed.as_ref().is_none_or(|placed| {
            placed
                .get()
                .is_some_and(|bounds| bounds.contains(&window.mouse_position()))
        });
        if !over {
            return div().into_any_element();
        }
        #[cfg(any(test, feature = "testing"))]
        DRAWN.with(|drawn| *drawn.borrow_mut().entry(self.text.clone()).or_default() += 1);
        self.tip.clone().into_any_element()
    }
}

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static DRAWN: RefCell<std::collections::HashMap<SharedString, usize>> = RefCell::default();
}

/// How many frames have drawn the tooltip `text` so far, for UI tests.
#[cfg(any(test, feature = "testing"))]
pub fn drawn(text: &str) -> usize {
    DRAWN.with(|drawn| drawn.borrow().get(text).copied().unwrap_or(0))
}
