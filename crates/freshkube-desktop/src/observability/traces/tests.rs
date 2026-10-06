use super::Traces;
use freshkube_core::coroot as api;

#[test]
fn span_times_read_in_local_time() {
    let span: api::Span = serde_json::from_value(serde_json::json!({
        "service":"api","trace_id":"t1","id":"s1","parent_id":"","name":"GET /cart",
        "timestamp":1_789_999_000_000_i64,"duration":42.,"status":{"error":true,"message":"HTTP 503"},
    }))
    .unwrap();
    let mut traces = Traces::default();
    let opened = traces.prepare_list(&api::Tracing {
        spans: vec![span],
        ..Default::default()
    });
    assert_eq!(opened.as_deref(), Some("t1"));
    let local = chrono::DateTime::from_timestamp_millis(1_789_999_000_000)
        .unwrap()
        .with_timezone(&chrono::Local)
        .format("%H:%M:%S")
        .to_string();
    assert_eq!(traces.rows[0].started.to_string(), local);
    assert_eq!(traces.rows[0].id.to_string(), "obs-live-span-t1-s1");
    assert!(traces.rows[0].label.contains("failed: HTTP 503"));
}

#[test]
fn the_heatmap_cursor_stays_inside_a_rebuilt_grid() {
    let tracing = |points: usize, rows: &[&str]| api::Tracing {
        heatmap: Some(api::Heatmap {
            from_ms: 0,
            to_ms: points as i64 * 60_000,
            step_ms: 60_000,
            rows: rows
                .iter()
                .map(|name| api::HeatRow {
                    name: name.to_string(),
                    value: if *name == "errors" { "err" } else { "0.5" }.into(),
                    points: vec![Some(1.); points],
                })
                .collect(),
        }),
        ..Default::default()
    };
    let mut traces = Traces::default();
    traces.prepare_list(&tracing(40, &["5ms", "500ms", "errors"]));
    traces.cursor = Some((2, 39));
    // A refresh with a shorter window and fewer rows keeps the cursor inside it.
    traces.prepare_list(&tracing(30, &["5ms", "errors"]));
    assert_eq!(traces.cursor, Some((1, 29)));
    // A new window keeps the cursor until its heatmap arrives.
    traces.reset();
    assert_eq!(traces.cursor, Some((1, 29)));
    traces.prepare_list(&tracing(10, &["5ms", "errors"]));
    assert_eq!(traces.cursor, Some((1, 9)));
    // No heatmap, no cursor.
    traces.prepare_list(&api::Tracing::default());
    assert_eq!(traces.cursor, None);
}

