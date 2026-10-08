//! A resizable split's size, kept per page across restarts
//! ([docs/DESIGN.md](../../docs/DESIGN.md#split-sizes-remembered)). A page
//! keeps a [`SplitSize`] for each split the user can drag: it starts at the
//! size the user left, lays out from it, and saves a drag once. The app
//! keeps the sizes in its navigation file and makes that store a global
//! ([`set_store`]); without one, as in a test that sets none, sizes last
//! the session.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui_kit::{App, Global, Task};

/// How long after a drag's last move its size is saved, so a drag writes
/// once however many frames it spans.
pub const SAVE_DELAY: Duration = Duration::from_millis(300);

/// Where a split's size is kept: `group.name` in the app's file, such as
/// `inspector.nodes`, `drawer.resources` or `dock.height`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SizeKey {
    pub group: &'static str,
    pub name: &'static str,
}

impl SizeKey {
    pub const fn new(group: &'static str, name: &'static str) -> Self {
        Self { group, name }
    }
}

/// A store of split sizes, in dp.
pub trait SizeStore {
    /// The size kept under `key`, if any.
    fn size(&self, key: SizeKey) -> Option<f32>;
    /// Keeps `size` under `key`.
    fn save(&self, key: SizeKey, size: f32, cx: &App);
}

#[derive(Clone)]
struct Store(Rc<dyn SizeStore>);

impl Global for Store {}

/// Makes `store` the one every split reads and saves to.
pub fn set_store(store: Rc<dyn SizeStore>, cx: &mut App) {
    cx.set_global(Store(store));
}

/// The app's store, or one that keeps nothing when none is set.
pub fn store(cx: &App) -> Rc<dyn SizeStore> {
    match cx.try_global::<Store>() {
        Some(store) => store.0.clone(),
        None => Rc::new(Unsaved),
    }
}

struct Unsaved;

impl SizeStore for Unsaved {
    fn size(&self, _: SizeKey) -> Option<f32> {
        None
    }

    fn save(&self, _: SizeKey, _: f32, _: &App) {}
}

/// One split's size in dp, as the user left it. Clones share it.
#[derive(Clone)]
pub struct SplitSize(Rc<Inner>);

struct Inner {
    key: SizeKey,
    least: f32,
    size: Cell<f32>,
    store: Rc<dyn SizeStore>,
    /// Whether a drag's size waits to be saved.
    pending: Cell<bool>,
    /// The wait for a drag to pause; a later move replaces it.
    save: RefCell<Option<Task<()>>>,
}

impl SplitSize {
    /// The split under `key`, starting at the size saved there: at least
    /// `least`, or `default` when none is saved or it isn't a size.
    pub fn new(key: SizeKey, default: f32, least: f32, cx: &App) -> Self {
        let store = store(cx);
        let size = start(store.size(key), default, least);
        Self(Rc::new(Inner {
            key,
            least,
            size: Cell::new(size),
            store,
            pending: Cell::new(false),
            save: RefCell::new(None),
        }))
    }

    pub fn key(&self) -> SizeKey {
        self.0.key
    }

    /// The size the user left the split at, which it lays out from.
    pub fn size(&self) -> f32 {
        self.0.size.get()
    }

    /// The size bounded to `most`, the room the window has now; the size
    /// the user gave is kept for a larger window.
    pub fn fit(&self, most: f32) -> f32 {
        self.size().min(most).max(self.0.least)
    }

    /// A drag's move to `size`: kept at once, saved once the drag pauses
    /// for [`SAVE_DELAY`]. Returns whether the size changed.
    pub fn drag(&self, size: f32, cx: &mut App) -> bool {
        if !self.keep(size) {
            return false;
        }
        self.0.pending.set(true);
        let this: Weak<Inner> = Rc::downgrade(&self.0);
        let task = cx.spawn(async move |cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            cx.update(|cx| {
                if let Some(this) = this.upgrade().map(SplitSize) {
                    this.save_pending(cx);
                }
            });
        });
        *self.0.save.borrow_mut() = Some(task);
        true
    }

    /// A drag that ended at `size`, for a split told only then, as Kit's
    /// resizable panels tell it: kept and saved at once.
    pub fn release(&self, size: f32, cx: &mut App) {
        if self.keep(size) {
            self.0.pending.set(true);
        }
        self.0.save.borrow_mut().take();
        self.save_pending(cx);
    }

    fn keep(&self, size: f32) -> bool {
        if !size.is_finite() {
            return false;
        }
        let size = size.max(self.0.least);
        if (size - self.size()).abs() < 0.5 {
            return false;
        }
        self.0.size.set(size);
        true
    }

    fn save_pending(&self, cx: &App) {
        if self.0.pending.replace(false) {
            self.0.store.save(self.0.key, self.size(), cx);
        }
    }
}

/// Where a split starts: a saved size, at least `least`, or `default`.
pub fn start(saved: Option<f32>, default: f32, least: f32) -> f32 {
    match saved {
        Some(size) if size.is_finite() && size > 0. => size.max(least),
        _ => default,
    }
}

