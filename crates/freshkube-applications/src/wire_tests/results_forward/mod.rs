mod fake;
mod review;
use super::*;

// These cases share the automatic port candidates; keep another case from
// taking a just-stopped listener before the restart assertion.
static PORT_CASE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
use freshkube_core::setup::{Forward, Results};
use gpui_kit::test::TestWindowExt;

#[gpui_kit::test]
async fn only_the_toolbar_press_starts_a_forward_and_results_use_its_listener(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let _case = PORT_CASE.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ca = dir.path().join("service.pem");
    std::fs::write(&ca, fake::CERT).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = fake::Server::new(runtime.handle());
    let mut configured = setup(None, None, false);
    let mut settings = Results::default();
    settings.forward = Some(Forward::new("acme", "results", 8443));
    settings.ca_file = Some(ca);
    settings.token = Some(freshkube_core::setup::KeysJson);
    server.token.store(true, Ordering::SeqCst);
    let store = Arc::new(MemoryStore::default());
    store
        .write(
            "Tekton Results forward://acme/results:8443",
            super::fake::TOKEN,
        )
        .unwrap();
    configured.workspace.clusters[0].results = Some(Box::new(settings));
    let (window, app) = mount_access(
        cx,
        &runtime,
        &configured,
        Some(store),
        false,
        Arc::default(),
        Some(server.access.clone()),
    );
    wait_list(cx, window, &app).await;
    assert_eq!(server.forwards.load(Ordering::SeqCst), 0);
    assert_eq!(server.results.load(Ordering::SeqCst), 0);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("changes-forward-results", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.wait_for_io(window, |_, _| server.results.load(Ordering::SeqCst) >= 3)
        .await;
    wait_list(cx, window, &app).await;
    assert!(server.forwards.load(Ordering::SeqCst) >= 3);
    let changes = cx.read(|cx| list(&app, cx));
    let owner = cx.update(|cx| {
        freshkube_resources::forwards::list(cx)
            .read(cx)
            .find("invented-connection", "acme", "results", 8443, cx)
            .unwrap()
    });
    let before = cx.read(|cx| owner.read(cx).snapshot());
    let forwarded = cx.read(|cx| owner.read(cx).local_port.unwrap());
    assert_ne!(forwarded, 8443);
    let id = before.epoch.id;
    cx.update(|cx| {
        freshkube_resources::forwards::list(cx).update(cx, |list, cx| list.stop(id, cx))
    });
    cx.run_until_parked();
    cx.wait_for_io(window, |_, cx| {
        changes
            .read(cx)
            .status
            .text(None)
            .contains("ended: stopped")
    })
    .await;
    cx.update(|cx| {
        freshkube_resources::forwards::list(cx).update(cx, |list, cx| list.start_again(id, cx))
    });
    cx.run_until_parked();
    cx.wait_for_io(window, |_, cx| {
        owner.read(cx).snapshot().epoch.start > before.epoch.start
            && matches!(
                owner.read(cx).snapshot().state,
                freshkube_resources::forwards::State::Listening(_)
            )
    })
    .await;
    wait_list(cx, window, &app).await;
    assert_eq!(cx.read(|cx| owner.read(cx).local_port.unwrap()), forwarded);
    let change = open_change(cx, window, &app);
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    cx.update(|cx| {
        freshkube_resources::forwards::list(cx).update(cx, |list, cx| list.stop(id, cx))
    });
}

fn configured(dir: &std::path::Path) -> Setup {
    let ca = dir.join("service.pem");
    std::fs::write(&ca, fake::CERT).unwrap();
    let mut setup = setup(None, None, false);
    let mut results = Results::default();
    results.forward = Some(Forward::new("acme", "results", 8443));
    results.ca_file = Some(ca);
    setup.workspace.clusters[0].results = Some(Box::new(results));
    setup
}

fn archived(page: &ChangePage) -> bool {
    use freshkube_delivery::change::{Shows, Value};
    page.change.hops.iter().any(|hop| match &hop.shows {
        Shows::Hop(detail) => detail.fields.iter().any(|field| {
            field.label == "Read from"
                && matches!(&field.value, Value::Mono(value) if value.contains("Tekton Results"))
        }),
        _ => false,
    })
}

#[gpui_kit::test]
async fn the_builds_inspector_starts_the_owner_and_hiding_does_not_stop_it(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let _case = PORT_CASE.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let setup = configured(dir.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = fake::Server::new(runtime.handle());
    let (window, app) = mount_access(
        cx,
        &runtime,
        &setup,
        None,
        false,
        Arc::default(),
        Some(server.access.clone()),
    );
    wait_list(cx, window, &app).await;
    let change = open_change(cx, window, &app);
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    assert_eq!(server.forwards.load(Ordering::SeqCst), 0);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let key = change.read(cx).change.hops.iter().find(|hop| matches!(&hop.shows, freshkube_delivery::change::Shows::Hop(detail) if detail.kind == "Builds" || detail.kind == "PipelineRun")).unwrap().key.clone();
        window.click(gpui_kit::SharedString::from(format!("change-hop-{key}")), cx);
        window.render_frame(cx);
        window.click("change-forward-results", cx);
    }).unwrap();
    cx.run_until_parked();
    cx.wait_for_io(window, |_, cx| archived(change.read(cx)))
        .await;
    let owner = cx.update(|cx| {
        freshkube_resources::forwards::list(cx)
            .read(cx)
            .find("invented-connection", "acme", "results", 8443, cx)
            .unwrap()
    });
    let epoch = cx.read(|cx| owner.read(cx).snapshot().epoch);
    cx.update(|cx| app.update(cx, |page, cx| page.set_visible(false, cx)));
    cx.run_until_parked();
    cx.read(|cx| {
        assert!(matches!(
            owner.read(cx).snapshot().state,
            freshkube_resources::forwards::State::Listening(_)
        ))
    });
    cx.update(|cx| app.update(cx, |page, cx| page.set_visible(true, cx)));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| owner.read(cx).snapshot().epoch), epoch);
    cx.update(|cx| {
        freshkube_resources::forwards::list(cx).update(cx, |list, cx| list.stop(epoch.id, cx))
    });
    cx.run_until_parked();
    cx.wait_for_io(window, |_, cx| {
        change.read(cx).answered()
            && change
                .read(cx)
                .change
                .missing
                .iter()
                .any(|missing| missing.text.contains("forward") && missing.text.contains("stopped"))
    })
    .await;
}