mod ui_tests {
    use crate::observability::{Destination, ObservabilityPage, tests::mount_size};
    use freshkube_core::coroot::TraceSelection;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Entity, SharedString, TestAppContext};

    fn open_example(
        cx: &mut TestAppContext,
        width: f32,
    ) -> (
        tokio::runtime::Runtime,
        gpui_kit::AnyWindowHandle,
        Entity<ObservabilityPage>,
    ) {
        let (runtime, handle, page) = mount_size(cx, true, width, 900.);
        cx.update(|cx| page.update(cx, |page, cx| page.open(Destination::Traces, cx)));
        cx.run_until_parked();
        (runtime, handle, page)
    }

    /// The id of the request the table draws on `line`.
    fn row(page: &Entity<ObservabilityPage>, line: usize, cx: &gpui_kit::App) -> SharedString {
        let traces = &page.read(cx).live_traces;
        traces.rows[traces.shown[line]].id.clone()
    }

    #[gpui_kit::test]
    fn heatmap_selection_and_error_filters_change_the_trace_view(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = open_example(cx, 1260.);
        // Example ids name the request: x0–x2 failed, x3 healthy.
        let trace = |cx: &mut gpui_kit::App| page.read(cx).live_traces.trace.clone();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // Example traces answer through the live list, which opens the
            // first failed request.
            assert!(window.find("obs-live-heatmap").visible());
            assert!(window.find(row(&page, 0, cx)).visible());
            assert!(window.find("obs-live-waterfall").visible());
            assert!(trace(cx).unwrap().starts_with("x0"));

            // Failures begin two hours before the window's end.
            window.click("obs-live-bucket-0-0", cx);
            window.render_frame(cx);
            assert!(window.find("obs-traces-empty").visible());
            assert!(trace(cx).is_none());
            window.click("obs-live-bucket-0-35", cx);
            window.render_frame(cx);
            assert!(matches!(
                page.read(cx).live_traces.selection,
                TraceSelection::Errors { .. }
            ));
            assert!(!trace(cx).unwrap().starts_with("x3"));

            // Slowest first: >5s, 5s, 2.5s, 1s, 500ms. The empty rows list nothing.
            window.click("obs-live-bucket-2-10", cx);
            window.render_frame(cx);
            assert!(trace(cx).is_none());
            window.click("obs-live-bucket-5-10", cx);
            window.render_frame(cx);
            let TraceSelection::Latency { above, up_to, .. } =
                page.read(cx).live_traces.selection.clone()
            else {
                panic!("a latency cell");
            };
            assert_eq!((above.as_str(), up_to.as_str()), ("0.25", "0.5"));
            assert!(trace(cx).unwrap().starts_with("x3"));
            let first = trace(cx);
            window.click(row(&page, 1, cx), cx);
            window.render_frame(cx);
            assert_ne!(trace(cx), first);
            assert!(window.find("obs-live-trace-span-1").visible());

            window.click("obs-trace-failed", cx);
            assert!(!trace(cx).unwrap().starts_with("x3"));
            window.click("obs-trace-all", cx);
            assert_eq!(page.read(cx).live_traces.selection, TraceSelection::Recent);
            window.click("obs-trace-source-agent", cx);
            assert_eq!(page.read(cx).live_traces.source, "agent");
            window.render_frame(cx);
            assert!(window.find(row(&page, 0, cx)).visible());
            // eBPF isn't the source Coroot chose; OpenTelemetry, chosen
            // again, is.
            assert!(page.read(cx).trace_source_changed());
            window.click("obs-trace-source-otel", cx);
            assert!(!page.read(cx).trace_source_changed());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn traces_use_the_pods_frame_and_table_at_both_text_sizes(cx: &mut TestAppContext) {
        use crate::desktop::layout_check::{
            PageFrame, Table, assert_bare, assert_edge_frame, assert_inspector, assert_table,
        };
        let (_runtime, handle, _page) = open_example(cx, 1260.);
        let frame = PageFrame {
            page: "obs-frame",
            title: "obs-title",
            title_text: "Traces",
            content: "obs-traces-split",
        };
        let table = Table {
            table: Some("obs-traces-table-scroll"),
            list: "obs-traces-list",
        };
        for text_size in [crate::ui::BASE_TEXT, 20.] {
            cx.update_window(handle, |_, _, cx| crate::text_size::set(text_size, cx))
                .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                assert_edge_frame(window, cx, &frame);
                let rows = assert_table(window, cx, &table);
                assert!(rows.header.is_some(), "{rows:#?}");
                assert_bare(window, "obs-traces-table");
                assert_inspector(
                    window,
                    cx,
                    "obs-traces-split",
                    "obs-traces-table",
                    "obs-trace-detail",
                    "obs-trace-title",
                );
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn example_traces_select_by_key_filter_and_clear(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = open_example(cx, 1260.);
        let selected = |cx: &mut gpui_kit::App| {
            let traces = &page.read(cx).live_traces;
            (traces.selected.clone(), traces.trace.clone())
        };
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let table = window.find("obs-traces-table").bounds();
            let detail = window.find("obs-trace-detail").bounds();
            assert!(
                detail.left() >= table.right(),
                "a wide page puts the trace beside"
            );
            assert!(window.find("obs-trace-title").visible());

            let second = row(&page, 1, cx);
            window.click(second.clone(), cx);
            let (key, trace) = selected(cx);
            let Some(crate::observability::tables::TableKey::Span(trace_id, _)) = key.clone()
            else {
                panic!("a span key");
            };
            assert_eq!(trace, Some(trace_id));
            page.update(cx, |page, cx| page.refresh(cx));
            assert_eq!(
                selected(cx),
                (key.clone(), trace.clone()),
                "a new answer keeps the selection by key"
            );

            let width = page.read(cx).live_traces.width;
            window.click("obs-columns", cx);
            window.press("down", cx);
            window.press("enter", cx);
            assert!(page.read(cx).live_traces.width < width);

            window.click("obs-filter", cx);
            window.input("auth", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let traces = &page.read(cx).live_traces;
            assert!(!traces.shown.is_empty() && traces.shown.len() < traces.rows.len());
            assert!(
                window
                    .within("obs-traces-footer")
                    .find("obs-traces-showing")
                    .visible()
            );
            window.click("obs-traces-show-all", cx);
            window.render_frame(cx);
            let traces = &page.read(cx).live_traces;
            assert_eq!(traces.shown.len(), traces.rows.len());
            assert!(window.try_find("obs-traces-showing").is_none());
            window.click("obs-filter", cx);
            window.input("no-such-request", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("obs-traces-empty").visible());
            window.click("obs-traces-clear", cx);
            let traces = &page.read(cx).live_traces;
            assert_eq!(traces.shown.len(), traces.rows.len());
            assert!(traces.query.is_empty());
        })
        .unwrap();
    }

    /// The heatmap leaves a short window no room to fill, so the frame
    /// scrolls and the split keeps a height of its own.
    #[gpui_kit::test]
    fn a_short_window_scrolls_to_a_split_of_its_own_height(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = mount_size(cx, true, 1260., 500.);
        cx.update(|cx| page.update(cx, |page, cx| page.open(Destination::Traces, cx)));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let split = window.find("obs-traces-split").bounds();
            let detail = window.find("obs-trace-detail").bounds();
            let height = crate::ui::dp_px(freshkube_ui::inspector::SHORT_HEIGHT, window);
            assert!(
                (split.size.height - height).abs() <= gpui_kit::px(1.),
                "{split:?}"
            );
            assert!((detail.bottom() - split.bottom()).abs() <= gpui_kit::px(1.));
            assert!(split.top() >= window.find("obs-traces-evidence").bounds().bottom());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_narrow_page_puts_the_trace_below_the_requests(cx: &mut TestAppContext) {
        let (_runtime, handle, _page) = open_example(cx, 760.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let table = window.find("obs-traces-table").bounds();
            let detail = window.find("obs-trace-detail").bounds();
            assert!(detail.top() >= table.bottom());
            assert!(detail.right() <= window.viewport_size().width);
        })
        .unwrap();
    }
}
