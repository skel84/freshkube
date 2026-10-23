use super::*;

#[gpui_kit::test]
async fn in_cluster_aliases_save_only_to_the_selected_argo_cd_host(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(
        guard.path(),
        r#"{"version":1,"clusters":[
        {"id":"mgmt","role":"core","context":"example-mgmt"},
        {"id":"prod","role":"environment","context":"example-prod"}],
        "sources":{"argocd":"prod"}}"#,
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    for (by, value) in [
        (0usize, "https://Kubernetes.Default.Svc:443/"),
        (1, "in-cluster"),
    ] {
        let before = std::fs::read(guard.path().join("freshkube.yaml")).unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.click("settings-destinations-map", cx);
            window.render_frame(cx);
            window.click(("settings-destination-by", by), cx);
            window.click("settings-destination-value", cx);
            window.input(value, cx);
            let form = list(&view, cx).form().unwrap();
            form.update(cx, |form, cx| form.pick("mgmt", window, cx));
            window.click("settings-destination-save", cx);
            window.render_frame(cx);
            assert_field_error(window, &view, "entry", "The mapping names Argo CD's own cluster but maps it to another entry; use prod, the selected Argo CD source entry", cx);
        }).unwrap();
        assert_eq!(
            std::fs::read(guard.path().join("freshkube.yaml")).unwrap(),
            before
        );
        cx.update_window(handle, |_, window, cx| {
            let form = list(&view, cx).form().unwrap();
            form.update(cx, |form, cx| form.pick("prod", window, cx));
            window.click("settings-destination-save", cx);
        })
        .unwrap();
        saved(cx, handle, &view).await;
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("settings-destination-form").is_none());
            assert!(window.find("settings-saved").visible());
        })
        .unwrap();
    }
    let workspace = read_workspace(guard.path());
    assert_eq!(
        workspace.destinations,
        vec![
            server("https://Kubernetes.Default.Svc:443/", "prod"),
            Destination::new(name("in-cluster"), "prod"),
        ]
    );
    assert_eq!(workspace.sources.pin(workspace::Part::ArgoCd), Some("prod"));
}

#[gpui_kit::test]
async fn in_cluster_aliases_without_a_host_save_nothing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), ONE);
    let before = std::fs::read(guard.path().join("freshkube.yaml")).unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destinations-map", cx);
        window.render_frame(cx);
        window.click(("settings-destination-by", 1usize), cx);
        window.click("settings-destination-value", cx);
        window.input("in-cluster", cx);
        window.click("settings-destination-save", cx);
        window.render_frame(cx);
        assert_field_error(window, &view, "entry", "The mapping names Argo CD's own cluster but its host entry cannot be determined; set sources.argocd to a usable cluster entry", cx);
    }).unwrap();
    assert_eq!(
        std::fs::read(guard.path().join("freshkube.yaml")).unwrap(),
        before
    );
}
