use super::{Destination, MatrixRow, ObservabilityPage, Report, example};
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled,
    TestAppContext, Window, div, px, size,
};
pub(super) fn mount(
    cx: &mut TestAppContext,
    fixture: bool,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<ObservabilityPage>,
) {
    mount_size(cx, fixture, 1260., 900.)
}
pub(super) fn mount_size(
    cx: &mut TestAppContext,
    fixture: bool,
    width: f32,
    height: f32,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<ObservabilityPage>,
) {
    mount_with(cx, fixture, width, height, None, None, false)
}
/// A live page that remembers its connection in `preferences`' folder and
/// keeps keys in `secrets`.
pub(super) fn mount_remembering(
    cx: &mut TestAppContext,
    preferences: &std::path::Path,
    secrets: crate::secrets::Secrets,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<ObservabilityPage>,
) {
    mount_with(
        cx,
        false,
        1260.,
        900.,
        Some(preferences),
        Some(secrets),
        false,
    )
}
fn mount_with(
    cx: &mut TestAppContext,
    fixture: bool,
    width: f32,
    height: f32,
    preferences: Option<&std::path::Path>,
    secrets: Option<crate::secrets::Secrets>,
    with_chrome: bool,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<ObservabilityPage>,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.set_reduce_motion(true);
        crate::screens::set_chrome_width(crate::desktop::RAIL_WIDTH + crate::desktop::COLUMN_WIDTH);
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut page = None;
    let handle = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| {
            ObservabilityPage::new(
                fixture,
                runtime.handle().clone(),
                preferences,
                secrets.clone(),
                window,
                cx,
            )
        });
        page = Some(view.clone());
        if with_chrome {
            let chrome = cx.new(|_| TestChrome { page: view });
            Root::new(chrome, window, cx)
        } else {
            Root::new(view, window, cx)
        }
    });
    cx.run_until_parked();
    let page = page.unwrap();
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(true, cx)));
    (runtime, handle.into(), page)
}
/// Reserve the shell's actual navigation width while keeping the full window
/// size, so content-width breakpoints and page bounds agree.
struct TestChrome {
    page: Entity<ObservabilityPage>,
}
impl Render for TestChrome {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let column = if window.viewport_size().width / crate::ui::dp_px(1., window) < 1000. {
            52.
        } else {
            crate::desktop::COLUMN_WIDTH
        };
        let chrome = crate::desktop::RAIL_WIDTH + column;
        crate::screens::set_chrome_width(chrome);
        gpui_kit::component::h_flex()
            .size_full()
            .child(div().w(crate::ui::dp(chrome)).h_full().flex_none())
            .child(self.page.clone())
    }
}
fn mount_geometry(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<ObservabilityPage>,
) {
    mount_with(cx, true, width, height, None, None, true)
}
fn settle_header(window: &mut Window, cx: &mut gpui_kit::App) {
    for _ in 0..4 {
        window.render_frame(cx);
        if window.simulate_next_frame(cx) == 0 {
            return;
        }
    }
    panic!("Applications header must settle without further input");
}

#[gpui_kit::test]
fn applications_filter_and_cells_open_the_selected_report(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-filter-all", cx);
        assert_eq!(
            page.read(cx).counts[1],
            example::applications()
                .iter()
                .filter(|app| app.category == "application")
                .count()
        );
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
        window.click("obs-check-fixture:payments:Deployment:worker-net", cx);
        assert_eq!(page.read(cx).destination, Destination::Application);
        assert_eq!(page.read(cx).report, Report::Net);
        window.click("obs-report-memory", cx);
        assert_eq!(page.read(cx).report_name, "Memory");
    })
    .unwrap();
}
#[gpui_kit::test]
fn live_mode_never_contains_example_applications(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, false);
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
            assert!(window.try_find("obs-applications-table").is_none());
            assert!(page.read(cx).applications.is_empty());
        })
        .unwrap();
    }
}
#[gpui_kit::test]
fn map_selection_survives_reordering_and_clears_on_removal(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| page.update(cx, |page, cx| page.open(Destination::ServiceMap, cx)));
    cx.update_window(handle,|_,window,cx| {
        window.render_frame(cx);
        window.click("obs-map-link-fixture:payments:Deployment:worker--fixture:payments:Deployment:ledger-db",cx);
        let selected=page.read(cx).selected_link.clone();assert!(selected.is_some());
        page.update(cx,|page,_| {
            let mut raw=example::map();raw.nodes.reverse();raw.edges.reverse();
            (page.nodes,page.connections)=super::projection::map(&raw);page.prepare_map();
            assert_eq!(page.selected_link,selected);
            raw.edges.clear();(page.nodes,page.connections)=super::projection::map(&raw);page.prepare_map();
            assert!(page.selected_link.is_none());
        });
    }).unwrap();
}
#[gpui_kit::test]
fn heatmap_selection_and_error_filters_change_the_trace_view(cx: &mut TestAppContext) {
    use freshkube_core::coroot::TraceSelection;
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| page.update(cx, |page, cx| page.open(Destination::Traces, cx)));
    // Example ids name the request: x0–x2 failed, x3 healthy.
    let trace = |cx: &mut gpui_kit::App| page.read(cx).live_traces.trace.clone();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Example traces answer through the live list, which opens the
        // first failed request.
        assert!(window.find("obs-live-heatmap").visible());
        assert!(window.find("obs-live-span-0").visible());
        assert!(window.find("obs-live-waterfall").visible());
        assert!(trace(cx).unwrap().starts_with("x0"));

        // Failures begin two hours before the window's end.
        window.click("obs-live-bucket-0-0", cx);
        window.render_frame(cx);
        assert!(window.try_find("obs-live-span-0").is_none());
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
        window.click("obs-live-span-1", cx);
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
        assert!(window.find("obs-live-span-0").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn threshold_edits_validate_and_remain_local(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.open_app(example::id(example::WORKER), Report::Memory, cx)
        })
    });
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
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.open_app(example::id(example::WORKER), Report::Memory, cx)
        })
    });
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
fn ranges_reports_and_traces_have_independent_data(cx: &mut TestAppContext) {
    let (_runtime, _handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let prior = page.charts[0].series[0].values.clone();
            page.set_range(1, cx);
            assert_ne!(prior, page.charts[0].series[0].values);
            page.open_app(example::id("payments/api"), Report::Latency, cx);
            assert_eq!(page.selected_application().unwrap().key, "payments/api");
            page.open(Destination::Traces, cx);
            let trace = page.live_traces.trace.clone();
            assert!(trace.is_some());
            let charts = page.charts[0].series[0].values.clone();
            page.set_range(6, cx);
            assert_ne!(
                page.live_traces.trace, trace,
                "another window answers its own requests"
            );
            assert!(page.live_traces.trace.is_some());
            assert_ne!(charts, page.charts[0].series[0].values);
        })
    });
}

