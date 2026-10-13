use super::*;
use freshkube_ui::real_io::RealIo;

async fn traced(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    page: &gpui_kit::Entity<ObservabilityPage>,
) {
    let page = page.clone();
    cx.wait_for_io(handle, move |_, cx| {
        let live = &page.read(cx).live;
        !live.tracing.is_loading() && !live.trace.is_loading()
    })
    .await;
}

#[gpui_kit::test]
async fn traces_list_requests_open_a_trace_and_narrow_to_a_cell(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    connected(cx, handle, &page, server.url.clone()).await;
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            let app = page.applications[0].id.clone();
            page.open_app(app, Report::Cpu, cx);
        });
        window.render_frame(cx);
        window.click("obs-app-traces", cx);
    })
    .unwrap();
    traced(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        let current = page.read(cx);
        assert_eq!(current.destination, Destination::Traces);
        assert_eq!(current.live.trace.data().unwrap().spans.len(), 2);
        window.render_frame(cx);
        // Failed requests first, then slowest: errors, >5s, 5ms.
        assert!(window.try_find("obs-live-bucket-2-2").is_some());
        assert!(window.try_find("obs-live-span-t1-root").is_some());
        assert!(window.try_find("obs-live-trace-span-1").is_some());
        assert!(window.try_find("obs-live-trace-span-2").is_none());
        window.click("obs-live-trace-span-1", cx);
        window.render_frame(cx);
        assert!(window.try_find("obs-live-span-detail").is_some());
        window.click("obs-live-bucket-1-0", cx);
    })
    .unwrap();
    traced(cx, handle, &page).await;
    let path = last_path(&server, "/tracing");
    assert!(path.contains("0.005-inf"), "{path}");
    assert!(!path.contains("t1%3A%3A"), "{path}");

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-trace-failed", cx);
    })
    .unwrap();
    traced(cx, handle, &page).await;
    let path = last_path(&server, "/tracing");
    assert!(path.contains("inf-err"), "{path}");

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-trace-source-agent", cx);
    })
    .unwrap();
    traced(cx, handle, &page).await;
    assert!(last_path(&server, "/tracing").contains("trace=agent%3A"));

    // Hidden, the page asks nothing more.
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(false, cx)));
    let reads = server.requests.load(Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    cx.run_until_parked();
    assert_eq!(server.requests.load(Ordering::SeqCst), reads);
}

