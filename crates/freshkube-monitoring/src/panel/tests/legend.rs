use super::*;

#[gpui_kit::test]
fn the_legend_fades_other_series_on_hover_and_a_click_keeps_one(cx: &mut TestAppContext) {
    let (dashboard, specs) = cluster();
    let cpu = index_of(&specs, "CPU usage by node");
    let (handle, panels) = mount(cx, specs);
    answer(cx, &dashboard, &panels);
    frame(cx, handle);
    let focus = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            let panel = panels[cpu].read(cx);
            let plot = panel.plot.as_ref().unwrap().read(cx).focus;
            (panel.focus(), plot)
        })
    };
    let paths = probe::count("monitoring-path");
    let row: gpui_kit::SharedString = format!("monitoring-panel-{cpu}-legend-1").into();
    cx.update_window(handle, |_, window, cx| window.hover(row.clone(), cx))
        .unwrap();
    assert_eq!(focus(cx), (Some(1), Some(1)));

    let title: gpui_kit::SharedString = format!("monitoring-panel-{cpu}-title").into();
    cx.update_window(handle, |_, window, cx| window.hover(title.clone(), cx))
        .unwrap();
    assert_eq!(focus(cx), (None, None));

    cx.update_window(handle, |_, window, cx| window.click(row.clone(), cx))
        .unwrap();
    cx.update_window(handle, |_, window, cx| window.hover(title.clone(), cx))
        .unwrap();
    assert_eq!(focus(cx), (Some(1), Some(1)));

    // Fading draws the same shapes in another opacity; only the focused
    // series is built again, its line and area alone, to draw on top.
    assert!(probe::count("monitoring-path") <= paths + 2);

    // Clicking it again lets go.
    cx.update_window(handle, |_, window, cx| window.click(row.clone(), cx))
        .unwrap();
    cx.update_window(handle, |_, window, cx| window.hover(title.clone(), cx))
        .unwrap();
    assert_eq!(focus(cx), (None, None));
}

/// A timeseries of `count` series `width` wide, its legend a table of last
/// and max, answered at once. The series in `stopped` end before the
/// window does, at the value the others end at.
fn legend_panel(
    cx: &mut TestAppContext,
    count: usize,
    width: f32,
    stopped: &[usize],
) -> (AnyWindowHandle, Entity<PanelView>) {
    let names: Vec<String> = (0..count).map(|n| format!("pod-{n}")).collect();
    named_panel(cx, &names, width, 320., stopped)
}

/// A timeseries panel `width` by `height` answered with a series per name.
fn named_panel(
    cx: &mut TestAppContext,
    names: &[String],
    width: f32,
    height: f32,
    stopped: &[usize],
) -> (AnyWindowHandle, Entity<PanelView>) {
    use freshkube_core::monitoring::{
        PanelResult,
        model::data::{Frame, Series},
    };
    let dashboard = serde_json::json!({"title": "Test", "panels": [{
        "type": "timeseries",
        "title": "Memory",
        "gridPos": {"x": 0, "y": 0, "w": 12, "h": 8},
        "targets": [{"refId": "A", "expr": "memory", "legendFormat": "{{pod}}"}],
    }]});
    let spec = Dashboard::parse(&dashboard.to_string())
        .unwrap()
        .panels
        .remove(0);
    let (handle, panels) = mount_sized(cx, vec![Rc::new(spec)], width, height);
    let times: Vec<f64> = window_range()
        .times()
        .into_iter()
        .map(|t| t as f64)
        .collect();
    let series = names
        .iter()
        .enumerate()
        .map(|(n, name)| {
            let mut values = vec![5.; times.len()];
            if stopped.contains(&n) {
                let end = values.len();
                values[end - 3..].fill(f64::NAN);
            }
            Series {
                name: name.clone(),
                query: "A".into(),
                field: None,
                labels: vec![("pod".into(), name.clone())],
                values,
            }
        })
        .collect();
    let panel = panels[0].clone();
    cx.update(|cx| {
        panel.update(cx, |panel, cx| {
            let result = PanelResult {
                frame: Frame { times, series },
                warnings: Vec::new(),
                expressions: Vec::new(),
            };
            panel.set_result(result, window_range(), cx);
        })
    });
    (handle, panel)
}

/// One wheel event at `position`, as the start of a gesture.
fn wheel(
    window: &mut Window,
    position: gpui_kit::Point<gpui_kit::Pixels>,
    y: f32,
    cx: &mut gpui_kit::App,
) {
    use gpui_kit::{InputEvent, MouseMoveEvent, ScrollDelta, ScrollWheelEvent, TouchPhase};
    window.dispatch_event(
        MouseMoveEvent {
            position,
            ..Default::default()
        }
        .to_platform_input(),
        cx,
    );
    window.dispatch_event(
        ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.), px(y))),
            touch_phase: TouchPhase::Started,
            ..Default::default()
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

fn bounds_of(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    ids: &[String],
) -> Vec<gpui_kit::Bounds<gpui_kit::Pixels>> {
    let ids = ids.to_vec();
    cx.update_window(handle, move |_, window, cx| {
        window.render_frame(cx);
        ids.into_iter()
            .map(|id| window.find(gpui_kit::SharedString::from(id)).bounds())
            .collect()
    })
    .unwrap()
}

