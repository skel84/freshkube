use super::{Destination, MatrixRow, ObservabilityPage, Report, example};
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, px, size};
fn mount(cx: &mut TestAppContext, fixture: bool) -> (AnyWindowHandle, Entity<ObservabilityPage>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.set_reduce_motion(true);
    });
    let mut page = None;
    let handle = cx.open_window(size(px(1260.), px(900.)), |window, cx| {
        let view = cx.new(|cx| ObservabilityPage::new(fixture, window, cx));
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (handle.into(), page.unwrap())
}
#[gpui_kit::test]
fn applications_filter_and_cells_open_the_selected_report(cx: &mut TestAppContext) {
    let (handle, page) = mount(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-filter-all", cx);
        assert_eq!(page.read(cx).counts[1], 47);
        window.click("obs-filter", cx);
        window.input("worker", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            page.read(cx)
                .matrix
                .iter()
                .filter(|r| matches!(r, MatrixRow::App(_)))
                .count(),
            1
        );
        window.click("obs-check-payments/worker-net", cx);
        assert_eq!(page.read(cx).destination, Destination::Application);
        assert_eq!(page.read(cx).report, Report::Net);
        window.click("obs-report-memory", cx);
        assert_eq!(page.read(cx).report, Report::Memory);
    })
    .unwrap();
}
#[gpui_kit::test]
fn live_mode_never_contains_example_applications(cx: &mut TestAppContext) {
    let (handle, page) = mount(cx, false);
    for destination in [
        Destination::Applications,
        Destination::ServiceMap,
        Destination::Application,
        Destination::Incidents,
        Destination::Deployments,
        Destination::Profiling,
        Destination::Traces,
    ] {
        cx.update(|cx| page.update(cx, |page, cx| page.open(destination, cx)));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("obs-integration-required").visible());
            assert!(window.try_find("obs-matrix").is_none());
            assert!(page.read(cx).applications.is_empty());
        })
        .unwrap();
    }
}
#[gpui_kit::test]
fn map_selection_release_comparison_and_profile_zoom_are_retained(cx: &mut TestAppContext) {
    let (handle, page) = mount(cx, true);
    cx.update(|cx| page.update(cx, |page, cx| page.open(Destination::ServiceMap, cx)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-map-link-4", cx);
        assert_eq!(page.read(cx).selected_link, 4);
        window.click("obs-map-problems", cx);
        window.render_frame(cx);
        assert!(window.try_find("obs-map-edge-0").is_none());
        assert!(window.try_find("obs-map-link-0").is_none());
        window.click("obs-map-compare", cx);
        assert_eq!(page.read(cx).destination, Destination::Deployments);
        window.click("obs-release-marker-1.8.1", cx);
        assert_eq!(page.read(cx).release, 1);
        window.click("obs-release-compare", cx);
        assert_eq!(page.read(cx).comparison, 2);
        page.update(cx, |page, cx| page.open(Destination::Profiling, cx));
        window.render_frame(cx);
        window.click("obs-flame-3", cx);
        assert_eq!(page.read(cx).frame_root, Some(3));
        assert!(page.read(cx).visible_frames.len() < page.read(cx).frames.len());
        window.click("obs-profile-reset", cx);
        assert!(page.read(cx).frame_root.is_none());
        window.click("obs-profile-compare", cx);
        assert_eq!(page.read(cx).charts[2].series.len(), 1);
    })
    .unwrap();
}
#[gpui_kit::test]
fn heatmap_selection_and_error_filters_change_the_trace_view(cx: &mut TestAppContext) {
    let (handle, page) = mount(cx, true);
    cx.update(|cx| page.update(cx, |page, cx| page.open(Destination::Traces, cx)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-bucket-0-18", cx);
        assert_eq!(page.read(cx).bucket, Some((0, 18)));
        window.press("right", cx);
        assert_eq!(page.read(cx).bucket, Some((0, 19)));
        window.click("obs-trace-errors", cx);
        assert!(page.read(cx).trace_errors_only);
        window.render_frame(cx);
        assert!(window.try_find("obs-span-0").is_none());
        window.click("obs-trace-clear", cx);
        assert!(page.read(cx).bucket.is_none());
        assert!(!page.read(cx).trace_errors_only);
    })
    .unwrap();
}

#[gpui_kit::test]
fn threshold_edits_validate_and_remain_local(cx: &mut TestAppContext) {
    let (handle, page) = mount(cx, true);
    cx.update(|cx| page.update(cx, |page, cx| page.open_app(1, Report::Memory, cx)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-threshold", cx);
        window.render_frame(cx);
        window.click("obs-threshold-input", cx);
        window.press("secondary-a", cx);
        window.input("-1", cx);
        window.click("ok", cx);
        window.render_frame(cx);
        assert!(page.read(cx).thresholds.is_empty());
        assert!(window.find("obs-threshold-input").visible());
        window.click("obs-threshold-input", cx);
        window.press("secondary-a", cx);
        window.input("90", cx);
        window.click("ok", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx)
                .thresholds
                .get(&(example::WORKER.into(), Report::Memory))
                .map(String::as_str),
            Some("90")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn rollback_preview_does_not_change_a_release(cx: &mut TestAppContext) {
    let (handle, page) = mount(cx, true);
    cx.update(|cx| page.update(cx, |page, cx| page.open_app(1, Report::Memory, cx)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-app-rollback", cx);
        window.render_frame(cx);
        assert_eq!(page.read(cx).release, 0);
        window.click("ok", cx);
        assert_eq!(page.read(cx).release, 0);
    })
    .unwrap();
}

#[gpui_kit::test]
fn ranges_reports_and_trace_causes_have_independent_data(cx: &mut TestAppContext) {
    let (handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let prior = page.charts[0].series[0].values.clone();
            page.set_range(1, cx);
            assert_ne!(prior, page.charts[0].series[0].values);
            page.open_app(0, Report::Latency, cx);
            assert_eq!(page.applications[page.selected_app].key, "payments/api");
            page.open(Destination::Traces, cx);
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-error-cause-2", cx);
        assert_eq!(page.read(cx).trace_error, 2);
        assert_eq!(page.read(cx).trace_snapshot.spans[1].0, "oauth2-proxy");
        window.click("obs-bucket-7-18", cx);
        assert_eq!(page.read(cx).trace_error, 3);
        assert!(
            page.read(cx)
                .trace_snapshot
                .spans
                .iter()
                .all(|span| !span.4)
        );
    })
    .unwrap();
}
