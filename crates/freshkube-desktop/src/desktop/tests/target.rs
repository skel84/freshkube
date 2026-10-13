use super::*;

const SILENT_NODE: &str = "talos-wk-fra1-03";

#[gpui_kit::test]
fn target_picker_and_tiles_change_the_target_node(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        assert!(window.try_find("target-node").is_none());
        pick_target(window, cx, 3);
        assert_eq!(
            view.read(cx).selected_node.as_deref(),
            Some("talos-wk-fra1-01")
        );
        window.click("nav-overview", cx);
        window.render_frame(cx);
        window.click("tile-services", cx);
    })
    .unwrap();
    // Overview asks the shell, which hears it once the click's effects run.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let pilot = view.read(cx);
        assert_eq!(pilot.page, Page::SystemServices);
        assert_eq!(pilot.selected_node.as_deref(), Some("talos-wk-fra1-01"));
        assert!(
            window
                .find("system-service-talos-wk-fra1-02-kubelet")
                .visible()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn silent_node_is_unknown_not_failed(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let card = window.find("node-talos-wk-fra1-03");
        assert!(card.label().unwrap().contains(SILENT_NODE));
        pick_target(window, cx, 5);
        open_node_tab(window, cx, super::super::nodes::NodeTab::Services);
    })
    .unwrap();
    // The tab hears the new node once the shell's effects run, and the
    // shell hears its button the same way.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_node.as_deref(), Some(SILENT_NODE));
        assert!(window.find("back-to-overview").visible());
        assert!(window.try_find("services-region").is_none());
        window.click("back-to-overview", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Overview);
    })
    .unwrap();
}

#[gpui_kit::test]
fn table_view_keeps_node_identity_and_selection(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        expand_healthy_nodes(window, cx);
        window.click("nodes-view-table", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).node_workspace.view, NodeView::Table);
        assert_eq!(
            window
                .within("nodes-page")
                .find("node-talos-cp-fra1-01")
                .selected(),
            Some(false)
        );
        window
            .within("nodes-page")
            .click("node-talos-wk-fra1-01", cx);
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).selected_node.as_deref(),
            Some("talos-wk-fra1-01")
        );
        window.click("node-close", cx);
        window.render_frame(cx);
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).node_workspace.view, NodeView::Cards);
        assert_eq!(
            window
                .within("nodes-page")
                .find("node-talos-wk-fra1-01")
                .selected(),
            Some(true)
        );
    })
    .unwrap();
}
