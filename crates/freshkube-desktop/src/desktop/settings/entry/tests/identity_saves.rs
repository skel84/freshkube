use super::*;

#[gpui_kit::test]
async fn saving_identity_changes_only_that_entrys_identity_keys(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), &rich());
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "acme-prod-2");
    set(
        cx,
        handle,
        &view,
        Field::Talosconfig,
        &abs("other-talosconfig"),
    );
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("settings-identity-dirty").visible());
        assert!(window.try_find("settings-metrics-dirty").is_none());
        window.click(("settings-identity-role", 1usize), cx);
        window.click("settings-identity-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let mut expected = setup::parse(rich().as_bytes()).unwrap();
    let prod = expected
        .workspace
        .clusters
        .iter_mut()
        .find(|e| e.id == "prod")
        .unwrap();
    prod.role = Role::Cicd;
    prod.context = "acme-prod-2".into();
    prod.talosconfig = Some(abs("other-talosconfig").into());
    let written =
        setup::parse(&std::fs::read(guard.path().join("freshkube.yaml")).unwrap()).unwrap();
    // The other entry, its unknown keys, the kubeconfig, the metrics mode,
    // Coroot, the supply chain and every unknown key are as they were.
    assert_eq!(written, expected);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Saved: the group is clean again, on the new baseline.
        assert!(window.try_find("settings-identity-dirty").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_clean_group_saves_nothing_and_revert_restores_the_file(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let before = std::fs::read(guard.path().join("freshkube.yaml")).unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-identity-save", cx);
    })
    .unwrap();
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read(guard.path().join("freshkube.yaml")).unwrap(),
        before
    );
    set(cx, handle, &view, Field::Context, "typed");
    set(cx, handle, &view, Field::Talosconfig, &abs("typed"));
    to_top(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click(("settings-identity-role", 0usize), cx);
        window.click("settings-identity-revert", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings-identity-dirty").is_none());
    })
    .unwrap();
    let (identity, _) = values(cx, handle, &view);
    assert_eq!(identity.context, "acme-prod");
    assert_eq!(identity.talosconfig, "");
    assert_eq!(identity.role, Role::Environment);
    assert_eq!(
        std::fs::read(guard.path().join("freshkube.yaml")).unwrap(),
        before
    );
}

#[gpui_kit::test]
async fn escape_reverts_the_group_holding_the_keyboard_once(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-identity-context", cx);
    })
    .unwrap();
    set(cx, handle, &view, Field::Context, "typed");
    cx.update_window(handle, |_, window, cx| {
        window.press("escape", cx);
        window.render_frame(cx);
    })
    .unwrap();
    let (identity, _) = values(cx, handle, &view);
    assert_eq!(identity.context, "acme-prod");
}

#[gpui_kit::test]
async fn text_a_save_trims_leaves_the_group_clean_afterwards(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "acme-prod-5 ");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-source-service", cx);
        window.render_frame(cx);
    })
    .unwrap();
    set(cx, handle, &view, Field::Namespace, " monitoring ");
    set(cx, handle, &view, Field::Service, "prometheus");
    set(cx, handle, &view, Field::Port, "09090");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-save", cx)
    })
    .unwrap();
    saved(cx, handle, &view).await;
    to_top(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-identity-save", cx)
    })
    .unwrap();
    saved(cx, handle, &view).await;
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        // Nothing is left to save, so no prompt will ask about it.
        assert!(!page.has_unsaved_edits(cx));
        assert!(window.try_find("settings-identity-dirty").is_none());
        assert!(window.try_find("settings-metrics-dirty").is_none());
    })
    .unwrap();
    let (identity, metrics) = values(cx, handle, &view);
    assert_eq!(identity.context, "acme-prod-5");
    assert_eq!(
        (metrics.namespace.as_str(), metrics.port.as_str()),
        ("monitoring", "9090")
    );
    let written = read_workspace(guard.path());
    let prod = entry(&written, "prod");
    assert_eq!(prod.context, "acme-prod-5");
    let service = prod
        .metrics
        .as_deref()
        .unwrap()
        .source
        .as_ref()
        .unwrap()
        .service
        .as_ref()
        .unwrap();
    assert_eq!(
        (service.namespace.as_str(), service.port),
        ("monitoring", 9090)
    );
}

#[test]
fn identity_applies_its_keys_and_nothing_else() {
    let mut entry = Entry::new("prod", Role::Environment, "acme-prod");
    entry.metrics = Some(Box::default());
    let values = identity::Values {
        role: Role::Cicd,
        context: " acme-ci ".into(),
        talosconfig: abs("talosconfig"),
        talos_context: "".into(),
    };
    assert!(values.check().first().is_none());
    values.apply(&mut entry);
    assert_eq!(entry.role, Role::Cicd);
    assert_eq!(entry.context, "acme-ci");
    assert_eq!(
        entry.talosconfig.as_deref(),
        Some(std::path::Path::new(&abs("talosconfig")))
    );
    assert_eq!(entry.talos_context, None);
    assert!(entry.metrics.is_some());
    let bad = identity::Values {
        talosconfig: "".into(),
        talos_context: "acme".into(),
        ..values
    };
    assert_eq!(bad.check().first(), Some(Field::TalosContext));
}