#[gpui_kit::test]
fn projection_preserves_signal_states_and_selection_by_id(cx: &mut TestAppContext) {
    use super::Status;
    let (_runtime, _handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let id = example::id(example::WORKER);
            page.open_app(id.clone(), Report::Cpu, cx);
            let raw = example::applications();
            let app = page.selected_application().unwrap();
            assert_eq!(app.check(Report::Cpu).status, Status::Ok);
            assert_eq!(app.check(Report::Errors).status, Status::Unknown);
            assert_eq!(app.check(Report::DiskIo).status, Status::Absent);
            let mut reversed = raw.clone();
            reversed.reverse();
            page.apply_applications(&reversed);
            assert_eq!(page.selected_app.as_ref(), Some(&id));
            reversed.retain(|a| a.id != id);
            page.apply_applications(&reversed);
            assert!(page.selected_app.is_none());
            assert!(page.report_snapshot.is_none());
            // A live publication uses exactly the same projection as these fixtures.
            page.fixture = false;
            page.apply_applications(&raw);
            let live: Vec<_> = page
                .applications
                .iter()
                .map(|a| (a.id.clone(), a.status))
                .collect();
            let fixture: Vec<_> = super::projection::applications(&raw)
                .into_iter()
                .map(|a| (a.id, a.status))
                .collect();
            assert_eq!(live, fixture);
        })
    });
}

#[gpui_kit::test]
async fn hidden_page_cancels_tokio_work_and_rejects_a_queued_answer(cx: &mut TestAppContext) {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    cx.executor().allow_parking();
    let (_runtime, handle, page) = mount(cx, false);
    let dropped = Arc::new(AtomicBool::new(false));
    let started = Arc::new(AtomicBool::new(false));
    struct OnDrop(Arc<AtomicBool>);
    impl Drop for OnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let (dropped, started) = (dropped.clone(), started.clone());
            page.spawn_read(
                async move {
                    let _drop = OnDrop(dropped);
                    started.store(true, Ordering::SeqCst);
                    std::future::pending::<()>().await;
                    Ok(example::applications())
                },
                |page, result, cx| {
                    page.apply_applications(&result.unwrap());
                    cx.notify();
                },
                cx,
            );
        })
    });
    cx.wait_for(handle, std::time::Duration::from_secs(3), move |_, _| {
        started.load(Ordering::SeqCst)
    })
    .await;
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(false, cx)));
    cx.wait_for(handle, std::time::Duration::from_secs(3), move |_, _| {
        dropped.load(Ordering::SeqCst)
    })
    .await;
    cx.update(|cx| {
        assert!(page.read(cx).applications.is_empty());
        assert!(page.read(cx).live.jobs.is_empty());
    });
}