#[gpui_kit::test]
async fn refresh_reload_and_show_do_not_start_a_forward_but_f_does(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _case = PORT_CASE.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let setup = configured(dir.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = fake::Server::new(runtime.handle());
    let (window, app) = mount_access(
        cx,
        &runtime,
        &setup,
        None,
        false,
        Arc::default(),
        Some(server.access.clone()),
    );
    wait_list(cx, window, &app).await;
    cx.update_window(window, |_, _, cx| {
        app.update(cx, |page, cx| page.set_setup(&setup, cx))
    })
    .unwrap();
    wait_list(cx, window, &app).await;
    let changes = cx.read(|cx| list(&app, cx));
    cx.update(|cx| changes.update(cx, |page, cx| page.refresh(cx)));
    wait_list(cx, window, &app).await;
    cx.update(|cx| {
        app.update(cx, |page, cx| {
            page.set_visible(false, cx);
            page.set_visible(true, cx);
        })
    });
    wait_list(cx, window, &app).await;
    assert_eq!(server.forwards.load(Ordering::SeqCst), 0);
    cx.update_window(window, |_, window, cx| {
        changes.update(cx, |page, cx| page.focus(window, cx));
        window.press("f", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.wait_for_io(window, |_, _| server.results.load(Ordering::SeqCst) >= 3)
        .await;
    wait_list(cx, window, &app).await;
    let owner = cx.update(|cx| {
        freshkube_resources::forwards::list(cx)
            .read(cx)
            .find("invented-connection", "acme", "results", 8443, cx)
            .unwrap()
    });
    let epoch = cx.read(|cx| owner.read(cx).snapshot().epoch);
    let mut changed = setup.clone();
    changed.workspace.clusters[0]
        .results
        .as_mut()
        .unwrap()
        .forward
        .as_mut()
        .unwrap()
        .port = 9443;
    cx.update_window(window, |_, _, cx| {
        app.update(cx, |page, cx| {
            page.set_setup(&changed, cx);
            page.set_clusters(
                Clusters {
                    entries: vec!["cluster-b".into()],
                    active: Some("cluster-b".into()),
                    mapping: Mapping {
                        open: Some("cluster-b".into()),
                        ..Default::default()
                    },
                    sources: freshkube_core::workspace::Sources::of(&changed.workspace),
                    kubeconfig: None,
                },
                cx,
            );
        })
    })
    .unwrap();
    wait_list(cx, window, &app).await;
    cx.read(|cx| {
        assert_eq!(owner.read(cx).snapshot().epoch, epoch);
        assert!(owner.read(cx).running());
    });
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("changes-forward-results").label(), Some("Forward"));
        let key = changes.read(cx).display.rows[0].key.clone();
        let menu = changes.update(cx, |page, cx| freshkube_ui::table::TableSource::row_menu(page, &key, window, cx));
        assert!(menu.iter().any(|item| matches!(item, freshkube_ui::menu::MenuAction::Item {label, enabled:true, ..} if label.as_ref() == "Forward")));
    }).unwrap();
    let started = server.forwards.load(Ordering::SeqCst);
    cx.update_window(window, |_, _, cx| {
        app.update(cx, |page, cx| {
            page.set_source(
                Some(freshkube_core::cluster_source::ClusterSource {
                    id: "replacement-connection".into(),
                    context: "cluster-b".into(),
                    access: server.access.clone(),
                }),
                Some("cluster-b".into()),
                cx,
            )
        })
    })
    .unwrap();
    wait_list(cx, window, &app).await;
    assert_eq!(server.forwards.load(Ordering::SeqCst), started);
    cx.read(|cx| assert!(owner.read(cx).running()));
    cx.update(|cx| {
        freshkube_resources::forwards::list(cx).update(cx, |list, cx| list.stop(epoch.id, cx))
    });
}

#[gpui_kit::test]
fn the_other_build_entry_offers_only_a_switch_and_never_starts(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let setup = configured(dir.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (window, app) = mount_access(
        cx,
        &runtime,
        &setup,
        None,
        false,
        Arc::default(),
        Some(freshkube_core::cluster_source::ClusterAccess::EXAMPLE),
    );
    let mut workspace = setup.workspace.clone();
    workspace
        .clusters
        .push(Entry::new("cluster-a", Role::Environment, "cluster-a"));
    cx.update_window(window, |_, _, cx| {
        app.update(cx, |page, cx| {
            page.set_clusters(
                Clusters {
                    entries: vec!["cluster-a".into(), "cluster-b".into()],
                    active: Some("cluster-a".into()),
                    mapping: Mapping {
                        open: Some("cluster-a".into()),
                        ..Default::default()
                    },
                    sources: freshkube_core::workspace::Sources::of(&workspace),
                    kubeconfig: None,
                },
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    let changes = cx.read(|cx| list(&app, cx));
    let asked = Arc::new(std::sync::Mutex::new(None));
    let seen = asked.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(
            &changes,
            move |_, event: &crate::changes::ChangesEvent, _| {
                if let crate::changes::ChangesEvent::Switch(entry, _) = event {
                    *seen.lock().unwrap() = Some(entry.to_string());
                }
            },
        )
    });
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("changes-forward-results").label(),
            Some("Switch to cluster-b")
        );
        window.click("changes-forward-results", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(asked.lock().unwrap().as_deref(), Some("cluster-b"));
    assert_eq!(cx.read(freshkube_resources::forwards::running), 0);
}

#[gpui_kit::test]
async fn a_refused_results_request_does_not_end_the_listening_forward(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _case = PORT_CASE.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let setup = configured(dir.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = fake::Server::new(runtime.handle());
    server.status.store(403, Ordering::SeqCst);
    let (window, app) = mount_access(
        cx,
        &runtime,
        &setup,
        None,
        false,
        Arc::default(),
        Some(server.access.clone()),
    );
    wait_list(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("changes-forward-results", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.wait_for_io(window, |_, cx| {
        list(&app, cx)
            .read(cx)
            .display
            .missing
            .as_ref()
            .is_some_and(|(text, _)| text.contains("refused the request"))
    })
    .await;
    let owner = cx.update(|cx| {
        freshkube_resources::forwards::list(cx)
            .read(cx)
            .find("invented-connection", "acme", "results", 8443, cx)
            .unwrap()
    });
    let id = cx.read(|cx| {
        assert!(matches!(
            owner.read(cx).snapshot().state,
            freshkube_resources::forwards::State::Listening(_)
        ));
        owner.read(cx).id
    });
    cx.update(|cx| {
        freshkube_resources::forwards::list(cx).update(cx, |list, cx| list.stop(id, cx))
    });
}

#[gpui_kit::test]
async fn stop_invalidates_a_late_results_response_and_never_publishes_its_build(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let _case = PORT_CASE.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let setup = configured(dir.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = fake::Server::new(runtime.handle());
    let (window, app) = mount_access(
        cx,
        &runtime,
        &setup,
        None,
        false,
        Arc::default(),
        Some(server.access.clone()),
    );
    wait_list(cx, window, &app).await;
    let change = open_change(cx, window, &app);
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    server.hold.store(true, Ordering::SeqCst);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let key = change.read(cx).change.hops.iter().find(|hop| matches!(&hop.shows, freshkube_delivery::change::Shows::Hop(detail) if detail.kind == "Builds" || detail.kind == "PipelineRun")).unwrap().key.clone();
        window.click(gpui_kit::SharedString::from(format!("change-hop-{key}")), cx);
        window.render_frame(cx); window.click("change-forward-results", cx);
    }).unwrap();
    cx.run_until_parked();
    cx.wait_for_io(window, |_, _| server.results.load(Ordering::SeqCst) > 0)
        .await;
    let owner = cx.update(|cx| {
        freshkube_resources::forwards::list(cx)
            .read(cx)
            .find("invented-connection", "acme", "results", 8443, cx)
            .unwrap()
    });
    let id = cx.read(|cx| owner.read(cx).id);
    cx.update(|cx| {
        freshkube_resources::forwards::list(cx).update(cx, |list, cx| list.stop(id, cx))
    });
    cx.run_until_parked();
    cx.wait_for_io(window, |_, cx| {
        change.read(cx).answered()
            && change
                .read(cx)
                .change
                .missing
                .iter()
                .any(|line| line.text.contains("forward") && line.text.contains("stopped"))
    })
    .await;
    let held = server.results.load(Ordering::SeqCst);
    server.hold.store(false, Ordering::SeqCst);
    server.release.notify_waiters();
    cx.wait_for_io(window, |_, _| {
        server.completed.load(Ordering::SeqCst) >= held
    })
    .await;
    cx.run_until_parked();
    cx.read(|cx| {
        assert!(!archived(change.read(cx)));
        assert!(
            change
                .read(cx)
                .change
                .missing
                .iter()
                .any(|line| line.text.contains("stopped"))
        );
    });
}

#[gpui_kit::test]
async fn the_wrong_tls_name_is_a_request_failure_while_the_forward_listens(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let _case = PORT_CASE.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let mut setup = configured(dir.path());
    setup.workspace.clusters[0]
        .results
        .as_mut()
        .unwrap()
        .forward
        .as_mut()
        .unwrap()
        .service = "other-results".into();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = fake::Server::new(runtime.handle());
    let (window, app) = mount_access(
        cx,
        &runtime,
        &setup,
        None,
        false,
        Arc::default(),
        Some(server.access.clone()),
    );
    wait_list(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("changes-forward-results", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.wait_for_io(window, |_, cx| {
        list(&app, cx)
            .read(cx)
            .display
            .missing
            .as_ref()
            .is_some_and(|(text, _)| text.contains("certificate"))
    })
    .await;
    let owner = cx.update(|cx| {
        freshkube_resources::forwards::list(cx)
            .read(cx)
            .find("invented-connection", "acme", "other-results", 8443, cx)
            .unwrap()
    });
    let id = cx.read(|cx| {
        assert!(matches!(
            owner.read(cx).snapshot().state,
            freshkube_resources::forwards::State::Listening(_)
        ));
        owner.read(cx).id
    });
    assert_eq!(server.results.load(Ordering::SeqCst), 0);
    cx.update(|cx| {
        freshkube_resources::forwards::list(cx).update(cx, |list, cx| list.stop(id, cx))
    });
}

#[gpui_kit::test]
async fn a_broken_results_response_does_not_become_an_ended_route(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _case = PORT_CASE.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let setup = configured(dir.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = fake::Server::new(runtime.handle());
    server.truncate.store(true, Ordering::SeqCst);
    let (window, app) = mount_access(
        cx,
        &runtime,
        &setup,
        None,
        false,
        Arc::default(),
        Some(server.access.clone()),
    );
    wait_list(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("changes-forward-results", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.wait_for_io(window, |_, cx| {
        server.completed.load(Ordering::SeqCst) > 0 && !list(&app, cx).read(cx).is_reading()
    })
    .await;
    let owner = cx.update(|cx| {
        freshkube_resources::forwards::list(cx)
            .read(cx)
            .find("invented-connection", "acme", "results", 8443, cx)
            .unwrap()
    });
    let id = cx.read(|cx| {
        let changes = list(&app, cx);
        let changes = changes.read(cx);
        let words = &changes.display.missing.as_ref().unwrap().0;
        assert!(words.contains("didn't answer"));
        assert!(!words.contains("forward ended"));
        assert!(matches!(
            owner.read(cx).snapshot().state,
            freshkube_resources::forwards::State::Listening(_)
        ));
        owner.read(cx).id
    });
    cx.update(|cx| {
        freshkube_resources::forwards::list(cx).update(cx, |list, cx| list.stop(id, cx))
    });
}

#[gpui_kit::test]
async fn closing_a_change_releases_its_subscription_but_leaves_the_owner_listening(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let _case = PORT_CASE.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let setup = configured(dir.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = fake::Server::new(runtime.handle());
    let (window, app) = mount_access(
        cx,
        &runtime,
        &setup,
        None,
        false,
        Arc::default(),
        Some(server.access.clone()),
    );
    wait_list(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("changes-forward-results", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.wait_for_io(window, |_, _| server.results.load(Ordering::SeqCst) >= 3)
        .await;
    wait_list(cx, window, &app).await;
    let change = open_change(cx, window, &app);
    cx.wait_for_io(window, |_, cx| archived(change.read(cx)))
        .await;
    let weak = change.downgrade();
    let owner = cx.update(|cx| {
        freshkube_resources::forwards::list(cx)
            .read(cx)
            .find("invented-connection", "acme", "results", 8443, cx)
            .unwrap()
    });
    let id = cx.read(|cx| owner.read(cx).id);
    cx.update_window(window, |_, window, cx| {
        list(&app, cx).update(cx, |page, cx| page.close_change(window, cx))
    })
    .unwrap();
    drop(change);
    cx.run_until_parked();
    cx.read(|cx| {
        assert!(weak.upgrade().is_none());
        assert!(matches!(
            owner.read(cx).snapshot().state,
            freshkube_resources::forwards::State::Listening(_)
        ));
    });
    cx.update(|cx| {
        freshkube_resources::forwards::list(cx).update(cx, |list, cx| list.stop(id, cx))
    });
}

#[gpui_kit::test]
async fn a_selected_owner_ending_does_not_fall_back_to_another_matching_forward(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let _case = PORT_CASE.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let setup = configured(dir.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = fake::Server::new(runtime.handle());
    let (window, app) = mount_access(
        cx,
        &runtime,
        &setup,
        None,
        false,
        Arc::default(),
        Some(server.access.clone()),
    );
    wait_list(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("changes-forward-results", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.wait_for_io(window, |_, _| server.results.load(Ordering::SeqCst) >= 3)
        .await;
    wait_list(cx, window, &app).await;
    let first = cx.update(|cx| {
        freshkube_resources::forwards::list(cx)
            .read(cx)
            .find("invented-connection", "acme", "results", 8443, cx)
            .unwrap()
    });
    // A second explicit start, as the Service's Ports section permits.
    let second = cx.update(|cx| {
        freshkube_resources::forwards::list(cx).update(cx, |list, cx| {
            list.start(
                freshkube_resources::forwards::ForwardSpec {
                    runtime: runtime.handle().clone(),
                    access: server.access.clone(),
                    identity: freshkube_nav::ResourceIdentity {
                        connection: "invented-connection".into(),
                        resource: "services".into(),
                        namespace: "acme".into(),
                        name: "results".into(),
                        uid: String::new(),
                    },
                    target: freshkube_core::resources::ForwardTarget::Service {
                        name: "results".into(),
                    },
                    context: "cluster-b".into(),
                    port: 8443,
                    local_port: None,
                },
                cx,
            )
        })
    });
    cx.wait_for_io(window, |_, cx| {
        matches!(
            second.read(cx).snapshot().state,
            freshkube_resources::forwards::State::Listening(_)
        )
    })
    .await;
    let id = cx.read(|cx| first.read(cx).id);
    cx.update(|cx| {
        freshkube_resources::forwards::list(cx).update(cx, |list, cx| list.stop(id, cx))
    });
    cx.run_until_parked();
    cx.wait_for_io(window, |_, cx| {
        list(&app, cx)
            .read(cx)
            .status
            .text(None)
            .contains("ended: stopped")
    })
    .await;
    let second_id = cx.read(|cx| {
        assert!(second.read(cx).running());
        second.read(cx).id
    });
    cx.update(|cx| {
        freshkube_resources::forwards::list(cx).update(cx, |list, cx| list.stop(second_id, cx))
    });
}

#[gpui_kit::test]
async fn a_forward_restarted_while_hidden_is_read_only_when_the_page_shows(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let _case = PORT_CASE.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let setup = configured(dir.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = fake::Server::new(runtime.handle());
    let (window, app) = mount_access(
        cx,
        &runtime,
        &setup,
        None,
        false,
        Arc::default(),
        Some(server.access.clone()),
    );
    wait_list(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("changes-forward-results", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.wait_for_io(window, |_, _| server.results.load(Ordering::SeqCst) >= 3)
        .await;
    wait_list(cx, window, &app).await;
    let change = open_change(cx, window, &app);
    cx.wait_for_io(window, |_, cx| archived(change.read(cx)))
        .await;
    let owner = cx.update(|cx| {
        freshkube_resources::forwards::list(cx)
            .read(cx)
            .find("invented-connection", "acme", "results", 8443, cx)
            .unwrap()
    });
    let before = cx.read(|cx| owner.read(cx).snapshot().epoch);
    cx.update(|cx| app.update(cx, |page, cx| page.set_visible(false, cx)));
    cx.run_until_parked();
    let reads = server.results.load(Ordering::SeqCst);
    cx.update(|cx| {
        freshkube_resources::forwards::list(cx).update(cx, |list, cx| list.stop(before.id, cx))
    });
    cx.run_until_parked();
    cx.update(|cx| {
        freshkube_resources::forwards::list(cx)
            .update(cx, |list, cx| list.start_again(before.id, cx))
    });
    cx.run_until_parked();
    cx.wait_for_io(window, |_, cx| {
        matches!(
            owner.read(cx).snapshot().state,
            freshkube_resources::forwards::State::Listening(_)
        )
    })
    .await;
    cx.run_until_parked();
    assert_eq!(server.results.load(Ordering::SeqCst), reads);
    assert!(!cx.read(|cx| change.read(cx).is_reading()));
    assert!(cx.read(|cx| owner.read(cx).snapshot().epoch.start) > before.start);
    cx.update(|cx| app.update(cx, |page, cx| page.set_visible(true, cx)));
    cx.run_until_parked();
    cx.wait_for_io(window, |_, cx| {
        archived(change.read(cx))
            && !change.read(cx).is_reading()
            && server.results.load(Ordering::SeqCst) > reads
    })
    .await;
    assert!(server.results.load(Ordering::SeqCst) > reads);
    cx.update(|cx| {
        freshkube_resources::forwards::list(cx).update(cx, |list, cx| list.stop(before.id, cx))
    });
}
