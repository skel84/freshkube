use super::*;

/// Logs and a shell open in the dock, under the drawer, which steps aside
/// and keeps its row selected.
#[gpui_kit::test]
fn logs_and_a_shell_close_the_drawer_and_keep_its_row(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let pods = running_pods(&pilot, cx);
    pick_shell(handle, &pilot, &pods[0], cx);
    assert_eq!(cx.read(shell::running_anywhere).len(), 1);
    assert_eq!(drawer(handle, &pilot, cx), (false, Some(pods[0].clone())));

    open_logs(handle, &pilot, "pods", &pods[1], cx);
    assert_eq!(drawer(handle, &pilot, cx), (false, Some(pods[1].clone())));
    assert_eq!(titles(&dock(&pilot, cx), cx).len(), 2);
}

/// Shells live in the dock: closing the drawer, swapping its object or a
/// click beside the rows leaves a running shell running and asks nothing.
#[gpui_kit::test]
fn the_drawer_closes_and_swaps_around_a_running_shell(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let pods = running_pods(&pilot, cx);
    pick_shell(handle, &pilot, &pods[0], cx);
    // Healthy pods fold while anything is wrong: swap to one that is drawn.
    let other = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            pods[1..]
                .iter()
                .find(|pod| window.try_find(crate::resources::row_id(pod)).is_some())
                .cloned()
        })
        .unwrap()
        .expect("another running pod drawn");
    let untouched = |cx: &mut TestAppContext| {
        assert!(cx.pending_prompt().is_none());
        assert_eq!(cx.read(shell::running_anywhere).len(), 1);
    };
    // Near the left edge, beside the drawer, as a person would.
    let click = |id: gpui_kit::ElementId, cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let height = window.find(id.clone()).bounds().size.height;
            window.click_at(id, gpui_kit::point(px(40.), height / 2.), cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let row = |pod: &ResourceIdentity| crate::resources::row_id(pod);

    // The shell's own pod, then another in the same drawer.
    click(row(&pods[0]), cx);
    assert_eq!(drawer(handle, &pilot, cx), (true, Some(pods[0].clone())));
    click(row(&other), cx);
    assert_eq!(drawer(handle, &pilot, cx), (true, Some(other.clone())));
    untouched(cx);

    // Its ×.
    cx.update_window(handle, |_, window, cx| window.click("detail-close", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(!drawer(handle, &pilot, cx).0);
    untouched(cx);

    // A click under the rows.
    click(row(&pods[0]), cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let area = window.find("resource-list-area").bounds();
        window.click_at(
            "resource-list-area",
            gpui_kit::point(px(40.), area.size.height - px(10.)),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(drawer(handle, &pilot, cx), (false, Some(pods[0].clone())));
    untouched(cx);
}