#[gpui_kit::test]
fn source_project_credentials_and_range_invalidate_old_requests(cx: &mut TestAppContext) {
    use freshkube_core::coroot::{Association, Credentials, ProjectInfo, Provider};
    let (_runtime, handle, page) = mount(cx, false);
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            let provider = Provider::new("http://127.0.0.1:1", Credentials::None).unwrap();
            page.live.provider = Some(provider.clone());
            let project = ProjectInfo {
                id: "one".into(),
                name: "Same name".into(),
            };
            page.live.source = Some(provider.source(&project));
            page.destination = Destination::Deployments; // Unsupported destination starts no I/O.
            let generation = page.live.generation;
            page.select_project(
                &ProjectInfo {
                    id: "two".into(),
                    name: "Same name".into(),
                },
                cx,
            );
            assert!(page.live.generation > generation);
            let prior = page.live.source.clone();
            page.live.access = Some("access:a".into());
            page.associate(Some("coroot-cluster".into()), cx);
            assert_ne!(prior, page.live.source);
            assert_eq!(
                page.live.source.as_ref().unwrap().association(),
                Some(&Association::new(
                    "access:a".into(),
                    "coroot-cluster".into()
                ))
            );
            let generation = page.live.generation;
            page.set_source(None, cx);
            assert!(page.live.generation > generation);
            assert!(page.live.source.as_ref().unwrap().association().is_none());
            let prior = page.live.range;
            page.set_range(24, cx);
            assert_ne!(prior, page.live.range);
            let generation = page.live.generation;
            page.disconnect(window, cx);
            assert!(page.live.generation > generation);
            assert!(page.live.provider.is_none());
            assert!(page.live.source.is_none());
            assert_ne!(
                provider.id(),
                Provider::new("http://127.0.0.1:1", Credentials::None)
                    .unwrap()
                    .id()
            );
        })
    })
    .unwrap();
}

#[gpui_kit::test]
fn bounded_projection_measurement(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    let seed = example::applications()[0].clone();
    let apps: Vec<_> = (0..2000)
        .map(|ix| {
            let mut app = seed.clone();
            app.id =
                freshkube_core::coroot::AppId::new(format!("c:ns-{ix}:Deployment:app-{ix:04}"));
            app.category = format!("Category-{ix:04}");
            app
        })
        .collect();
    cx.update(|cx| {
        page.update(cx, |page, _| {
            let start = std::time::Instant::now();
            page.active_categories =
                std::rc::Rc::new(apps.iter().map(|app| app.category.clone()).collect());
            page.apply_applications(&apps);
            let app_time = start.elapsed();
            let raw = large_map();
            let start = std::time::Instant::now();
            (page.nodes, page.connections) = super::projection::map(&raw);
            page.prepare_map();
            eprintln!(
                "Coroot projection: 2000 applications {:?}; 120 nodes/300 links {:?}",
                app_time,
                start.elapsed()
            );
            assert_eq!(page.applications.len(), 2000);
            assert_eq!(page.connections.len(), 300);
        })
    });
    for destination in [Destination::Applications, Destination::ServiceMap] {
        cx.update(|cx| page.update(cx, |page, _| page.destination = destination));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let start = std::time::Instant::now();
            for _ in 0..10 {
                window.render_frame(cx);
            }
            eprintln!(
                "Coroot {:?}: ten headless frames {:?}",
                destination,
                start.elapsed()
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn reopen_and_explicit_refresh_advance_the_window_but_navigation_keeps_it(cx: &mut TestAppContext) {
    let (_runtime, _handle, page) = mount(cx, false);
    let original = cx.update(|cx| page.read(cx).live.range);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(3600));
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.open(Destination::ServiceMap, cx);
            assert_eq!(
                page.live.range, original,
                "inspect the same captured window across destinations"
            );
            let generation = page.live.generation;
            page.refresh_current(cx);
            assert_eq!(
                page.live.range.to.unwrap() - original.to.unwrap(),
                chrono::Duration::hours(1)
            );
            assert_eq!(
                page.live.range.to.unwrap() - page.live.range.from.unwrap(),
                chrono::Duration::hours(3)
            );
            assert!(page.live.generation > generation);
            page.set_visible(false, cx);
        })
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(120));
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_visible(true, cx);
            assert_eq!(
                page.live.range.to.unwrap() - original.to.unwrap(),
                chrono::Duration::seconds(3720)
            );
        })
    });
}

impl ObservabilityPage {
    /// Configure a sanitized Coroot subject for the shell's navigation regression.
    pub(crate) fn fixture_link(
        &mut self,
        access: String,
        namespace: &str,
        name: &str,
        cx: &mut gpui_kit::Context<Self>,
    ) -> super::ObservabilityEvent {
        use freshkube_core::coroot as api;
        let provider =
            api::Provider::new("https://coroot.example.com", api::Credentials::None).unwrap();
        let source = provider
            .source(&api::ProjectInfo {
                id: "fixture-project".into(),
                name: "Fixture".into(),
            })
            .with_association(Some(api::Association::new(
                access.clone(),
                "fixture".into(),
            )));
        let app = api::AppId::new(format!("fixture:{namespace}:Pod:{name}"));
        let subject = api::ObjectSubject::for_app(&source, &app).unwrap();
        self.live.access = Some(access);
        self.live.source = Some(source.clone());
        self.live.provider = Some(provider);
        self.open_app(app.clone(), Report::Net, cx);
        super::ObservabilityEvent::OpenObject {
            source,
            app,
            subject: Box::new(subject),
        }
    }
}

