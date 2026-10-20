use super::*;

#[gpui_kit::test]
fn every_kind_in_the_cluster_dashboard_draws_from_example_data(cx: &mut TestAppContext) {
    let (dashboard, specs) = cluster();
    let (handle, panels) = mount(cx, specs.clone());
    for index in 0..specs.len() {
        assert!(shown(
            cx,
            handle,
            &format!("monitoring-panel-{index}-title")
        ));
        assert!(!shown(
            cx,
            handle,
            &format!("monitoring-panel-{index}-failed")
        ));
    }
    answer(cx, &dashboard, &panels);
    let cpu = index_of(&specs, "CPU usage by node");
    assert!(shown(cx, handle, &format!("monitoring-panel-{cpu}-plot")));
    assert!(shown(
        cx,
        handle,
        &format!("monitoring-panel-{cpu}-legend-0")
    ));
    let pods = index_of(&specs, "Top pods by CPU");
    assert!(shown(cx, handle, &format!("monitoring-panel-{pods}-bars")));
    let alerts = index_of(&specs, "Firing alerts");
    assert!(shown(
        cx,
        handle,
        &format!("monitoring-panel-{alerts}-table-list")
    ));
    for (index, panel) in panels.iter().enumerate() {
        cx.read(|cx| {
            let panel = panel.read(cx);
            assert_eq!(panel.state, State::Ready, "{index}");
            assert!(
                !matches!(
                    panel.data.as_ref().map(|data| &data.body),
                    Some(Body::NoData | Body::NotDrawn(_))
                ),
                "{}",
                panel.spec.title
            );
        });
    }
    // The latency panel gives its unit to the title.
    let latency = index_of(&specs, "API server latency");
    cx.read(|cx| {
        let data = panels[latency].read(cx).data.clone().unwrap();
        assert_eq!(data.unit.as_deref(), Some("ms"));
    });
}

#[gpui_kit::test]
fn a_failure_shows_in_the_card_until_an_answer_then_marks_it_stale(cx: &mut TestAppContext) {
    let (dashboard, specs) = cluster();
    let cpu = index_of(&specs, "CPU usage by node");
    let (handle, panels) = mount(cx, specs);
    let failed = format!("monitoring-panel-{cpu}-failed");
    let stale = format!("monitoring-panel-{cpu}-stale");
    let plot: gpui_kit::SharedString = format!("monitoring-panel-{cpu}-plot").into();
    let error = QueryError::refused();
    cx.update(|cx| panels[cpu].update(cx, |panel, cx| panel.set_error(&error, cx)));
    assert!(shown(cx, handle, &failed));
    assert!(!shown(cx, handle, &plot));

    answer(cx, &dashboard, &panels);
    assert!(!shown(cx, handle, &failed));
    assert!(shown(cx, handle, &plot));

    // A later failure keeps the chart and marks it.
    cx.update(|cx| panels[cpu].update(cx, |panel, cx| panel.set_error(&error, cx)));
    assert!(shown(cx, handle, &plot));
    assert!(shown(cx, handle, &stale));
    assert!(!shown(cx, handle, &failed));
}

#[gpui_kit::test]
fn a_table_panel_waits_under_its_header_and_loading_rows(cx: &mut TestAppContext) {
    use freshkube_ui::table::TableColumn;
    let (dashboard, specs) = cluster();
    let alerts = index_of(&specs, "Firing alerts");
    let cpu = index_of(&specs, "CPU usage by node");
    let (handle, panels) = mount(cx, specs);
    let loading = format!("monitoring-panel-{alerts}-table-loading");
    assert!(shown(cx, handle, &loading));
    // The organize transform names the columns, in its order, after the
    // severity's glyph.
    let table = cx.read(|cx| panels[alerts].read(cx).table()).unwrap();
    let labels: Vec<String> = cx.read(|cx| {
        table
            .read(cx)
            .columns()
            .iter()
            .map(|column| column.label().to_string())
            .collect()
    });
    assert_eq!(labels, ["", "Severity", "Alert", "Namespace"]);
    assert!(cx.read(|cx| panels[alerts].read(cx).loading_motion(cx).is_some()));
    // Only a table waits on rows: a chart keeps its skeleton, and no motion.
    assert!(!shown(
        cx,
        handle,
        &format!("monitoring-panel-{cpu}-table-loading")
    ));
    assert!(cx.read(|cx| panels[cpu].read(cx).loading_motion(cx).is_none()));

    // The answer takes the same table, with its rows and no motion.
    answer(cx, &dashboard, &panels);
    assert!(!shown(cx, handle, &loading));
    assert!(shown(
        cx,
        handle,
        &format!("monitoring-panel-{alerts}-table-list")
    ));
    assert_eq!(
        cx.read(|cx| panels[alerts].read(cx).table()).unwrap(),
        table
    );
    assert!(cx.read(|cx| panels[alerts].read(cx).loading_motion(cx).is_none()));
}

#[gpui_kit::test]
fn a_failed_first_read_replaces_a_tables_loading_rows(cx: &mut TestAppContext) {
    let (_, specs) = cluster();
    let alerts = index_of(&specs, "Firing alerts");
    let (handle, panels) = mount(cx, specs);
    let error = QueryError::refused();
    cx.update(|cx| panels[alerts].update(cx, |panel, cx| panel.set_error(&error, cx)));
    assert!(shown(
        cx,
        handle,
        &format!("monitoring-panel-{alerts}-failed")
    ));
    assert!(!shown(
        cx,
        handle,
        &format!("monitoring-panel-{alerts}-table-loading")
    ));
    assert!(cx.read(|cx| panels[alerts].read(cx).loading_motion(cx).is_none()));
}
