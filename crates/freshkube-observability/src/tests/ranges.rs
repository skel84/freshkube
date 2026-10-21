use super::*;

#[gpui_kit::test]
fn ranges_reports_and_traces_have_independent_data(cx: &mut TestAppContext) {
    let (_runtime, _handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let prior = page.charts[0].series[0].values.clone();
            page.set_range(1, cx);
            assert_ne!(prior, page.charts[0].series[0].values);
            page.open_app(example::id("payments/api"), Report::Latency, cx);
            assert_eq!(page.selected_application().unwrap().key, "payments/api");
            page.open(Destination::Traces, cx);
            let trace = page.live_traces.trace.clone();
            assert!(trace.is_some());
            let charts = page.charts[0].series[0].values.clone();
            page.set_range(6, cx);
            assert_ne!(
                page.live_traces.trace, trace,
                "another window answers its own requests"
            );
            assert!(page.live_traces.trace.is_some());
            assert_ne!(charts, page.charts[0].series[0].values);
        })
    });
}

#[gpui_kit::test]
fn reopen_and_explicit_refresh_advance_the_window_but_navigation_keeps_it(cx: &mut TestAppContext) {
    let (_runtime, _handle, page) = mount(cx, false);
    let original = cx.update(|cx| page.read(cx).live.range);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(3600));
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.open(Destination::ServiceMap, cx);
            assert_eq!(
                page.live.range, original,
                "inspect the same captured window across destinations"
            );
            let read = hold_read(page, cx);
            let revision = read.borrow().revision();
            page.refresh_current(cx);
            assert_eq!(
                page.live.range.to.unwrap() - original.to.unwrap(),
                chrono::Duration::hours(1)
            );
            assert_eq!(
                page.live.range.to.unwrap() - page.live.range.from.unwrap(),
                chrono::Duration::hours(3)
            );
            assert!(read.borrow().revision() > revision);
            page.set_visible(false, cx);
        })
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(120));
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_visible(true, cx);
            assert_eq!(
                page.live.range.to.unwrap() - original.to.unwrap(),
                chrono::Duration::seconds(3720)
            );
        })
    });
}

#[gpui_kit::test]
fn page_time_picker_preserves_every_range_and_set_range_path(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    for (ix, hours) in [1, 3, 24, 168].into_iter().enumerate() {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("obs-time", cx);
            for _ in 0..=ix {
                window.press("down", cx);
            }
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(page.read(cx).hours(), hours));
    }
}