pub(super) fn large_map() -> freshkube_core::coroot::ServiceMap {
    let seed = example::map().nodes[0].clone();
    let nodes: Vec<_> = (0..120)
        .map(|ix| {
            let mut node = seed.clone();
            node.id = freshkube_core::coroot::AppId::new(format!("c:ns:Deployment:node-{ix:03}"));
            node
        })
        .collect();
    let seed = example::map().edges[0].clone();
    let edges = (0..300)
        .map(|ix| {
            let mut edge = seed.clone();
            edge.from = nodes[ix % 120].id.clone();
            edge.to = nodes[(ix / 120 + ix + 1) % 120].id.clone();
            edge
        })
        .collect();
    freshkube_core::coroot::ServiceMap { nodes, edges }
}

#[gpui_kit::test]
fn applications_categories_and_filter_count_follow_the_visible_scope(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).active_categories.as_ref(),
            &std::collections::BTreeSet::from(["application".to_owned()])
        );
        assert_eq!(page.read(cx).shown_apps, 7);
        assert_eq!(page.read(cx).app_count, "7 apps");
        assert!(
            window
                .find("obs-filter-problems")
                .path()
                .contains(&gpui_kit::ElementId::from("obs-view"))
        );
        assert!(
            window
                .find("obs-category-application")
                .path()
                .contains(&gpui_kit::ElementId::from("obs-categories"))
        );
        window.click("obs-filter-all", cx);
        assert!(page.read(cx).shown_apps > 7);
        window.click("obs-category-control-plane", cx);
        assert!(page.read(cx).active_categories.contains("control-plane"));
        assert!(page.read(cx).active_categories.contains("application"));
        window.click("obs-filter", cx);
        window.input("kube-apiserver", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(page.read(cx).shown_apps, 1);
        assert_eq!(page.read(cx).app_count, "1 app");
        assert_eq!(page.read(cx).counts[1], 1);
        assert!(
            matches!(&page.read(cx).matrix[0], MatrixRow::Group { label, summary, .. }
            if label == "kube-system" && summary == "1 app")
        );
        window.click("obs-category-control-plane", cx);
        assert_eq!(page.read(cx).shown_apps, 0);
    })
    .unwrap();
}

#[gpui_kit::test]
fn page_time_picker_preserves_every_range_and_set_range_path(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    for (ix, hours) in [1, 3, 24, 168].into_iter().enumerate() {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("obs-time", cx);
            for _ in 0..=ix {
                window.press("down", cx);
            }
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(page.read(cx).hours(), hours));
    }
}

