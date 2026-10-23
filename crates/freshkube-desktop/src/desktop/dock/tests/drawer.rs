use super::*;

/// With a pod open in the drawer, L on the list, the pane's Logs and
/// Shell → Start open the dock under the page, and the drawer stays open
/// above it at the shell's 1280×880 (#922). An object opened straight on
/// Logs leaves the drawer closed and its row selected.
#[gpui_kit::test]
fn opening_the_dock_keeps_the_pod_pane_open(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let pods = running_pods(&pilot, cx);
    let beside_the_dock = |pod: &ResourceIdentity, tabs: usize, cx: &mut TestAppContext| {
        assert_eq!(drawer(handle, &pilot, cx), (true, Some(pod.clone())));
        assert_eq!(titles(&dock(&pilot, cx), cx).len(), tabs);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let drawer = window.find("resource-drawer").bounds();
            let dock = window.find("dock-tab-row").bounds();
            assert!(drawer.bottom() <= dock.top(), "{drawer:?} over {dock:?}");
        })
        .unwrap();
    };

    let open_pane = |pod: &ResourceIdentity, cx: &mut TestAppContext| {
        let pod = pod.clone();
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| {
                pilot.open_object(
                    builtin("pods").unwrap(),
                    pod.into(),
                    Tab::Overview,
                    window,
                    cx,
                )
            });
        })
        .unwrap();
        cx.run_until_parked();
    };

    // L on the list, with a drawn pod's drawer open from its row.
    open_pane(&pods[0], cx);
    let first = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            pods.iter()
                .find(|pod| window.try_find(crate::resources::row_id(pod)).is_some())
                .cloned()
        })
        .unwrap()
        .expect("a running pod drawn");
    cx.update_window(handle, |_, window, cx| {
        window
            .within(crate::resources::row_id(&first))
            .click("name", cx);
        window.render_frame(cx);
        window.press("l", cx);
    })
    .unwrap();
    cx.run_until_parked();
    beside_the_dock(&first, 1, cx);

    // The pane's Logs.
    let second = pods.iter().find(|pod| **pod != first).unwrap().clone();
    open_pane(&second, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("detail-open-logs", cx);
    })
    .unwrap();
    cx.run_until_parked();
    beside_the_dock(&second, 2, cx);

    // Shell → Start.
    pick_shell(handle, &pilot, &first, cx);
    assert_eq!(cx.read(shell::running_anywhere).len(), 1);
    beside_the_dock(&first, 3, cx);

    // Straight on Logs, the drawer reads nothing and stays closed.
    open_logs(handle, &pilot, "pods", &second, cx);
    assert_eq!(drawer(handle, &pilot, cx), (false, Some(second.clone())));
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
