use super::*;

#[tokio::test]
async fn app_view_reads_the_map_reports_checks_and_widgets() {
    let server = server(vec![(200, app_view_wire())]).await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let app = AppId::new("c:shop:Deployment:auth");
    let view = provider
        .app_view(&source(&provider), range(), &app)
        .await
        .unwrap();

    let map = &view.map;
    assert_eq!(map.app.id, app);
    assert!(map.app.link.is_none());
    assert_eq!(map.instances.len(), 2);
    assert_eq!(map.instances[1].labels["role"], "primary");
    let client = map.clients[0].link.as_ref().unwrap();
    assert_eq!(client.stats, ["12 rps", "3ms"]);
    assert_eq!(client.weight, Some(12.));
    assert!(!client.both_ways);
    let db = map.dependencies[0].link.as_ref().unwrap();
    assert_eq!(db.status, Status::Critical);
    assert_eq!(db.reason, "failed connections");
    assert!(db.both_ways);
    assert_eq!(db.weight, None);
    assert_eq!(
        map.dependencies[1].link.as_ref().unwrap().status,
        Status::Unknown
    );

    let names: Vec<_> = view.reports.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "SLO",
            "CPU",
            "Instances",
            "Logs",
            "DNS",
            "Profiling",
            "Tracing"
        ]
    );
    assert!(!view.reports[0].judged());

    let cpu = &view.reports[1];
    assert!(cpu.judged());
    assert_eq!(cpu.checks[1].status, Status::Warning);
    assert_eq!(cpu.checks[1].message, "high CPU utilization");
    assert_eq!(
        cpu.checks[1].condition(),
        (
            "the CPU usage of a container > ",
            "80%".into(),
            " of its CPU limit"
        )
    );
    let WidgetKind::ChartGroup { title, charts } = &cpu.widgets[0].kind else {
        panic!("a chart group");
    };
    assert_eq!(title, "CPU usage <selector>, cores");
    assert_eq!(cpu.widgets[0].width, 0.5);
    assert_eq!(charts.len(), 2);
    assert!(charts[0].featured);
    assert_eq!(
        (charts[0].step_ms, charts[0].from_ms),
        (60_000, 1_789_996_400_000)
    );
    // The history holds the samples; the layout keeps no second copy.
    assert!(charts[0].series[0].points.is_empty());
    assert_eq!(
        charts[0].history.as_ref().unwrap().series[0].samples,
        [Some(0.1), None, Some(0.2)]
    );
    assert_eq!(charts[0].threshold.as_ref().unwrap().name, "limit");
    assert_eq!(charts[0].annotations[0].from_ms, 1_789_998_000_000);
    assert!(charts[1].series.is_empty());
    let WidgetKind::Chart(node) = &cpu.widgets[1].kind else {
        panic!("a chart");
    };
    assert!(node.column && node.stacked);
    assert_eq!(node.series[0].color, "red");
    assert!(node.series[0].fill);
    assert!(matches!(&cpu.widgets[2].kind, WidgetKind::Header(h) if h == "Disks"));
    assert_eq!(cpu.widgets[2].width, 1.);

    let instances = &view.reports[2];
    assert_eq!(
        instances.checks[0].condition().1,
        "0.75%",
        "the threshold is shown as Coroot sends it"
    );
    let WidgetKind::Table(table) = &instances.widgets[0].kind else {
        panic!("a table");
    };
    assert_eq!(table.header.len(), 4);
    let row = &table.rows[0];
    assert_eq!(row[1].status, Some(Status::Ok));
    assert!(row[2].stub);
    let link = row[3].link.as_ref().unwrap();
    assert_eq!((link.view.as_str(), link.id.as_str()), ("nodes", "node-a"));

    assert!(
        matches!(&view.reports[3].widgets[0].kind, WidgetKind::Logs(Some(c)) if c.id == "LogErrors")
    );
    let dns = &view.reports[4];
    assert_eq!(dns.checks[0].condition().1, "100ms");
    assert!(matches!(
        dns.widgets[0].kind,
        WidgetKind::Other("dependency map")
    ));
    assert!(matches!(
        view.reports[5].widgets[0].kind,
        WidgetKind::Profiling
    ));
    assert!(matches!(
        view.reports[6].widgets[0].kind,
        WidgetKind::Tracing
    ));
}

