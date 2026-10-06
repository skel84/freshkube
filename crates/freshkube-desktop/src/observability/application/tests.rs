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
    let api::WidgetKind::Table(table) = &instances.widgets[0].kind else {
        panic!("Instances has its table");
    };
    let up = table.rows.iter().filter(|r| r[1].value == "up").count();
    assert_eq!((table.rows.len(), up), (2, 1));
    assert_eq!(view.map.instances.len(), 2);
    // Another application lists its own pod, not the worker's.
    let ledger = super::example::app_view(&example::id("payments/ledger-db"));
    assert_eq!(ledger.map.instances[0].id, "ledger-db-0");
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
