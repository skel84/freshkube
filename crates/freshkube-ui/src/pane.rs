//! What a pane knows about its own room: the width of the chrome beside it,
//! the height the dock takes under it, and the width of the node inspector
//! in it (#879).
//!
//! A page keeps the [`Pane`] it draws in and asks it for `page_width`,
//! `inset_width`, `content_width`, `is_short` and the rest. Nothing is looked
//! up by what is being drawn, so a handler may ask outside a frame.
//!
//! This is the target the split reaches in steps. Today there is one pane,
//! and a page made without being told its pane is in the window's main pane
//! (`Pane::default()`), which every page and the shell share. That default is
//! removed, and each parent hands its handle to the children it makes, in
//! slice B2, so that an answer can never be another pane's.
use gpui_kit::Window;
use std::cell::Cell;
use std::rc::Rc;

use crate::page::{COLUMN_WIDTH, PAGE_PADDING, PANE_PADDING, RAIL_WIDTH, SHORT_HEIGHT};
use crate::ui::dp_px;

/// One pane's room, in dp. Cells, so the shell sets them as it draws.
#[derive(Debug)]
struct Room {
    chrome: Cell<f32>,
    below: Cell<f32>,
    node_pane: Cell<f32>,
}

/// A handle to one pane's room. Clones share it.
#[derive(Clone, Debug)]
pub struct Pane(Rc<Room>);

thread_local! {
    /// The main pane. Windows draw on one thread.
    static MAIN: Pane = Pane::new();
}

impl Default for Pane {
    /// The main pane: the one pane there is until a second is made.
    fn default() -> Self {
        MAIN.with(Pane::clone)
    }
}

impl Pane {
    /// A pane of its own, with the rail and the column beside it, nothing
    /// under it and no node inspector.
    pub fn new() -> Self {
        Self(Rc::new(Room {
            chrome: Cell::new(RAIL_WIDTH + COLUMN_WIDTH),
            below: Cell::new(0.),
            node_pane: Cell::new(f32::MAX),
        }))
    }

    /// Whether `other` is this same pane.
    pub fn is(&self, other: &Pane) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// The width of the navigation beside the pane: the rail, and the column
    /// when it shows.
    pub fn chrome(&self) -> f32 {
        self.0.chrome.get()
    }

    /// Sets it; the shell does whenever the column shows or hides.
    pub fn set_chrome(&self, width: f32) {
        self.0.chrome.set(width);
    }

    /// The height in dp the dock takes under the pane.
    pub fn below(&self) -> f32 {
        self.0.below.get()
    }

    /// Tells pages how much of the window's height, in dp, the dock takes
    /// under them, so a page under an open dock lays out as in a window that
    /// much shorter.
    pub fn set_below(&self, height: f32) {
        self.0.below.set(height.max(0.));
    }

    /// The node inspector's width: the page's when it stacks or fills it.
    pub fn node_pane(&self) -> f32 {
        self.0.node_pane.get()
    }

    pub fn set_node_pane(&self, width: f32) {
        self.0.node_pane.set(width);
    }

    /// The width of a page in `dp` (pixels at the default text size): the
    /// window less the rail and column. A page without margins has all of
    /// it, so its breakpoints compare this; a larger text size leaves less
    /// room, as a narrower window would.
    pub fn page_width(&self, window: &Window) -> f32 {
        let viewport = window.viewport_size().width / dp_px(1., window);
        (viewport - self.chrome()).max(240.)
    }

    /// The width inside a page's [`inset`](crate::page::inset), such as its
    /// header's, in `dp`.
    pub fn inset_width(&self, window: &Window) -> f32 {
        (self.page_width(window) - PANE_PADDING * 2.).max(240.)
    }

    /// The width inside a padded page's margins in `dp`, for choosing
    /// between side-by-side and stacked layouts there.
    pub fn content_width(&self, window: &Window) -> f32 {
        (self.page_width(window) - PAGE_PADDING * 2.).max(240.)
    }

    /// The width inside the node inspector's tab body in `dp`, where
    /// embedded screens choose between side-by-side and stacked layouts.
    pub fn embedded_width(&self, window: &Window) -> f32 {
        let pane = self.node_pane().min(self.page_width(window));
        (pane - PANE_PADDING * 2.).max(240.)
    }

    /// Whether the window, less the dock under the page, is too short for a
    /// table page's header and a usable list together, so the page scrolls
    /// its frame instead: under `SHORT_HEIGHT`.
    pub fn is_short(&self, window: &Window) -> bool {
        window.viewport_size().height - dp_px(self.below(), window) < dp_px(SHORT_HEIGHT, window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_negative_height_is_clamped() {
        let pane = Pane::new();
        pane.set_below(-5.);
        assert_eq!(pane.below(), 0.);
        pane.set_below(120.);
        assert_eq!(pane.below(), 120.);
    }

    #[test]
    fn a_default_pane_is_the_main_pane_and_a_new_one_is_its_own() {
        let (a, b) = (Pane::default(), Pane::default());
        assert!(a.is(&b));
        a.set_chrome(52.);
        assert_eq!(b.chrome(), 52.);
        let other = Pane::new();
        assert!(!other.is(&a));
        assert_eq!(other.chrome(), RAIL_WIDTH + COLUMN_WIDTH);
        a.set_chrome(RAIL_WIDTH + COLUMN_WIDTH);
    }
}
