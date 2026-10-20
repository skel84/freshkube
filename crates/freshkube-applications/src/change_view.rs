//! The change page's view, kept by the window's navigation store. The page
//! knows the choice; desktop owns the file, as it does for split sizes.
use std::rc::Rc;

use gpui_kit::{App, BorrowAppContext, Global, Window, WindowId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

struct Store {
    backend: Rc<dyn ViewStore>,
    windows: HashMap<WindowId, Rc<dyn ViewStore>>,
}
impl Global for Store {}

pub fn set_store(backend: Rc<dyn ViewStore>, cx: &mut App) {
    if cx.has_global::<Store>() {
        // Installing another window's file leaves open windows on their
        // own store. A newly opened window starts from the installed file.
        cx.update_global::<Store, _>(|store, _| store.backend = backend);
    } else {
        cx.set_global(Store {
            backend,
            windows: HashMap::new(),
        });
    }
}

pub(crate) fn store(window: &Window, cx: &mut App) -> Rc<dyn ViewStore> {
    if !cx.has_global::<Store>() {
        set_store(Rc::new(Session(std::cell::Cell::default())), cx);
    }
    let id = window.window_handle().window_id();
    cx.update_global::<Store, _>(|store, _| {
        let backend = store.backend.clone();
        store
            .windows
            .entry(id)
            .or_insert_with(|| {
                Rc::new(Kept {
                    current: std::cell::Cell::new(backend.view()),
                    backend,
                })
            })
            .clone()
    })
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
