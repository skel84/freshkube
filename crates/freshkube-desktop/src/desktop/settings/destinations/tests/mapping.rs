use super::*;

#[gpui_kit::test]
async fn example_destinations_are_never_changed(cx: &mut TestAppContext) {
    use crate::GpuiOptions;
    use crate::desktop::tests::mount;
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let options =
        GpuiOptions::fixture().with_preferences(Some(guard.path().join("preferences.json")));
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destination-0", cx);
        for key in ["a", "e", "enter", "backspace"] {
            window.press(key, cx);
        }
        window.click("settings-destinations-map", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings-destination-form").is_none());
        assert!(
            window
                .try_find("settings-destination-remove-dialog")
                .is_none()
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read_dir(guard.path()).unwrap().count(), 0);
}

#[gpui_kit::test]
async fn mapping_a_destination_saves_it_and_selects_it(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), ONE);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destination-0", cx);
        window.press("a", cx);
        window.render_frame(cx);
        assert!(window.find("settings-destination-form").visible());
        window.click("settings-destination-value", cx);
        window.input("https://prod.example.test", cx);
        window.click("settings-destination-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let rows = destinations(guard.path());
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1], server("https://prod.example.test", "prod"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-destination-form").is_none());
        assert!(window.find("settings-destination-1").visible());
        let list = view.read(cx).settings_page.read(cx).destinations.read(cx);
        assert_eq!(
            list.selected(),
            Some(&Key::Server("https://prod.example.test".into()))
        );
        assert!(window.find("settings-saved").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn changing_a_mapping_keeps_its_place_and_unknown_keys(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), ONE);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destination-0", cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(window.find("settings-destination-form").visible());
        // From a name to a server: select what was there, type over it.
        window.click(("settings-destination-by", 0usize), cx);
        window.click("settings-destination-value", cx);
        window.press("secondary-a", cx);
        window.input("https://lon.example.test", cx);
        window.click("settings-destination-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let file = serde_json::to_value(read_workspace(guard.path())).unwrap();
    assert_eq!(
        file["destinations"],
        serde_json::json!([{"server": "https://lon.example.test", "entry": "prod", "note": "kept"}])
    );
}

#[gpui_kit::test]
async fn removing_a_mapping_asks_first(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), ONE);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destination-0", cx);
        window.press("backspace", cx);
        window.render_frame(cx);
        assert!(window.find("settings-destination-remove-dialog").visible());
        window.click("settings-destination-remove-cancel", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(destinations(guard.path()).len(), 1);
    cx.update_window(handle, |_, window, cx| {
        window.press("backspace", cx);
        window.render_frame(cx);
        window.click("settings-destination-remove-confirm", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    assert!(destinations(guard.path()).is_empty());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-destinations-empty").visible());
    })
    .unwrap();
}

/// A save refused because the file changed on disk keeps the form and what
/// was typed, selects nothing new, and leaves the hand edit alone.
#[gpui_kit::test]
async fn a_file_changed_on_disk_refuses_a_mapping(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), ONE);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    let edited = ONE.replace("acme-prod", "acme-prod-2");
    write(guard.path(), &edited);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destination-0", cx);
        window.press("a", cx);
        window.render_frame(cx);
        window.click("settings-destination-value", cx);
        window.input("https://prod.example.test", cx);
        window.click("settings-destination-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let why = window
            .find("settings-destination-error")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(why.contains("changed on disk"), "{why}");
        assert_eq!(list(&view, cx).selected(), Some(&name("prod-lon")));
        assert_eq!(list(&view, cx).rows.len(), 1);
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap(),
        edited
    );
}

/// An Edit that changes only the cluster keeps what the row matches, its
/// place, and its selection.
#[gpui_kit::test]
async fn an_edit_can_change_only_the_cluster(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destination-0", cx);
        window.press("e", cx);
        window.render_frame(cx);
        let form = list(&view, cx).form().expect("the form");
        form.update(cx, |form, cx| form.pick("prod", window, cx));
        window.click("settings-destination-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    assert_eq!(
        destinations(guard.path()),
        vec![
            server("https://a.example.test", "prod"),
            Destination::new(name("prod-lon"), "prod")
        ]
    );
    cx.update_window(handle, |_, _, cx| {
        assert_eq!(
            list(&view, cx).selected(),
            Some(&Key::Server("https://a.example.test".into()))
        );
    })
    .unwrap();
}
