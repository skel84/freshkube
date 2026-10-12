use super::*;

#[gpui_kit::test]
fn every_screen_is_reachable_and_loads_only_when_shown(cx: &mut TestAppContext) {
    // Minimum window size: the rail and column must scroll, not clip.
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for (area_id, pages) in [
            (
                "nav-control-plane",
                &[
                    Page::Etcd,
                    Page::Security,
                    Page::Lifecycle,
                    Page::Operations,
                ][..],
            ),
            ("nav-k8s-group-workloads", &[Page::Health][..]),
        ] {
            area(window, cx, area_id);
            for page in pages {
                reveal(window, cx, &format!("nav-{}", page.slug()));
            }
        }
        window.press("secondary-1", cx);
        window.render_frame(cx);
        // Hidden screens never ask for data.
        assert!(window.try_find("processes-page").is_none());
        open_node_tab(window, cx, super::super::nodes::NodeTab::Processes);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Nodes);
        assert_eq!(window.find("node-tab-processes").selected(), Some(true));
        assert!(window.find("processes-list").visible());
        // Rows are keyed by PID. Init leads the first sample, which has no
        // CPU deltas to sort by yet.
        assert!(window.find(("process", 1usize)).visible());
        // Navigating focuses the list, so arrow keys work without a click.
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(window.find(("process", 1usize)).selected(), Some(true));
        // A node switch reaches the visible screen as a new target.
        pick_target(window, cx, 4);
        assert_eq!(view.read(cx).selected_node.as_deref(), Some(DEGRADED_NODE));
        window.render_frame(cx);
        assert!(window.find("partial-notice").visible());
        // The retained screen scrolls under the node header in a short pane.
        for _ in 0..10 {
            if window.find("processes-list").visible() {
                break;
            }
            window.scroll(
                "processes-page",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-60.))),
                cx,
            );
            window.render_frame(cx);
        }
        assert!(window.find("processes-list").visible());
        for tab in [
            super::super::nodes::NodeTab::Storage,
            super::super::nodes::NodeTab::Network,
            super::super::nodes::NodeTab::Diagnostics,
            super::super::nodes::NodeTab::Logs,
            super::super::nodes::NodeTab::Services,
        ] {
            open_node_tab(window, cx, tab);
            assert_eq!(view.read(cx).node_workspace.tab, tab);
        }
        for (key, page) in [("secondary-6", Page::Etcd), ("secondary-5", Page::Health)] {
            window.press(key, cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).page, page);
        }
        // Control plane's pages are in its column.
        area(window, cx, "nav-control-plane");
        assert_eq!(view.read(cx).page, Page::Etcd);
        for page in [Page::Security, Page::Lifecycle] {
            let nav = format!("nav-{}", page.slug());
            reveal(window, cx, &nav);
            window.click(SharedString::from(nav), cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).page, page);
        }
        // Kubernetes kinds are in their group's column.
        assert!(window.try_find("resources-page").is_none());
        area(window, cx, "nav-k8s-group-workloads");
        reveal(window, cx, "nav-k8s-pods");
        window.click("nav-k8s-pods", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Resources);
        assert_eq!(window.find("nav-k8s-pods").checked(), Some(true));
        assert!(window.find("resource-list").visible());
        contexts(window, cx);
        assert!(window.find(("context", 0usize)).visible());
        area(window, cx, "nav-control-plane");
        reveal(window, cx, "nav-operations");
        window.click("nav-operations", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Operations);
        assert!(window.try_find("resources-page").is_none());
    })
    .unwrap();
}
