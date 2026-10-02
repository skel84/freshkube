use std::sync::Arc;

use freshkube_core::inspection::{ProcessSort, ProcessTree};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use tokio::runtime::{Builder, Runtime};

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{ProcessesScreen, ScreenPanel, ScreenSource, StateFilter};
use crate::backend::Target;
use crate::{fixture, presentation};

fn source(node: &str) -> ScreenSource {
    let nodes = presentation::node_summaries(&fixture::cluster("prod-fra", 1));
    let summary = nodes.iter().find(|summary| summary.name == node).unwrap();
    ScreenSource {
        target: Target {
            epoch: 1,
            context: "prod-fra".into(),
            node: summary.name.clone(),
            address: summary.address.clone(),
        },
        nodes: Arc::new(nodes),
        live: None,
    }
}

fn mount(
    cx: &mut TestAppContext,
    node: &str,
) -> (Runtime, Entity<ProcessesScreen>, WindowHandle<Root>) {
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
    });
    let source = source(node);
    let mut screen = None;
    let handle = cx.open_window(size(px(1100.), px(760.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = ProcessesScreen::new(runtime.handle().clone(), window, cx);
            view.set_source(Some(source), window, cx);
            view.activate(window, cx);
            view
        });
        screen = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, screen.unwrap(), handle)
}

#[gpui_kit::test]
fn keyboard_selects_and_toggles_the_subtree(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(screen.read(cx).loader.data().is_some());
        window.click(("process", 0usize), cx);
        window.press("down", cx);
        window.render_frame(cx);
        let selected = screen.read(cx).selected;
        assert!(selected.is_some());
        assert_eq!(window.find(("process", 1usize)).selected(), Some(true));
        window.press("t", cx);
        assert!(matches!(
            screen.read(cx).tree,
            ProcessTree::Subtree { root_pid } if Some(root_pid) == selected
        ));
        window.press("t", cx);
        assert_eq!(screen.read(cx).tree, ProcessTree::Flat);
    })
    .unwrap();
}

#[gpui_kit::test]
fn state_filter_and_copy_command(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("state-zombie", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).rows(cx).len(), 1, "one example zombie");
        window.click(("process", 0usize), cx);
        window.render_frame(cx);
        window.click("copy-command", cx);
        assert!(screen.read(cx).copied.is_some());
        assert!(cx.read_from_clipboard().is_some());
        // Optional sources that failed are reported, not shown as failures.
        window.find("partial-notice");
    })
    .unwrap();
}

#[gpui_kit::test]
fn silent_node_offers_retry_without_data(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-03");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(screen.read(cx).loader.data().is_none());
        window.find("screen-retry");
        assert!(window.try_find("process-list").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn changing_target_drops_old_data(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.selected = Some(1);
            screen.set_source(Some(source("talos-wk-fra1-02")), window, cx);
            assert!(screen.loader.data().is_none());
            assert!(screen.selected.is_none());
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn rows_are_cached_until_the_data_or_the_view_change(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let computed = || crate::desktop::probe::count("processes.rows");
        let first = screen.read(cx).rows(cx);
        let base = computed();
        // Render, the list processor and navigation share one list.
        window.render_frame(cx);
        window.press("down", cx);
        let again = screen.read(cx).rows(cx);
        assert!(std::rc::Rc::ptr_eq(&first, &again));
        assert_eq!(computed(), base, "nothing it depends on changed");
        // The state filter.
        screen.update(cx, |screen, cx| {
            screen.state_filter = StateFilter::Zombie;
            cx.notify();
        });
        let zombies = screen.read(cx).rows(cx);
        assert_eq!(computed(), base + 1);
        assert!(zombies.len() < first.len());
        // Sort and tree mode.
        screen.update(cx, |screen, cx| {
            screen.state_filter = StateFilter::All;
            screen.set_sort(ProcessSort::ResidentMemory, cx);
        });
        screen.read(cx).rows(cx);
        assert_eq!(computed(), base + 2);
        screen.update(cx, |screen, cx| {
            screen.tree = ProcessTree::Full;
            cx.notify();
        });
        screen.read(cx).rows(cx);
        assert_eq!(computed(), base + 3);
        // The filter text.
        screen.update(cx, |screen, cx| {
            screen
                .query
                .update(cx, |input, cx| input.set_value("kubelet", window, cx));
        });
        let narrowed = screen.read(cx).rows(cx);
        assert_eq!(computed(), base + 4);
        assert!(narrowed.len() < first.len());
        screen.read(cx).rows(cx);
        assert_eq!(computed(), base + 4);
        // A new sample.
        screen.update(cx, |screen, cx| screen.refresh(window, cx));
        screen.read(cx).rows(cx);
        assert_eq!(computed(), base + 5);
    })
    .unwrap();
}
