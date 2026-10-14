use super::*;
use freshkube_ui::real_io::RealIo;

#[gpui_kit::test]
fn settings_shows_the_metrics_source_read_only_for_example_data(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        assert!(window.try_find("monitoring-source-settings").is_some());
        // Example data has nothing to choose.
        assert!(window.try_find("monitoring-source-mode").is_none());
        assert!(window.try_find("monitoring-source-test").is_none());
    })
    .unwrap();
}

/// Example data never reads or writes the saved Coroot connection, even
/// when the launch has a preferences folder: the field stays empty, nothing
/// reconnects, and `coroot.json` is left as it was.
#[gpui_kit::test]
async fn example_data_leaves_the_saved_coroot_connection_alone(cx: &mut TestAppContext) {
    // The live launch reads its kubeconfig on Tokio, which answers here.
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let directory = guard.path();
    let preferences = directory.join("preferences.json");
    let saved = directory.join("coroot.json");
    let contents = r#"{"url": "https://coroot.example.com", "auth": "anonymous", "project": null, "reconnect": false, "remember": false}"#;
    std::fs::write(&saved, contents).unwrap();

    // A live launch reads it, so the file is one the page would load. The
    // kubeconfig is missing, so nothing is contacted; its read is waited
    // for, so no answer arrives after the test.
    let live = GpuiOptions::kubernetes_only(Some(directory.join("missing")), Some("x".into()), 100)
        .with_preferences(Some(preferences.clone()));
    let (_runtime, handle, pilot) = mount(cx, live, 1280., 820.);
    cx.wait_for_io(handle, |_, cx| !pilot.read(cx).config_loading)
        .await;
    cx.update(|cx| {
        let url = pilot.read(cx).observability.read(cx).coroot_url(cx);
        assert_eq!(url, "https://coroot.example.com");
    });

    let example = GpuiOptions::fixture().with_preferences(Some(preferences));
    let (_runtime, handle, pilot) = mount(cx, example, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.navigate(Page::Observability, window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(pilot.read(cx).observability.read(cx).coroot_url(cx), "");
    });
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), contents);
}

#[gpui_kit::test]
fn browse_is_unavailable_for_example_data(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        window.click("browse-config", cx);
    })
    .unwrap();
    assert!(!cx.did_prompt_for_paths());
}

