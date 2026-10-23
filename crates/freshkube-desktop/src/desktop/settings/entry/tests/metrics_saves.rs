use super::*;

#[gpui_kit::test]
async fn metrics_saves_a_value_and_a_service_of_its_own(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), &rich());
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "mgmt");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-source-service", cx);
        window.render_frame(cx);
    })
    .unwrap();
    set(cx, handle, &view, Field::Label, "acme-core-b");
    set(cx, handle, &view, Field::Namespace, "monitoring");
    set(cx, handle, &view, Field::Service, "prometheus");
    set(cx, handle, &view, Field::Port, "9090");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("settings-metrics-dirty").visible());
        window.click("settings-metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let mut expected = setup::parse(rich().as_bytes()).unwrap();
    let mgmt = expected
        .workspace
        .clusters
        .iter_mut()
        .find(|e| e.id == "mgmt")
        .unwrap();
    let mut applied = mgmt.clone();
    metrics::Values {
        value: "acme-core-b".into(),
        kind: metrics::Kind::Service,
        namespace: "monitoring".into(),
        name: "prometheus".into(),
        port: "9090".into(),
        url: String::new(),
    }
    .apply(&mut applied);
    *mgmt = applied;
    let written =
        setup::parse(&std::fs::read(guard.path().join("freshkube.yaml")).unwrap()).unwrap();
    assert_eq!(written, expected);
    let mgmt = entry(&written.workspace, "mgmt");
    let metrics = mgmt.metrics.as_deref().unwrap();
    assert_eq!(metrics.value.as_deref(), Some("acme-core-b"));
    let service = metrics.source.as_ref().unwrap().service.as_ref().unwrap();
    assert_eq!(
        (
            service.namespace.as_str(),
            service.name.as_str(),
            service.port
        ),
        ("monitoring", "prometheus", 9090)
    );
    // Identity was not part of it.
    assert_eq!(mgmt.context, "acme-mgmt");
}

#[gpui_kit::test]
async fn changing_a_url_keeps_its_token_and_choosing_none_drops_the_store(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), &rich());
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(
        cx,
        handle,
        &view,
        Field::Address,
        "https://metrics-b.example.test",
    );
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-save", cx)
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let written = read_workspace(guard.path());
    let source = entry(&written, "prod")
        .metrics
        .as_deref()
        .unwrap()
        .source
        .clone()
        .unwrap();
    assert_eq!(
        source.url.as_deref(),
        Some("https://metrics-b.example.test")
    );
    assert!(source.token.is_some(), "the token stays");
    let text =
        String::from_utf8(std::fs::read(guard.path().join("freshkube.yaml")).unwrap()).unwrap();
    assert!(text.contains("note: kept"), "{text}");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-source-none", cx);
        window.click("settings-metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let written = read_workspace(guard.path());
    assert!(entry(&written, "prod").metrics.is_none());
}

#[test]
fn metrics_applies_its_keys_and_an_empty_block_is_not_written() {
    let mut entry = Entry::new("prod", Role::Environment, "acme-prod");
    let mut values = metrics::Values::of(&entry);
    assert_eq!(values.kind, metrics::Kind::Mode);
    values.apply(&mut entry);
    assert!(entry.metrics.is_none());
    values.value = "acme-b".into();
    values.apply(&mut entry);
    assert_eq!(
        entry.metrics.as_deref().unwrap().value.as_deref(),
        Some("acme-b")
    );
    values.value = String::new();
    values.apply(&mut entry);
    assert!(entry.metrics.is_none());
}
