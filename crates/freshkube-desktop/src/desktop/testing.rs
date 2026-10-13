//! The shared harness for the shell's UI tests: mounting a `Pilot` on example
//! or explicit options, and the page, kind and node-tab openers. Dialogs run
//! with reduced motion (the test clock can't advance their animation).

use super::{GpuiOptions, NodeView, Page, Pilot};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext,
    component::{Root, Theme, ThemeMode},
    px, size,
};

pub(crate) fn fixture(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    mount(cx, GpuiOptions::fixture(), width, height)
}

pub(super) fn mount(
    cx: &mut TestAppContext,
    options: GpuiOptions,
    width: f32,
    height: f32,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        Theme::change(ThemeMode::Light, None, cx);
        // Dialogs animate on the real clock, which the test clock can't
        // advance; reduced motion opens and closes them immediately.
        cx.set_reduce_motion(true);
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut view = None;
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let pilot = cx.new(|cx| Pilot::new(options, runtime.handle().clone(), window, cx));
        view = Some(pilot.clone());
        Root::new(pilot, window, cx)
    });
    cx.run_until_parked();
    (runtime, window.into(), view.unwrap())
}

/// Starts a shell in `pod`'s first running container, in a dock tab, as a
/// pick from the pane's Shell menu does.
pub(crate) fn start_shell(
    handle: AnyWindowHandle,
    pilot: &Entity<Pilot>,
    pod: &crate::resources::model::ResourceIdentity,
    cx: &mut TestAppContext,
) {
    use crate::resources::{ResourceLink, ShellRequest, detail::DetailTarget, example, live};
    let containers = example::document(pod, live::now())
        .unwrap()
        .overview
        .pod
        .unwrap();
    let container = crate::resources::shell::choices(&containers)
        .into_iter()
        .find(|choice| choice.enabled)
        .unwrap()
        .name;
    let request = ShellRequest {
        target: DetailTarget {
            identity: pod.clone(),
            kind: freshkube_core::resources::builtin("pods").unwrap(),
        },
        container,
        containers: None,
    };
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.resource_link(ResourceLink::Shell(request), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
}

pub(super) fn root_pilot(window: &gpui_kit::Window, cx: &gpui_kit::App) -> Entity<Pilot> {
    window
        .root::<Root>()
        .unwrap()
        .unwrap()
        .read(cx)
        .view()
        .clone()
        .downcast::<Pilot>()
        .unwrap()
}

/// Test setup that needs a healthy table row follows the actual group control.
/// Folding itself is exercised separately by Nodes' projection/entry-point tests.
pub(super) fn expand_healthy_nodes(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    let pilot = root_pilot(window, cx);
    let nodes = &pilot.read(cx).node_workspace;
    if nodes.healthy_collapsed() && nodes.view == NodeView::Table {
        window.click("nodes-healthy-toggle", cx);
        window.render_frame(cx);
    }
}

/// The shell's retained screen of type `T`.
pub(crate) fn screen<T: 'static>(pilot: &Entity<Pilot>, cx: &gpui_kit::App) -> Entity<T> {
    pilot
        .read(cx)
        .screens
        .iter()
        .find_map(|(_, screen)| screen.view().downcast::<T>().ok())
        .expect("a retained screen of this type")
}

/// Choose through the Nodes page, then return to the previously visible view.
pub(crate) fn pick_target(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, ix: usize) {
    let view = root_pilot(window, cx);
    let page = view.read(cx).page;
    let tab = view.read(cx).node_workspace.tab;
    window.click("nav-nodes", cx);
    window.render_frame(cx);
    if view.read(cx).node_workspace.open {
        window.click("node-close", cx);
        window.render_frame(cx);
    }
    expand_healthy_nodes(window, cx);
    let id = view.read(cx).node_workspace.rows[ix].id.clone();
    window.click(id, cx);
    window.render_frame(cx);
    if page == Page::Nodes {
        view.update(cx, |view, cx| view.show_node_tab(tab, window, cx));
    } else {
        view.update(cx, |view, cx| view.navigate(page, window, cx));
    }
    window.render_frame(cx);
}

/// Opens a built-in kind on the Resources page by its kubectl key, as the
/// rail's column does.
pub(crate) fn open_kind(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, key: &str) {
    let view = root_pilot(window, cx);
    view.update(cx, |view, cx| view.open_builtin(key, window, cx));
}

/// Old page coverage now follows the actual node-pane tab controls.
pub(crate) fn open_node_tab(
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
    tab: super::nodes::NodeTab,
) {
    let view = root_pilot(window, cx);
    let name = view.read(cx).selected_node.clone().unwrap();
    area(window, cx, "nav-nodes");
    if !view.read(cx).node_workspace.open {
        expand_healthy_nodes(window, cx);
        let id = view
            .read(cx)
            .node_workspace
            .rows
            .iter()
            .find(|row| row.key.talos.as_ref() == Some(&name))
            .unwrap()
            .id
            .clone();
        window.click(id, cx);
        window.render_frame(cx);
    }
    if window.try_find(tab.id()).is_some_and(|button| {
        let pane = window.find("node-pane").bounds();
        button.visible()
            && button.bounds().left() >= pane.left()
            && button.bounds().right() <= pane.right()
    }) {
        window.click(tab.id(), cx);
    } else {
        for _ in 0..10 {
            if view.read(cx).node_workspace.tab == tab {
                break;
            }
            window.press("secondary-}", cx);
            window.render_frame(cx);
        }
    }
    window.render_frame(cx);
}

/// Clicks the rail button `id`, scrolling the rail to it first.
pub(super) fn area(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, id: &str) {
    let id = SharedString::from(id.to_owned());
    for _ in 0..30 {
        let rail = window.find("nav-rail").bounds();
        let button = window.find(id.clone()).bounds();
        if button.top() >= rail.top() && button.bottom() <= rail.bottom() {
            window.click(id, cx);
            window.render_frame(cx);
            return;
        }
        let above = button.top() < rail.top();
        window.scroll(
            "nav-rail",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                px(0.),
                px(if above { 40. } else { -40. }),
            )),
            cx,
        );
        window.render_frame(cx);
    }
    panic!("{id} never scrolled into the rail");
}
