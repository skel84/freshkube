use super::super::{Destination, Report, example, tests::mount};
use super::Subject;
use freshkube_core::coroot as api;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};

fn worker() -> api::AppId {
    example::id(example::WORKER)
}

#[gpui_kit::test]
fn the_map_tabs_and_checks_follow_coroots_view(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| page.update(cx, |page, cx| page.open_app(worker(), Report::Net, cx)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-app-map-app").visible());
        for side in ["payments:Deployment:api", "payments:Deployment:ledger-db"] {
            assert!(window.find(format!("obs-app-map-fixture:{side}")).visible());
        }
        let view = page.read(cx).app_page.as_ref().unwrap();
        let tabs: Vec<_> = view
            .tabs
            .iter()
            .map(|t| (t.name.as_str(), t.status.is_some()))
            .collect();
        assert_eq!(tabs[0], ("SLO", true));
        assert!(tabs.contains(&("Net", true)));
        // Reports with nothing to judge by carry no glyph, as in Coroot.
        assert!(tabs.contains(&("Profiling", false)));
        assert!(tabs.contains(&("Tracing", false)));
        assert!(window.find("obs-app-checks").visible());
        let rtt = view
            .checks
            .iter()
            .find(|c| c.title == "Network round-trip time")
            .unwrap();
        let [head, threshold, _] = rtt.condition.as_ref().unwrap();
        assert_eq!(head, "the round-trip time to a dependency >");
        assert_eq!(threshold, "10ms");
        assert!(rtt.verdict.contains("ledger-db:5432"));
        // A dependency's box opens it on the same report.
        window.click("obs-app-map-fixture:payments:Deployment:ledger-db", cx);
        let page = page.read(cx);
        assert_eq!(page.selected_app, Some(example::id("payments/ledger-db")));
        assert_eq!(page.report_name, "Net");
        assert_eq!(page.destination, Destination::Application);
    })
    .unwrap();
}

#[gpui_kit::test]
fn report_tables_draw_and_their_links_open_applications(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.open_app(worker(), Report::Instances, cx)
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(page.read(cx).app_tables.len(), 1);
        assert!(window.find("obs-app-table-0-list").visible());
        assert!(window.find("obs-app-table-0-row-1").visible());
        // A node link isn't an application, so it opens nothing.
        assert!(window.try_find("obs-app-table-0-link-0-4").is_none());
        window.click("obs-report-net", cx);
        window.render_frame(cx);
        assert_eq!(page.read(cx).report_name, "Net");
        window.click("obs-app-table-0-link-0-0", cx);
        assert_eq!(
            page.read(cx).selected_app,
            Some(example::id("payments/ledger-db"))
        );
        window.render_frame(cx);
        // The dependency has no table on its Net report.
        assert!(page.read(cx).app_tables.is_empty());
        assert!(window.try_find("obs-app-table-0-list").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn tracing_and_profiling_reports_embed_their_pages(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| page.update(cx, |page, cx| page.open_app(worker(), Report::Net, cx)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-report-tracing", cx);
        window.render_frame(cx);
        assert!(page.read(cx).embeds_tracing());
        assert!(window.find("obs-live-traces").visible());
        assert!(window.find("obs-traces-list").visible());
        assert!(page.read(cx).live_traces.trace.is_some());
        window.click("obs-report-profiling", cx);
        window.render_frame(cx);
        assert!(!page.read(cx).embeds_tracing());
        assert!(window.try_find("obs-live-traces").is_none());
        // The profile is the worker's own, not another application's.
        assert!(window.find("obs-live-flame").visible());
        let names = page.read(cx).live_profiles.frame_names().join(" ");
        assert!(names.contains("main.(*Worker).settle"), "{names}");
        assert!(!names.contains("ApplicationController"), "{names}");
    })
    .unwrap();
    // An application without profiles says so.
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.open_app(example::id("payments/ledger-db"), Report::Net, cx);
            page.select_report("Profiling".into(), cx);
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-live-profiling").visible());
        assert!(window.try_find("obs-live-flame").is_none());
        assert!(page.read(cx).live_profiles.frame_names().is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_header_is_the_applications_breadcrumb(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| page.update(cx, |page, cx| page.open_app(worker(), Report::Net, cx)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let crumb = window.find("obs-breadcrumb").bounds();
        let title = window.find("obs-title").bounds();
        // One row: Applications / payments / ● worker.
        assert!((crumb.center().y - title.center().y).abs() < gpui_kit::px(4.));
        assert!(crumb.right() < title.left());
        window.click("obs-breadcrumb", cx);
        assert_eq!(page.read(cx).destination, Destination::Applications);
    })
    .unwrap();
}

#[test]
fn the_example_agrees_with_itself() {
    let view = super::example::app_view(&worker());
    let ledger = view
        .map
        .dependencies
        .iter()
        .find(|d| d.id == example::id("payments/ledger-db"))
        .unwrap();
    // The box, its link and its row in Net all say critical.
    assert_eq!(ledger.status, api::Status::Critical);
    assert_eq!(ledger.link.as_ref().unwrap().status, api::Status::Critical);
    let instances = view.reports.iter().find(|r| r.name == "Instances").unwrap();
    let check = &instances.checks[0];
    assert!(check.message.contains("1/2"), "{}", check.message);
    let table = instances
        .widgets
        .iter()
        .find_map(|w| match &w.kind {
            api::WidgetKind::Table(table) => Some(table),
            _ => None,
        })
        .expect("Instances has its table");
    let up = table.rows.iter().filter(|r| r[1].value == "up").count();
    assert_eq!((table.rows.len(), up), (2, 1));
    assert_eq!(view.map.instances.len(), 2);
    // Another application lists its own pod, not the worker's.
    let ledger = super::example::app_view(&example::id("payments/ledger-db"));
    assert_eq!(ledger.map.instances[0].id, "ledger-db-0");
    // Every report stays under core's limit on widgets.
    assert!(view.reports.iter().all(|r| r.widgets.len() < 128));
}

/// A live page on the worker's report, its view not read yet.
fn live(
    cx: &mut TestAppContext,
) -> (
    tokio::runtime::Runtime,
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<super::ObservabilityPage>,
) {
    let (runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, _| {
            page.fixture = false;
            let provider =
                api::Provider::new("http://127.0.0.1:1", api::Credentials::None).unwrap();
            page.live.source = Some(provider.source(&api::ProjectInfo {
                id: "p".into(),
                name: "Project".into(),
            }));
            page.live.provider = Some(provider);
            page.live.visible = false;
            page.destination = Destination::Application;
            page.selected_app = Some(worker());
            page.prepare_report();
        })
    });
    (runtime, handle, page)
}