fn legend_rows(count: usize) -> Vec<String> {
    (0..count)
        .map(|n| format!("monitoring-panel-0-legend-{n}"))
        .collect()
}

/// Eleven series: the eleventh row takes half the width, as the ten above
/// it do, not the whole line.
#[gpui_kit::test]
fn an_odd_last_legend_row_keeps_half_the_width(cx: &mut TestAppContext) {
    let (handle, _panel) = legend_panel(cx, 11, 640., &[]);
    let rows = bounds_of(cx, handle, &legend_rows(11));
    let (first, second, last) = (rows[0], rows[1], rows[10]);
    assert!(second.left() > first.right(), "two columns: {rows:?}");
    assert_eq!(second.top(), first.top());
    for row in &rows {
        assert!(
            (row.size.width - first.size.width).abs() < px(0.5),
            "{rows:?}"
        );
    }
    assert_eq!(last.left(), first.left());
}

/// Where two rows don't fit, each takes the line, the odd last one too.
/// Seven series, so the short names still take the table.
#[gpui_kit::test]
fn a_narrow_legend_takes_one_column(cx: &mut TestAppContext) {
    let (handle, _panel) = legend_panel(cx, 7, 300., &[]);
    let rows = bounds_of(cx, handle, &legend_rows(7));
    for pair in rows.windows(2) {
        assert_eq!(pair[1].left(), pair[0].left());
        assert!(pair[1].top() > pair[0].top(), "{rows:?}");
        assert_eq!(pair[1].size.width, pair[0].size.width);
    }
    // Nothing but the padding beside the rows.
    let card = bounds_of(cx, handle, &["monitoring-panel-0".into()])[0];
    assert!(
        rows[0].size.width > card.size.width - px(40.),
        "{rows:?} in {card:?}"
    );
}

/// A legend that scrolls keeps its bottom padding: its rows end above the
/// card's edge, not at it.
#[gpui_kit::test]
fn a_long_legend_stops_short_of_the_cards_edge(cx: &mut TestAppContext) {
    let (handle, _panel) = legend_panel(cx, 30, 640., &[]);
    let found = bounds_of(
        cx,
        handle,
        &[
            "monitoring-panel-0".into(),
            "monitoring-panel-0-legend".into(),
        ],
    );
    let (card, legend) = (found[0], found[1]);
    assert!(
        card.bottom() - legend.bottom() >= px(10.),
        "{legend:?} in {card:?}"
    );
}

/// Six 16-character names on a narrow, short panel list inline one a line,
/// more than half the panel: the legend stops at half, above the card's
/// padding, and scrolls to its last line.
#[gpui_kit::test]
fn a_narrow_inline_legend_scrolls_to_its_last_line(cx: &mut TestAppContext) {
    let names: Vec<String> = (0..6).map(|n| format!("worker-pool-ab-{n}")).collect();
    assert_eq!(names[0].chars().count(), 16);
    let (handle, panel) = named_panel(cx, &names, 250., 200., &[]);
    cx.update(|cx| {
        assert_eq!(
            panel.read(cx).chart().unwrap().legend.mode,
            crate::derive::LegendMode::Inline
        )
    });
    let ids = [
        "monitoring-panel-0".to_string(),
        "monitoring-panel-0-plot".into(),
        "monitoring-panel-0-legend".into(),
        "monitoring-panel-0-legend-5".into(),
    ];
    let found = bounds_of(cx, handle, &ids);
    let (card, plot, legend, last) = (found[0], found[1], found[2], found[3]);
    assert!(
        plot.size.height >= legend.size.height,
        "{plot:?} {legend:?}"
    );
    assert!(
        card.bottom() - legend.bottom() >= px(10.),
        "{legend:?} in {card:?}"
    );
    assert!(
        last.bottom() > legend.bottom(),
        "fits unscrolled: {last:?} {legend:?}"
    );
    cx.update_window(handle, |_, window, cx| {
        wheel(window, legend.center(), -400., cx)
    })
    .unwrap();
    let last = bounds_of(cx, handle, &ids[3..])[0];
    assert!(
        last.bottom() <= legend.bottom(),
        "{last:?} below {legend:?}"
    );
    assert!(last.top() >= legend.top(), "{last:?} above {legend:?}");
}

/// A series that stopped early shows its last value muted, with the time in
/// the row's tooltip: its value takes no more room than any other. Seven
/// series, so the legend is a table.
#[gpui_kit::test]
fn a_stopped_series_keeps_its_value_as_wide_as_the_others(cx: &mut TestAppContext) {
    let (handle, panel) = legend_panel(cx, 7, 640., &[0]);
    let stale = cx.read(|cx| {
        let chart = panel.read(cx).chart().unwrap();
        (
            chart.legend.rows[0].stale.clone(),
            chart.legend.rows[1].stale.clone(),
        )
    });
    assert!(stale.0.unwrap().starts_with("Last value at "));
    assert_eq!(stale.1, None);
    let values = bounds_of(
        cx,
        handle,
        &[
            "monitoring-panel-0-legend-0-last".into(),
            "monitoring-panel-0-legend-1-last".into(),
        ],
    );
    assert_eq!(values[0].size.width, values[1].size.width, "{values:?}");
}
