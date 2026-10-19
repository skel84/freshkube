use super::*;
use freshkube_ui::real_io::RealIo;

/// Open the section through the same navigation as its column row, then
/// let the deferred field setup finish outside the shell update.
fn open_connection(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<Pilot>) {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.show_settings(
                super::super::settings::SettingsSection::Connection,
                window,
                cx,
            )
        });
    })
    .unwrap();
    cx.run_until_parked();
}

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
        let url = pilot.read(cx).pages.observability.read(cx).coroot_url(cx);
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
        assert_eq!(
            pilot.read(cx).pages.observability.read(cx).coroot_url(cx),
            ""
        );
    });
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), contents);
}

#[gpui_kit::test]
fn browse_is_unavailable_for_example_data(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    open_connection(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
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
    open_connection(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
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
        window.render_frame(cx);
        window.click("kubeconfig-automatic", cx);
        assert_eq!(view.read(cx).kubeconfig, KubeconfigSelection::Automatic);
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
    open_connection(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
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
        assert!(window.find("kubeconfig-error").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubeconfig_choice_is_unavailable_for_example_data(cx: &mut TestAppContext) {
    use freshkube_core::cluster_overview::KubeconfigSelection;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    open_connection(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
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
    open_connection(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("browse-config", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && !options.directories && !options.multiple);
        Some(vec![chosen.clone()])
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(view.read(cx).contexts.is_empty());
        window.render_frame(cx);
        window.click("save-connection", cx);
    })
    .unwrap();
    cx.wait_for_io(handle, |_, cx| view.read(cx).contexts == ["alpha", "beta"])
        .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let pilot = view.read(cx);
        assert_eq!(pilot.applied.path.as_deref(), Some(chosen.as_path()));
        assert_eq!(
            window.find("talosconfig-path").value(),
            Some(chosen.to_str().unwrap())
        );
        assert!(pilot.config_error.is_none());
    })
    .unwrap();
}

/// No client can dial with these synthetic certificates.
fn talos_file(directory: &std::path::Path) -> std::path::PathBuf {
    let path = directory.join("talosconfig");
    std::fs::write(&path, "context: alpha\ncontexts:\n  alpha: &entry\n    endpoints: [192.0.2.1]\n    ca: YQ==\n    crt: Yg==\n    key: Yw==\n  beta: *entry\n").unwrap();
    path
}

fn replace_input(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, id: &str, value: &str) {
    window.click(id.to_owned(), cx);
    window.press("secondary-a", cx);
    window.input(value, cx);
}

#[gpui_kit::test]
async fn talos_context_saves_on_return_and_revert_restores_both_fields(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let path = talos_file(directory.path());
    let preferences = directory.path().join("preferences.json");
    let saved = directory.path().join("connection.json");
    std::fs::write(
        &saved,
        serde_json::to_vec(&serde_json::json!({
            "talosconfig": path, "context": "alpha", "future": {"kept": true}
        }))
        .unwrap(),
    )
    .unwrap();
    let (_runtime, handle, view) = mount(
        cx,
        GpuiOptions::new(Some(path.clone()), Some("alpha".into()), 100)
            .with_preferences(Some(preferences)),
        1280.,
        880.,
    );
    cx.wait_for_io(handle, |_, cx| view.read(cx).contexts == ["alpha", "beta"])
        .await;
    open_connection(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        replace_input(window, cx, "talos-context", "beta");
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("alpha"));
        window.render_frame(cx);
        assert!(window.find("connection-talos-dirty").visible());
        window.press("enter", cx);
    })
    .unwrap();
    cx.wait_for_io(handle, |_, cx| {
        view.read(cx).applied.context.as_deref() == Some("beta")
    })
    .await;
    cx.wait_for_io(handle, |_, _| {
        std::fs::read(&saved)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|value| value["context"] == "beta")
    })
    .await;
    let before = std::fs::read(&saved).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&before).unwrap()["future"]["kept"],
        true
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("connection-talos-dirty").is_none());
        replace_input(
            window,
            cx,
            "talosconfig-path",
            &directory.path().join("draft").display().to_string(),
        );
        replace_input(window, cx, "talos-context", "draft-context");
        window.render_frame(cx);
        window.click("revert-connection", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("talosconfig-path").value(),
            Some(path.to_str().unwrap())
        );
        assert_eq!(window.find("talos-context").value(), Some("beta"));
        assert!(window.try_find("connection-talos-dirty").is_none());
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("beta"));
        window.click("save-connection", cx);
        window.click("revert-connection", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read(&saved).unwrap(), before);
}

