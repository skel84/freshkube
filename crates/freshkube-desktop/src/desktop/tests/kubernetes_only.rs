use super::*;
use freshkube_ui::real_io::RealIo;

#[gpui_kit::test]
fn kubernetes_only_never_waits_for_talos_services(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = mount_kubernetes_only(cx, kubeconfig_file("services"), None);
    wait_until(cx, handle, "the kubeconfig to load", |_, cx| {
        !view.read(cx).contexts.is_empty()
    });
    cx.update(|cx| {
        let reading = view
            .read(cx)
            .pages
            .system_services
            .read(cx)
            .reading()
            .clone();
        assert_eq!(reading, super::super::system_services::Reading::Answered);
    });
}

fn kubeconfig_file(name: &str) -> std::path::PathBuf {
    let path = scratch(name);
    std::fs::write(&path, KUBECONFIG).unwrap();
    path
}

// Renders until `ready` holds, letting Tokio work land between frames (`freshkube_ui::real_io`).
fn wait_until(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    what: &str,
    ready: impl Fn(&mut gpui_kit::Window, &mut gpui_kit::App) -> bool,
) {
    freshkube_ui::real_io::until_io(cx, what, |cx| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            ready(window, cx)
        })
        .unwrap()
    });
}

fn kubernetes_status(window: &gpui_kit::Window) -> String {
    window
        .find("kubernetes-status")
        .label()
        .unwrap_or_default()
        .to_owned()
}

#[gpui_kit::test]
fn kubernetes_only_ctrl_tab_skips_pages_that_need_talos(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = mount_kubernetes_only(cx, kubeconfig_file("ctrl-tab"), None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).navigation.page, Page::Overview);
        for (page, kind) in [
            (Page::Nodes, None),
            (Page::Resources, Some("namespaces")),
            (Page::Resources, Some("events")),
            (Page::Health, None),
            (Page::Resources, Some("pods")),
            (Page::Overview, None),
        ] {
            window.press("ctrl-tab", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).navigation.page, page);
            if let Some(kind) = kind {
                assert_eq!(view.read(cx).navigation.resource_kind.key(), kind);
            }
        }
        for key in ["secondary-6", "secondary-7", "secondary-8", "secondary-9"] {
            window.press(key, cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).navigation.page, Page::Overview);
        }
        window.press("ctrl-shift-tab", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).navigation.page, Page::Resources);
        assert_eq!(window.find("resource-body").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubernetes_only_command_keys_open_the_kubernetes_pages_and_skip_the_talos_ones(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = mount_kubernetes_only(cx, kubeconfig_file("command-keys"), None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for (key, page, kind) in [
            ("secondary-2", Page::Nodes, None),
            ("secondary-3", Page::Resources, Some("namespaces")),
            ("secondary-4", Page::Resources, Some("events")),
            ("secondary-5", Page::Health, None),
            ("secondary-1", Page::Overview, None),
        ] {
            window.press(key, cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).navigation.page, page, "{key}");
            if let Some(kind) = kind {
                assert_eq!(view.read(cx).navigation.resource_kind.key(), kind, "{key}");
            }
        }
        // The Talos pages stay shut, wherever the keys are pressed from.
        window.press("secondary-5", cx);
        for key in ["secondary-6", "secondary-7", "secondary-8", "secondary-9"] {
            window.press(key, cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).navigation.page, Page::Health, "{key}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubernetes_only_lists_kubeconfig_contexts_and_connects_to_the_current_one(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = mount_kubernetes_only(cx, kubeconfig_file("contexts"), None);
    wait_until(cx, handle, "lab to refuse", |window, _| {
        kubernetes_status(window).starts_with("Couldn't connect to lab: ")
    });
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).navigation.page, Page::Overview);
        view.update(cx, |view, cx| view.open_builtin("pods", window, cx));
    })
    .unwrap();
    // The page read through that same connection, and failed with it.
    wait_until(cx, handle, "the page to fail", |window, _| {
        window.try_find("resource-failed").is_some()
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let pilot = view.read(cx);
        assert_eq!(pilot.contexts, ["lab", "staging"]);
        assert_eq!(pilot.applied.context.as_deref(), Some("lab"));
        assert_eq!(pilot.navigation.page, Page::Resources);
        assert_eq!(pilot.context_display.detail.as_ref(), "Couldn't connect");
        contexts(window, cx);
        assert_eq!(window.find(("context", 0usize)).selected(), Some(true));
        assert_eq!(window.find("page-title").label(), Some("Pods"));
        assert!(
            window
                .find("applied-config")
                .label()
                .unwrap()
                .starts_with("Kubeconfig: ")
        );
        // No nodes to target without Talos.
        assert!(window.try_find("target-node").is_none());

        // Choosing the failed context again retries it.
        contexts(window, cx);
        window.click(("context", 0usize), cx);
        window.render_frame(cx);
        assert_eq!(kubernetes_status(window), "Connecting to lab…");
    })
    .unwrap();
    wait_until(cx, handle, "lab to refuse again", |window, _| {
        kubernetes_status(window).starts_with("Couldn't connect to lab: ")
    });

    cx.update_window(handle, |_, window, cx| {
        contexts(window, cx);
        window.click(("context", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("staging"));
        contexts(window, cx);
        assert_eq!(window.find(("context", 1usize)).selected(), Some(true));
        contexts(window, cx);
        assert_eq!(window.find(("context", 0usize)).selected(), Some(false));
    })
    .unwrap();
    wait_until(cx, handle, "staging to refuse", |window, _| {
        kubernetes_status(window).starts_with("Couldn't connect to staging: ")
    });
}