#[gpui_kit::test]
async fn chosen_talosconfig_and_context_survive_a_launch_without_arguments(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let directory = guard.path().to_path_buf();
    let preferences = directory.join("preferences.json");
    let chosen = directory.join("talosconfig");
    // Invalid synthetic certificates stop before any network connection.
    std::fs::write(&chosen, "context: alpha\ncontexts:\n  alpha: &entry\n    endpoints: [192.0.2.1]\n    ca: YQ==\n    crt: Yg==\n    key: Yw==\n  beta: *entry\n").unwrap();
    let options = GpuiOptions::new(Some(directory.join("missing")), None, 100)
        .with_preferences(Some(preferences.clone()));
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.wait_for_io(handle, |_, cx| view.read(cx).config_error.is_some())
        .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("browse-config-empty", cx);
    })
    .unwrap();
    cx.simulate_path_prompt_response(|_| Some(vec![chosen.clone()]));
    cx.wait_for_io(handle, |_, cx| view.read(cx).contexts == ["alpha", "beta"])
        .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        contexts(window, cx);
        window.click(("context", 1usize), cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("beta"));
    })
    .unwrap();
    let saved_path = directory.join("connection.json");
    cx.wait_for_io(handle, |_, _| {
        std::fs::read_to_string(&saved_path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|saved| saved["context"] == "beta")
    })
    .await;
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&saved_path).unwrap()).unwrap();
    assert_eq!(saved["talosconfig"], chosen.display().to_string());
    assert!(saved.get("ca").is_none() && saved.get("crt").is_none() && saved.get("key").is_none());

    let restored = GpuiOptions::new(None, None, 100).with_preferences(Some(preferences));
    let (_runtime2, handle2, view2) = mount(cx, restored, 1280., 820.);
    cx.wait_for_io(handle2, |_, cx| {
        view2.read(cx).contexts == ["alpha", "beta"]
    })
    .await;
    cx.update_window(handle2, |_, window, cx| {
        window.render_frame(cx);
        let pilot = view2.read(cx);
        assert_eq!(pilot.applied.path.as_deref(), Some(chosen.as_path()));
        assert_eq!(pilot.applied.context.as_deref(), Some("beta"));
        contexts(window, cx);
        assert_eq!(window.find(("context", 1usize)).selected(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
async fn kubeconfig_file_applies_its_current_context_then_a_chosen_one(cx: &mut TestAppContext) {
    use freshkube_core::cluster_overview::KubeconfigSelection;
    // Inspection runs on Tokio; let GPUI park for its completion.
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let directory = guard.path().to_path_buf();
    let chosen = directory.join("kubeconfig");
    std::fs::write(
        &chosen,
        "apiVersion: v1\nkind: Config\ncurrent-context: beta\nclusters:\n- name: c\n  cluster:\n    server: https://192.0.2.1:6443\ncontexts:\n- name: alpha\n  context: {cluster: c, user: u}\n- name: beta\n  context: {cluster: c, user: u}\nusers:\n- name: u\n  user: {}\n",
    )
    .unwrap();
    // Never the user's default talosconfig: start from a missing file.
    let options = GpuiOptions::new(Some(directory.join("missing")), None, 100);
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        // File mode waits for a file; nothing changes yet.
        window.click("kubeconfig-file", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).kubeconfig, KubeconfigSelection::Automatic);
        assert!(window.find("kubeconfig-path").visible());
        window.click("browse-kubeconfig", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && !options.directories && !options.multiple);
        Some(vec![chosen.clone()])
    });
    cx.wait_for_io(handle, |_, cx| {
        view.read(cx).kubeconfig_draft.inspection.is_some()
    })
    .await;
    // A valid current context is used right away.
    cx.update(|cx| {
        assert_eq!(
            view.read(cx).kubeconfig,
            KubeconfigSelection::File {
                path: chosen.clone(),
                context: None
            }
        );
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        assert_eq!(window.find(("kube-context", 1usize)).selected(), Some(true));
        window.click(("kube-context", 0usize), cx);
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).kubeconfig,
            KubeconfigSelection::File {
                path: chosen.clone(),
                context: Some("alpha".into())
            }
        );
        assert_eq!(window.find(("kube-context", 0usize)).selected(), Some(true));
        window.click("kubeconfig-talos", cx);
        assert_eq!(
            view.read(cx).kubeconfig,
            KubeconfigSelection::TalosControlPlane
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn connection_failure_offers_a_picker_and_bad_files_do_not_replace_saved_selection(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let directory = guard.path().to_path_buf();
    let preferences = directory.join("preferences.json");
    let first = directory.join("first");
    let second = directory.join("second");
    let invalid = directory.join("invalid");
    // These synthetic certificates fail parsing before a network dial.
    for (path, context) in [(&first, "alpha"), (&second, "beta")] {
        std::fs::write(path, format!("context: {context}\ncontexts:\n  {context}:\n    endpoints: [192.0.2.1]\n    ca: YQ==\n    crt: Yg==\n    key: Yw==\n")).unwrap();
    }
    std::fs::write(&invalid, "not a talosconfig").unwrap();
    let options = GpuiOptions::new(Some(first.clone()), None, 100)
        .with_preferences(Some(preferences.clone()));
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.wait_for_io(handle, |window, _| {
        window.try_find("browse-config-unreachable").is_some()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.click("browse-config-unreachable", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(view.read(cx).applied.path.as_deref(), Some(first.as_path())));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("browse-config-unreachable", cx);
    })
    .unwrap();
    cx.simulate_path_prompt_response(|_| Some(vec![invalid.clone()]));
    cx.wait_for_io(handle, |_, cx| view.read(cx).config_error.is_some())
        .await;
    let saved_path = directory.join("connection.json");
    cx.wait_for_io(handle, |_, _| {
        std::fs::read_to_string(&saved_path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|saved| saved["context"] == "alpha")
    })
    .await;
    let restored = GpuiOptions::new(None, None, 100).with_preferences(Some(preferences.clone()));
    assert_eq!(restored.config_path(), Some(first.as_path()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("browse-config-empty", cx);
    })
    .unwrap();
    cx.simulate_path_prompt_response(|_| Some(vec![second.clone()]));
    cx.wait_for_io(handle, |_, cx| view.read(cx).contexts == ["beta"])
        .await;
    cx.wait_for_io(handle, |_, _| {
        std::fs::read_to_string(&saved_path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|saved| saved["context"] == "beta")
    })
    .await;
    let restored = GpuiOptions::new(None, None, 100).with_preferences(Some(preferences));
    assert_eq!(restored.config_path(), Some(second.as_path()));
    assert_eq!(restored.context(), Some("beta"));
    cx.run_until_parked();
}

#[gpui_kit::test]
async fn kubeconfig_without_a_usable_context_is_not_applied(cx: &mut TestAppContext) {
    use freshkube_core::cluster_overview::KubeconfigSelection;
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let directory = guard.path().to_path_buf();
    let unreadable = directory.join("not-yaml");
    std::fs::write(&unreadable, "{{{ not a kubeconfig").unwrap();
    let options = GpuiOptions::new(Some(directory.join("missing")), None, 100);
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        window.click("kubeconfig-file", cx);
        window.render_frame(cx);
        window.click("browse-kubeconfig", cx);
    })
    .unwrap();
    cx.simulate_path_prompt_response(|_| Some(vec![unreadable.clone()]));
    cx.wait_for_io(handle, |_, cx| {
        view.read(cx).kubeconfig_draft.inspection.is_some()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).kubeconfig, KubeconfigSelection::Automatic);
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        assert!(window.find("kubeconfig-error").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubeconfig_choice_is_unavailable_for_example_data(cx: &mut TestAppContext) {
    use freshkube_core::cluster_overview::KubeconfigSelection;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        window.click("kubeconfig-talos", cx);
        assert_eq!(view.read(cx).kubeconfig, KubeconfigSelection::Automatic);
    })
    .unwrap();
}

#[gpui_kit::test]
async fn browse_picks_a_talosconfig_and_reloads_contexts(cx: &mut TestAppContext) {
    // Configuration loads on Tokio; let GPUI park for its completion.
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let directory = guard.path().to_path_buf();
    let chosen = directory.join("config");
    std::fs::write(
        &chosen,
        "context: alpha\ncontexts:\n  alpha: &entry\n    endpoints: [192.0.2.1]\n    ca: YQ==\n    crt: Yg==\n    key: Yw==\n  beta: *entry\n",
    )
    .unwrap();
    // Never the user's default talosconfig: start from a missing file.
    let options = GpuiOptions::new(Some(directory.join("missing")), None, 100);
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.wait_for_io(handle, |_, cx| view.read(cx).config_error.is_some())
        .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The error screen offers Browse directly; cancelling changes nothing.
        window.click("browse-config-empty", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    cx.update(|cx| assert!(view.read(cx).config_error.is_some()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        window.click("browse-config", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && !options.directories && !options.multiple);
        Some(vec![chosen.clone()])
    });
    cx.wait_for_io(handle, |_, cx| view.read(cx).contexts == ["alpha", "beta"])
        .await;
    cx.update(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.applied.path.as_deref(), Some(chosen.as_path()));
        assert_eq!(pilot.path.read(cx).value(), chosen.display().to_string());
        assert!(pilot.config_error.is_none());
    });
}
