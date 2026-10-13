use super::*;

#[gpui_kit::test]
fn a_right_click_selects_its_node_and_its_menu_folds_the_healthy_ones(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        window.right_click("node-talos-wk-fra1-03", cx);
    })
    .unwrap();
    // The menu builds on the next frame.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace.read(cx);
        assert_eq!(nodes.row().unwrap().name, "talos-wk-fra1-03");
        // Selected as an arrow selects: the closed pane stays closed.
        assert!(!nodes.open);
        let menu = window.within("popup-menu");
        let items: Vec<_> = (0usize..)
            .map_while(|ix| menu.try_find(ix))
            .map(|item| item.label().map(str::to_owned))
            .collect();
        assert_eq!(
            items,
            [
                Some("Open".to_owned()),
                None,
                Some("Expand healthy nodes".to_owned())
            ]
        );
        window.within("popup-menu").click(2usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert!(!pilot.read(cx).node_workspace.read(cx).healthy_collapsed()));
}

/// While the pane shows one node, a right-click on another opens it on the
/// same tab through the shell, after the menu took the keyboard to the
/// table; node usage keeps reading. Dismissed, the menu leaves the keyboard
/// on the table; its Open moves it into the pane.
#[gpui_kit::test]
fn a_right_click_beside_an_open_pane_opens_its_node_on_the_same_tab(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    let selected = |cx: &mut TestAppContext| {
        pilot.read_with(cx, |pilot, cx| {
            let nodes = pilot.node_workspace.read(cx);
            (
                pilot.selected_node.clone(),
                nodes.open,
                nodes.tab,
                nodes.metrics.visible,
            )
        })
    };
    let focused = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            let nodes = pilot.read(cx).node_workspace.read(cx);
            (
                nodes.focus.is_focused(window),
                nodes.pane_focus.contains_focused(window, cx),
            )
        })
        .unwrap()
    };
    let opened = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            crate::desktop::tests::open_node_tab(window, cx, NodeTab::Processes);
            window.right_click("node-talos-wk-fra1-03", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        assert_eq!(
            selected(cx),
            (
                Some("talos-wk-fra1-03".to_owned()),
                true,
                NodeTab::Processes,
                true
            )
        );
    };
    opened(cx);
    step(cx, handle, |window, cx| window.press("escape", cx));
    assert_eq!(focused(cx), (true, false), "dismissed, on the table");
    opened(cx);
    step(cx, handle, |window, cx| {
        window.within("popup-menu").click(0usize, cx)
    });
    assert_eq!(selected(cx).0.as_deref(), Some("talos-wk-fra1-03"));
    assert_eq!(focused(cx), (false, true), "Open, in the pane");
}
