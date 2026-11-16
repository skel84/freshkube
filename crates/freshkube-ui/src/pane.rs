//! What a pane knows about its own room: the width of the chrome beside it,
//! the height the dock takes under it, and the width of the node inspector
//! in it (#879).
//!
//! A page asks through `page::page_width`, `page::below` and the like with
//! only its window. The answer is the metrics of the pane the view being
//! drawn belongs to: a view registers its pane's metrics with [`register`],
//! and any view that did not (every view while there is one pane) gets the
//! focused pane's. That fallback is how one pane works: nothing registers,
//! and every page reads the one pane's room. Windows draw on one thread, so
//! the registry is thread-local; it is keyed by window and view, and an
//! entry goes when its view is released.
use gpui_kit::{App, Entity, EntityId, Window, WindowId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::page::{COLUMN_WIDTH, RAIL_WIDTH};

/// One pane's room, in dp. Cells, so the shell sets them as it draws.
#[derive(Debug)]
pub struct PaneMetrics {
    chrome: Cell<f32>,
    below: Cell<f32>,
    node_pane: Cell<f32>,
}

impl PaneMetrics {
    /// A pane with the rail and the column beside it, nothing under it and
    /// no node inspector.
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            chrome: Cell::new(RAIL_WIDTH + COLUMN_WIDTH),
            below: Cell::new(0.),
            node_pane: Cell::new(f32::MAX),
        })
    }

    /// The width of the navigation beside the pane: the rail, and the column
    /// when it shows.
    pub fn chrome(&self) -> f32 {
        self.chrome.get()
    }

    pub fn set_chrome(&self, width: f32) {
        self.chrome.set(width);
    }

    /// The height the dock takes under the pane.
    pub fn below(&self) -> f32 {
        self.below.get()
    }

    pub fn set_below(&self, height: f32) {
        self.below.set(height.max(0.));
    }

    /// The node inspector's width: the page's when it stacks or fills it.
    pub fn node_pane(&self) -> f32 {
        self.node_pane.get()
    }

    pub fn set_node_pane(&self, width: f32) {
        self.node_pane.set(width);
    }
}

/// A view's place in the registry: its window and its entity.
type Key = (WindowId, EntityId);

thread_local! {
    static FOCUSED: RefCell<Rc<PaneMetrics>> = RefCell::new(PaneMetrics::new());
    static VIEWS: RefCell<HashMap<Key, Rc<PaneMetrics>>> = RefCell::new(HashMap::new());
}

/// The focused pane's metrics: what a view that registered none gets, and
/// what the shell sets while there is one pane.
pub fn focused() -> Rc<PaneMetrics> {
    FOCUSED.with(|focused| focused.borrow().clone())
}

/// Makes `metrics` the focused pane's.
pub fn focus(metrics: Rc<PaneMetrics>) {
    FOCUSED.with(|focused| *focused.borrow_mut() = metrics);
}

/// Says that `view` draws in the pane with `metrics`, in `window`. The entry
/// goes when the view is released.
pub fn register<T: 'static>(
    view: &Entity<T>,
    metrics: Rc<PaneMetrics>,
    window: &Window,
    cx: &mut App,
) {
    let key = (window.window_handle().window_id(), view.entity_id());
    VIEWS.with(|views| views.borrow_mut().insert(key, metrics));
    cx.observe_release(view, move |_, _| {
        VIEWS.with(|views| views.borrow_mut().remove(&key));
    })
    .detach();
}

/// How many views are registered, for tests.
#[cfg(any(test, feature = "testing"))]
pub fn registered() -> usize {
    VIEWS.with(|views| views.borrow().len())
}

/// The metrics of the pane the view being drawn is in.
///
/// While no view is registered, which is the case with one pane, this is the
/// focused pane's and never asks the window which view is drawing: that is
/// allowed only while a frame is being laid out or painted, and a page also
/// asks its width from an event handler. A view that is not registered gets
/// the focused pane's too, with no panic.
pub fn current(window: &Window) -> Rc<PaneMetrics> {
    if VIEWS.with(|views| views.borrow().is_empty()) {
        return focused();
    }
    let key = (window.window_handle().window_id(), window.current_view());
    VIEWS
        .with(|views| views.borrow().get(&key).cloned())
        .unwrap_or_else(focused)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AppContext, Context, IntoElement, Render, TestAppContext, div, px, size};

    struct Empty;

    impl Render for Empty {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[test]
    fn a_negative_height_is_clamped() {
        let pane = PaneMetrics::new();
        pane.set_below(-5.);
        assert_eq!(pane.below(), 0.);
        pane.set_below(120.);
        assert_eq!(pane.below(), 120.);
    }

    #[test]
    fn the_focused_pane_is_what_one_pane_reads() {
        let one = PaneMetrics::new();
        one.set_chrome(52.);
        focus(one);
        assert_eq!(focused().chrome(), 52.);
        assert_eq!(VIEWS.with(|v| v.borrow().len()), 0);
    }

    #[gpui_kit::test]
    fn a_registered_view_goes_when_it_is_released(cx: &mut TestAppContext) {
        let handle = cx.open_window(size(px(400.), px(300.)), |window, cx| {
            let view = cx.new(|_| Empty);
            gpui_kit::component::Root::new(view, window, cx)
        });
        let page = handle
            .update(cx, |_, window, cx| {
                let page = cx.new(|_| Empty);
                register(&page, PaneMetrics::new(), window, cx);
                page
            })
            .unwrap();
        assert_eq!(registered(), 1);
        drop(page);
        // Releases are noticed when the app next flushes its effects.
        cx.update(|_| ());
        cx.run_until_parked();
        assert_eq!(registered(), 0, "no entry is left once the page is gone");
    }
}
