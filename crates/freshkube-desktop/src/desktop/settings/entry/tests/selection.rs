use super::*;

#[gpui_kit::test]
async fn a_first_right_click_can_edit_without_bypassing_unsaved_changes(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.right_click("settings-cluster-prod", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("popup-menu").visible());
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-detail").visible());
        assert_eq!(
            window.find("settings-identity-context").focused(),
            Some(true)
        );
        window.press("secondary-a", cx);
        window.input("acme-edited", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click("settings-cluster-mgmt", cx);
    })
    .unwrap();
    let (message, _) = cx.pending_prompt().expect("unsaved changes still ask");
    assert!(message.contains("prod"), "{message}");
    cx.simulate_prompt_answer("Stay");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The popup is for mgmt, so its commands cannot change prod.
        assert!(window.find("popup-menu").visible());
        window.within("popup-menu").click(0usize, cx);
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).settings_page.read(cx).selected.as_deref(),
            Some("prod")
        );
    })
    .unwrap();
    assert_eq!(values(cx, handle, &view).0.context, "acme-edited");
    assert_eq!(
        entry(&read_workspace(guard.path()), "prod").context,
        "acme-prod"
    );
}

#[gpui_kit::test]
async fn selecting_a_cluster_fills_its_groups(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), &rich());
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-detail").is_none());
    })
    .unwrap();
    select(cx, handle, "prod");
    let (identity, metrics) = values(cx, handle, &view);
    assert_eq!(identity.role, Role::Environment);
    assert_eq!(identity.context, "acme-prod");
    assert_eq!(identity.talosconfig, abs("talosconfig"));
    assert_eq!(identity.talos_context, "acme-prod-talos");
    assert_eq!(metrics.kind, metrics::Kind::Url);
    assert_eq!(metrics.url, "https://metrics.example.test");
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("settings-detail-title").label(), Some("prod"));
        for id in [
            "settings-identity",
            // Protection sits between Identity and Metrics (#852), so Metrics
            // is below the fold until the Inspector scrolls to it.
            "settings-protection",
            "settings-identity-save",
            "settings-identity-revert",
            "settings-identity-saved-in",
        ] {
            assert!(window.find(id).visible(), "{id}");
        }
        // Nothing is typed, so no group is marked.
        assert!(window.try_find("settings-identity-dirty").is_none());
        assert!(window.try_find("settings-metrics-dirty").is_none());
    })
    .unwrap();
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, _| {
        for id in [
            "settings-metrics-save",
            "settings-metrics-revert",
            "settings-metrics-saved-in",
        ] {
            assert!(window.find(id).visible(), "{id}");
        }
    })
    .unwrap();
    select(cx, handle, "mgmt");
    let (_, metrics) = values(cx, handle, &view);
    assert_eq!(metrics.kind, metrics::Kind::Mode);
    assert_eq!(metrics.value, "acme-core");
}