#[gpui_kit::test]
fn kubernetes_only_talos_pages_ask_for_a_talosconfig(cx: &mut TestAppContext) {
    let path = kubeconfig_file("talos-pages");
    let (_runtime, handle, view) = mount_kubernetes_only(cx, path.clone(), None);
    // Settings lists the files read, and the read runs on Tokio, which
    // `run_until_parked` doesn't wait for.
    let read = view.clone();
    wait_until(cx, handle, "the kubeconfig to be read", move |_, cx| {
        !read.read(cx).config_loading
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for page in Page::ALL.into_iter().filter(|page| {
            !matches!(
                page,
                Page::Overview
                    | Page::Resources
                    | Page::Databases
                    | Page::Access
                    | Page::Health
                    | Page::Nodes
                    | Page::Monitoring
                    | Page::Observability
                    | Page::Applications
                    | Page::Settings
            )
        }) {
            view.update(cx, |view, cx| view.navigate(page, window, cx));
            window.render_frame(cx);
            assert_eq!(view.read(cx).navigation.page, page);
            assert!(window.find("needs-talosconfig").visible(), "{page:?}");
        }
        view.update(cx, |view, cx| view.navigate(Page::Databases, window, cx));
        window.render_frame(cx);
        assert!(window.find("databases-page").visible());
        assert!(window.try_find("needs-talosconfig").is_none());
        // Your access reads the cluster's reviews, which need no Talos.
        view.update(cx, |view, cx| view.navigate(Page::Access, window, cx));
        window.render_frame(cx);
        assert!(window.find("access-page").visible());
        assert!(window.try_find("needs-talosconfig").is_none());
        // Monitoring reads the cluster's Prometheus, which needs no Talos.
        view.update(cx, |view, cx| view.navigate(Page::Monitoring, window, cx));
        window.render_frame(cx);
        assert!(window.find("monitoring-page").visible());
        assert!(window.try_find("needs-talosconfig").is_none());
        view.update(cx, |view, cx| view.navigate(Page::Etcd, window, cx));
        window.render_frame(cx);
        window.click("browse-kubernetes", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).navigation.page, Page::Resources);
        assert!(window.try_find("needs-talosconfig").is_none());

        view.update(cx, |view, cx| view.navigate(Page::Etcd, window, cx));
        window.render_frame(cx);
        window.click("open-settings", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("talosconfig-path").visible());
        let files = format!("Kubeconfig files: {}", path.display());
        assert_eq!(window.find("kubeconfig-path").label(), Some(files.as_str()));
        // The Talos kubeconfig choices don't apply without Talos.
        assert!(window.try_find("kubeconfig-automatic").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubernetes_only_control_plane_offers_a_talosconfig(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = mount_kubernetes_only(cx, kubeconfig_file("rail"), None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        area(window, cx, "nav-control-plane");
        // The area shows why it is empty and keeps the page.
        assert_eq!(view.read(cx).navigation.page, Page::Overview);
        assert!(window.try_find("nav-etcd").is_none());
        window.click("add-talosconfig", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("talosconfig-path").is_none());
        window.click("settings-all", cx);
        window.render_frame(cx);
        window.click("nav-settings-connection", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("talosconfig-path").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubernetes_only_never_swaps_in_another_context(cx: &mut TestAppContext) {
    let (_runtime, handle, view) =
        mount_kubernetes_only(cx, kubeconfig_file("asked-for"), Some("prod"));
    wait_until(cx, handle, "prod to be missing", |window, _| {
        kubernetes_status(window).starts_with("Couldn't connect to prod: ")
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let status = kubernetes_status(window);
        assert!(status.contains("Context 'prod' was not found"), "{status}");
        let pilot = view.read(cx);
        assert_eq!(pilot.applied.context.as_deref(), Some("prod"));
        assert_eq!(pilot.contexts, ["lab", "staging"]);
        contexts(window, cx);
        assert_eq!(window.find(("context", 0usize)).selected(), Some(false));
    })
    .unwrap();
}

/// Kubernetes-only Health, after `what` failed: the reason, never "No node
/// selected". Returns the reason.
fn kubernetes_only_health_failure(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<Pilot>,
    what: &str,
) -> String {
    wait_until(cx, handle, what, |window, _| {
        window.try_find("k8s-unavailable").is_some()
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("screen-no-node").is_none());
        assert!(window.try_find("workloads-loading").is_none());
        match &view.read(cx).switch.registry.active().summary_health {
            Some(Err(reason)) => reason.clone(),
            _ => panic!("{what}: Health has no failure"),
        }
    })
    .unwrap()
}

fn show_health(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<Pilot>) {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.navigate(Page::Health, window, cx))
    })
    .unwrap();
}

/// Staging, not the kubeconfig's current lab: Health's Retry connects to
/// the applied context again, never the current one.
#[gpui_kit::test]
fn kubernetes_only_health_says_why_its_context_cant_be_read(cx: &mut TestAppContext) {
    let (_runtime, handle, view) =
        mount_kubernetes_only(cx, kubeconfig_file("health-refused"), Some("staging"));
    show_health(cx, handle, &view);
    let reason = kubernetes_only_health_failure(cx, handle, &view, "staging to refuse");
    // Health can say why before the connection's own check has answered.
    wait_until(cx, handle, "the connection to fail", |_, cx| {
        let pilot = view.read(cx);
        let kube = pilot.mode().kubernetes_only().unwrap();
        matches!(
            &kube.connection,
            super::super::kubernetes_only::KubeConnection::Failed(error) if *error == reason
        )
    });
    cx.update_window(handle, |_, window, cx| {
        let pilot = view.read(cx);
        let kube = pilot.mode().kubernetes_only().unwrap();
        assert!(
            matches!(
                &kube.connection,
                super::super::kubernetes_only::KubeConnection::Failed(error) if *error == reason
            ),
            "{reason}"
        );
        window.click("screen-retry", cx);
    })
    .unwrap();
    // Retry reads the kubeconfig again and shows "Connecting…" while it
    // does. Against a refusal that is over in an instant, so the status
    // can already be back to the failure before a frame draws it: the
    // test doesn't look for it, and "Connecting…" is covered where the
    // connection holds (`kubernetes_only_lists_kubeconfig_contexts_and_connects_to_the_current_one`).
    wait_until(cx, handle, "staging to refuse again", |window, _| {
        kubernetes_status(window).starts_with("Couldn't connect to staging: ")
    });
    kubernetes_only_health_failure(cx, handle, &view, "Health to say so again");
    cx.update_window(handle, |_, _, cx| {
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("staging"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubernetes_only_health_says_the_kubeconfig_is_missing_and_retries(cx: &mut TestAppContext) {
    let (_runtime, handle, view) =
        mount_kubernetes_only(cx, scratch("no-such-health-kubeconfig"), None);
    show_health(cx, handle, &view);
    let reason = kubernetes_only_health_failure(cx, handle, &view, "the kubeconfig to fail");
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).config_error.as_deref(), Some(reason.as_str()));
        // Retry reads the kubeconfig again: Health's table shows its
        // loading rows while it does.
        window.click("screen-retry", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).config_loading);
        assert!(window.try_find("workloads-loading").is_some());
        assert!(window.try_find("k8s-unavailable").is_none());
    })
    .unwrap();
    kubernetes_only_health_failure(cx, handle, &view, "the kubeconfig to fail again");
}

#[gpui_kit::test]
fn kubernetes_only_reports_a_missing_kubeconfig_and_switches_to_talos(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = mount_kubernetes_only(cx, scratch("no-such-kubeconfig"), None);
    wait_until(cx, handle, "the kubeconfig to fail", |window, _| {
        kubernetes_status(window).starts_with("No kubeconfig loaded: ")
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let status = kubernetes_status(window);
        assert!(
            status.contains("file is unreadable or does not exist"),
            "{status}"
        );
        assert!(view.read(cx).contexts.is_empty());
        view.update(cx, |view, cx| view.open_builtin("pods", window, cx));
        window.render_frame(cx);
        assert!(window.find("resource-disconnected").visible());

        // Applying no talosconfig keeps the window as it is: the default
        // one is what's missing.
        view.update(cx, |view, cx| {
            view.apply_talos_config(
                crate::backend::AppliedConfig {
                    path: None,
                    context: None,
                },
                window,
                cx,
            )
        });
        assert!(view.read(cx).mode().kubernetes_only().is_some());
        // A talosconfig switches the window to Talos.
        view.update(cx, |view, cx| {
            view.apply_talos_config(
                crate::backend::AppliedConfig {
                    path: Some(scratch("no-such-talosconfig")),
                    context: None,
                },
                window,
                cx,
            );
        });
        window.render_frame(cx);
        assert!(view.read(cx).mode().has_talos());
        assert!(window.find("context-switcher").visible());
        assert!(window.try_find("kubernetes-status").is_none());
    })
    .unwrap();
}

/// A kubeconfig whose server refuses at once, so nothing leaves this
/// machine, with a fake token.
const KUBECONFIG: &str = "apiVersion: v1
kind: Config
current-context: lab
clusters:
- name: lab
  cluster:
    server: https://127.0.0.1:1
contexts:
- name: lab
  context:
    cluster: lab
    user: lab
- name: staging
  context:
    cluster: lab
    user: lab
    namespace: web
users:
- name: lab
  user:
    token: not-a-real-token
";

/// The window `--kubernetes-only --kubeconfig <path>` opens. Never the
/// ambient kubeconfig: the path is always explicit.
fn mount_kubernetes_only(
    cx: &mut TestAppContext,
    kubeconfig: std::path::PathBuf,
    context: Option<&str>,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    // Reading and connecting run on Tokio and wake GPUI from there.
    cx.executor().allow_parking();
    let options = GpuiOptions::kubernetes_only(Some(kubeconfig), context.map(Into::into), 100);
    mount(cx, options, 1280., 860.)
}

fn retry_after_kubeconfig_edit(
    cx: &mut TestAppContext,
    page: Page,
    control: &'static str,
    inspecting: bool,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("kubeconfig");
    let config = KUBECONFIG.replace("https://127.0.0.1:1", "https://127.0.0.1:9");
    std::fs::write(&path, &config).unwrap();
    let (runtime, handle, view) = mount_kubernetes_only(cx, path.clone(), Some("lab"));
    wait_until(cx, handle, "the refused connection", |window, _| {
        kubernetes_status(window).starts_with("Couldn't connect to lab: ")
    });
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.navigate(page, window, cx));
    })
    .unwrap();
    let retry = if page == Page::Monitoring {
        "monitoring-retry"
    } else {
        "resource-retry"
    };
    wait_until(cx, handle, "the page's refused source", |window, _| {
        window.try_find(retry).is_some()
    });
    let old = cx.read(|cx| view.read(cx).source().kube.unwrap().id);
    // Keep the selected context, despite changing the file's current one,
    // and make the server edit observable without contacting any cluster.
    std::fs::write(
        &path,
        config
            .replace("current-context: lab", "current-context: staging")
            .replace("https://127.0.0.1:9", "https://127.0.0.1:9/edited"),
    )
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        if inspecting {
            view.update(cx, |view, cx| view.refresh_kubernetes(false, window, cx));
        }
        window.click(control, cx);
    })
    .unwrap();
    cx.run_until_parked();
    wait_until(
        cx,
        handle,
        "the first action to take up the edit",
        |window, cx| {
            (control == retry || view.read(cx).source().kube.unwrap().id != old)
                && window.try_find(retry).is_some()
                && kubernetes_status(window).starts_with("Couldn't connect to lab: ")
        },
    );
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert_ne!(
            pilot.source().kube.unwrap().id,
            old,
            "{control} must replace access on its first press"
        );
        assert_eq!(pilot.applied.context.as_deref(), Some("lab"));
    });
    let source = cx.read(|cx| view.read(cx).source().kube.unwrap());
    let crate::resources::ClusterAccess::Direct(access) = source.access else {
        panic!("the page must keep direct access");
    };
    assert_eq!(
        access.revision(),
        freshkube_core::ConfigurationRevision::from_kubeconfig_sources(&[path])
    );
    let probe = runtime.spawn(async move { access.connect().await.err().unwrap() });
    freshkube_ui::real_io::until_io(cx, "the edited source to refuse", |_| probe.is_finished());
    let error = runtime.block_on(probe).unwrap();
    assert_ne!(error.kind, freshkube_core::failure::FailureKind::Config);
    assert!(!error.message.contains("configuration changed"));
}

