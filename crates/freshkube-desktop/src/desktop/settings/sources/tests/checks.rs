use super::*;

#[gpui_kit::test]
fn check_sources_finds_each_state(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open(window, cx, &view);
        list(&view, cx).update(cx, |list, _| {
            list.set_example_answer("cicd-fra", KARGO, Reach::refused("forbidden"));
            list.set_example_answer(
                "cicd-fra",
                BUILDS,
                Reach::unreachable("no answer within 5 s"),
            );
            list.set_example_answer("core-fra", ARGOCD, Reach::not_served("not served"));
            list.set_example_answer(
                "prod-ams",
                WORKLOADS,
                Reach::unreachable("no answer within 5 s"),
            );
        });
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert_eq!(
            states(&view, cx),
            vec![
                row(KARGO, "cicd-fra", "Refused"),
                row(BUILDS, "cicd-fra", "Unreachable"),
                row(ARGOCD, "core-fra", "Not served"),
                row(WORKLOADS, MAPPED, "2 readable, 1 unreachable"),
            ]
        );
        for read in ["kargo", "builds", "argocd", "workloads"] {
            let mark = format!("settings-source-{read}-mark");
            assert!(window.find(mark).visible(), "{read}");
        }
        assert_eq!(
            window.find("settings-source-kargo-mark").label(),
            Some("Refused")
        );
        // Each workloads entry says its own state.
        let workloads = tooltip(&view, WORKLOADS, cx);
        assert!(
            workloads.contains("Workloads answered on dev-fra")
                && workloads.contains("Couldn't reach prod-ams: no answer within 5 s"),
            "{workloads}"
        );
        // Argo CD says where it was read.
        let argocd = tooltip(&view, ARGOCD, cx);
        assert!(argocd.contains("argocd-cm in argocd"), "{argocd}");

        // Checked again, everything answers.
        list(&view, cx).update(cx, |list, _| list.example_answers.clear());
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert_eq!(state(&view, WORKLOADS, cx), "3 readable");
        assert!(
            states(&view, cx)[..3]
                .iter()
                .all(|(_, _, state)| state == "Readable")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_part_with_no_entry_says_so_before_any_check(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        // No core entry, Builds pinned to an entry that is gone, and no
        // destination mapped.
        let mut lone = Workspace::new(vec![
            Entry::new("cluster-b", Role::Cicd, "cluster-b"),
            Entry::new("cluster-c", Role::Environment, "cluster-c"),
        ]);
        lone.sources.set(Part::Builds, Some("cluster-x".into()));
        set_workspace(&view, lone, true, cx);
        open(window, cx, &view);
        assert_eq!(
            states(&view, cx),
            vec![
                row(KARGO, "cluster-b", "Not checked"),
                row(BUILDS, "cluster-x", "Not in the workspace"),
                row(ARGOCD, "None", "Not in the workspace"),
                row(WORKLOADS, "None", "Not in the workspace"),
            ]
        );
        assert!(window.find("settings-source-argocd-mark").visible());
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert_eq!(
            states(&view, cx),
            vec![
                row(KARGO, "cluster-b", "Readable"),
                row(BUILDS, "cluster-x", "Not in the workspace"),
                row(ARGOCD, "None", "Not in the workspace"),
                row(WORKLOADS, "None", "Not in the workspace"),
            ]
        );
        let argocd = tooltip(&view, ARGOCD, cx);
        assert!(
            argocd.contains("No Core entry in the workspace"),
            "{argocd}"
        );
        let workloads = tooltip(&view, WORKLOADS, cx);
        assert!(
            workloads.contains("map one under Destinations"),
            "{workloads}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn c_checks_from_the_list(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open(window, cx, &view);
        list(&view, cx).read(cx).focus_handle().focus(window, cx);
        window.press("c", cx);
        window.render_frame(cx);
        assert_eq!(state(&view, KARGO, cx), "Readable");
    })
    .unwrap();
}

/// An entry the kubeconfig doesn't define is unreachable, and says why; the
/// check reads only the workspace's own kubeconfig, which here is missing.
#[gpui_kit::test]
async fn a_live_entry_missing_from_the_kubeconfig_is_unreachable(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let kubeconfig = guard.path().join("kubeconfig");
    write(
        guard.path(),
        &serde_json::json!({
            "version": 1,
            "kubeconfig": kubeconfig,
            "clusters": [
                {"id": "cluster-a", "role": "core", "context": "acme-a"},
                {"id": "cluster-b", "role": "cicd", "context": "acme-b"},
                {"id": "cluster-c", "role": "environment", "context": "acme-c"},
            ],
            "destinations": [{"name": "c", "entry": "cluster-c"}],
        })
        .to_string(),
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert!(
            states(&view, cx)
                .iter()
                .all(|(_, _, state)| state == "Reading")
        );
    })
    .unwrap();
    let waiting = view.clone();
    cx.wait_for_io(handle, move |_, cx| {
        !list(&waiting, cx).read(cx).is_checking()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            states(&view, cx),
            vec![
                row(KARGO, "cluster-b", "Unreachable"),
                row(BUILDS, "cluster-b", "Unreachable"),
                row(ARGOCD, "cluster-a", "Unreachable"),
                row(WORKLOADS, "cluster-c", "Unreachable"),
            ]
        );
        let kargo = tooltip(&view, KARGO, cx);
        assert!(
            kargo.contains("Couldn't reach cluster-b: the kubeconfig has no context acme-b"),
            "{kargo}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn checking_uses_loading_rows_and_completion_publishes_the_status_time(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open(window, cx, &view);
        let sources = list(&view, cx);
        let request = sources.update(cx, |list, cx| {
            let request = list.read.start(list.identity());
            list.check = super::Check::Running;
            list.derive_rows();
            list.publish_status(cx);
            cx.notify();
            request
        });
        window.render_frame(cx);
        assert!(window.find("settings-sources-loading").visible());
        assert!(window.try_find("settings-source-kargo").is_none());
        assert!(
            !gpui_kit::base::test_support::snapshots(window)
                .iter()
                .any(|s| s.label() == Some("Checking…"))
        );
        sources.update(cx, |list, cx| {
            list.answer(&request, Ok(list.example_check()), cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("settings-sources-loading").is_none());
        assert!(window.find("settings-source-kargo").visible());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let checked = list(&view, cx).read(cx).checked_at.clone().unwrap();
        let page = view.read(cx).settings_page.read(cx);
        let status = page.status().unwrap();
        assert_eq!(
            status.text(None).as_ref(),
            format!("Delivery sources · Last checked {checked}")
        );
    })
    .unwrap();
}
