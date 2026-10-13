use super::*;

#[gpui_kit::test]
fn the_dock_resizes_minimizes_below_its_least_and_fits_the_window(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    // The dock spans the page cell under the page.
    cx.update_window(handle, |_, window, _| {
        let cell = window.find("page-cell").bounds();
        let bounds = window.find("dock").bounds();
        assert!((bounds.left() - cell.left()).abs() <= px(1.), "{bounds:?}");
        assert!((bounds.right() - cell.right()).abs() <= px(1.));
        assert!((bounds.bottom() - cell.bottom()).abs() <= px(1.));
        let tall = crate::ui::dp_px(DEFAULT_HEIGHT, window);
        assert!((bounds.size.height - tall).abs() <= px(1.), "{bounds:?}");
    })
    .unwrap();

    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| dock.resize(420., window, cx))
    })
    .unwrap();
    assert_eq!(cx.update(|cx| dock.read(cx).height()), 420.);
    // Below the least height it minimizes, keeping the dragged height.
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| dock.resize(MIN_HEIGHT - 10., window, cx))
    })
    .unwrap();
    assert!(cx.update(|cx| !dock.read(cx).is_open()));
    assert_eq!(cx.update(|cx| dock.read(cx).height()), 420.);
    cx.run_until_parked();
    draw(handle, cx);
    cx.update_window(handle, |_, window, _| {
        let bounds = window.find("dock").bounds();
        let bar = crate::ui::dp_px(freshkube_ui::dock::BAR_HEIGHT, window);
        assert!(bounds.size.height <= bar + px(2.), "{bounds:?}");
    })
    .unwrap();

    // Fit to window fills the page cell; Restore brings the height back.
    cx.update_window(handle, |_, window, cx| window.click("dock-maximize", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert!(cx.update(|cx| dock.read(cx).is_maximized()));
    cx.update_window(handle, |_, window, cx| {
        let cell = window.find("page-cell").bounds();
        let bounds = window.find("dock").bounds();
        assert!(
            (bounds.size.height - cell.size.height).abs() <= px(1.),
            "{bounds:?}"
        );
        window.click("dock-maximize", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.update(|cx| !dock.read(cx).is_maximized()));
    assert_eq!(cx.update(|cx| dock.read(cx).height()), 420.);
}

#[gpui_kit::test]
fn a_crash_looping_pod_shows_its_notice_and_three_lines_in_a_small_window(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    let dock = dock(&pilot, cx);
    let crashing = objects(
        &pilot,
        "pods",
        |cells, _| cells[2] == "CrashLoopBackOff",
        cx,
    );
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| dock.resize(MIN_HEIGHT, window, cx))
    })
    .unwrap();
    open_logs(handle, &pilot, "pods", &crashing[0], cx);
    settle(handle, cx);
    let view = pod_view(&dock, 0, cx);
    assert_whole_without_scrolling(handle, &view, "pod-logs-note", cx);
}

#[gpui_kit::test]
fn a_gone_pods_notice_counts_in_the_docks_least_height(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    let dock = dock(&pilot, cx);
    let mut gone = running_pods(&pilot, cx).remove(0);
    gone.name = format!("{}-gone", gone.name);
    gone.uid = String::new();
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| dock.resize(MIN_HEIGHT, window, cx))
    })
    .unwrap();
    // No pane opens an object that isn't there; ask the dock as a link
    // to a pod that has since gone would.
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| {
            let target = crate::resources::detail::DetailTarget {
                identity: gone,
                kind: builtin("pods").unwrap(),
            };
            dock.open_logs(
                crate::resources::LogsRequest { target, at: None },
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    settle(handle, cx);
    let view = pod_view(&dock, 0, cx);
    assert_whole_without_scrolling(handle, &view, "dock-tab-notice", cx);
    // Settled: another frame changes nothing.
    let height = cx
        .update_window(handle, |_, window, _| window.find("dock").bounds())
        .unwrap();
    settle(handle, cx);
    let again = cx
        .update_window(handle, |_, window, _| window.find("dock").bounds())
        .unwrap();
    assert_eq!(height, again);
}

#[gpui_kit::test]
fn l_in_the_smallest_window_keeps_a_running_pods_row_above_three_lines(cx: &mut TestAppContext) {
    let (_runtime, _, pilot) = fixture(cx, 760., 560.);
    // Low in the list, among the healthy pods.
    let pod = running_pods(&pilot, cx).remove(0);
    l_keeps_the_row_above_three_lines(&pod, 13., true, cx);
}

#[gpui_kit::test]
fn l_in_the_smallest_window_keeps_a_crash_looping_pods_row_above_three_lines(
    cx: &mut TestAppContext,
) {
    let (_runtime, _, pilot) = fixture(cx, 760., 560.);
    let pod = lowest_crash_looping_pod(&pilot, cx);
    l_keeps_the_row_above_three_lines(&pod, 13., true, cx);
}

#[gpui_kit::test]
fn l_at_the_largest_text_size_opens_a_log_whose_panel_doesnt_scroll(cx: &mut TestAppContext) {
    let (_runtime, _, pilot) = fixture(cx, 760., 560.);
    let pod = lowest_crash_looping_pod(&pilot, cx);
    l_keeps_the_row_above_three_lines(&pod, 20., false, cx);
}

#[gpui_kit::test]
fn the_toolbar_is_one_row_wide_and_two_narrow_or_large(cx: &mut TestAppContext) {
    for (width, height, text_size, most) in [
        (1280., 880., 13., 1),
        (760., 560., 13., 2),
        (760., 560., 20., 2),
    ] {
        let (_runtime, handle, pilot) = fixture(cx, width, height);
        while cx
            .update_window(handle, |_, window, _| window.rem_size())
            .unwrap()
            < px(text_size)
        {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.press("secondary-=", cx)
            })
            .unwrap();
        }
        let pod = lowest_crash_looping_pod(&pilot, cx);
        open_logs(handle, &pilot, "pods", &pod, cx);
        settle(handle, cx);
        let rows = tool_rows(handle, POD_TOOLS, cx);
        assert!(rows <= most, "{width}×{height} at {text_size}: {rows} rows");
        // Previous keeps its label in a compact toolbar too, where the
        // crash-loop note points to it: wider than an icon alone.
        cx.update_window(handle, |_, window, _| {
            let mut width = |id| window.within("dock").find(id).bounds().size.width;
            let (previous, icon) = (width("pod-logs-previous"), width("pod-logs-timestamps"));
            assert!(
                previous > icon * 2.,
                "{text_size}: Previous {previous:?} beside an icon {icon:?}"
            );
            // Levels keeps room for its icon and its chevron.
            let levels = width("logs-levels");
            assert!(
                levels > icon * 1.5,
                "{text_size}: Levels {levels:?} beside an icon {icon:?}"
            );
        })
        .unwrap();
        // A workload's tab keeps to the same rows.
        let api = objects(&pilot, "deployments.apps", |_, name| name == "api", cx);
        open_logs(handle, &pilot, "deployments.apps", &api[0], cx);
        settle(handle, cx);
        let rows = tool_rows(handle, WORKLOAD_TOOLS, cx);
        assert!(
            rows <= most,
            "workload, {width}×{height} at {text_size}: {rows} rows"
        );
    }
}
