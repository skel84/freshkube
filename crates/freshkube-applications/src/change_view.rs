//! The change page's view, kept by the window's navigation store. The page
//! knows the choice; desktop owns the file, as it does for split sizes.
use std::rc::Rc;

use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeView {
    #[default]
    List,
    Chain,
}

pub trait ViewStore {
    fn view(&self) -> ChangeView;
    fn save(&self, view: ChangeView, cx: &App);
}

struct Store(Rc<dyn ViewStore>);
impl Global for Store {}

pub fn set_store(store: Rc<dyn ViewStore>, cx: &mut App) {
    let current = store.view();
    cx.set_global(Store(Rc::new(Kept {
        current: std::cell::Cell::new(current),
        backend: store,
    })));
}

pub(crate) fn store(cx: &mut App) -> Rc<dyn ViewStore> {
    if !cx.has_global::<Store>() {
        set_store(Rc::new(Session(std::cell::Cell::default())), cx);
    }
    cx.global::<Store>().0.clone()
}

struct Session(std::cell::Cell<ChangeView>);
impl ViewStore for Session {
    fn view(&self) -> ChangeView {
        self.0.get()
    }
    fn save(&self, view: ChangeView, _: &App) {
        self.0.set(view);
    }
}

// Also remember the choice when a fixture has no file or a save fails.
struct Kept {
    current: std::cell::Cell<ChangeView>,
    backend: Rc<dyn ViewStore>,
}
impl ViewStore for Kept {
    fn view(&self) -> ChangeView {
        self.current.get()
    }
    fn save(&self, view: ChangeView, cx: &App) {
        self.current.set(view);
        self.backend.save(view, cx);
    }
}
