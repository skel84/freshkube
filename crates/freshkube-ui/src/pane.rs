//! What a pane knows about its own room: the width of the chrome beside it,
//! the height the dock takes under it, and the width of the node inspector
//! in it (#879).
//!
//! A page asks through `page::page_width`, `page::below` and the like with
//! only its window. The answer is the metrics of the pane the view being
//! drawn belongs to: a view registers its pane's metrics with [`register`],
//! and any view that did not (every view while there is one pane) gets the
//! focused pane's. Windows draw on one thread, so the registry is
//! thread-local.
use gpui_kit::{EntityId, Window};
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

thread_local! {
    static FOCUSED: RefCell<Rc<PaneMetrics>> = RefCell::new(PaneMetrics::new());
    static VIEWS: RefCell<HashMap<EntityId, Rc<PaneMetrics>>> = RefCell::new(HashMap::new());
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

/// Says that `view` draws in the pane with `metrics`.
pub fn register(view: EntityId, metrics: Rc<PaneMetrics>) {
    VIEWS.with(|views| views.borrow_mut().insert(view, metrics));
}

/// Forgets `view`, which went away.
pub fn unregister(view: EntityId) {
    VIEWS.with(|views| views.borrow_mut().remove(&view));
}

/// The metrics of the pane the view being drawn is in.
pub fn current(window: &Window) -> Rc<PaneMetrics> {
    let view = window.current_view();
    VIEWS
        .with(|views| views.borrow().get(&view).cloned())
        .unwrap_or_else(focused)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unregistered_view_gets_the_focused_pane() {
        let one = PaneMetrics::new();
        one.set_chrome(52.);
        focus(one.clone());
        assert_eq!(focused().chrome(), 52.);
        assert_eq!(VIEWS.with(|v| v.borrow().len()), 0);
    }

    #[test]
    fn a_registered_view_keeps_its_own_pane() {
        let (a, b) = (PaneMetrics::new(), PaneMetrics::new());
        a.set_below(120.);
        b.set_below(-5.);
        let id = EntityId::from(1_u64);
        register(id, b.clone());
        let found = VIEWS.with(|v| v.borrow().get(&id).cloned()).unwrap();
        assert_eq!(found.below(), 0., "a negative height is clamped");
        assert_eq!(a.below(), 120.);
        unregister(id);
        assert!(VIEWS.with(|v| v.borrow().get(&id).is_none()));
    }
}