fn answer(
    page: &mut super::ObservabilityPage,
    app: &api::AppId,
    result: Result<api::AppView, api::ReadError>,
) {
    let identity = page.live.identity(Subject::View(app.clone())).unwrap();
    let request = page.live.view.begin(identity);
    page.live.view_refused = matches!(result, Err(api::ReadError::Refused));
    page.live
        .view
        .apply(&request, result.map_err(|e| e.to_string()));
    page.prepare_report();
}

#[gpui_kit::test]
fn loading_failure_refusal_empty_and_stale_stay_distinct(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = live(cx);
    cx.update(|cx| {
        page.update(cx, |page, _| {
            let identity = page.live.identity(Subject::View(worker())).unwrap();
            page.live.view.begin(identity);
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-app-loading").visible());
        assert!(window.try_find("obs-app-map").is_none());
    })
    .unwrap();
    for (error, surface) in [
        (api::ReadError::Refused, "obs-app-refused"),
        (api::ReadError::Missing, "obs-app-failed"),
    ] {
        cx.update(|cx| page.update(cx, |page, _| answer(page, &worker(), Err(error))));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find(surface).visible());
            assert!(window.find("obs-app-retry").visible());
            assert!(window.try_find("obs-app-loading").is_none());
        })
        .unwrap();
    }
    let empty = api::AppView {
        map: api::AppMap {
            app: api::MapApp {
                id: worker(),
                ..Default::default()
            },
            ..Default::default()
        },
        reports: vec![],
        ..Default::default()
    };
    cx.update(|cx| page.update(cx, |page, _| answer(page, &worker(), Ok(empty))));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-app-empty").visible());
        assert!(window.try_find("obs-app-failed").is_none());
    })
    .unwrap();
    cx.update(|cx| {
        page.update(cx, |page, _| {
            answer(page, &worker(), Ok(super::example::app_view(&worker())));
            answer(page, &worker(), Err(api::ReadError::Timeout));
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // A failed refresh keeps the last view, under the stale banner.
        assert!(window.find("obs-stale").visible());
        assert!(window.find("obs-app-map").visible());
        assert!(window.find("obs-report-net").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn another_applications_view_never_shows(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = live(cx);
    cx.update(|cx| {
        page.update(cx, |page, _| {
            let other = example::id("payments/api");
            let identity = page.live.identity(Subject::View(worker())).unwrap();
            let request = page.live.view.begin(identity);
            // The answer for the worker arrives after the page moved on.
            let replaced = page.live.identity(Subject::View(other.clone())).unwrap();
            page.live.view.begin(replaced);
            page.selected_app = Some(other);
            assert!(
                !page
                    .live
                    .view
                    .apply(&request, Ok(super::example::app_view(&worker())))
            );
            page.prepare_report();
            assert!(page.app_page.is_none());
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-app-loading").visible());
        assert!(window.try_find("obs-app-map").is_none());
    })
    .unwrap();
}

fn chart_keys(page: &super::ObservabilityPage) -> Vec<(String, String, usize)> {
    page.app_charts
        .iter()
        .map(|c| {
            let (app, report, ix) = c.key();
            (app.as_str().to_string(), report.clone(), *ix)
        })
        .collect()
}

#[gpui_kit::test]
fn only_the_shown_reports_charts_are_made(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| page.update(cx, |page, cx| page.open_app(worker(), Report::Cpu, cx)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for id in ["obs-chart-cpu-0", "obs-chart-cpu-1", "obs-chart-cpu-2"] {
            assert!(window.find(id).visible(), "{id}");
        }
        let app = worker().as_str().to_string();
        let cpu: Vec<_> = (0..3).map(|ix| (app.clone(), "CPU".into(), ix)).collect();
        assert_eq!(chart_keys(page.read(cx)), cpu);
        // A report without charts makes none, and keeps none of the last.
        window.click("obs-report-profiling", cx);
        window.render_frame(cx);
        assert!(page.read(cx).app_charts.is_empty());
        assert!(window.try_find("obs-chart-cpu-0").is_none());
        window.click("obs-report-slo", cx);
        window.render_frame(cx);
        assert!(window.find("obs-chart-slo-0").visible());
        assert!(window.find("obs-chart-slo-1").visible());
        assert!(window.find("obs-heatmap-slo-2").visible());
        let slo: Vec<_> = (0..2).map(|ix| (app.clone(), "SLO".into(), ix)).collect();
        assert_eq!(chart_keys(page.read(cx)), slo);
        // The worker's rollout marks its requests.
        let requests = page.read(cx).app_charts[0].view().clone();
        let labels = requests.read(cx).marker_labels();
        assert!(
            labels.iter().any(|l| l.contains("worker:1.8.2")),
            "{labels:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_chart_group_opens_on_its_featured_chart_and_picks_another(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| page.update(cx, |page, cx| page.open_app(worker(), Report::Cpu, cx)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let title = |cx: &gpui_kit::App| page.read(cx).app_charts[0].title().to_string();
        // The instance that is up is featured, not the first listed.
        assert_eq!(
            title(cx),
            "CPU usage of container worker-6c4f8da0-x2k9q, cores"
        );
        window.click("obs-chart-cpu-0-pick-0", cx);
        window.render_frame(cx);
        assert!(title(cx).contains(example::POD), "{}", title(cx));
        assert_eq!(page.read(cx).app_charts.len(), 3);
        // The pick outlives a trip to another report.
        window.click("obs-report-net", cx);
        window.render_frame(cx);
        assert!(window.find("obs-chart-net-0").visible());
        window.click("obs-report-cpu", cx);
        window.render_frame(cx);
        assert!(title(cx).contains(example::POD), "{}", title(cx));
    })
    .unwrap();
}

#[gpui_kit::test]
fn another_applications_charts_never_show(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| page.update(cx, |page, cx| page.open_app(worker(), Report::Net, cx)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(page.read(cx).app_charts.len(), 2);
        // ledger-db's Net report has no charts of its own.
        window.click("obs-app-table-0-link-0-0", cx);
        window.render_frame(cx);
        assert!(page.read(cx).app_charts.is_empty());
        assert!(window.try_find("obs-chart-net-0").is_none());
    })
    .unwrap();
}

/// A three-minute chart of `series`, carrying the history coroot-rs would
/// decode from it.
fn chart(title: &str, series: Vec<api::Series>) -> api::AppChart {
    let mut chart = api::AppChart {
        title: title.into(),
        from_ms: 1_760_000_000_000,
        to_ms: 1_760_000_120_000,
        step_ms: 60_000,
        series,
        ..Default::default()
    };
    super::example::give_history(&mut chart, None);
    chart
}

fn points(name: &str, points: Vec<Option<f32>>) -> api::Series {
    api::Series {
        name: name.into(),
        points,
        ..Default::default()
    }
}

/// The worker's view with one CPU report of half-width charts.
fn cpu_view(charts: Vec<api::AppChart>) -> api::AppView {
    api::AppView {
        map: api::AppMap {
            app: api::MapApp {
                id: worker(),
                ..Default::default()
            },
            ..Default::default()
        },
        reports: vec![api::AppReport {
            name: "CPU".into(),
            status: api::Status::Ok,
            checks: vec![],
            widgets: charts
                .into_iter()
                .map(|chart| api::Widget {
                    kind: api::WidgetKind::Chart(chart),
                    width: 0.5,
                })
                .collect(),
            custom: false,
            instrumentation: String::new(),
        }],
        ..Default::default()
    }
}

#[gpui_kit::test]
fn a_chart_without_points_says_so(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = live(cx);
    let view = cpu_view(vec![
        chart("CPU usage, cores", vec![]),
        chart(
            "CPU delay, seconds/second",
            vec![points("worker", vec![Some(0.1), None, Some(0.2)])],
        ),
    ]);
    cx.update(|cx| page.update(cx, |page, _| answer(page, &worker(), Ok(view))));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-chart-cpu-0-empty").visible());
        assert!(window.try_find("obs-chart-cpu-0").is_none());
        assert!(window.find("obs-chart-cpu-1").visible());
        // A full window says nothing of its coverage.
        assert!(window.try_find("obs-chart-cpu-1-coverage").is_none());
        // Only the chart with points has a panel.
        let keys: Vec<_> = chart_keys(page.read(cx)).into_iter().map(|k| k.2).collect();
        assert_eq!(keys, [1]);
        // The two halves share a row.
        let (left, right) = (
            window.find("obs-chart-cpu-0-empty").bounds(),
            window.find("obs-chart-cpu-1").bounds(),
        );
        assert!((left.top() - right.top()).abs() < gpui_kit::px(1.));
        assert!(left.right() <= right.left());
    })
    .unwrap();
}

#[gpui_kit::test]
fn partial_empty_and_truncated_series_say_so_under_their_chart(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = live(cx);
    let mut cut = chart(
        "CPU delay, seconds/second",
        vec![points("worker", vec![Some(0.1), Some(0.1), Some(0.2)])],
    );
    cut.history.as_mut().unwrap().truncated = true;
    let view = cpu_view(vec![
        chart(
            "CPU usage, cores",
            vec![
                points("worker-a", vec![Some(0.5), None, Some(0.7)]),
                points("worker-b", vec![Some(0.2), Some(0.3)]),
                points("worker-c", vec![]),
            ],
        ),
        cut,
    ]);
    cx.update(|cx| page.update(cx, |page, _| answer(page, &worker(), Ok(view))));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-chart-cpu-0").visible());
        assert!(window.find("obs-chart-cpu-0-coverage").visible());
        assert!(window.find("obs-chart-cpu-1-coverage").visible());
        let page = page.read(cx);
        let words: Vec<_> = page
            .app_page
            .as_ref()
            .unwrap()
            .chart_words()
            .into_iter()
            .map(|(_, coverage, _)| coverage)
            .collect();
        assert_eq!(
            words,
            [
                Some("worker-b covers 2 of 3 points · worker-c: no data".into()),
                Some("Truncated range".into()),
            ]
        );
        // The empty series is left out of the chart, never drawn as zero.
        let drawn: Vec<_> = page.app_charts[0]
            .view()
            .read(cx)
            .series_colors(cx)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(drawn, ["worker-a", "worker-b"]);
    })
    .unwrap();
}

#[gpui_kit::test]
fn histories_that_fail_to_decode_say_so_once_for_the_page(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = live(cx);
    let mut view = cpu_view(vec![chart(
        "CPU usage, cores",
        vec![points("worker", vec![Some(0.1), None, Some(0.2)])],
    )]);
    // As core leaves it: the layout without its history, and why.
    for widget in &mut view.reports[0].widgets {
        if let api::WidgetKind::Chart(chart) = &mut widget.kind {
            chart.history = None;
        }
    }
    view.history_error = Some(api::ReadError::Limit);
    cx.update(|cx| page.update(cx, |page, _| answer(page, &worker(), Ok(view))));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-app-history-error").visible());
        // The layout's points never stand in for the history.
        assert!(window.find("obs-chart-cpu-0-empty").visible());
        let page = page.read(cx);
        let app = page.app_page.as_ref().unwrap();
        assert!(
            app.history_note
                .as_ref()
                .unwrap()
                .contains("exceeded the size limit")
        );
        assert_eq!(
            app.chart_words()[0].2,
            "Coroot's history for this chart couldn't be read."
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_deployment_is_marked_only_on_the_window_it_started_in(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = live(cx);
    let mut view = cpu_view(vec![chart(
        "CPU usage, cores",
        vec![points("worker", vec![Some(0.1), None, Some(0.2)])],
    )]);
    let revision = |hash: &str, at: i64| api::DeploymentRevision {
        id: format!("{hash}:{at}"),
        hash: hash.into(),
        started_at: chrono::DateTime::from_timestamp(at, 0).unwrap(),
        version: format!("{hash}: example.test/payments/worker:1.8.2"),
        status: api::Status::Unknown,
        findings: vec![],
        note: Some("Collecting data...".into()),
    };
    view.revisions = Some(Ok(vec![
        revision("4f2a9c", 1_760_000_060),
        revision("b71e03", 1_759_000_000),
    ]));
    cx.update(|cx| page.update(cx, |page, _| answer(page, &worker(), Ok(view))));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = page.read(cx);
        let labels = page.app_charts[0].view().read(cx).marker_labels();
        assert_eq!(labels, ["4f2a9c · worker:1.8.2"]);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_example_draws_from_histories_with_the_worker_s_rollout(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, _| {
            page.destination = Destination::Application;
            page.selected_app = Some(worker());
            page.report_name = "CPU".into();
            page.prepare_report();
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = page.read(cx);
        assert!(!page.app_charts.is_empty());
        let app = page.app_page.as_ref().unwrap();
        assert!(app.history_note.is_none());
        // Every example series covers its whole hour.
        assert!(
            app.chart_words()
                .iter()
                .all(|(_, coverage, _)| coverage.is_none())
        );
        let labels = page.app_charts[0].view().read(cx).marker_labels();
        assert_eq!(labels, ["4f2a9c · worker:1.8.2"]);
    })
    .unwrap();
}

#[gpui_kit::test]
fn no_tab_adds_evidence_beside_coroots_view(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| page.update(cx, |page, cx| page.open_app(worker(), Report::Net, cx)));
    let tabs: Vec<_> = cx.update(|cx| {
        let page = page.read(cx);
        let view = page.app_page.as_ref().unwrap();
        view.tabs.iter().map(|t| t.id.clone()).collect()
    });
    assert!(tabs.len() > 8, "{tabs:?}");
    let ledger = example::id("payments/ledger-db");
    let api = example::id("payments/api");
    cx.update_window(handle, |_, window, cx| {
        for tab in tabs {
            window.click(tab.clone(), cx);
            window.render_frame(cx);
            assert!(window.find("obs-app-map-app").visible(), "{tab}");
            // The card's calls, clients, pager and Retry are gone with it.
            for id in [
                format!("obs-rest-dependency-{ledger}"),
                format!("obs-rest-client-{api}"),
                "obs-evidence-false-true".into(),
                "obs-retry-rest".into(),
                "obs-retry-extended".into(),
            ] {
                assert!(window.try_find(id.clone()).is_none(), "{tab}: {id}");
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_narrow_page_keeps_charts_and_checks_inside_its_margin(cx: &mut TestAppContext) {
    for text_size in [13., 20.] {
        let (_runtime, handle, page) = super::super::tests::mount_geometry(cx, 760., 560.);
        cx.update_window(handle, |_, _, cx| crate::text_size::set(text_size, cx))
            .unwrap();
        cx.run_until_parked();
        cx.update(|cx| page.update(cx, |page, cx| page.open_app(worker(), Report::Errors, cx)));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let card = window.find("obs-app-checks").bounds();
            let inside = |id: &'static str, window: &gpui_kit::Window| {
                let bounds = window.find(id).bounds();
                assert!(
                    bounds.right() <= card.right() + gpui_kit::px(0.5),
                    "{text_size}: {id} {bounds:?} runs past the margin {card:?}"
                );
                bounds
            };
            // The half-width pair can't share a row, so each takes its own.
            let requests = inside("obs-chart-slo-0", window);
            let errors = inside("obs-chart-slo-1", window);
            inside("obs-heatmap-slo-2", window);
            assert!(errors.top() >= requests.bottom(), "{text_size}");
            assert!(
                (requests.right() - card.right()).abs() < gpui_kit::px(1.),
                "{text_size}: a lone chart fills the row"
            );
            // Coroot's long words wrap inside the Checks card.
            window.click("obs-report-net", cx);
            window.render_frame(cx);
            let card = window.find("obs-app-checks").bounds();
            for id in [
                "obs-app-check-0",
                "obs-app-check-0-condition",
                "obs-app-check-1",
            ] {
                let bounds = window.find(id).bounds();
                assert!(
                    bounds.right() <= card.right(),
                    "{text_size}: {id} {bounds:?} runs past the card {card:?}"
                );
            }
        })
        .unwrap();
    }
}
