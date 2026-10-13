mod drawer;
mod flash;
mod keys;
mod list;
mod menu;
mod pods;
mod reads;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Bounds, Entity, Pixels, TestAppContext, px, size};
use tokio::runtime::Runtime;
// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{
    KEYBOARD_PAUSE, ClusterAccess, ClusterSource, ListView, NotServed, ResourcesScreen, WATCH_COALESCE,
    row_id,
};
use crate::resources::example;
use crate::resources::model::{ReadState, ResourceIdentity, ResourceRow};
use crate::resources::store::ResourceEvent;
use freshkube_core::resources::ResourceKind;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A built-in or example custom kind by its kubectl key.
fn kind(key: &str) -> ResourceKind {
    example::kind(key).unwrap()
}

fn source(context: &str) -> ClusterSource {
    ClusterSource {
        id: example::connection(context),
        context: context.into(),
        access: ClusterAccess::Example,
    }
}

/// The page as the shell shows it: example data for `context`, visible.
fn mount(
    cx: &mut TestAppContext,
    context: Option<&str>,
) -> (Runtime, Entity<ResourcesScreen>, AnyWindowHandle) {
    mount_sized(cx, context, 1280.)
}

fn mount_sized(
    cx: &mut TestAppContext,
    context: Option<&str>,
    width: f32,
) -> (Runtime, Entity<ResourcesScreen>, AnyWindowHandle) {
    mount_window(cx, context, width, 800.)
}

fn mount_window(
    cx: &mut TestAppContext,
    context: Option<&str>,
    width: f32,
    height: f32,
) -> (Runtime, Entity<ResourcesScreen>, AnyWindowHandle) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        cx.set_reduce_motion(true);
    });
    let runtime = Runtime::new().unwrap();
    let source = context.map(source);
    let mut screen = None;
    let handle = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = ResourcesScreen::new(runtime.handle().clone(), window, cx);
            view.set_source(source, window, cx);
            view.set_visible(true, window, cx);
            // Most tests read the rows in one sorted list; the problems
            // view has tests of its own.
            view.set_list_view(ListView::All, cx);
            view
        });
        screen = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, screen.unwrap(), handle.into())
}

fn identity_at(
    screen: &Entity<ResourcesScreen>,
    ix: usize,
    cx: &gpui_kit::App,
) -> ResourceIdentity {
    let screen = screen.read(cx);
    screen
        .projection
        .row(&screen.store, ix)
        .unwrap()
        .identity
        .clone()
}

fn selected(screen: &Entity<ResourcesScreen>, cx: &gpui_kit::App) -> Option<ResourceIdentity> {
    screen.read(cx).projection.selected().cloned()
}

fn shown(screen: &Entity<ResourcesScreen>, cx: &gpui_kit::App) -> Option<ResourceIdentity> {
    screen.read(cx).detail.read(cx).target_identity().cloned()
}

/// Applies events as if the current read delivered them.
fn deliver(screen: &Entity<ResourcesScreen>, events: Vec<ResourceEvent>, cx: &mut gpui_kit::App) {
    screen.update(cx, |screen, cx| {
        let epoch = screen.store.epoch();
        screen.apply(epoch, vec![events], cx);
    });
}

/// The pods page as it opens: problems first.
fn problems(screen: &Entity<ResourcesScreen>, cx: &mut gpui_kit::App) {
    screen.update(cx, |screen, cx| {
        screen.set_list_view(ListView::Problems, cx)
    });
}

/// Opens a context menu with `press`, a right-click, and reads its items'
/// labels, a separator as `None`.
fn open_menu(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    press: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
) -> Vec<Option<String>> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        press(window, cx);
    })
    .unwrap();
    // The menu builds on the next frame.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menu = window.within("popup-menu");
        (0usize..)
            .map_while(|ix| menu.try_find(ix))
            .map(|item| item.label().map(str::to_owned))
            .collect()
    })
    .unwrap()
}

/// Picks `label` from the open menu.
fn pick(
    window: &mut gpui_kit::Window,
    items: &[Option<String>],
    label: &str,
    cx: &mut gpui_kit::App,
) {
    let ix = items.iter().position(|item| item.as_deref() == Some(label));
    window.within("popup-menu").click(ix.unwrap(), cx);
}
