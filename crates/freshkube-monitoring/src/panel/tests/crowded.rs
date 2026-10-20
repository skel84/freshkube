use super::*;

/// Thirty of forty series past the two colours: the grey lines draw as one
/// path and the grey areas as another, not one of each per series.
#[gpui_kit::test]
fn a_crowded_chart_draws_lines_of_one_look_as_one_path(cx: &mut TestAppContext) {
    let (handle, panels, result) = crowded(cx);
    let before = probe::count("monitoring-path");
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.set_result(result, window_range(), cx)));
    frame(cx, handle);
    let series = cx.read(|cx| panels[0].read(cx).chart().unwrap().series.len());
    assert_eq!(series, derive::MOST_SERIES);
    // Two coloured series and the grey rest, each a line and an area.
    assert_eq!(probe::count("monitoring-path") - before, 3 * 2);
}

/// A group of grey lines past GPUI's 65,536 vertices a path still draws:
/// it is built in chunks joined into one path, where it once failed to
/// build and drew nothing.
#[gpui_kit::test]
fn a_group_past_the_vertex_limit_still_draws(cx: &mut TestAppContext) {
    use freshkube_core::monitoring::model::data::{Frame, Series};
    let dashboard = Dashboard::parse(
        r#"{
          "title": "Dense",
          "panels": [{
            "type": "timeseries", "title": "Dense",
            "gridPos": {"x": 0, "y": 0, "w": 24, "h": 8},
            "targets": [{"refId": "A", "expr": "go_goroutines", "legendFormat": "{{pod}}"}]
          }]
        }"#,
    )
    .unwrap();
    let specs: Vec<Rc<PanelSpec>> = dashboard.panels.iter().cloned().map(Rc::new).collect();
    let (handle, panels) = mount(cx, specs);
    // The 600 samples a panel asks for by default.
    let times: Vec<f64> = (0..600).map(|n| (END - 6 * 3600 + n * 36) as f64).collect();
    // Seventy jagged series: the 68 grey ones pass the limit as one path.
    let series = (0..70)
        .map(|pod| Series {
            name: format!("app-{pod}"),
            query: "A".into(),
            field: None,
            labels: vec![("pod".into(), format!("app-{pod}"))],
            values: (0..600).map(|n| ((n * 7 + pod * 13) % 17) as f64).collect(),
        })
        .collect();
    let result = freshkube_core::monitoring::PanelResult {
        frame: Frame { times, series },
        warnings: Vec::new(),
        expressions: Vec::new(),
    };
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.set_result(result, window_range(), cx)));
    frame(cx, handle);
    let before = probe::count("monitoring-path-painted");
    frame(cx, handle);
    // One frame paints the two coloured lines and the grey ones as one.
    assert_eq!(probe::count("monitoring-path-painted") - before, 3);
}

/// Forty series draw the thirty highest until Show all, which draws every
/// one as before, the grey ones still sharing their paths; Top 30 returns.
/// A refresh keeps Show all, and a query that picks its series with topk
/// draws them all with no line to offer.
#[gpui_kit::test]
fn a_crowded_chart_draws_its_highest_peaks_until_show_all(cx: &mut TestAppContext) {
    let (handle, panels, result) = crowded(cx);
    let again = result.clone();
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.set_result(result, window_range(), cx)));
    let drawn = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            let chart = panels[0].read(cx).chart().unwrap();
            (chart.series.len(), chart.capped)
        })
    };
    assert!(shown(cx, handle, "monitoring-panel-0-cap"));
    assert_eq!(drawn(cx), (30, Some(derive::Capped { of: 40, all: false })));
    // The lowest pods are the ones left out.
    let first = cx.read(|cx| panels[0].read(cx).chart().unwrap().series[0].name.clone());
    assert_eq!(first, "app-10");

    let before = probe::count("monitoring-path");
    cx.update_window(handle, |_, window, cx| {
        window.click("monitoring-panel-0-cap-toggle", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(drawn(cx), (40, Some(derive::Capped { of: 40, all: true })));
    assert_eq!(probe::count("monitoring-path") - before, 3 * 2);

    // A refresh answers again and keeps every series.
    cx.update(|cx| {
        panels[0].update(cx, |panel, cx| {
            panel.set_result(again.clone(), window_range(), cx)
        })
    });
    assert_eq!(drawn(cx).0, 40);

    // Top 30.
    cx.update_window(handle, |_, window, cx| {
        window.click("monitoring-panel-0-cap-toggle", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(drawn(cx).0, 30);

    let mut picked = again;
    picked.expressions = vec![("A".into(), "topk(40, go_goroutines)".into())];
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.set_result(picked, window_range(), cx)));
    assert_eq!(drawn(cx), (40, None));
    assert!(!shown(cx, handle, "monitoring-panel-0-cap"));
}

/// Show all lasts until the page asks for something else.
#[gpui_kit::test]
fn asking_again_returns_a_chart_to_its_highest_peaks(cx: &mut TestAppContext) {
    let (handle, panels, result) = crowded(cx);
    let again = result.clone();
    cx.update(|cx| {
        panels[0].update(cx, |panel, cx| {
            panel.set_result(result, window_range(), cx);
            panel.show_all_series(true, cx);
        })
    });
    frame(cx, handle);
    cx.update(|cx| {
        panels[0].update(cx, |panel, cx| {
            panel.cap_again();
            panel.set_result(again, window_range(), cx);
        })
    });
    let drawn = cx.read(|cx| panels[0].read(cx).chart().unwrap().series.len());
    assert_eq!(drawn, 30);
}
