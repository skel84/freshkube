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
