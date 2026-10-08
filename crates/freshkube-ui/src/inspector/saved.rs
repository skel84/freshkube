//! Where inspectors' widths are kept between runs, by page. The app keeps
//! them in its preferences and makes that store a global
//! ([`set_saved_widths`]); a page reads it as it builds its splits
//! ([`saved_widths`]). Without one, as in a test that sets none, nothing is
//! read or kept.

use std::rc::Rc;

use gpui_kit::{App, Global};

/// A store of inspector widths, in dp, keyed by page.
pub trait SavedWidths {
    /// The width `page`'s inspector was left at.
    fn width(&self, page: &str) -> Option<f32>;
    /// Keeps `width` for `page`.
    fn save(&self, page: &str, width: f32, cx: &App);
}

#[derive(Clone)]
struct Saved(Rc<dyn SavedWidths>);

impl Global for Saved {}

/// Makes `widths` the store every page reads and saves to.
pub fn set_saved_widths(widths: Rc<dyn SavedWidths>, cx: &mut App) {
    cx.set_global(Saved(widths));
}

/// The app's store, or one that keeps nothing when none is set.
pub fn saved_widths(cx: &App) -> Rc<dyn SavedWidths> {
    match cx.try_global::<Saved>() {
        Some(saved) => saved.0.clone(),
        None => Rc::new(Unsaved),
    }
}

struct Unsaved;

impl SavedWidths for Unsaved {
    fn width(&self, _: &str) -> Option<f32> {
        None
    }

    fn save(&self, _: &str, _: f32, _: &App) {}
}

/// Widths kept in memory, rounded to whole dp as the app's file keeps them:
/// for tests that reopen a page.
#[cfg(any(test, feature = "testing"))]
#[derive(Default)]
pub struct MemoryWidths(std::cell::RefCell<std::collections::HashMap<String, f32>>);

#[cfg(any(test, feature = "testing"))]
impl SavedWidths for MemoryWidths {
    fn width(&self, page: &str) -> Option<f32> {
        self.0.borrow().get(page).copied()
    }

    fn save(&self, page: &str, width: f32, _: &App) {
        self.0.borrow_mut().insert(page.into(), width.round());
    }
}
