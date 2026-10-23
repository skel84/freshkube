use super::*;

#[gpui_kit::test]
async fn the_form_refuses_what_wouldnt_map_and_saves_nothing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), ONE);
    let before = std::fs::read(guard.path().join("freshkube.yaml")).unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    for (by, value, why) in [
        (
            1usize,
            "prod-lon",
            "Argo CD's cluster prod-lon is already mapped to prod",
        ),
        (
            0,
            "prod.example.test",
            "The server must be an http or https address",
        ),
        (0, "", "Enter the server URL the Application names"),
    ] {
        cx.update_window(handle, |_, window, cx| {
            window.click("settings-destinations-map", cx);
            window.render_frame(cx);
            window.click(("settings-destination-by", by), cx);
            window.click("settings-destination-value", cx);
            if !value.is_empty() {
                window.input(value, cx);
            }
            window.click("settings-destination-save", cx);
            window.render_frame(cx);
            assert_field_error(window, &view, "value", why, cx);
            window.click("settings-destination-cancel", cx);
            window.render_frame(cx);
        })
        .unwrap();
    }
    cx.run_until_parked();
    assert_eq!(
        std::fs::read(guard.path().join("freshkube.yaml")).unwrap(),
        before
    );
}

#[gpui_kit::test]
async fn with_two_clusters_the_form_asks_which(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(
        guard.path(),
        r#"{"version":1,"clusters":[
            {"id":"mgmt","role":"core","context":"acme-mgmt"},
            {"id":"prod","role":"environment","context":"acme-prod"}]}"#,
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("settings-destinations-empty").visible());
        window.click("settings-destinations-map", cx);
        window.render_frame(cx);
        window.click("settings-destination-value", cx);
        window.input("https://prod.example.test", cx);
        window.click("settings-destination-save", cx);
        window.render_frame(cx);
        assert_field_error(window, &view, "entry", "Pick a workspace cluster", cx);
    })
    .unwrap();
}

/// A server is a duplicate however it is written, as Argo CD matches it; a
/// duplicate whose cluster is gone says so.
#[gpui_kit::test]
async fn a_duplicate_is_found_as_argo_cd_matches(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(
        guard.path(),
        r#"{"version":1,"clusters":[
            {"id":"prod","role":"environment","context":"acme-prod"}],
          "destinations":[
            {"server":"https://a.example.test","entry":"prod"},
            {"server":"https://b.example.test","entry":"old"}]}"#,
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    for (value, why) in [
        (
            "https://A.example.test:443/",
            "Server https://A.example.test:443/ is already mapped to prod",
        ),
        (
            "https://b.example.test/",
            "Server https://b.example.test/ is already mapped to old, which the workspace no \
             longer lists: edit that mapping",
        ),
    ] {
        cx.update_window(handle, |_, window, cx| {
            window.click("settings-destinations-map", cx);
            window.render_frame(cx);
            window.click("settings-destination-value", cx);
            window.input(value, cx);
            window.click("settings-destination-save", cx);
            window.render_frame(cx);
            assert_field_error(window, &view, "value", why, cx);
            window.click("settings-destination-cancel", cx);
            window.render_frame(cx);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
async fn value_errors_show_on_blur_and_save_focuses_the_first_invalid_field(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    // GPUI emits focus/blur events only for an active window.
    cx.update_window(handle, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destinations-map", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destination-value", cx);
        window.input("invalid", cx);
        window.click("settings-destination-entry", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_field_error(
            window,
            &view,
            "value",
            "The server must be an http or https address",
            cx,
        );
        assert!(window.try_find("settings-destination-error").is_none());
        // Close the picker and submit with both fields invalid.
        window.press("escape", cx);
        window.click("settings-destination-save", cx);
        window.render_frame(cx);
        let form = list(&view, cx).form().unwrap();
        assert_eq!(form.read(cx).focused_field(window, cx), Some("value"));
        assert!(
            window
                .find("settings-destination-entry-field-error")
                .visible()
        );
        window.click("settings-destination-value", cx);
        window.press("secondary-a", cx);
        window.input("https://new.example.test", cx);
        window.click("settings-destination-save", cx);
        window.render_frame(cx);
        assert!(
            window
                .try_find("settings-destination-value-field-error")
                .is_none()
        );
        assert_eq!(form.read(cx).focused_field(window, cx), Some("entry"));
        let bounds = window
            .find("settings-destination-entry-control")
            .bounds()
            .scale(window.scale_factor());
        let crit = crate::palette::palette(cx).crit;
        assert!(
            window
                .painted_quads()
                .iter()
                .any(|quad| quad.border_color == crit
                    && quad.bounds == bounds
                    && quad.border_widths.top.0 > 0.)
        );
        assert!(window.try_find("settings-destination-error").is_none());
        let save = window.find("settings-destination-save").bounds();
        let cancel = window.find("settings-destination-cancel").bounds();
        assert_eq!(
            save.left() < cancel.left(),
            freshkube_ui::platform::default_button() == freshkube_ui::platform::Side::Leading
        );
    })
    .unwrap();
    assert_eq!(destinations(guard.path()).len(), 2);
}