/// Sizes kept in memory, rounded to whole dp as the app's file keeps them,
/// with a count of the saves: for tests that reopen a page.
#[cfg(any(test, feature = "testing"))]
#[derive(Default)]
pub struct MemorySizes {
    sizes: RefCell<std::collections::HashMap<SizeKey, f32>>,
    saves: Cell<usize>,
}

#[cfg(any(test, feature = "testing"))]
impl MemorySizes {
    /// A store holding `size` under `key`, as an earlier run left it; it
    /// counts no save.
    pub fn with(key: SizeKey, size: f32) -> Self {
        let sizes = Self::default();
        sizes.sizes.borrow_mut().insert(key, size);
        sizes
    }

    /// How many times a size was saved.
    pub fn saves(&self) -> usize {
        self.saves.get()
    }
}

#[cfg(any(test, feature = "testing"))]
impl SizeStore for MemorySizes {
    fn size(&self, key: SizeKey) -> Option<f32> {
        self.sizes.borrow().get(&key).copied()
    }

    fn save(&self, key: SizeKey, size: f32, _: &App) {
        self.saves.set(self.saves.get() + 1);
        self.sizes.borrow_mut().insert(key, size.round());
    }
}

#[cfg(test)]
mod tests {
    use super::{MemorySizes, SAVE_DELAY, SizeKey, SizeStore, SplitSize, set_store, start};
    use gpui_kit::TestAppContext;
    use std::rc::Rc;

    const KEY: SizeKey = SizeKey::new("inspector", "nodes");

    fn memory(cx: &mut TestAppContext) -> Rc<MemorySizes> {
        let sizes = Rc::new(MemorySizes::default());
        cx.update(|cx| set_store(sizes.clone(), cx));
        sizes
    }

    #[test]
    fn a_split_starts_at_its_saved_size_within_its_least() {
        assert_eq!(start(Some(512.), 460., 320.), 512.);
        assert_eq!(start(Some(120.), 460., 320.), 320.);
        assert_eq!(start(None, 460., 320.), 460.);
        assert_eq!(start(Some(f32::NAN), 460., 320.), 460.);
        assert_eq!(start(Some(-4.), 460., 320.), 460.);
    }

    #[gpui_kit::test]
    fn a_drag_writes_once_when_it_pauses(cx: &mut TestAppContext) {
        let sizes = memory(cx);
        let size = cx.update(|cx| SplitSize::new(KEY, 460., 320., cx));
        cx.update(|cx| {
            // A drag across sixty frames.
            for step in 0..60 {
                size.drag(460. + step as f32 * 4., cx);
            }
        });
        cx.run_until_parked();
        assert_eq!(sizes.saves(), 0, "nothing is written while it moves");
        assert_eq!(size.size(), 696.);
        cx.executor().advance_clock(SAVE_DELAY);
        cx.run_until_parked();
        assert_eq!(sizes.saves(), 1);
        assert_eq!(sizes.size(KEY), Some(696.));
    }

    #[gpui_kit::test]
    fn a_release_writes_at_once_and_ends_a_pending_drag(cx: &mut TestAppContext) {
        let sizes = memory(cx);
        let size = cx.update(|cx| SplitSize::new(KEY, 460., 320., cx));
        cx.update(|cx| {
            size.drag(500., cx);
            size.release(500., cx);
        });
        assert_eq!(sizes.saves(), 1);
        cx.executor().advance_clock(SAVE_DELAY * 2);
        cx.run_until_parked();
        assert_eq!(sizes.saves(), 1, "the drag's save was dropped");
        // A release where it started writes nothing.
        cx.update(|cx| size.release(500., cx));
        assert_eq!(sizes.saves(), 1);
    }

    #[gpui_kit::test]
    fn a_smaller_window_clamps_without_writing(cx: &mut TestAppContext) {
        let sizes = memory(cx);
        cx.update(|cx| sizes.save(KEY, 900., cx));
        let size = cx.update(|cx| SplitSize::new(KEY, 460., 320., cx));
        assert_eq!(size.fit(1200.), 900.);
        assert_eq!(size.fit(600.), 600.);
        assert_eq!(size.fit(100.), 320., "the least wins over the room");
        cx.executor().advance_clock(SAVE_DELAY * 2);
        cx.run_until_parked();
        assert_eq!(size.size(), 900.);
        assert_eq!(sizes.saves(), 1, "only the test's own save");
        assert_eq!(sizes.size(KEY), Some(900.));
    }

    #[gpui_kit::test]
    fn without_a_store_sizes_last_the_session(cx: &mut TestAppContext) {
        let size = cx.update(|cx| SplitSize::new(KEY, 460., 320., cx));
        cx.update(|cx| size.release(600., cx));
        assert_eq!(size.size(), 600.);
        let again = cx.update(|cx| SplitSize::new(KEY, 460., 320., cx));
        assert_eq!(again.size(), 460.);
    }
}
