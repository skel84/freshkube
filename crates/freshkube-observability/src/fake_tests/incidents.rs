use super::*;

#[gpui_kit::test]
async fn incidents_keep_identity_stale_evidence_and_read_only_links(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let provider = api::Provider::new(&server.url, api::Credentials::None).unwrap();
            page.live.source = Some(provider.source(&api::ProjectInfo {
                id: "p1".into(),
                name: "Production".into(),
            }));
            page.live.provider = Some(provider);
            page.open(Destination::Incidents, cx);
        })
    });
    incident_loaded(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).live.incident.data().unwrap().incident().key,
            "k1"
        );
        assert_eq!(page.read(cx).incident_count(), Some("2 sampled"));
        for id in [
            "obs-incident-mute",
            "obs-incident-fix",
            "obs-incident-traces",
        ] {
            assert!(window.try_find(id).is_none());
        }
        window.click("obs-live-incident-k2", cx);
    })
    .unwrap();
    incident_loaded(cx, handle, &page).await;
    server.incident_mode.store(1, Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    incident_loaded(cx, handle, &page).await;
    cx.update(|cx| {
        assert_eq!(
            page.read(cx).live.incident.data().unwrap().incident().key,
            "k2"
        )
    });
    server.status.store(403, Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    incident_loaded(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).live.incidents.is_stale());
        assert!(page.read(cx).live.incident.is_stale());
        window.render_frame(cx);
        window.click("obs-incident-primary-app", cx);
        assert_eq!(
            page.read(cx).destination,
            Destination::Incidents,
            "stale source cannot navigate"
        );
    })
    .unwrap();
    server.status.store(200, Ordering::SeqCst);
    cx.update_window(handle, |_, window, cx| {
        window.click("obs-incident-retry", cx)
    })
    .unwrap();
    incident_loaded(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for _ in 0..20 {
            // Wholly inside the scroll area, so the click lands on it.
            let link = window.find("obs-incident-app-cluster-a:prod:Deployment:worker");
            if link.visible()
                && link.bounds().bottom() <= window.find("obs-scroll").bounds().bottom()
            {
                break;
            }
            window.scroll(
                "obs-scroll",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                    gpui_kit::px(0.),
                    gpui_kit::px(-100.),
                )),
                cx,
            );
        }
        window.click("obs-incident-app-cluster-a:prod:Deployment:worker", cx);
        assert_eq!(
            page.read(cx).selected_app.as_ref().unwrap().as_str(),
            "cluster-a:prod:Deployment:worker"
        );
        assert_eq!(page.read(cx).destination, Destination::Application);
    })
    .unwrap();
    server.incident_mode.store(2, Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.open(Destination::Incidents, cx)));
    incident_loaded(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-incidents-empty").visible());
        assert!(page.read(cx).live.incident.data().is_none());
    })
    .unwrap();
}

async fn incident_loaded(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    page: &gpui_kit::Entity<ObservabilityPage>,
) {
    let observed = page.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        let live = &observed.read(cx).live;
        !live.incidents.is_loading() && !live.incident.is_loading()
    })
    .await;
}

#[gpui_kit::test]
async fn incidents_drop_delayed_replaced_hidden_and_project_answers(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    server.incident_mode.store(5, Ordering::SeqCst);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let provider = api::Provider::new(&server.url, api::Credentials::None).unwrap();
            page.live.source = Some(provider.source(&api::ProjectInfo {
                id: "p1".into(),
                name: "Production".into(),
            }));
            page.live.provider = Some(provider);
            page.open(Destination::Incidents, cx);
        })
    });
    let paths = server.paths.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, _| {
        paths
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.contains("/incident/k1"))
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-live-incident-k2", cx);
    })
    .unwrap();
    incident_loaded(cx, handle, &page).await;
    server.incident_gate.notify_one();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            page.read(cx).live.incident.data().unwrap().incident().key,
            "k2"
        )
    });
    let previous = server
        .paths
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.contains("/incident/k1"))
        .count();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-live-incident-k1", cx);
    })
    .unwrap();
    let paths = server.paths.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, _| {
        paths
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.contains("/incident/k1"))
            .count()
            > previous
    })
    .await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_visible(false, cx);
            assert!(page.live.jobs.is_empty());
            assert!(page.live.incident_job.is_none());
        })
    });
    let reads = server.requests.load(Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    cx.run_until_parked();
    assert_eq!(server.requests.load(Ordering::SeqCst), reads);
    server.incident_gate.notify_one();
    cx.run_until_parked();
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.select_project(
                &api::ProjectInfo {
                    id: "p2".into(),
                    name: "Production".into(),
                },
                cx,
            );
            assert!(page.live.incidents.data().is_none());
            assert!(page.live.incident.data().is_none());
            assert_eq!(page.incident_count(), None);
        })
    });
}
