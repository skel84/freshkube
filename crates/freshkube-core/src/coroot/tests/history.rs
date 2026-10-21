use super::*;

#[test]
fn every_chart_takes_its_history_from_the_answer_its_layout_came_from() {
    let view = decode_wire(app_view_wire()).unwrap();
    assert_eq!(view.history_error, None);
    let WidgetKind::ChartGroup { charts, .. } = &view.reports[1].widgets[0].kind else {
        panic!("the CPU group");
    };
    let history = charts[0].history.as_ref().unwrap();
    assert_eq!(history.series[0].samples, [Some(0.1), None, Some(0.2)]);
    // Three samples of a 61-point window: Coroot covered only its start.
    assert_eq!(history.series[0].coverage, SeriesCoverage::Partial);
    assert_eq!(history.expected_points(), 61);
    assert_eq!(
        history.group.as_deref(),
        Some("CPU usage <selector>, cores")
    );
    // The group's second chart and the report's own chart keep their places.
    assert!(charts[1].history.as_ref().unwrap().series.is_empty());
    let WidgetKind::Chart(node) = &view.reports[1].widgets[1].kind else {
        panic!("the node chart");
    };
    assert_eq!(node.history.as_ref().unwrap().title, "Node CPU usage, %");
    // No Deployments report: Coroot knows no deployment, which is an answer.
    assert_eq!(view.revisions, Some(Ok(vec![])));
}

#[test]
fn a_history_coroot_rs_refuses_leaves_the_page_and_says_why() {
    let mut wire = app_view_wire();
    // Longer than its window: the layout takes it, the history doesn't.
    wire["data"]["reports"][1]["widgets"][1]["chart"]["series"][0]["data"] = vec![1.0; 62].into();
    let view = decode_wire(wire).unwrap();
    assert_eq!(view.history_error, Some(ReadError::InvalidResponse));
    let WidgetKind::Chart(node) = &view.reports[1].widgets[1].kind else {
        panic!("the node chart");
    };
    assert!(node.history.is_none());
    assert_eq!(view.reports.len(), 7);
}

#[test]
fn a_window_over_the_point_bound_keeps_no_history() {
    let mut wire = app_view_wire();
    // One sample, but a `ctx` from the epoch to now by the second: about
    // 1.8e9 points, which a panel would lay out in full.
    wire["data"]["reports"][1]["widgets"][1]["chart"]["ctx"] =
        serde_json::json!({"from":1,"to":1790000000000_i64,"step":1000});
    wire["data"]["reports"][1]["widgets"][1]["chart"]["series"][0]["data"] = serde_json::json!([1]);
    let view = decode_wire(wire).unwrap();
    assert_eq!(view.history_error, Some(ReadError::Limit));
    let WidgetKind::Chart(node) = &view.reports[1].widgets[1].kind else {
        panic!("the node chart");
    };
    assert!(node.history.is_none());
    assert!(ChartPanel::from_history(node).is_none());
    // The group beside it keeps its histories.
    let WidgetKind::ChartGroup { charts, .. } = &view.reports[1].widgets[0].kind else {
        panic!("the CPU group");
    };
    assert!(charts.iter().all(|c| c.history.is_some()));
}

#[test]
fn a_bound_coroot_rs_holds_reads_as_a_limit() {
    // The layout's bounds match coroot-rs's chart bounds, so it is the
    // revisions' count, which only coroot-rs holds, that shows it.
    let rows: Vec<_> = (0..=coroot_rs::DEFAULT_MAX_REVISIONS as i64)
        .map(|i| {
            deployment_row(
                &format!("9c41e7:{}", 1_789_990_000 + i),
                "9c41e7",
                serde_json::json!([{"report":"SLO","ok":true,"message":"ok","time":null}]),
            )
        })
        .collect();
    let mut wire = app_view_wire();
    wire["data"]["reports"]
        .as_array_mut()
        .unwrap()
        .push(deployments_report(rows.into()));
    let view = decode_wire(wire).unwrap();
    assert_eq!(view.revisions, Some(Err(ReadError::Limit)));
    assert_eq!(view.history_error, None);
}
