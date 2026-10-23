use super::*;

#[gpui_kit::test]
async fn monitoring_context_outside_the_workspace_saves_monitoring_json(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("freshkube.yaml"), SETUP).unwrap();
    let (_runtime, handle, _pilot, _page) = live(cx, directory.path(), "cluster-outside").await;
    cx.update_window(handle, |_, window, cx| {
        window.click("monitoring-source-test", cx);
    })
    .unwrap();
    cx.wait_for_io(handle, |window, _| {
        window
            .find("monitoring-context-test-result")
            .label()
            .is_some_and(|label| label.starts_with("Failed"))
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("monitoring-source-test").focused(), Some(false));
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.click("monitoring-source-service", cx);
        window.render_frame(cx);
        assert!(window.find("monitoring-context-current").visible());
        assert!(window.find("monitoring-context-name").visible());
        window.click("monitoring-source-url", cx);
        window.render_frame(cx);
        replace(
            window,
            cx,
            "monitoring-source-address",
            "https://metrics.example.test",
        );
        window.render_frame(cx);
        window.click("monitoring-source-save", cx);
    })
    .unwrap();
    let file = directory.path().join("monitoring.json");
    cx.wait_for_io(handle, |_, _| {
        std::fs::read_to_string(&file)
            .is_ok_and(|text| text.contains("https://metrics.example.test/"))
    })
    .await;
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(value["sources"]["cluster-outside"]["kind"], "url");
    assert_eq!(
        std::fs::read_to_string(directory.path().join("freshkube.yaml")).unwrap(),
        SETUP
    );
}