#[gpui_kit::test]
fn every_observability_destination_keeps_its_shared_header(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    for destination in Destination::NAVIGATION
        .into_iter()
        .chain([Destination::Application])
    {
        cx.update(|cx| page.update(cx, |page, cx| page.open(destination, cx)));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("obs-title").visible());
            assert!(window.find("obs-time").visible());
            assert!(window.find("obs-refresh").visible());
            let page_id = gpui_kit::ElementId::from("observability-page");
            let refreshes = gpui_kit::base::test_support::snapshots(window)
                .into_iter()
                .filter(|element| {
                    element.path().contains(&page_id)
                        && element.role() == Some(gpui_kit::Role::Button)
                        && element
                            .label()
                            .is_some_and(|label| label.starts_with("Refresh"))
                })
                .count();
            assert_eq!(
                refreshes, 1,
                "each Observability destination has one page refresh"
            );
            assert_eq!(window.find("obs-source").label(), Some("Example data"));
            assert!(
                window
                    .find("obs-source")
                    .path()
                    .contains(&gpui_kit::ElementId::from("obs-scope")),
                "the source belongs to the shared meta line"
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn applications_loading_failure_refusal_and_stale_keep_distinct_surfaces(cx: &mut TestAppContext) {
    use freshkube_core::coroot as api;
    let (_runtime, handle, page) = mount(cx, true);
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
            page.applications.clear();
            page.project();
            page.live.apps.begin(
                page.live
                    .identity(super::connection::Subject::Applications)
                    .unwrap(),
            );
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-loading").visible());
        assert!(window.find("obs-title").visible());
        assert!(window.try_find("obs-failed").is_none());
    })
    .unwrap();
    for (error, surface) in [
        (api::ReadError::Refused, "obs-refused"),
        (api::ReadError::Missing, "obs-failed"),
    ] {
        cx.update(|cx| {
            page.update(cx, |page, _| {
                let request = page.live.apps.begin(
                    page.live
                        .identity(super::connection::Subject::Applications)
                        .unwrap(),
                );
                page.live.apps.apply(&request, Err(error.to_string()));
                page.live.capabilities[0] = api::Capability::Unavailable(error);
            })
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find(surface).visible());
            assert!(window.find("obs-retry").visible());
            assert!(window.try_find("obs-loading").is_none());
        })
        .unwrap();
    }
    cx.update(|cx| {
        page.update(cx, |page, _| {
            let identity = page
                .live
                .identity(super::connection::Subject::Applications)
                .unwrap();
            let request = page.live.apps.begin(identity.clone());
            let apps = example::applications();
            page.live.apps.apply(&request, Ok(apps.clone()));
            page.apply_applications(&apps);
            let request = page.live.apps.begin(identity);
            page.live
                .apps
                .apply(&request, Err("Coroot could not be reached.".into()));
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-stale").visible());
        assert_eq!(page.read(cx).shown_apps, 7);
        assert!(page.read(cx).live.apps.last_successful().is_some());
        assert!(window.try_find("obs-failed").is_none());
        window.click("obs-retry", cx);
        assert!(page.read(cx).live.apps.data().is_some());
        assert!(page.read(cx).live.apps.last_successful().is_some());
    })
    .unwrap();
}

#[test]
fn application_report_values_preserve_missing_and_healthy_states_without_glyphs() {
    use super::Status;
    let applications = super::projection::applications(&example::applications());
    let worker = applications
        .iter()
        .find(|app| app.key == example::WORKER)
        .unwrap();
    assert_eq!(worker.label.as_ref(), "payments / worker · Deployment");
    let missing = worker.check(Report::DiskIo);
    assert_eq!(missing.value.as_ref(), "—");
    assert_eq!(missing.status, Status::Absent);
    assert!(missing.status.report_tone().is_none());
    let unknown = worker.check(Report::Errors);
    assert!(unknown.value.is_empty());
    assert_eq!(unknown.status, Status::Unknown);
    assert_eq!(unknown.status.report_tone(), Some(super::Tone::Unknown));
    let healthy = worker.check(Report::Cpu);
    assert_eq!(healthy.value.as_ref(), "ok");
    assert_eq!(healthy.status, Status::Ok);
    assert!(healthy.status.report_tone().is_none());
    assert!(
        missing
            .tooltip
            .starts_with("payments / worker · Deployment")
    );
    assert!(!missing.tooltip.contains("fixture:"));
    assert_eq!(
        worker.check(Report::Net).status.report_tone(),
        Some(super::Tone::Crit)
    );
    assert_eq!(
        worker.check(Report::Logs).status.report_tone(),
        Some(super::Tone::Warn)
    );
}

#[gpui_kit::test]
fn application_cells_expose_missing_unknown_and_healthy_values(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-filter", cx);
        window.input("worker", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let worker = page
            .read(cx)
            .applications
            .iter()
            .find(|app| app.key == example::WORKER)
            .unwrap();
        for (report, label) in [
            (Report::DiskIo, "Disk I/O: not reported"),
            (Report::Errors, "Errors: unknown"),
            (Report::Cpu, "CPU: healthy"),
            (Report::Net, "Net: critical · refused"),
        ] {
            assert_eq!(
                window.find(worker.check(report).element_id.clone()).label(),
                Some(label)
            );
        }
        let p = crate::palette::palette(cx);
        let healthy: gpui_kit::Background = p.good.into();
        let side = crate::ui::dp_px(8., window).scale(window.scale_factor()).0;
        for report in [Report::Cpu, Report::DiskIo] {
            let bounds = window
                .find(worker.check(report).element_id.clone())
                .bounds()
                .scale(window.scale_factor());
            assert!(
                !window.painted_quads().iter().any(|quad| {
                    bounds.contains(&quad.bounds.center())
                        && (quad.bounds.size.width.0 - side).abs() < 0.5
                        && (quad.bounds.size.height.0 - side).abs() < 0.5
                        && (quad.background == healthy || quad.border_color == p.unk_ink)
                }),
                "healthy and unreported cells must not paint status dots: {report:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn unknown_tally_uses_the_same_circle_as_unknown_rows_and_filters_apps(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, _| {
            let mut apps = example::applications();
            let unknown = apps
                .iter_mut()
                .find(|app| app.id == example::id("payments/checkout"))
                .unwrap();
            unknown.status = freshkube_core::coroot::Status::Unknown;
            page.apply_applications(&apps);
        });
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let chip = window.find("obs-tally-unknown");
        assert_eq!(chip.label(), Some("1 Unknown"));
        // The chip and the unknown rows both draw `ui::status_glyph(Tone::Unknown)`,
        // the dashed ring whose drawing and size freshkube-ui's tests check. The
        // glyph is a sprite, so the chip paints no outline of its own.
        let chip_bounds = chip.bounds().scale(window.scale_factor());
        let unknown = crate::palette::palette(cx).unk_ink;
        assert!(
            !window.painted_quads().into_iter().any(|quad| {
                quad.border_color == unknown && chip_bounds.contains(&quad.bounds.center())
            }),
            "the Unknown chip draws the shared glyph, not an outline of its own"
        );
        window.click("obs-tally-unknown", cx);
        assert_eq!(page.read(cx).filter, super::Filter::Unknown);
        assert_eq!(page.read(cx).shown_apps, 1);
        window.click("obs-tally-unknown", cx);
        assert_eq!(page.read(cx).filter, super::Filter::All);
        assert!(page.read(cx).shown_apps > 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn applications_live_header_controls_fit_a_narrow_page_at_large_text(cx: &mut TestAppContext) {
    use freshkube_core::coroot as api;
    let (_runtime, handle, page) = mount_geometry(cx, 760., 560.);
    cx.update(|cx| {
        page.update(cx, |page, _| {
            page.fixture = false;
            page.live.visible = false;
            let provider = api::Provider::new(
                "https://coroot-header-geometry.invalid",
                api::Credentials::None,
            )
            .unwrap();
            let project = api::ProjectInfo {
                id: "geometry".into(),
                name: "Project with a long display name".into(),
            };
            page.live.source = Some(provider.source(&project));
            page.live.provider = Some(provider);
            page.live.project_label = project.name.clone();
            page.live.project_labels = vec![project.name.clone()];
            page.live.projects = vec![project];
        });
    });
    cx.update_window(handle, |_, _, cx| crate::text_size::set(20., cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        settle_header(window, cx);
        let frame = window.find("obs-frame").bounds();
        let padding = crate::ui::dp_px(freshkube_ui::page::PAGE_PADDING, window);
        for id in [
            "obs-source",
            "obs-project",
            "obs-namespace",
            "obs-density",
            "obs-columns",
            "obs-time",
            "obs-refresh",
        ] {
            let control = window.find(id);
            let bounds = control.bounds();
            assert!(control.visible(), "{id} must remain visible");
            assert!(bounds.size.width > px(1.), "{id} must retain its width");
            assert!(
                bounds.left() >= frame.left() + padding - px(0.5)
                    && bounds.right() <= frame.right() - padding + px(0.5),
                "{id} must stay within the page padding: {bounds:?}; frame {frame:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn applications_secondary_header_fits_actual_desktop_widths(cx: &mut TestAppContext) {
    for (width, height, text_size) in [
        (1280., 880., 13.),
        (1280., 880., 14.),
        (760., 560., 20.),
        (1920., 880., 13.),
    ] {
        let (_runtime, handle, _page) = mount_geometry(cx, width, height);
        cx.update_window(handle, |_, _, cx| crate::text_size::set(text_size, cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            settle_header(window, cx);
            let frame = window.find("obs-frame").bounds();
            let title = window.find("obs-title").bounds();
            let secondary = window.find("obs-secondary").bounds();
            let controls = window.find("obs-controls").bounds();
            let scope = window.find("obs-scope").bounds();
            let padding = crate::ui::dp_px(freshkube_ui::page::PAGE_PADDING, window);
            assert!(
                secondary.top() >= title.bottom(),
                "categories belong below the title"
            );
            assert!(
                (secondary.left() - (frame.left() + padding)).abs() < px(0.5),
                "categories stay left-aligned"
            );
            assert!(
                scope.top() >= secondary.bottom() && scope.top() >= controls.bottom(),
                "metadata follows the header rows"
            );
            let mut previous = None;
            for id in [
                "obs-filter",
                "obs-filter-problems",
                "obs-filter-all",
                "obs-tally-critical",
                "obs-tally-warning",
                "obs-tally-unknown",
                "obs-tally-ok",
                "obs-tally-logs",
                "obs-category-application",
                "obs-category-control-plane",
                "obs-category-monitoring",
                "obs-more-categories",
                "obs-namespace",
                "obs-density",
                "obs-columns",
                "obs-time",
                "obs-refresh",
            ] {
                let element = window.find(id);
                let bounds = element.bounds();
                assert!(
                    element.visible(),
                    "{id} must remain visible at {width}/{text_size}"
                );
                assert!(bounds.size.width > px(1.), "{id} keeps its width");
                assert!(
                    bounds.left() >= frame.left() + padding - px(0.5)
                        && bounds.right() <= frame.right() - padding + px(0.5),
                    "{id} leaves the page at {width}/{text_size}: {bounds:?}; {frame:?}"
                );
                if matches!(
                    id,
                    "obs-namespace" | "obs-density" | "obs-columns" | "obs-time" | "obs-refresh"
                ) {
                    if let Some(prior) = previous {
                        let prior: gpui_kit::Bounds<gpui_kit::Pixels> = prior;
                        assert!(
                            bounds.top() >= prior.bottom() || bounds.left() >= prior.right(),
                            "controls overlap at {width}/{text_size}"
                        );
                    }
                    previous = Some(bounds);
                }
            }
            if width == 1920. {
                assert!(
                    controls.top() < title.bottom(),
                    "controls stay on the title row when there is room"
                );
            } else if text_size > 13. {
                assert!(
                    controls.top() >= secondary.bottom(),
                    "narrow controls follow the categories"
                );
            } else if controls.top() < title.bottom() {
                assert!(
                    (controls.right() - (frame.right() - padding)).abs() < px(0.5),
                    "fitting controls align with the page's right edge"
                );
            } else {
                assert!(
                    controls.top() >= secondary.bottom() || controls.left() >= secondary.right(),
                    "controls cannot overlap categories"
                );
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn applications_uses_the_pods_frame_and_table_at_both_text_sizes(cx: &mut TestAppContext) {
    use crate::desktop::layout_check::{TablePage, assert_table_page};
    let (_runtime, handle, _page) = mount(cx, true);
    let table = TablePage {
        page: "obs-frame",
        title: "obs-title",
        title_text: "Applications",
        table: "obs-applications-table-scroll",
        list: "obs-applications-list",
        density: "obs-density",
    };
    for text_size in [crate::ui::BASE_TEXT, 20.] {
        cx.update_window(handle, |_, _, cx| crate::text_size::set(text_size, cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let layout = assert_table_page(window, cx, &table);
            assert!(
                layout.group.is_some(),
                "namespace groups use the selected row density: {layout:#?}"
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn applications_show_all_and_columns_keep_the_same_projection(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let problems = page.read(cx).shown_apps;
        assert!(window.find("obs-applications-collapsed").visible());
        let total = page.read(cx).counts[1];
        assert!(total > problems);
        assert_eq!(
            window.find("obs-show-all").label(),
            Some(format!("Show all {total}").as_str()),
            "Show all names the count, as Pods' does"
        );
        window.click("obs-show-all", cx);
        assert!(page.read(cx).shown_apps > problems);
        window.render_frame(cx);
        assert!(window.try_find("obs-applications-collapsed").is_none());
        let shown = page.read(cx).shown_apps;
        let width = page.read(cx).application_width;
        window.click("obs-columns", cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert!(
            page.read(cx)
                .hidden_application_columns
                .contains(&super::application_columns::ColumnKind::Type)
        );
        assert!(page.read(cx).application_width < width);
        assert_eq!(page.read(cx).shown_apps, shown);
        window.render_frame(cx);
        let table_id = gpui_kit::ElementId::from("obs-applications-table");
        assert!(
            !gpui_kit::base::test_support::snapshots(window)
                .iter()
                .any(|element| {
                    element.path().contains(&table_id)
                        && element.role() == Some(gpui_kit::Role::ColumnHeader)
                        && element.label() == Some("Type")
                })
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn applications_empty_filter_has_a_clear_action_and_empty_observations_do_not(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-filter", cx);
        window.input("no-such-application", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-applications-empty").visible());
        assert_eq!(page.read(cx).shown_apps, 0);
        window.click("obs-clear-filters", cx);
        assert!(page.read(cx).shown_apps > 0);
        assert!(page.read(cx).query_text.is_empty());
        assert_eq!(page.read(cx).filter, super::Filter::All);
        page.update(cx, |page, _| page.apply_applications(&[]));
        window.render_frame(cx);
        assert!(window.find("obs-applications-empty").visible());
        assert!(window.try_find("obs-clear-filters").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn applications_table_fits_short_results_and_caps_long_results(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let short = page.read(cx).matrix.len();
        assert!(short < 16);
        assert_eq!(
            window.find("obs-applications-list").bounds().size.height,
            crate::ui::dp_px(short as f32 * 34., window)
        );
        window.click("obs-show-all", cx);
        window.render_frame(cx);
        assert!(page.read(cx).matrix.len() > 16);
        assert_eq!(
            window.find("obs-applications-list").bounds().size.height,
            crate::ui::dp_px(16. * 34., window)
        );
        window.click("obs-density", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("obs-applications-list").bounds().size.height,
            crate::ui::dp_px(16. * 26., window)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn categories_fall_back_to_all_and_keep_new_categories_after_all_is_chosen(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, _| {
            let mut apps = example::applications();
            apps.iter_mut()
                .for_each(|app| app.category = "infrastructure".into());
            page.categories = Default::default();
            page.category_defaults_pending = true;
            page.apply_applications(&apps);
            assert!(page.all_categories);
            assert!(page.shown_apps > 0);
            page.all_categories = false;
            page.active_categories = std::rc::Rc::new(["infrastructure".into()].into());
            page.project_filters();
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-more-categories", cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert!(page.read(cx).all_categories);
        page.update(cx, |page, _| {
            let mut apps = example::applications();
            apps.iter_mut()
                .for_each(|app| app.category = "new-category".into());
            page.apply_applications(&apps);
            assert!(
                page.shown_apps > 0,
                "all includes categories discovered later"
            );
            page.all_categories = false;
            page.project_filters();
            assert_eq!(
                page.shown_apps, 0,
                "an explicit category selection stays explicit"
            );
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn filtering_applications_resets_the_uniform_list_to_the_first_row(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, _| {
            let seed = example::applications()[0].clone();
            let apps: Vec<_> = (0..100)
                .map(|ix| {
                    let mut app = seed.clone();
                    app.id =
                        freshkube_core::coroot::AppId::new(format!("c:ns:Deployment:app-{ix:03}"));
                    app.status = if ix < 50 {
                        freshkube_core::coroot::Status::Critical
                    } else {
                        freshkube_core::coroot::Status::Warning
                    };
                    app
                })
                .collect();
            page.filter = super::Filter::All;
            page.apply_applications(&apps);
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-name-c:ns:Deployment:app-000").visible());
        page.read(cx)
            .application_table
            .scroll
            .scroll_to_item_strict(20, gpui_kit::ScrollStrategy::Top);
        window.render_frame(cx);
        assert!(
            window
                .try_find("obs-name-c:ns:Deployment:app-000")
                .is_none()
        );
        window.click("obs-tally-critical", cx);
        window.render_frame(cx);
        assert_eq!(page.read(cx).shown_apps, 50);
        assert!(window.find("obs-name-c:ns:Deployment:app-000").visible());
        page.read(cx)
            .application_table
            .scroll
            .scroll_to_item_strict(20, gpui_kit::ScrollStrategy::Top);
        window.render_frame(cx);
        assert!(
            window
                .try_find("obs-name-c:ns:Deployment:app-000")
                .is_none()
        );
        window.click("obs-filter", cx);
        window.input("app-", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(page.read(cx).shown_apps, 50);
        assert!(window.find("obs-name-c:ns:Deployment:app-000").visible());
    })
    .unwrap();
}

/// Measure independently from the cached column builder, in physical pixels.
fn shaped_width(
    window: &gpui_kit::Window,
    value: &str,
    face: gpui_kit::Font,
    size: f32,
) -> gpui_kit::Pixels {
    let run = gpui_kit::TextRun {
        len: value.len(),
        font: face,
        color: Default::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(value.to_owned().into(), px(size), &[run], None)
        .width
}

#[gpui_kit::test]
fn application_captions_and_problem_values_fit_their_cells_after_text_size_changes(
    cx: &mut TestAppContext,
) {
    use super::application_columns::ColumnKind;
    use freshkube_ui::table::TableColumn;
    let (_runtime, handle, page) = mount_size(cx, true, 1280., 880.);
    for base in [13., 14., 20., 13.] {
        cx.update(|cx| crate::text_size::set(base, cx));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let scale = base / crate::ui::BASE_TEXT;
            let mut face = gpui_kit::font(Theme::global(cx).font_family.clone());
            face.weight = crate::ui::HEADING_WEIGHT;
            for (ix, column) in page.read(cx).application_columns.iter().enumerate() {
                if column.label().is_empty() {
                    continue;
                }
                let header =
                    window.find((gpui_kit::SharedString::from("obs-applications-sort"), ix));
                let caption = shaped_width(
                    window,
                    &column.label().to_uppercase(),
                    face.clone(),
                    11. * scale,
                );
                assert!(
                    header.bounds().size.width >= caption + px(19.5 * scale),
                    "{} caption must fit at {base}: {:?}, text {caption:?}",
                    column.label(),
                    header.bounds()
                );
                if let ColumnKind::Report(report) = column.kind {
                    for app in &page.read(cx).applications {
                        let check = app.check(report);
                        if let Some(button) = window.try_find(check.element_id.clone()) {
                            let value = shaped_width(
                                window,
                                &check.value,
                                gpui_kit::font(crate::ui::MONO_FONT),
                                12. * scale,
                            );
                            let glyph = if check.status.report_tone().is_some() {
                                px(14. * scale)
                            } else {
                                px(0.)
                            };
                            assert!(
                                button.bounds().size.width >= value + glyph,
                                "{} {} must fit at {base}: {:?}, text {value:?}, glyph {glyph:?}",
                                app.name,
                                report.label(),
                                button.bounds()
                            );
                        }
                    }
                }
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn applications_sideways_scroll_reaches_the_last_report_and_opens_it(cx: &mut TestAppContext) {
    // This harness mounts only the page: subtract the shell's rail and column
    // to give its table the same viewport as the real 1280-wide desktop.
    let width = 1280. - crate::desktop::RAIL_WIDTH - crate::desktop::COLUMN_WIDTH;
    let (_runtime, handle, page) = mount_size(cx, true, width, 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let card = window.find("obs-applications-table-scroll").bounds();
        assert!(
            window
                .find("obs-check-fixture:payments:Deployment:worker-logs")
                .bounds()
                .right()
                > card.right(),
            "the last report must begin clipped, so the scroll proves reachability"
        );
        window.scroll(
            "obs-applications-table-scroll",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(-2000.), px(0.))),
            cx,
        );
        let report = window.find("obs-check-fixture:payments:Deployment:worker-logs");
        let card = window.find("obs-applications-table-scroll").bounds();
        assert!(report.visible());
        assert!(report.bounds().left() >= card.left() && report.bounds().right() <= card.right());
        window.click("obs-check-fixture:payments:Deployment:worker-logs", cx);
        assert_eq!(page.read(cx).destination, Destination::Application);
        assert_eq!(page.read(cx).report, Report::Logs);
    })
    .unwrap();
}

#[test]
fn upstream_count_preserves_failing_names_and_state_in_its_details() {
    use freshkube_core::coroot as api;
    let mut raw = example::applications();
    raw[0].signals.insert(
        "upstreams".into(),
        api::Signal {
            status: api::Status::Critical,
            value: "worker, ledger-db".into(),
        },
    );
    let applications = super::projection::applications(&raw);
    let app = applications.iter().find(|app| app.id == raw[0].id).unwrap();
    let check = app.check(Report::Upstreams);
    assert_eq!(check.value.as_ref(), "2");
    assert!(check.tooltip.contains("worker, ledger-db"));
    assert!(check.tooltip.contains("Critical"));
    assert!(check.label.contains("worker, ledger-db"));
    assert!(check.label.contains("critical"));
}

#[gpui_kit::test]
fn application_namespace_select_searches_and_applies_the_chosen_namespace(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-namespace", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.input("cache", cx);
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(110));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(page.read(cx).namespace.as_deref(), Some("cache"));
        assert_eq!(page.read(cx).shown_apps, 1);
    })
    .unwrap();
}
