use super::*;

#[gpui_kit::test]
fn log_tabs_keep_their_streams_on_every_page_and_minimized(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    let api = objects(&pilot, "deployments.apps", |_, name| name == "api", cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    open_logs(handle, &pilot, "deployments.apps", &api[0], cx);
    let pod = pod_view(&dock, 0, cx);
    let workload = workload_view(&dock, 1, cx);
    let open = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            (
                pod.read(cx).source().stream_open(),
                workload.read(cx).source().reads_open(),
            )
        })
    };
    let read = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            (
                pod.read(cx).retained().len(),
                workload.read(cx).retained().len(),
            )
        })
    };
    // The workload's feed opens with its first example line.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(3));
    cx.run_until_parked();
    assert_eq!(open(cx), (true, true));

    // Every page hides and shows the others; no page's hide reaches a tab.
    for page in Page::ALL {
        let before = read(cx);
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| pilot.navigate(page, window, cx))
        })
        .unwrap();
        cx.run_until_parked();
        draw(handle, cx);
        assert_eq!(open(cx), (true, true), "on {page:?}");
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(3));
        cx.run_until_parked();
        let after = read(cx);
        assert!(after.0 > before.0 && after.1 > before.1, "read on {page:?}");
    }

    // Minimized to its tabs, the dock reads on.
    cx.update_window(handle, |_, window, cx| window.click("dock-toggle", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    let before = read(cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(3));
    cx.run_until_parked();
    assert_eq!(open(cx), (true, true));
    let after = read(cx);
    assert!(after.0 > before.0 && after.1 > before.1, "read minimized");
}

#[gpui_kit::test]
fn a_tab_reads_across_pages_and_objects_until_it_closes(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    let view = pod_view(&dock, 0, cx);
    assert!(cx.update(|cx| view.read(cx).streaming()));

    // Another object in the pane, and another page, keep it.
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                builtin("pods").unwrap(),
                pods[1].clone().into(),
                Tab::Overview,
                window,
                cx,
            );
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Overview, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert!(cx.update(|cx| view.read(cx).streaming()));
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("dock").visible(), "the dock spans every page");
    })
    .unwrap();

    // The status bar minimizes it to its tabs, which keep reading, and
    // opens it again.
    cx.update_window(handle, |_, window, cx| window.click("dock-toggle", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert!(cx.update(|cx| view.read(cx).streaming()));
    cx.update_window(handle, |_, window, cx| {
        assert!(
            window.find("dock-tab-0").visible(),
            "minimized, its tabs show"
        );
        assert!(window.try_find("dock-tab-body").is_none());
        window.click("dock-toggle", cx);
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    // A middle-click closes a tab and drops its stream.
    let weak = view.downgrade();
    drop(view);
    cx.update_window(handle, |_, window, cx| {
        use gpui_kit::InputEvent as _;
        let position = window.find("dock-tab-0").bounds().center();
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Middle,
                position,
                click_count: 1,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert!(weak.upgrade().is_none(), "the closed tab's view is dropped");
    draw(handle, cx);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("dock").is_none());
        assert!(window.try_find("dock-toggle").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_gone_pod_stops_reading_and_one_made_again_with_its_name_takes_a_new_tab(
    cx: &mut TestAppContext,
) {
    use crate::logs::PodLogPanel as _;
    use freshkube_core::resources::WorkloadPods;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pod = running_pods(&pilot, cx).remove(0);
    open_logs(handle, &pilot, "pods", &pod, cx);
    let view = pod_view(&dock, 0, cx);
    assert!(cx.update(|cx| view.read(cx).streaming()));
    // The watch's next list no longer has it.
    cx.update(|cx| {
        dock.update(cx, |dock, cx| {
            let id = dock.tabs[0].id;
            let pods = WorkloadPods {
                listed: true,
                ..WorkloadPods::default()
            };
            assert!(!dock.apply_pod(id, pods, cx), "the delivery ends");
        })
    });
    cx.run_until_parked();
    assert!(!cx.update(|cx| view.read(cx).streaming()));
    // Showing it again reads nothing by its name.
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| {
            dock.set_open(false, window, cx);
            dock.set_open(true, window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!cx.update(|cx| view.read(cx).streaming()));
    draw(handle, cx);
    cx.update_window(handle, |_, window, _| {
        assert!(window.within("dock").find("dock-tab-notice").visible());
    })
    .unwrap();
    // The same name with another UID is another pod.
    let mut again = pod.clone();
    again.uid = "00000000-0000-4000-8000-000000000099".into();
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| {
            let target = crate::resources::detail::DetailTarget {
                identity: again,
                kind: builtin("pods").unwrap(),
            };
            dock.open_logs(
                crate::resources::LogsRequest { target, at: None },
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        titles(&dock, cx),
        [format!("Pod {}", pod.name), format!("Pod {} (2)", pod.name)]
    );
}

#[gpui_kit::test]
fn a_pod_tab_asked_for_by_name_keeps_the_first_pod_it_finds(cx: &mut TestAppContext) {
    use freshkube_core::resources::{WorkloadPod, WorkloadPods};
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let mut pod = running_pods(&pilot, cx).remove(0);
    let uid = std::mem::take(&mut pod.uid);
    open_logs(handle, &pilot, "pods", &pod, cx);
    let listed = |uid: &str| WorkloadPods {
        listed: true,
        pods: vec![WorkloadPod {
            name: pod.name.clone(),
            uid: uid.to_owned(),
            ..WorkloadPod::default()
        }],
        ..WorkloadPods::default()
    };
    cx.update(|cx| {
        dock.update(cx, |dock, cx| {
            let id = dock.tabs[0].id;
            assert!(dock.apply_pod(id, listed(&uid), cx));
            assert_eq!(dock.tabs[0].target.identity.uid, uid);
            // The pod is made again with its name: the tab's pod is gone.
            assert!(!dock.apply_pod(id, listed("00000000-0000-4000-8000-000000000099"), cx));
            assert_eq!(dock.tabs[0].feed.state, super::super::feed::FeedState::Gone);
        })
    });
    // The new pod takes a tab of its own.
    pod.uid = "00000000-0000-4000-8000-000000000099".into();
    open_logs(handle, &pilot, "pods", &pod, cx);
    assert_eq!(titles(&dock, cx).len(), 2);
}

/// A pod asked for by name that is already gone at its first listing
/// never takes a later pod of the name.
#[gpui_kit::test]
fn a_pod_tab_gone_at_its_first_listing_keeps_no_later_pod(cx: &mut TestAppContext) {
    use crate::resources::LogsRequest;
    use crate::resources::detail::DetailTarget;
    use freshkube_core::resources::WorkloadPods;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let mut pod = running_pods(&pilot, cx).remove(0);
    let open = |identity: ResourceIdentity, cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            dock.update(cx, |dock, cx| {
                let target = DetailTarget {
                    identity,
                    kind: builtin("pods").unwrap(),
                };
                dock.open_logs(LogsRequest { target, at: None }, window, cx)
            })
        })
        .unwrap();
        cx.run_until_parked();
    };
    // By name alone, as a restored tab is.
    pod.uid.clear();
    open(pod.clone(), cx);
    cx.update(|cx| {
        dock.update(cx, |dock, cx| {
            assert!(dock.tabs[0].key.wildcard);
            let id = dock.tabs[0].id;
            let none = WorkloadPods {
                listed: true,
                ..WorkloadPods::default()
            };
            assert!(!dock.apply_pod(id, none, cx));
            assert_eq!(dock.tabs[0].feed.state, super::super::feed::FeedState::Gone);
        })
    });
    // A pod made with the name later takes a tab of its own.
    pod.uid = "00000000-0000-4000-8000-000000000099".into();
    open(pod, cx);
    assert_eq!(titles(&dock, cx).len(), 2);
}
