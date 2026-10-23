//! The change page's view, kept by the window's navigation store. The page
//! knows the choice; desktop owns the file, as it does for split sizes.
use std::rc::Rc;

use gpui_kit::{App, BorrowAppContext, Global, Window, WindowId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The page's two views. Windows that saved `list` or `chain` before the
/// Overview keep their choice: `list` was All hops, and the Chain is gone,
/// so `chain` opens the Overview.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeView {
    #[default]
    #[serde(alias = "chain")]
    Overview,
    #[serde(alias = "list")]
    Hops,
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
        cx.on_window_closed(|cx, id| {
            cx.update_global::<Store, _>(|store, _| {
                store.windows.remove(&id);
            });
        })
        .detach();
    }
}

/// Tests: a store that starts every window on `view`.
#[cfg(test)]
pub(crate) fn use_session(view: ChangeView, cx: &mut App) {
    set_store(Rc::new(Session(std::cell::Cell::new(view))), cx);
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AppContext, Context, IntoElement, Render, TestAppContext, div, px, size};

    struct View;
    impl Render for View {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[test]
    fn a_choice_saved_before_the_overview_keeps_its_meaning() {
        let read = |text: &str| serde_json::from_str::<ChangeView>(text).unwrap();
        assert_eq!(read("\"list\""), ChangeView::Hops);
        assert_eq!(read("\"chain\""), ChangeView::Overview);
        assert_eq!(read("\"hops\""), ChangeView::Hops);
        assert_eq!(read("\"overview\""), ChangeView::Overview);
        assert_eq!(ChangeView::default(), ChangeView::Overview);
        assert_eq!(
            serde_json::to_string(&ChangeView::Hops).unwrap(),
            "\"hops\""
        );
    }

    #[gpui_kit::test]
    fn closing_a_window_releases_its_kept_view(cx: &mut TestAppContext) {
        let mut kept = None;
        let handle = cx.open_window(size(px(500.), px(400.)), |window, cx| {
            let view = store(window, cx);
            view.save(ChangeView::Hops, cx);
            kept = Some(Rc::downgrade(&view));
            View
        });
        assert!(kept.as_ref().unwrap().upgrade().is_some());
        cx.update_window(handle.into(), |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();
        assert!(kept.unwrap().upgrade().is_none());
    }
}
