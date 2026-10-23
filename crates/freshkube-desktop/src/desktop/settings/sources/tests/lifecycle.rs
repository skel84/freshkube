use super::*;

/// A live workspace whose entries' credential plugin sleeps, so a check
/// stays in flight for as long as a test needs.
#[cfg(unix)]
fn waiting_workspace(directory: &std::path::Path) {
    let kubeconfig = directory.join("kubeconfig");
    let user = serde_json::json!({"exec": {
        "apiVersion": "client.authentication.k8s.io/v1beta1",
        "command": "sleep",
        "args": ["2"],
        "interactiveMode": "Never",
    }});
    let config = serde_json::json!({
        "apiVersion": "v1",
        "kind": "Config",
        "clusters": [{"name": "lab", "cluster": {"server": "https://127.0.0.1:1"}}],
        "users": [{"name": "plugin", "user": user}],
        "contexts": [
            {"name": "acme-a", "context": {"cluster": "lab", "user": "plugin"}},
            {"name": "acme-b", "context": {"cluster": "lab", "user": "plugin"}},
        ],
    });
    std::fs::write(&kubeconfig, config.to_string()).unwrap();
    write(
        directory,
        &serde_json::json!({
            "version": 1,
            "kubeconfig": kubeconfig,
            "clusters": [
                {"id": "cluster-a", "role": "core", "context": "acme-a"},
                {"id": "cluster-b", "role": "cicd", "context": "acme-b"},
            ],
        })
        .to_string(),
    );
}

/// A check in flight is dropped by a new workspace, by another section and
/// by the page hiding, a press while it runs starts nothing, and a late
/// answer never lands.
#[cfg(unix)]
#[gpui_kit::test]
async fn a_check_in_flight_is_dropped_and_its_answer_never_lands(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    waiting_workspace(guard.path());
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert_eq!(state(&view, KARGO, cx), "Reading");
        let sources = list(&view, cx);
        let first = sources.read(cx).read.revision();
        let first_request = request(&sources, cx);
        // C while it runs starts nothing.
        list(&view, cx).read(cx).focus_handle().focus(window, cx);
        window.press("c", cx);
        assert_eq!(sources.read(cx).read.revision(), first);

        // (a) A new workspace drops it, and its answer is ignored.
        let reloaded = view.read(cx).settings_page.read(cx).workspace().clone();
        set_workspace(&view, reloaded, false, cx);
        assert_eq!(state(&view, KARGO, cx), "Not checked");
        assert!(!sources.read(cx).is_checking());
        sources.update(cx, |list, cx| {
            list.answer(&first_request, Ok(Checked::default()), cx)
        });
        assert_eq!(state(&view, KARGO, cx), "Not checked");

        // (b) Checked again, the first answer still never lands.
        window.click("settings-sources-check", cx);
        let second = sources.read(cx).read.revision();
        let second_request = request(&sources, cx);
        assert_ne!(second, first);
        sources.update(cx, |list, cx| {
            list.answer(&first_request, Ok(Checked::default()), cx)
        });
        assert_eq!(state(&view, KARGO, cx), "Reading");

        // (c) Another section of Settings drops it, and coming back starts
        // nothing.
        view.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Clusters, window, cx)
        });
        assert!(!sources.read(cx).is_checking());
        assert_eq!(sources.read(cx).check, Check::Never);
        sources.update(cx, |list, cx| {
            list.answer(&second_request, Ok(Checked::default()), cx)
        });
        open(window, cx, &view);
        assert!(!sources.read(cx).is_checking());
        assert_eq!(state(&view, KARGO, cx), "Not checked");

        // (d) Leaving Settings drops it.
        window.click("settings-sources-check", cx);
        let third_request = request(&sources, cx);
        view.update(cx, |pilot, cx| {
            pilot.navigate_from_keyboard(Page::Overview, window, cx)
        });
        assert!(!sources.read(cx).is_checking());
        assert_eq!(sources.read(cx).check, Check::Never);
        sources.update(cx, |list, cx| {
            list.answer(&third_request, Ok(Checked::default()), cx)
        });
        assert_eq!(state(&view, KARGO, cx), "Not checked");
    })
    .unwrap();
}

/// A check that outlasts its deadline fails, and says so on each row it
/// would have read. The deadline is Tokio's, so the test shortens it rather
/// than advancing GPUI's clock.
#[cfg(unix)]
#[gpui_kit::test]
async fn a_check_past_its_deadline_fails(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    waiting_workspace(guard.path());
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |list, _| list.deadline = Duration::from_secs(1));
        window.click("settings-sources-check", cx);
    })
    .unwrap();
    let waiting = view.clone();
    cx.wait_for_io(handle, move |_, cx| {
        !list(&waiting, cx).read(cx).is_checking()
    })
    .await;
    cx.update_window(handle, |_, _, cx| {
        assert!(matches!(list(&view, cx).read(cx).check, Check::Failed(_)));
        assert_eq!(state(&view, KARGO, cx), "Unreachable");
        assert_eq!(state(&view, ARGOCD, cx), "Unreachable");
        // No destination is mapped, which needed no read.
        assert_eq!(state(&view, WORKLOADS, cx), "Not in the workspace");
        let kargo = tooltip(&view, KARGO, cx);
        assert!(kargo.contains("took longer than 1 s"), "{kargo}");
    })
    .unwrap();
}
