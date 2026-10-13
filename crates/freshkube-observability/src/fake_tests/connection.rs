use super::*;
use freshkube_ui::real_io::RealIo;

#[gpui_kit::test]
async fn connection_project_partial_reports_refusal_and_recovery(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.url.update(cx, |input, cx| {
                input.set_value(server.url.clone(), window, cx)
            });
            page.secret
                .update(cx, |input, cx| input.set_value("sanitized-key", window, cx));
            page.connect(cx);
        });
    })
    .unwrap();
    let observed = page.clone();
    cx.wait_for_io(handle, move |_, cx| !observed.read(cx).live.connecting)
        .await;
    // Connected, the page asks for a project, not for a connection.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("obs-integration-required").label(),
            Some("Coroot project required")
        );
    })
    .unwrap();
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            assert!(page.live.provider.is_some(), "{:?}", page.live.error);
            assert!(
                page.live.source.is_none(),
                "never select a project implicitly"
            );
            let project = page.live.projects[1].clone();
            page.select_project(&project, cx);
        })
    });
    loaded(cx, handle, &page).await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            assert_eq!(page.live.source.as_ref().unwrap().project(), "p2");
            let app = &page.applications[0];
            assert_eq!(app.check(Report::Cpu).status, Status::Ok);
            assert_eq!(app.check(Report::Memory).status, Status::Unknown);
            assert_eq!(app.check(Report::Logs).status, Status::Absent);
            page.open_app(app.id.clone(), Report::Cpu, cx);
        })
    });
    let observed = page.clone();
    cx.wait_for_io(handle, move |_, cx| {
        let page = observed.read(cx);
        !page.live.view.is_loading()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        let current = page.read(cx);
        assert!(current.live.view.data().is_some());
        // The page shows Coroot's view alone: one read of the application,
        // and no report from the REST API or MCP beside it.
        let paths = server.paths.lock().unwrap();
        let app_reads = paths.iter().filter(|path| {
            let path = path.split('?').next().unwrap_or_default();
            path.rsplit_once("/app/")
                .is_some_and(|(_, id)| !id.contains('/'))
        });
        assert_eq!(app_reads.count(), 1, "{paths:?}");
        assert!(paths.iter().all(|path| !path.contains("mcp")), "{paths:?}");
        drop(paths);
        window.render_frame(cx);
        assert!(window.try_find("obs-threshold").is_none());
        page.update(cx, |page, cx| page.open(Destination::Applications, cx));
    })
    .unwrap();
    loaded(cx, handle, &page).await;
    for status in [403, 500, 200, 204] {
        server.status.store(status, Ordering::SeqCst);
        cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
        loaded(cx, handle, &page).await;
        cx.update(|cx| {
            let page = page.read(cx);
            if matches!(status, 403 | 500) {
                assert!(page.live.apps.is_stale());
                assert_eq!(page.applications.len(), 1);
                let expected = if status == 403 {
                    api::ReadError::Refused
                } else {
                    api::ReadError::Failed
                };
                assert_eq!(
                    page.live.capabilities[0],
                    api::Capability::Unavailable(expected)
                );
            } else {
                assert!(!page.live.apps.is_stale());
                assert!(page.live.apps.error().is_none());
                assert_eq!(page.applications.len(), usize::from(status != 204));
            }
        });
    }
    let previous = cx.read(|cx| page.read(cx).live.range);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(3600));
    cx.update(|cx| page.update(cx, |page, cx| page.refresh_current(cx)));
    loaded(cx, handle, &page).await;
    let current = cx.read(|cx| page.read(cx).live.range);
    assert!(current.to > previous.to);
    let paths = server.paths.lock().unwrap();
    let last = paths.last().unwrap();
    assert!(last.contains("/project/p2/overview/applications"));
    assert!(last.contains(&format!(
        "from={}",
        current.from.unwrap().timestamp_millis()
    )));
    assert!(last.contains(&format!("to={}", current.to.unwrap().timestamp_millis())));
    drop(paths);
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(false, cx)));
    let reads = server.requests.load(Ordering::SeqCst);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.refresh(cx);
            page.connect(cx);
        })
    });
    assert_eq!(server.requests.load(Ordering::SeqCst), reads);
}

