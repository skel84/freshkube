use super::*;

/// A table panel of `count` alerts, one per name, answered at once.
fn alert_table(cx: &mut TestAppContext, count: usize) -> (AnyWindowHandle, Entity<PanelView>) {
    use freshkube_core::monitoring::{
        PanelResult,
        model::data::{Frame, Series},
    };
    let dashboard = serde_json::json!({"title": "Test", "panels": [{
        "type": "table",
        "title": "Alerts",
        "gridPos": {"x": 0, "y": 0, "w": 12, "h": 8},
        "targets": [{"refId": "A", "expr": "ALERTS", "format": "table", "instant": true}],
        "transformations": [{"id": "organize", "options": {"excludeByName": {"Time": true}}}],
    }]});
    let spec = Dashboard::parse(&dashboard.to_string())
        .unwrap()
        .panels
        .remove(0);
    let (handle, panels) = mount(cx, vec![Rc::new(spec)]);
    let times: Vec<f64> = window_range()
        .times()
        .into_iter()
        .map(|t| t as f64)
        .collect();
    let series = (0..count)
        .map(|n| Series {
            name: "ALERTS".into(),
            query: "A".into(),
            field: None,
            labels: [
                ("alertname".into(), format!("alert-{n}")),
                (
                    "severity".into(),
                    if n % 2 == 0 { "critical" } else { "info" }.into(),
                ),
            ]
            .into_iter()
            .collect(),
            values: vec![1.; times.len()],
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

#[gpui_kit::test]
fn a_long_table_shows_its_first_rows_until_show_all(cx: &mut TestAppContext) {
    let (handle, panel) = alert_table(cx, 240);
    let table = cx.read(|cx| panel.read(cx).table()).expect("a table view");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // "Showing 100 of 240" and Show all.
        window.find("monitoring-panel-0-table-showing");
        window.find("monitoring-panel-0-table-show-all");
        assert_eq!(table.read(cx).line_count(), 100);
        // The severity leads each row with its glyph; the first is critical.
        assert!(table.read(cx).leads_with_glyph());
        window.click("monitoring-panel-0-table-show-all", cx);
        window.render_frame(cx);
        assert!(table.read(cx).shows_all());
        assert_eq!(table.read(cx).line_count(), 240);
        assert!(
            window
                .try_find("monitoring-panel-0-table-showing")
                .is_none()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn past_the_cap_the_table_says_how_many_it_left_out(cx: &mut TestAppContext) {
    // derive's MAX_ROWS, which its tests pin at 1,000.
    let max = 1_000;
    let (handle, panel) = alert_table(cx, max + 5);
    let table = cx.read(|cx| panel.read(cx).table()).expect("a table view");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("monitoring-panel-0-table-show-all", cx);
        window.render_frame(cx);
        assert!(table.read(cx).shows_all());
        // Every kept row shows; "Showing 1000 of 1005" stays, without
        // Show all.
        assert_eq!(table.read(cx).line_count(), max);
        window.find("monitoring-panel-0-table-showing");
        assert!(
            window
                .try_find("monitoring-panel-0-table-show-all")
                .is_none()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_new_answer_keeps_the_tables_show_all(cx: &mut TestAppContext) {
    let (handle, panel) = alert_table(cx, 240);
    let table = cx.read(|cx| panel.read(cx).table()).expect("a table view");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("monitoring-panel-0-table-show-all", cx);
    })
    .unwrap();
    // A refresh on the same dashboard answers the same panel again.
    let data = cx.read(|cx| panel.read(cx).data.clone().unwrap());
    let Body::Table(rows) = data.body else {
        panic!("not a table")
    };
    cx.update(|cx| table.update(cx, |table, _| table.set_data(rows)));
    assert!(cx.read(|cx| table.read(cx).shows_all()));
    frame(cx, handle);
}