#[gpui_kit::test]
async fn profiling_draws_a_flame_graph_and_compares_with_the_window_before(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    connected(cx, handle, &page, server.url.clone()).await;
    let profiled = |page: &gpui_kit::Entity<ObservabilityPage>| {
        let page = page.clone();
        move |_: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            !page.read(cx).live.profiling.is_loading()
        }
    };
    // The picker finds an application by part of its name.
    let wanted = cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.selected_app = None;
            page.open(Destination::Profiling, cx);
            page.applications.last().unwrap().id.clone()
        })
    });
    let asked = wanted.clone();
    for act in [
        &(|window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            window.within("obs-profile-app").click("input", cx)
        }) as &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App),
        &|window, cx| window.input(wanted.name(), cx),
        &|window, cx| window.press("enter", cx),
    ] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    assert_eq!(
        cx.update(|cx| page.read(cx).selected_app.clone()),
        Some(wanted)
    );
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let app = page.applications[0].id.clone();
            page.selected_app = Some(app);
            page.refresh(cx);
        })
    });
    cx.wait_for_io(handle, profiled(&page)).await;
    let path = last_path(&server, "/profiling");
    assert!(path.contains("query=cpu"));
    // The profile asked for is the page's application's.
    assert_eq!(
        cx.read(|cx| page.read(cx).selected_app.clone()),
        Some(asked.clone())
    );
    let app = asked.as_str().replace(':', "%3A");
    assert!(path.contains(&format!("/app/{app}/profiling")), "{path}");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for ix in 0..4 {
            assert!(
                window
                    .try_find(gpui_kit::SharedString::from(format!("obs-live-flame-{ix}")))
                    .is_some()
            );
        }
        assert!(window.try_find("obs-live-increase-2").is_none());
        // A frame is a data mark: 3 px corners, like a meter (DESIGN.md).
        let scale = window.scale_factor();
        let frame = window.find("obs-live-flame-0").bounds().scale(scale);
        let corners: Vec<f32> = window
            .painted_quads()
            .into_iter()
            .filter(|quad| {
                let b = quad.bounds;
                (b.left() - frame.left()).0.abs() < 1.
                    && (b.top() - frame.top()).0.abs() < 1.
                    && (b.right() - frame.right()).0.abs() < 1.
            })
            .map(|quad| quad.corner_radii.top_left.0 / scale)
            .collect();
        assert!(
            corners.iter().any(|radius| (radius - 3.).abs() < 0.01),
            "the frame's quads round at {corners:?}"
        );
        window.click("obs-live-flame-2", cx);
        window.render_frame(cx);
        assert!(window.try_find("obs-live-frame-detail").is_some());
        window.click("obs-live-profile-compare", cx);
    })
    .unwrap();
    cx.wait_for_io(handle, profiled(&page)).await;
    let path = last_path(&server, "/profiling");
    assert!(
        path.contains("diff") && path.contains("profile_cpu"),
        "{path}"
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // main gained 30 points of the total; gc lost them.
        assert!(window.try_find("obs-live-increase-2").is_some());
        assert!(window.try_find("obs-live-increase-1").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn the_trace_pane_names_its_read_and_retries_a_failed_trace(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    connected(cx, handle, &page, server.url.clone()).await;
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            let app = page.applications[0].id.clone();
            page.open_app(app, Report::Cpu, cx);
        });
        window.render_frame(cx);
        window.click("obs-app-traces", cx);
    })
    .unwrap();
    traced(cx, handle, &page).await;
    let read = |window: &mut gpui_kit::Window| {
        window
            .try_find("obs-trace-read")
            .map(|tag| tag.label().unwrap_or_default().to_string())
    };
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-trace-detail").visible());
        assert!(window.find("obs-live-waterfall").visible());
        assert_eq!(read(window), None, "a current trace shows no read tag");

        // A refresh reads the trace again; its tag says so until it answers.
        server.status.store(500, Ordering::SeqCst);
        page.update(cx, |page, cx| page.refresh(cx));
        window.render_frame(cx);
        assert_eq!(read(window).as_deref(), Some("Reading"));
    })
    .unwrap();
    traced(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The last trace stays, marked stale, with the error and Retry.
        assert_eq!(read(window).as_deref(), Some("Stale"));
        assert!(window.find("obs-live-waterfall").visible());
        assert!(window.find("obs-trace-retry").visible());

        // Another trace has nothing to keep, so its read failed.
        window.click("obs-live-span-t2-other", cx);
    })
    .unwrap();
    traced(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(read(window).as_deref(), Some("Failed"));
        assert!(window.try_find("obs-live-waterfall").is_none());
        server.status.store(200, Ordering::SeqCst);
        window.click("obs-trace-retry", cx);
    })
    .unwrap();
    traced(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(read(window), None);
        assert!(window.try_find("obs-trace-retry").is_none());
        assert!(window.find("obs-live-waterfall").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn traces_and_profiling_first_read_failures_show_retry_and_recover(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    cx.update(|cx| {
        page.update(cx, |page, _| {
            let provider = api::Provider::new(&server.url, api::Credentials::None).unwrap();
            page.live.source = Some(provider.source(&api::ProjectInfo {
                id: "p1".into(),
                name: "Test project".into(),
            }));
            page.live.provider = Some(provider);
            let app = api::Application {
                id: api::AppId::new("cluster-a:prod:Deployment:api"),
                cluster: "Test cluster".into(),
                category: "application".into(),
                app_type: "Go".into(),
                status: api::Status::Ok,
                signals: Default::default(),
            };
            page.selected_app = Some(app.id.clone());
            page.apply_applications(&[app]);
        })
    });
    for destination in [Destination::Traces, Destination::Profiling] {
        for status in [403, 500] {
            server.status.store(status, Ordering::SeqCst);
            cx.update(|cx| {
                page.update(cx, |page, cx| {
                    page.live.tracing = Default::default();
                    page.live.profiling = Default::default();
                    page.open(destination, cx);
                })
            });
            let observed = page.clone();
            cx.wait_for_io(handle, move |_, cx| {
                let page = observed.read(cx);
                match destination {
                    Destination::Traces => page.live.tracing.error().is_some(),
                    _ => page.live.profiling.error().is_some(),
                }
            })
            .await;
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(
                    window.find("obs-failed").visible(),
                    "{destination:?} first {status}"
                );
                assert!(window.find("obs-retry").visible());
                assert!(window.try_find("obs-stale").is_none());
                server.status.store(200, Ordering::SeqCst);
                window.click("obs-retry", cx);
            })
            .unwrap();
            let observed = page.clone();
            cx.wait_for_io(handle, move |_, cx| {
                let page = observed.read(cx);
                match destination {
                    Destination::Traces => page.live.tracing.data().is_some(),
                    _ => page.live.profiling.data().is_some(),
                }
            })
            .await;
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find("obs-failed").is_none());
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
async fn heatmap_arrows_read_nothing_until_enter(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    connected(cx, handle, &page, server.url.clone()).await;
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            let app = page.applications[0].id.clone();
            page.open_app(app, Report::Cpu, cx);
        });
        window.render_frame(cx);
        window.click("obs-app-traces", cx);
    })
    .unwrap();
    traced(cx, handle, &page).await;
    let cursor = |window: &mut gpui_kit::Window| {
        window
            .try_find("obs-heatmap-cursor")
            .map(|caption| caption.label().unwrap_or_default().to_string())
    };
    let before = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                cursor(window),
                None,
                "no cursor until the heatmap has focus"
            );
            // Tab reaches the heatmap; its cursor starts on the latest failures.
            let focus = page.read(cx).heat_focus.clone();
            window.focus(&focus, cx);
            window.render_frame(cx);
            let label = cursor(window).expect("a focused heatmap shows its cursor");
            assert!(label.starts_with("Failed requests · "), "{label}");
            assert!(label.ends_with(" · no failures"), "{label}");
            assert_eq!(
                window.find("obs-live-bucket-0-2").label(),
                Some(label.as_str())
            );
            server.requests.load(Ordering::SeqCst)
        })
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        // Moves clamp at the edges and read nothing.
        window.press("up", cx);
        window.press("right", cx);
        window.press("left", cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("home", cx);
        window.press("right", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).live_traces.selection,
            api::TraceSelection::Recent
        );
        let label = cursor(window).unwrap();
        assert!(label.starts_with("Up to 5ms · "), "{label}");
        assert_eq!(
            window.find("obs-live-bucket-2-1").label(),
            Some(label.as_str())
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(server.requests.load(Ordering::SeqCst), before);
    // Enter lists the cell's requests: one read.
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    traced(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        let api::TraceSelection::Latency { above, up_to, .. } =
            page.read(cx).live_traces.selection.clone()
        else {
            panic!("a latency cell");
        };
        assert_eq!((above.as_str(), up_to.as_str()), ("0", "0.005"));
        assert!(server.requests.load(Ordering::SeqCst) > before);
        // Escape leaves the heatmap for the page.
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(!page.read(cx).heat_focus.is_focused(window));
        assert!(page.read(cx).focus.is_focused(window));
        assert_eq!(cursor(window), None);
    })
    .unwrap();
}

#[gpui_kit::test]
async fn an_application_reads_coroots_own_view(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    connected(cx, handle, &page, server.url.clone()).await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let app = page.applications[0].id.clone();
            page.open_app(app, Report::Cpu, cx);
        })
    });
    let observed = page.clone();
    cx.wait_for_io(handle, move |_, cx| {
        !observed.read(cx).live.view.is_loading()
    })
    .await;
    assert!(
        last_path(&server, "/app/").contains("/project/p1/app/cluster-a%3Aprod%3ADeployment%3Aapi")
    );
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).live.view.error().is_none());
        window.render_frame(cx);
        assert!(
            window
                .find("obs-app-map-cluster-a:prod:Deployment:db")
                .visible()
        );
        assert!(window.find("obs-report-cpu").visible());
        assert!(window.find("obs-app-checks").visible());
        assert!(window.find("obs-app-table-0-row-0").visible());
    })
    .unwrap();
}
