use super::*;

#[gpui_kit::test]
async fn monitoring_url_tokens_save_separately_and_a_refused_file_restores_them(
    cx: &mut TestAppContext,
) {
    use freshkube_core::secrets::MemoryStore;
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    let (runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    let secrets = Arc::new(MemoryStore::default());
    cx.update_window(handle, |_, window, cx| {
        let monitoring = cx.new(|cx| {
            super::MonitoringPage::new(
                runtime.handle().clone(),
                Some(&directory.path().join("preferences.json")),
                Some(secrets.clone()),
                cx,
            )
        });
        page.update(cx, |page, _| {
            page.monitoring.as_mut().unwrap().page = monitoring
        });
        window.click("metrics-store-url", cx);
        window.render_frame(cx);
        replace(window, cx, "metrics-url", "https://metrics.example.test");
        replace(window, cx, "metrics-token-input", "example-token");
    })
    .unwrap();
    cx.run_until_parked();
    std::fs::write(&file, "version: [").unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    assert!(secrets.keys.lock().unwrap().is_empty());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "version: [");
    std::fs::write(&file, SETUP).unwrap();
    reload(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    let text = std::fs::read_to_string(&file).unwrap();
    let setup = setup::parse(text.as_bytes()).unwrap();
    assert!(setup.metrics.store.unwrap().token.is_some());
    assert!(!text.contains("example-token"));
    assert_eq!(
        secrets
            .keys
            .lock()
            .unwrap()
            .get("Prometheus https://metrics.example.test/")
            .map(String::as_str),
        Some("example-token")
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-token-forget", cx);
        window.render_frame(cx);
        assert!(!secrets.keys.lock().unwrap().is_empty());
        window.click("metrics-revert", cx);
        window.render_frame(cx);
        assert!(!secrets.keys.lock().unwrap().is_empty());
        window.click("metrics-token-forget", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    assert!(secrets.keys.lock().unwrap().is_empty());
    assert!(
        setup::parse(&std::fs::read(&file).unwrap())
            .unwrap()
            .metrics
            .store
            .unwrap()
            .token
            .is_none()
    );
}

#[gpui_kit::test]
async fn monitoring_retires_only_unreferenced_tokens_after_a_successful_save(
    cx: &mut TestAppContext,
) {
    for action in ["service", "url", "forget", "automatic"] {
        for condition in [
            "unreferenced",
            "shared",
            "context",
            "anonymous-context",
            "unreadable-contexts",
            "refused",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let file = directory.path().join("freshkube.yaml");
            let mut setup = setup::parse(url_setup().as_bytes()).unwrap();
            if condition == "shared" {
                let mut source = setup::Source::default();
                source.url = Some("https://metrics.example.test/".into());
                source.token = Some(setup::KeysJson);
                let mut metrics = setup::EntryMetrics::default();
                metrics.source = Some(source);
                setup.workspace.clusters[0].metrics = Some(Box::new(metrics));
            }
            let original = setup::to_text(&setup);
            std::fs::write(&file, &original).unwrap();
            if matches!(condition, "context" | "anonymous-context") {
                write_context_source(directory.path(), condition == "context");
            }
            let (runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
            let store = TokenStore::new(file.clone());
            set_secrets(cx, &runtime, &page, directory.path(), store.clone());
            if condition == "unreadable-contexts" {
                std::fs::write(directory.path().join("monitoring.json"), "{").unwrap();
            }
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                match action {
                    "service" => {
                        window.click("metrics-store-service", cx);
                        window.render_frame(cx);
                        // Pick the only hosting entry using its real select.
                        window.click("metrics-entry", cx);
                        window.press("down", cx);
                        window.press("enter", cx);
                    }
                    "url" => replace(
                        window,
                        cx,
                        "metrics-url",
                        "https://other-metrics.example.test",
                    ),
                    "forget" => window.click("metrics-token-forget", cx),
                    "automatic" => window.click("metrics-automatic", cx),
                    _ => unreachable!(),
                }
            })
            .unwrap();
            cx.run_until_parked();
            if action == "service" {
                cx.update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    replace(window, cx, "metrics-namespace", "metrics");
                    replace(window, cx, "metrics-name", "prometheus");
                    replace(window, cx, "metrics-port", "9090");
                })
                .unwrap();
                cx.run_until_parked();
            }
            cx.read(|cx| {
                let page = page.read(cx);
                page.monitoring
                    .as_ref()
                    .unwrap()
                    .metrics(&page.workspace, cx)
                    .map_err(|(_, why)| why)
                    .unwrap_or_else(|why| panic!("{action}, {condition}: {why}"));
            });
            if condition == "refused" {
                std::fs::write(&file, "version: [").unwrap();
            }
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click("metrics-save", cx);
            })
            .unwrap();
            saved(cx, handle, &page).await;
            store.assert_kept(
                !matches!(condition, "unreferenced" | "anonymous-context") || action == "automatic",
            );
            if condition == "refused" {
                assert_eq!(std::fs::read_to_string(&file).unwrap(), "version: [");
                cx.read(|cx| assert!(page.read(cx).monitoring.as_ref().unwrap().error.is_some()));
            } else {
                cx.read(|cx| {
                    let draft = page.read(cx).monitoring.as_ref().unwrap();
                    assert_eq!(
                        draft.error.is_some(),
                        condition == "unreadable-contexts" && action != "automatic"
                    );
                });
                let saved = setup::parse(&std::fs::read(&file).unwrap()).unwrap();
                if action == "service" {
                    let source = saved.metrics.store.unwrap();
                    assert!(source.service.is_some());
                    assert!(source.url.is_none());
                    assert!(source.token.is_none());
                }
            }
        }
    }
}