#[gpui_kit::test]
async fn invalid_talos_draft_keeps_the_applied_connection_and_reverts(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let path = talos_file(directory.path());
    let invalid = directory.path().join("invalid");
    std::fs::write(&invalid, "not a talosconfig").unwrap();
    let (_runtime, handle, view) = mount(
        cx,
        GpuiOptions::new(Some(path.clone()), None, 100),
        1280.,
        880.,
    );
    cx.wait_for_io(handle, |_, cx| view.read(cx).contexts == ["alpha", "beta"])
        .await;
    open_connection(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        replace_input(window, cx, "talosconfig-path", invalid.to_str().unwrap());
        window.render_frame(cx);
        assert_eq!(
            window.find("talosconfig-path").value(),
            Some(invalid.to_str().unwrap())
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("save-connection", cx);
    })
    .unwrap();
    cx.wait_for_io(handle, |window, _| {
        window.try_find("connection-talos-path-error").is_some()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).applied.path.as_deref(), Some(path.as_path()));
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("alpha"));
        assert_eq!(window.find("talosconfig-path").focused(), Some(true));
        window.click("revert-connection", cx);
        window.render_frame(cx);
        assert!(window.try_find("connection-talos-path-error").is_none());
        assert_eq!(
            window.find("talosconfig-path").value(),
            Some(path.to_str().unwrap())
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn talos_context_validates_on_blur_and_escape_reverts_the_draft(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let path = talos_file(directory.path());
    let (_runtime, handle, view) = mount(cx, GpuiOptions::new(Some(path), None, 100), 1280., 880.);
    cx.wait_for_io(handle, |_, cx| view.read(cx).contexts == ["alpha", "beta"])
        .await;
    cx.update_window(handle, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    open_connection(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        replace_input(window, cx, "talos-context", "missing-context");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("talosconfig-path", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.wait_for_io(handle, |window, _| {
        window.try_find("connection-talos-context-error").is_some()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("alpha"));
        replace_input(window, cx, "talos-context", "beta");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.press("escape", cx);
        window.render_frame(cx);
        assert_eq!(window.find("talos-context").value(), Some("alpha"));
        assert!(window.try_find("connection-talos-dirty").is_none());
        assert!(window.try_find("connection-talos-context-error").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn kubernetes_only_browse_from_connection_keeps_context_choice_explicit(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("kubeconfig");
    std::fs::write(&path, "apiVersion: v1\nkind: Config\ncurrent-context: alpha\nclusters:\n- name: example\n  cluster: {server: 'https://127.0.0.1:1'}\ncontexts:\n- name: alpha\n  context: {cluster: example, user: example}\nusers:\n- name: example\n  user: {}\n").unwrap();
    let (_runtime, handle, view) = mount(
        cx,
        GpuiOptions::kubernetes_only(Some(directory.path().join("missing")), None, 100),
        1280.,
        880.,
    );
    cx.wait_for_io(handle, |_, cx| !view.read(cx).config_loading)
        .await;
    open_connection(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("kubeconfig-mode").is_none());
        assert_eq!(window.find("talos-context").value(), Some(""));
        window.click("browse-kubeconfig", cx);
    })
    .unwrap();
    cx.simulate_path_prompt_response(|_| Some(vec![path.clone()]));
    cx.wait_for_io(handle, |_, cx| view.read(cx).contexts == ["alpha"])
        .await;
    cx.update_window(handle, |_, window, cx| {
        assert!(view.read(cx).applied.context.is_none());
        assert_eq!(
            window.find("kubeconfig-path").label(),
            Some(format!("Kubeconfig files: {}", path.display()).as_str())
        );
        contexts(window, cx);
        window.click(("context", 0usize), cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("alpha"));
    })
    .unwrap();
}

#[gpui_kit::test]
async fn error_state_browse_applies_and_reloads_the_chosen_talosconfig(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let chosen = talos_file(directory.path());
    let options = GpuiOptions::new(Some(directory.path().join("missing")), None, 100);
    let (_runtime, handle, view) = mount(cx, options, 1280., 880.);
    cx.wait_for_io(handle, |window, _| {
        window.try_find("browse-config-empty").is_some()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.click("browse-config-empty", cx);
    })
    .unwrap();
    cx.simulate_path_prompt_response(|_| Some(vec![chosen.clone()]));
    cx.wait_for_io(handle, |_, cx| view.read(cx).contexts == ["alpha", "beta"])
        .await;
    cx.update(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.applied.path.as_deref(), Some(chosen.as_path()));
        assert_eq!(pilot.applied.context.as_deref(), Some("alpha"));
        assert!(pilot.config_error.is_none());
    });

    // Invalid synthetic certificates reach the error state without a dial.
    // Choosing the same file must read its new current context too.
    cx.wait_for_io(handle, |window, _| {
        window.try_find("browse-config-unreachable").is_some()
    })
    .await;
    let contents = std::fs::read_to_string(&chosen)
        .unwrap()
        .replace("context: alpha\n", "context: beta\n");
    std::fs::write(&chosen, contents).unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.click("browse-config-unreachable", cx);
    })
    .unwrap();
    cx.simulate_path_prompt_response(|_| Some(vec![chosen.clone()]));
    cx.wait_for_io(handle, |_, cx| {
        view.read(cx).applied.context.as_deref() == Some("beta")
    })
    .await;
    cx.update(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.applied.path.as_deref(), Some(chosen.as_path()));
        assert_eq!(pilot.contexts, ["alpha", "beta"]);
        assert!(pilot.config_error.is_none());
    });
}