#[tokio::test]
async fn app_view_failures_stay_distinct_from_an_empty_page() {
    let mut other = app_view_wire();
    other["data"]["app_map"]["application"]["id"] = "c:shop:Deployment:other".into();
    let mut bad_chart = app_view_wire();
    bad_chart["data"]["reports"][1]["widgets"][1]["chart"]["ctx"]["step"] = 0.into();
    let mut no_map = app_view_wire();
    no_map["data"]["app_map"] = serde_json::Value::Null;
    let mut empty = app_view_wire();
    empty["data"]["reports"] = serde_json::Value::Null;
    let server = server(vec![
        (200, serde_json::json!({"context":{},"data":null})),
        (200, other),
        (200, bad_chart),
        (200, no_map),
        (403, serde_json::json!({})),
        (200, empty),
    ])
    .await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let app = AppId::new("c:shop:Deployment:auth");
    let source = source(&provider);
    let read = || provider.app_view(&source, range(), &app);
    assert_eq!(read().await.unwrap_err(), ReadError::Missing);
    assert_eq!(read().await.unwrap_err(), ReadError::InvalidResponse);
    assert_eq!(read().await.unwrap_err(), ReadError::InvalidResponse);
    assert_eq!(read().await.unwrap_err(), ReadError::InvalidResponse);
    assert_eq!(read().await.unwrap_err(), ReadError::Refused);
    let view = read().await.unwrap();
    assert!(view.reports.is_empty());
    assert_eq!(view.map.instances.len(), 2);
}

#[test]
fn app_view_bounds_reject_runaway_answers() {
    let view = |data: serde_json::Value| app_view::decode(data["data"].clone()).unwrap();
    let mut wire = app_view_wire();
    assert_eq!(limits::app_view(&view(wire.clone())), Ok(()));
    wire["data"]["app_map"]["instances"] = (0..2_001)
        .map(|i| serde_json::json!({"id": format!("i{i}")}))
        .collect();
    assert_eq!(limits::app_view(&view(wire)), Err(ReadError::Limit));

    let mut wire = app_view_wire();
    wire["data"]["reports"][1]["widgets"][1]["chart"]["series"][0]["data"] =
        vec![1.0; 4_097].into();
    assert_eq!(limits::app_view(&view(wire)), Err(ReadError::Limit));

    let mut wire = app_view_wire();
    wire["data"]["reports"][2]["checks"][0]["message"] = "x".repeat(8_193).into();
    assert_eq!(limits::app_view(&view(wire)), Err(ReadError::Limit));
}

#[test]
fn check_thresholds_read_as_coroot_writes_them() {
    let check = |threshold: f32, unit: &str| {
        Check {
            id: String::new(),
            title: String::new(),
            status: Status::Ok,
            message: String::new(),
            threshold,
            unit: unit.into(),
            condition: "x > <threshold>".into(),
        }
        .condition()
        .1
    };
    assert_eq!(check(0., ""), "0");
    assert_eq!(check(1., ""), "1");
    assert_eq!(check(0.01, "second"), "10ms");
    assert_eq!(check(0.05, "second"), "50ms");
    assert_eq!(check(2.5, "second"), "2.5s");
    assert_eq!(check(0.02, "seconds/second"), "0.02 seconds/second");
    assert_eq!(check(75., "percent"), "75%");
    assert_eq!(check(10., "percent"), "10%");
    let none = Check {
        id: String::new(),
        title: String::new(),
        status: Status::Ok,
        message: String::new(),
        threshold: 0.,
        unit: String::new(),
        condition: "IO or SQL replication thread is not running".into(),
    };
    assert_eq!(
        none.condition(),
        (
            "IO or SQL replication thread is not running",
            "0".into(),
            ""
        )
    );
}
