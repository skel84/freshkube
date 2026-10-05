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
        let chip_bounds = chip.bounds().scale(window.scale_factor());
        let unknown = crate::palette::palette(cx).unk_ink;
        let mut glyphs: Vec<_> = window
            .painted_quads()
            .into_iter()
            .filter(|quad| {
                quad.border_color == unknown && chip_bounds.contains(&quad.bounds.center())
            })
            .collect();
        // GPUI splits an outline into clipped strips that share its geometry.
        glyphs.dedup_by(|a, b| a.bounds == b.bounds && a.corner_radii == b.corner_radii);
        assert_eq!(glyphs.len(), 1, "Unknown needs its outlined circle");
        let glyph = &glyphs[0];
        let side = crate::ui::dp_px(8., window).scale(window.scale_factor()).0;
        assert!((glyph.bounds.size.width.0 - side).abs() < 0.5);
        assert!((glyph.bounds.size.height.0 - side).abs() < 0.5);
        assert!(
            glyph.corner_radii.top_left.0 >= side / 2.,
            "unknown must keep its circular glyph"
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
    let (_runtime, handle, page) = mount_size(cx, true, 760., 560.);
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
        window.render_frame(cx);
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

#[gpui_kit::test]
fn default_applications_columns_fit_the_1280_page_and_keep_reports_clickable(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, page) = mount_size(cx, true, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            page.read(cx).application_width <= crate::screens::content_width(window),
            "minimum columns {} must fit content {}",
            page.read(cx).application_width,
            crate::screens::content_width(window)
        );
        window.click("obs-check-fixture:payments:Deployment:worker-net", cx);
        assert_eq!(page.read(cx).destination, Destination::Application);
        assert_eq!(page.read(cx).report, Report::Net);
    })
    .unwrap();
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
        assert_eq!(page.read(cx).namespace.as_deref(), Some("cache"));
        assert_eq!(page.read(cx).shown_apps, 1);
    })
    .unwrap();
}
