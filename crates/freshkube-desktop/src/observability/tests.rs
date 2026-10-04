use super::{Destination, MatrixRow, ObservabilityPage, Report, example};
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, px, size};
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
    mount_with(cx, fixture, width, height, None, None)
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
    mount_with(cx, false, 1260., 900., Some(preferences), Some(secrets))
}
fn mount_with(
    cx: &mut TestAppContext,
    fixture: bool,
    width: f32,
    height: f32,
    preferences: Option<&std::path::Path>,
    secrets: Option<crate::secrets::Secrets>,
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
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let page = page.unwrap();
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(true, cx)));
    (runtime, handle.into(), page)
}
#[gpui_kit::test]
fn applications_filter_and_cells_open_the_selected_report(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-filter-all", cx);
        assert_eq!(page.read(cx).counts[1], 47);
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
            assert!(window.try_find("obs-matrix").is_none());
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
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| page.update(cx, |page, cx| page.open(Destination::Traces, cx)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-bucket-0-18", cx);
        assert_eq!(page.read(cx).bucket, Some((0, 18)));
        window.press("right", cx);
        assert_eq!(page.read(cx).bucket, Some((0, 19)));
        window.click("obs-trace-errors", cx);
        assert!(page.read(cx).trace_errors_only);
        window.render_frame(cx);
        assert!(window.try_find("obs-span-0").is_none());
        window.click("obs-trace-clear", cx);
        assert!(page.read(cx).bucket.is_none());
        assert!(!page.read(cx).trace_errors_only);
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
fn ranges_reports_and_trace_causes_have_independent_data(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let prior = page.charts[0].series[0].values.clone();
            page.set_range(1, cx);
            assert_ne!(prior, page.charts[0].series[0].values);
            page.open_app(example::id("payments/api"), Report::Latency, cx);
            assert_eq!(page.selected_application().unwrap().key, "payments/api");
            page.open(Destination::Traces, cx);
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-error-cause-2", cx);
        assert_eq!(page.read(cx).trace_error, 2);
        assert_eq!(page.read(cx).trace_snapshot.spans[1].0, "oauth2-proxy");
        window.click("obs-bucket-7-18", cx);
        assert_eq!(page.read(cx).trace_error, 3);
        assert!(
            page.read(cx)
                .trace_snapshot
                .spans
                .iter()
                .all(|span| !span.4)
        );
    })
    .unwrap();
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