#[gpui_kit::test]
async fn monitoring_inspector_retires_service_and_none_tokens_after_save(cx: &mut TestAppContext) {
    for service in [false, true] {
        for condition in ["unreferenced", "central", "entry", "context", "refused"] {
            let directory = tempfile::tempdir().unwrap();
            let file = directory.path().join("freshkube.yaml");
            let mut setup = setup::parse(url_setup().as_bytes()).unwrap();
            let mut source = setup::Source::default();
            source.url = Some("https://metrics.example.test/".into());
            source.token = Some(setup::KeysJson);
            let mut metrics = setup::EntryMetrics::default();
            metrics.source = Some(source);
            setup.workspace.clusters[0].metrics = Some(Box::new(metrics));
            if condition != "central" {
                setup.metrics = setup::Metrics::default();
            }
            if condition == "entry" {
                let mut other = setup.workspace.clusters[0].clone();
                other.id = "cluster-b".into();
                other.context = "cluster-b".into();
                setup.workspace.clusters.push(other);
            }
            std::fs::write(&file, setup::to_text(&setup)).unwrap();
            if condition == "context" {
                write_context_source(directory.path(), true);
            }
            let (_runtime, handle, pilot, page) = live(cx, directory.path(), "cluster-a").await;
            let store = TokenStore::new(file.clone());
            cx.update_window(handle, |_, window, cx| {
                page.update(cx, |page, _| page.metrics_secrets = Some(store.clone()));
                pilot.update(cx, |pilot, cx| {
                    pilot.show_settings(SettingsSection::Clusters, window, cx)
                });
                window.render_frame(cx);
                window.click("settings-cluster-cluster-a", cx);
                window.render_frame(cx);
                window.click(
                    if service {
                        "settings-metrics-source-service"
                    } else {
                        "settings-metrics-source-none"
                    },
                    cx,
                );
                window.render_frame(cx);
                if service {
                    replace(window, cx, "settings-metrics-namespace", "metrics");
                    replace(window, cx, "settings-metrics-service", "prometheus");
                    replace(window, cx, "settings-metrics-port", "9090");
                }
            })
            .unwrap();
            cx.run_until_parked();
            if condition == "refused" {
                std::fs::write(&file, "version: [").unwrap();
            }
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click("settings-metrics-save", cx);
            })
            .unwrap();
            crate::desktop::settings::edit_tests::saved(cx, handle, &pilot).await;
            store.assert_kept(condition != "unreferenced");
            if condition == "refused" {
                assert_eq!(std::fs::read_to_string(&file).unwrap(), "version: [");
            } else {
                let saved = setup::parse(&std::fs::read(&file).unwrap()).unwrap();
                let source = saved.workspace.clusters[0]
                    .metrics
                    .as_ref()
                    .and_then(|m| m.source.as_ref());
                assert_eq!(source.is_some(), service);
                if let Some(source) = source {
                    assert!(source.service.is_some());
                    assert!(source.url.is_none());
                    assert!(source.token.is_none());
                }
            }
        }
    }
}