#[gpui_kit::test]
async fn a_connected_page_keeps_connection_settings_behind_the_sidebar_action(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let (_runtime, handle, page) = mount(cx, false);
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, _| {
            let provider =
                api::Provider::new("http://127.0.0.1:1", api::Credentials::None).unwrap();
            let source = provider
                .source(&api::ProjectInfo {
                    id: "p".into(),
                    name: "Project".into(),
                })
                .with_association(Some(api::Association::new(
                    "access:a".into(),
                    "cluster-a".into(),
                )));
            page.live.provider = Some(provider);
            page.live.source = Some(source);
            page.live.access = Some("access:a".into());
            page.settings_open = false;
        });
        window.render_frame(cx);
        assert!(window.try_find("obs-refresh").is_some());
        assert!(window.try_find("obs-disconnect").is_none());
        assert!(window.try_find("obs-unmap").is_none());
        assert!(window.try_find("obs-connect-settings").is_none());
        page.update(cx, |page, cx| page.show_connection(cx));
        window.render_frame(cx);
        assert!(window.try_find("obs-disconnect").is_some());
        assert!(window.try_find("obs-unmap").is_some());
        window.click("obs-connect-settings", cx);
        window.render_frame(cx);
        assert!(window.try_find("obs-disconnect").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn editing_credentials_invalidates_connection_and_object_links(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (_runtime, handle, page) = mount(cx, false);
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, _| {
            let provider =
                api::Provider::new("http://127.0.0.1:1", api::Credentials::None).unwrap();
            let source = provider
                .source(&api::ProjectInfo {
                    id: "p".into(),
                    name: "Project".into(),
                })
                .with_association(Some(api::Association::new(
                    "access:a".into(),
                    "cluster-a".into(),
                )));
            let app = api::AppId::new("cluster-a:prod:Deployment:api");
            let subject = api::ObjectSubject::for_app(&source, &app).unwrap();
            page.live.provider = Some(provider);
            page.live.source = Some(source.clone());
            page.live.access = Some("access:a".into());
            page.selected_app = Some(app.clone());
            assert!(page.link_is_current(&source, &app, &subject));
            page.live.visible = false;
            assert!(!page.link_is_current(&source, &app, &subject));
            page.live.visible = true;
            page.settings_open = true;
        });
        window.render_frame(cx);
        window.click("obs-credential", cx);
        window.input("replacement", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let page = page.read(cx);
        assert!(page.live.provider.is_none());
        assert!(page.live.source.is_none());
        assert!(page.selected_app.is_none());
        assert_eq!(page.secret.read(cx).value(), "replacement");
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("obs-disconnect", cx);
        assert!(page.read(cx).secret.read(cx).value().is_empty());
        window.click("obs-credential", cx);
        window.input("retained-after-failure", cx);
        window.click("obs-url", cx);
        window.input("not-a-url", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.click("obs-connect", cx))
        .unwrap();
    let observed = page.clone();
    cx.wait_for_io(handle, move |_, cx| observed.read(cx).live.error.is_some())
        .await;
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).live.provider.is_none());
        assert_eq!(
            page.read(cx).secret.read(cx).value(),
            "retained-after-failure"
        );
        window.click("obs-disconnect", cx);
        assert!(page.read(cx).secret.read(cx).value().is_empty());
        assert!(page.read(cx).live.error.is_none());
        window.click("obs-credential", cx);
        window.input("pending-credential", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        // Hold the same owned delivery path used by initial connection reads.
        // No provider exists yet, and the request must remain cancellable.
        page.update(cx, |page, cx| {
            page.live.connecting = true;
            page.spawn_read(
                std::future::pending::<Result<(), api::ReadError>>(),
                |_, _, _| {
                    panic!("cancelled connection must not deliver");
                },
                cx,
            );
        });
        let generation = page.read(cx).live.generation;
        assert_eq!(page.read(cx).live.jobs.len(), 1);
        window.click("obs-disconnect", cx);
        let current = page.read(cx);
        assert!(current.secret.read(cx).value().is_empty());
        assert!(!current.live.connecting);
        assert!(current.live.jobs.is_empty());
        assert!(current.live.generation > generation);
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_remembered_connection_returns_on_the_next_launch_with_its_key_in_the_store(
    cx: &mut TestAppContext,
) {
    use super::super::remember::account;
    use crate::secrets::MemoryStore;
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let preferences = directory.path().join("preferences.json");
    let file = directory.path().join("coroot.json");
    let store = Arc::new(MemoryStore::default());
    let (runtime, handle, page) =
        super::super::tests::mount_remembering(cx, &preferences, store.clone());
    let server = Server::new(&runtime);
    let key_account = account(&server.url);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-remember", cx);
        page.update(cx, |page, cx| {
            assert!(page.memory.as_ref().unwrap().saved.remember);
            page.url.update(cx, |input, cx| {
                input.set_value(server.url.clone(), window, cx)
            });
            page.secret
                .update(cx, |input, cx| input.set_value("sanitized-key", window, cx));
            page.connect(cx);
        });
    })
    .unwrap();
    let observed = page.clone();
    cx.wait_for_io(handle, move |_, cx| {
        observed.read(cx).live.provider.is_some()
    })
    .await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let project = page.live.projects[1].clone();
            page.select_project(&project, cx);
        })
    });
    let (saved, account_name) = (store.clone(), key_account.clone());
    let path = file.clone();
    cx.wait_for_io(handle, move |_, _| {
        saved.keys.lock().unwrap().contains_key(&account_name)
            && std::fs::read_to_string(&path).is_ok_and(|text| text.contains("p2"))
    })
    .await;
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        !text.contains("sanitized-key"),
        "the key never reaches the file"
    );
    assert_eq!(
        store
            .keys
            .lock()
            .unwrap()
            .get(&key_account)
            .map(String::as_str),
        Some("sanitized-key")
    );

    // The next launch connects again with the stored key and opens the project.
    let (_second_runtime, second, reopened) =
        super::super::tests::mount_remembering(cx, &preferences, store.clone());
    let observed = reopened.clone();
    cx.wait_for_io(second, move |_, cx| {
        observed
            .read(cx)
            .live
            .source
            .as_ref()
            .is_some_and(|source| source.project() == "p2")
    })
    .await;
    cx.update(|cx| {
        let page = reopened.read(cx);
        assert_eq!(page.url.read(cx).value(), server.url.as_str());
        assert!(
            page.secret.read(cx).value().is_empty(),
            "the stored key is never put in the field"
        );
    });

    // Disconnect forgets the key and stops reconnecting.
    cx.update_window(second, |_, window, cx| {
        reopened.update(cx, |page, cx| page.disconnect(window, cx))
    })
    .unwrap();
    let (saved, account_name, path) = (store.clone(), key_account.clone(), file.clone());
    cx.wait_for_io(second, move |_, _| {
        !saved.keys.lock().unwrap().contains_key(&account_name)
            && std::fs::read_to_string(&path)
                .is_ok_and(|text| text.contains("\"reconnect\": false"))
    })
    .await;

    // A store that can't save turns Remember off and says so.
    store
        .unavailable
        .store(true, std::sync::atomic::Ordering::SeqCst);
    cx.update_window(second, |_, window, cx| {
        reopened.update(cx, |page, cx| {
            page.secret
                .update(cx, |input, cx| input.set_value("sanitized-key", window, cx));
            page.connect(cx);
        })
    })
    .unwrap();
    let observed = reopened.clone();
    cx.wait_for_io(second, move |_, cx| {
        observed
            .read(cx)
            .memory
            .as_ref()
            .is_some_and(|memory| !memory.saved.remember && memory.error.is_some())
    })
    .await;
    cx.update(|cx| {
        let error = reopened
            .read(cx)
            .memory
            .as_ref()
            .unwrap()
            .error
            .clone()
            .unwrap();
        assert!(error.contains("stays in memory"), "{error}");
        assert!(!error.contains("sanitized-key"));
    });
}

#[gpui_kit::test]
async fn a_pasted_key_is_trimmed_and_a_refused_one_says_which_key_coroot_wants(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    let connect = |cx: &mut TestAppContext, url: String, key: &'static str| {
        cx.update_window(handle, |_, window, cx| {
            page.update(cx, |page, cx| {
                page.url
                    .update(cx, |input, cx| input.set_value(url, window, cx));
                page.secret
                    .update(cx, |input, cx| input.set_value(key, window, cx));
                page.connect(cx);
            })
        })
        .unwrap();
    };
    connect(cx, server.url.clone(), "crt_wrong");
    settled(cx, handle, &page).await;
    cx.update(|cx| {
        let page = page.read(cx);
        assert!(page.live.provider.is_none());
        let error = page.live.error.clone().unwrap();
        assert!(error.contains("user API key"), "{error}");
        assert!(!error.contains("crt_wrong"), "{error}");
    });

    // Spaces around a pasted URL and key are not part of them.
    connect(cx, format!("  {}  ", server.url), "  sanitized-key  ");
    settled(cx, handle, &page).await;
    cx.update(|cx| {
        let page = page.read(cx);
        assert!(page.live.provider.is_some(), "{:?}", page.live.error);
    });
}
