use super::*;

#[gpui_kit::test]
async fn identity_errors_show_at_the_field_and_save_nothing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let before = std::fs::read(guard.path().join("freshkube.yaml")).unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "");
    set(
        cx,
        handle,
        &view,
        Field::Talosconfig,
        "relative/talosconfig",
    );
    set(cx, handle, &view, Field::TalosContext, "acme");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-identity-save", cx);
        window.render_frame(cx);
        for row in [
            "settings-identity-context-row-error",
            "settings-identity-talosconfig-row-error",
        ] {
            assert!(window.find(row).visible(), "{row}");
        }
        let page = view.read(cx).settings_page.read(cx);
        assert!(
            page.field_error(Field::Context)
                .unwrap()
                .contains("kubeconfig context")
        );
        assert!(
            page.field_error(Field::Talosconfig)
                .unwrap()
                .contains("absolute path")
        );
        // The first failing field takes the keyboard.
        let page = view.read(cx).settings_page.read(cx);
        let draft = page.entry.as_ref().unwrap();
        assert!(
            draft
                .identity
                .context
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read(guard.path().join("freshkube.yaml")).unwrap(),
        before
    );
    // Typing into a field clears its own error only.
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-identity-context", cx);
        window.input("x", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .try_find("settings-identity-context-row-error")
                .is_none()
        );
        assert!(
            window
                .find("settings-identity-talosconfig-row-error")
                .visible()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_field_is_checked_when_it_loses_the_keyboard(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    // Focus changes reach the fields only in an active window.
    cx.update_window(handle, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-identity-talosconfig-input", cx);
        window.input("relative/path", cx);
        window.render_frame(cx);
        assert_eq!(
            view.read(cx)
                .settings_page
                .read(cx)
                .entry
                .as_ref()
                .unwrap()
                .identity
                .values(cx)
                .talosconfig,
            "relative/path"
        );
        assert!(
            window
                .try_find("settings-identity-talosconfig-row-error")
                .is_none()
        );
        window.click("settings-identity-context", cx);
    })
    .unwrap();
    // The page hears of the keyboard moving once the click's update is over.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("settings-identity-talosconfig-row-error")
                .visible()
        );
        // Only the field that lost the keyboard is judged.
        assert!(
            window
                .try_find("settings-identity-talos-context-row-error")
                .is_none()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_service_needs_a_namespace_a_name_and_a_port_and_a_url_an_address(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let before = std::fs::read(guard.path().join("freshkube.yaml")).unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-source-service", cx);
        window.render_frame(cx);
    })
    .unwrap();
    set(cx, handle, &view, Field::Port, "70000");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-save", cx);
        window.render_frame(cx);
        for row in [
            "settings-metrics-namespace-row-error",
            "settings-metrics-service-row-error",
            "settings-metrics-port-row-error",
        ] {
            assert!(window.find(row).visible(), "{row}");
        }
        window.click("settings-metrics-source-url", cx);
        window.render_frame(cx);
    })
    .unwrap();
    set(
        cx,
        handle,
        &view,
        Field::Address,
        "ftp://metrics.example.test",
    );
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-save", cx);
        window.render_frame(cx);
        assert!(window.find("settings-metrics-url-row-error").visible());
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read(guard.path().join("freshkube.yaml")).unwrap(),
        before
    );
}

#[gpui_kit::test]
async fn a_failing_metrics_save_puts_the_keyboard_in_the_first_failing_field(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    select(cx, handle, "prod");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-source-service", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-save", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    // The very first Save puts the keyboard in Namespace, so a typed letter
    // lands there.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("settings-metrics-namespace").focused(),
            Some(true)
        );
        window.input("m", cx);
    })
    .unwrap();
    assert_eq!(values(cx, handle, &view).1.namespace, "m");
}