#[gpui_kit::test]
async fn monitoring_forget_then_a_new_token_keeps_the_new_token(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, url_setup()).unwrap();
    let (runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    let secrets = Arc::new(freshkube_core::secrets::MemoryStore::default());
    let account = "Prometheus https://metrics.example.test/";
    secrets
        .keys
        .lock()
        .unwrap()
        .insert(account.into(), "old-example-token".into());
    set_secrets(cx, &runtime, &page, directory.path(), secrets.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-token-forget", cx);
        window.render_frame(cx);
        replace(window, cx, "metrics-token-input", "new-example-token");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    assert_eq!(
        secrets
            .keys
            .lock()
            .unwrap()
            .get(account)
            .map(String::as_str),
        Some("new-example-token")
    );
    let text = std::fs::read_to_string(file).unwrap();
    assert!(
        setup::parse(text.as_bytes())
            .unwrap()
            .metrics
            .store
            .unwrap()
            .token
            .is_some()
    );
    assert!(!text.contains("new-example-token"));
}

#[gpui_kit::test]
async fn monitoring_a_changed_url_asks_for_its_own_token(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, url_setup()).unwrap();
    let (runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    let secrets = Arc::new(freshkube_core::secrets::MemoryStore::default());
    set_secrets(cx, &runtime, &page, directory.path(), secrets.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("metrics-token-kept").visible());
        replace(
            window,
            cx,
            "metrics-url",
            "https://other-metrics.example.test",
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("metrics-token-kept").is_none());
        assert!(window.find("metrics-token-input").visible());
        assert_eq!(
            page.read(cx)
                .monitoring
                .as_ref()
                .unwrap()
                .token
                .read(cx)
                .value(),
            ""
        );
        assert!(window.find("metrics-token-row-hint").visible());
        replace(window, cx, "metrics-token-input", "other-example-token");
        replace(window, cx, "metrics-url", "https://metrics.example.test/");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("metrics-token-kept").is_none());
        assert!(window.find("metrics-token-input").visible());
        replace(
            window,
            cx,
            "metrics-url",
            "https://other-metrics.example.test/",
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    let store = setup::parse(&std::fs::read(file).unwrap())
        .unwrap()
        .metrics
        .store
        .unwrap();
    assert_eq!(
        store.url.as_deref(),
        Some("https://other-metrics.example.test/")
    );
    assert!(store.token.is_some());
    assert_eq!(
        secrets
            .keys
            .lock()
            .unwrap()
            .get("Prometheus https://other-metrics.example.test/")
            .map(String::as_str),
        Some("other-example-token")
    );
}

#[gpui_kit::test]
async fn monitoring_a_refusal_after_staging_restores_the_previous_token(cx: &mut TestAppContext) {
    use freshkube_core::secrets::{MemoryStore, SecretStore};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct BreakAfterStaging {
        memory: MemoryStore,
        file: std::path::PathBuf,
        writes: AtomicUsize,
    }
    impl SecretStore for BreakAfterStaging {
        fn read(&self, account: &str) -> Result<Option<String>, String> {
            self.memory.read(account)
        }
        fn write(&self, account: &str, value: &str) -> Result<(), String> {
            self.memory.write(account, value)?;
            if self.writes.fetch_add(1, Ordering::SeqCst) == 0 {
                std::fs::write(&self.file, "version: [").unwrap();
            }
            Ok(())
        }
        fn forget(&self, account: &str) -> Result<(), String> {
            self.memory.forget(account)
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, url_setup()).unwrap();
    let (runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    let secrets = Arc::new(BreakAfterStaging {
        memory: MemoryStore::default(),
        file: file.clone(),
        writes: AtomicUsize::new(0),
    });
    let account = "Prometheus https://metrics.example.test/";
    secrets
        .memory
        .keys
        .lock()
        .unwrap()
        .insert(account.into(), "previous-example-token".into());
    set_secrets(cx, &runtime, &page, directory.path(), secrets.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-token-replace", cx);
        window.render_frame(cx);
        replace(
            window,
            cx,
            "metrics-token-input",
            "replacement-example-token",
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    assert_eq!(
        secrets.writes.load(Ordering::SeqCst),
        2,
        "the token was staged and then restored"
    );
    assert_eq!(
        secrets.read(account).unwrap().as_deref(),
        Some("previous-example-token")
    );
    assert_eq!(std::fs::read_to_string(file).unwrap(), "version: [");
    cx.read(|cx| assert!(page.read(cx).monitoring.as_ref().unwrap().error.is_some()));
}

#[gpui_kit::test]
async fn monitoring_a_first_token_save_can_skip_a_refused_fallback_read(cx: &mut TestAppContext) {
    use freshkube_core::secrets::{FileStore, MemoryStore, SecretStore};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    struct RefuseAfterWrite {
        store: FileStore,
        file: std::path::PathBuf,
        refuse: AtomicBool,
        writes: AtomicUsize,
    }
    impl SecretStore for RefuseAfterWrite {
        fn read(&self, account: &str) -> Result<Option<String>, String> {
            self.store.read(account)
        }
        fn write(&self, account: &str, value: &str) -> Result<(), String> {
            self.store.write(account, value)?;
            self.writes.fetch_add(1, Ordering::SeqCst);
            if self.refuse.swap(false, Ordering::SeqCst) {
                std::fs::write(&self.file, "version: [").unwrap();
            }
            Ok(())
        }
        fn forget(&self, account: &str) -> Result<(), String> {
            self.store.forget(account)
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    let (runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    assert!(!directory.path().join("keys.json").exists());
    let fallback = Arc::new(MemoryStore::default());
    fallback.unavailable.store(true, Ordering::SeqCst);
    let secrets = Arc::new(RefuseAfterWrite {
        store: FileStore::new(&directory.path().join("preferences.json"), Some(fallback)),
        file: file.clone(),
        refuse: AtomicBool::new(false),
        writes: AtomicUsize::new(0),
    });
    set_secrets(cx, &runtime, &page, directory.path(), secrets.clone());
    cx.update_window(handle, |_, window, cx| {
        window.click("metrics-store-url", cx);
        window.render_frame(cx);
        replace(window, cx, "metrics-url", "https://metrics.example.test");
        replace(window, cx, "metrics-token-input", "first-example-token");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    let first = "Prometheus https://metrics.example.test/";
    assert_eq!(
        secrets.read(first).unwrap().as_deref(),
        Some("first-example-token")
    );
    assert!(
        setup::parse(&std::fs::read(&file).unwrap())
            .unwrap()
            .metrics
            .store
            .unwrap()
            .token
            .is_some()
    );
    cx.read(|cx| assert!(page.read(cx).monitoring.as_ref().unwrap().error.is_none()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        replace(
            window,
            cx,
            "metrics-url",
            "https://other-metrics.example.test",
        );
        window.render_frame(cx);
        replace(window, cx, "metrics-token-input", "refused-example-token");
    })
    .unwrap();
    cx.run_until_parked();
    secrets.refuse.store(true, Ordering::SeqCst);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    assert_eq!(secrets.writes.load(Ordering::SeqCst), 2);
    assert_eq!(
        secrets
            .read("Prometheus https://other-metrics.example.test/")
            .unwrap(),
        None
    );
    assert_eq!(
        secrets.read(first).unwrap().as_deref(),
        Some("first-example-token")
    );
    assert_eq!(std::fs::read_to_string(file).unwrap(), "version: [");
    cx.read(|cx| {
        let error = page
            .read(cx)
            .monitoring
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap();
        assert!(error.contains("The new token was removed; its previous value couldn’t be read."));
    });
}