#[gpui_kit::test]
fn monitoring_try_again_takes_up_a_kubeconfig_edit_on_the_first_press(cx: &mut TestAppContext) {
    retry_after_kubeconfig_edit(cx, Page::Monitoring, "monitoring-retry", false);
}

#[gpui_kit::test]
fn monitoring_refresh_takes_up_a_kubeconfig_edit_on_the_first_press(cx: &mut TestAppContext) {
    retry_after_kubeconfig_edit(cx, Page::Monitoring, "monitoring-refresh", false);
}

#[gpui_kit::test]
fn resources_retry_takes_up_a_kubeconfig_edit_on_the_first_press(cx: &mut TestAppContext) {
    retry_after_kubeconfig_edit(cx, Page::Resources, "resource-retry", false);
}

#[gpui_kit::test]
fn resources_refresh_takes_up_a_kubeconfig_edit_on_the_first_press(cx: &mut TestAppContext) {
    retry_after_kubeconfig_edit(cx, Page::Resources, "resource-refresh", false);
}

#[gpui_kit::test]
fn monitoring_retry_joins_a_pending_kubeconfig_inspection(cx: &mut TestAppContext) {
    retry_after_kubeconfig_edit(cx, Page::Monitoring, "monitoring-retry", true);
}

#[gpui_kit::test]
async fn object_metadata_completion_rechecks_access_even_without_an_epoch_change(
    cx: &mut TestAppContext,
) {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let server_runtime = tokio::runtime::Runtime::new().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let received = Arc::new(AtomicBool::new(false));
    let release = Arc::new(tokio::sync::Notify::new());
    let (seen, gate) = (received.clone(), release.clone());
    let server = server_runtime.spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        while let Ok((mut socket, _)) = listener.accept().await {
            let (seen, gate) = (seen.clone(), gate.clone());
            tokio::spawn(async move {
                let mut request = vec![0u8; 16384];
                let n = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..n]);
                let (status, body) = if request.starts_with("GET /api/v1/namespaces/prod/pods/later ") {
                    assert!(request.contains("PartialObjectMetadata"));
                    seen.store(true, Ordering::SeqCst);
                    gate.notified().await;
                    (200, serde_json::json!({"apiVersion":"meta.k8s.io/v1","kind":"PartialObjectMetadata","metadata":{"namespace":"prod","name":"later","uid":"old-access-uid"}}))
                } else if request.starts_with("GET /version ") {
                    (200, serde_json::json!({"major":"1","minor":"32","gitVersion":"v1.32.0","gitCommit":"fixture","gitTreeState":"clean","buildDate":"2026-01-01T00:00:00Z","goVersion":"go1.24","compiler":"gc","platform":"test"}))
                } else {
                    (404, serde_json::json!({"apiVersion":"v1","kind":"Status","status":"Failure","reason":"NotFound","code":404,"message":"No fixture for this endpoint"}))
                };
                let body = body.to_string();
                let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config");
    std::fs::write(
        &config,
        KUBECONFIG.replace("https://127.0.0.1:1", &format!("http://{address}")),
    )
    .unwrap();
    let (_runtime, handle, view) = mount_kubernetes_only(cx, config, Some("lab"));
    let observed = view.clone();
    cx.wait_for_io(handle, move |_, cx| {
        observed.read(cx).source().kube.is_some()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.open_object(
                crate::resources::example::kind("pods").unwrap(),
                crate::resources::model::ObjectRef {
                    connection: None,
                    namespace: "prod".into(),
                    name: "later".into(),
                    uid: String::new(),
                },
                crate::resources::Tab::Overview,
                window,
                cx,
            );
        })
    })
    .unwrap();
    cx.wait_for_io(handle, move |_, _| received.load(Ordering::SeqCst))
        .await;
    let deliveries = super::super::probe::count("object-open-delivered");
    cx.update(|cx| {
        view.update(cx, |view, _| {
            // Simulate a replacement access independently of the existing
            // epoch/sequence guards, leaving the delayed worker alive.
            view.mode.set_example(true);
            view.applied.context = Some("replacement".into());
        })
    });
    release.notify_one();
    cx.wait_for_io(handle, move |_, _| {
        super::super::probe::count("object-open-delivered") > deliveries
    })
    .await;
    cx.read(|cx| {
        assert!(
            view.read(cx)
                .pages
                .resources
                .read(cx)
                .detail_identity(cx)
                .is_none()
        )
    });
    server.abort();
}
