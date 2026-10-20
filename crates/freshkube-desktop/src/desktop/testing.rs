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
    // Tests build clients against loopback servers, which the environment's
    // proxy must not take: a hermetic value that keeps only the proxy pair.
    crate::ambient::install_for_tests();
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

/// Puts `page` in the shell's map under `id`, and in the node pane's when
/// the pane embeds that screen, so both draw and tell the same one.
pub(super) fn put_page(
    view: &mut Pilot,
    id: super::pages::PageId,
    page: crate::screens::ScreenHandle,
    cx: &mut gpui_kit::App,
) {
    view.pages.node_workspace.update(cx, |nodes, _| {
        if let Some(screen) = nodes.screens.get_mut(&id) {
            *screen = page.clone();
        }
    });
    view.pages.map.insert(id, page);
}

/// Opens the Nodes row `id` as its click does. The click reaches the
/// shell as an event once the update ends, which a test inside one
/// `update_window` never sees, so this hands the shell that event at once;
/// `table::a_row_click_opens_its_node_once_the_shell_hears_it` proves the
/// click itself.
pub(super) fn click_node(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, id: &str) {
    assert!(window.find(id.to_string()).visible(), "{id} is drawn");
    let pilot = root_pilot(window, cx);
    let key = pilot
        .read(cx)
        .pages
        .node_workspace
        .read(cx)
        .rows
        .iter()
        .find(|row| row.id.as_ref() == id)
        .map(|row| row.key.clone())
        .expect("a node row");
    pilot.update(cx, |pilot, cx| {
        pilot.nodes_event(
            &super::nodes::NodesEvent::Open { key, focus: false },
            window,
            cx,
        )
    });
}

/// Test setup that needs a healthy table row follows the actual group control.
/// Folding itself is exercised separately by Nodes' projection/entry-point tests.
pub(super) fn expand_healthy_nodes(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    let pilot = root_pilot(window, cx);
    let nodes = &pilot.read(cx).pages.node_workspace.read(cx);
    if nodes.healthy_collapsed() && nodes.view == NodeView::Table {
        window.click("nodes-healthy-toggle", cx);
        window.render_frame(cx);
    }
}

/// The shell's retained screen of type `T`.
pub(crate) fn screen<T: 'static>(pilot: &Entity<Pilot>, cx: &gpui_kit::App) -> Entity<T> {
    pilot
        .read(cx)
        .pages
        .map
        .iter()
        .filter(|(id, _)| id.is_screen())
        .find_map(|(_, screen)| screen.view().downcast::<T>().ok())
        .expect("a retained screen of this type")
}

/// Choose through the Nodes page, then return to the previously visible view.
pub(crate) fn pick_target(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, ix: usize) {
    let view = root_pilot(window, cx);
    let page = view.read(cx).navigation.page;
    let tab = view.read(cx).pages.node_workspace.read(cx).tab;
    window.click("nav-nodes", cx);
    window.render_frame(cx);
    if view.read(cx).pages.node_workspace.read(cx).open {
        window.click("node-close", cx);
        window.render_frame(cx);
    }
    expand_healthy_nodes(window, cx);
    // A row's click reaches the shell as an event, after this update; open
    // the node as that event would.
    let key = view.read(cx).pages.node_workspace.read(cx).rows[ix]
        .key
        .clone();
    view.update(cx, |view, cx| view.open_node(key, window, cx));
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
    let name = view.read(cx).target.selected_node.clone().unwrap();
    area(window, cx, "nav-nodes");
    if !view.read(cx).pages.node_workspace.read(cx).open {
        expand_healthy_nodes(window, cx);
        let key = view
            .read(cx)
            .pages
            .node_workspace
            .read(cx)
            .rows
            .iter()
            .find(|row| row.key.talos.as_ref() == Some(&name))
            .unwrap()
            .key
            .clone();
        // As the row's click would, through the shell.
        view.update(cx, |view, cx| view.open_node(key, window, cx));
        window.render_frame(cx);
    }
    // A tab's click and Command-Shift-] reach the shell as events once the
    // update ends; hand the shell the tab's event at once, as they would.
    // The strip draws the tab, or More holds it.
    use super::nodes::{NodeTab, NodeTabs};
    assert!(
        window.try_find(tab.id()).is_some() || matches!(tab, NodeTab::Events | NodeTab::Yaml),
        "{} is reachable",
        tab.id()
    );
    view.update(cx, |view, cx| {
        view.nodes_event(&super::nodes::NodesEvent::Tab(tab), window, cx)
    });
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
