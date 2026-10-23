use super::*;

/// A mapping whose cluster the workspace no longer lists is kept, marked
/// with the Unknown ring, and says why.
#[gpui_kit::test]
async fn a_mapping_to_a_cluster_no_longer_listed_is_marked(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(
        guard.path(),
        r#"{"version":1,"clusters":[
            {"id":"prod","role":"environment","context":"acme-prod"}],
          "destinations":[
            {"server":"https://prod.example.test","entry":"prod"},
            {"server":"https://old.example.test","entry":"old"}]}"#,
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    cx.update_window(handle, |_, window, cx| {
        assert!(window.try_find("settings-destination-0-mark").is_none());
        let mark = window.find("settings-destination-1-mark");
        assert_eq!(mark.label(), Some("old isn’t in the workspace"));
        let list = view.read(cx).settings_page.read(cx).destinations.read(cx);
        assert_eq!(
            list.rows().collect::<Vec<_>>(),
            vec![
                ("https://prod.example.test", "prod", true),
                ("https://old.example.test", "old", false)
            ]
        );
    })
    .unwrap();
}

/// Editing a mapping whose cluster is gone names it in the picker, and the
/// status bar counts only mappings to listed clusters.
#[gpui_kit::test]
async fn a_gone_cluster_is_named_and_not_counted(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(
        guard.path(),
        r#"{"version":1,"clusters":[
            {"id":"prod","role":"environment","context":"acme-prod"},
            {"id":"stage","role":"environment","context":"acme-stage"}],
          "destinations":[
            {"server":"https://prod.example.test","entry":"prod"},
            {"server":"https://old.example.test","entry":"old"}]}"#,
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    cx.update_window(handle, |_, window, cx| {
        let scope = window
            .find("settings-scope")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(scope.contains("1 destination mapped"), "{scope}");
        window.click("settings-destination-1", cx);
        window.press("e", cx);
        window.render_frame(cx);
        let form = list(&view, cx).form().expect("the form");
        assert_eq!(
            form.read(cx).gone(),
            Some("old is gone, no longer in the workspace: pick a cluster")
        );
    })
    .unwrap();
}

/// With no mapping to a listed cluster the status bar says nothing of them,
/// and without a cluster Map opens nothing.
#[gpui_kit::test]
async fn without_a_cluster_there_is_nothing_to_map(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(
        guard.path(),
        r#"{"version":1,"clusters":[],
          "destinations":[{"name":"prod-lon","entry":"old"}]}"#,
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    cx.update_window(handle, |_, window, cx| {
        let scope = window
            .find("settings-scope")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(!scope.contains("destination"), "{scope}");
        window.click("settings-destinations-map", cx);
        window.click("settings-destination-0", cx);
        for key in ["a", "e"] {
            window.press(key, cx);
        }
        window.render_frame(cx);
        assert!(window.try_find("settings-destination-form").is_none());
        assert!(!list(&view, cx).mappable());
    })
    .unwrap();
}
