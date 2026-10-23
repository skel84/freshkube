use super::*;

#[gpui_kit::test]
fn monitoring_modes_revert_and_fixture_controls(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("freshkube.yaml"), SETUP).unwrap();
    let options =
        GpuiOptions::fixture().with_preferences(Some(directory.path().join("preferences.json")));
    let (_runtime, handle, pilot) = mount(cx, options, 1280., 1600.);
    let page = open(cx, handle, &pilot);
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("metrics-entry").visible());
        assert!(window.find("metrics-label").visible());
        window.click("metrics-automatic", cx);
        window.render_frame(cx);
        assert!(window.try_find("metrics-entry").is_none());
        assert!(window.try_find("metrics-url").is_none());
        window.click("metrics-per-entry", cx);
        window.render_frame(cx);
        assert!(window.find("metrics-per-entry-row").visible());
        window.click("metrics-central", cx);
        window.render_frame(cx);
        window.click("metrics-store-url", cx);
        window.render_frame(cx);
        assert!(window.find("metrics-url").visible());
        assert!(window.try_find("metrics-entry").is_none());
        window.click("metrics-save", cx);
        window.click("metrics-test", cx);
        window.click("monitoring-folder-choose", cx);
        assert!(page.read(cx).monitoring.as_ref().unwrap().saving.is_none());
        assert!(!page.read(cx).monitoring.as_ref().unwrap().read.is_loading());
        window.click("metrics-revert", cx);
        window.render_frame(cx);
        assert!(window.find("metrics-entry").visible());
        assert!(!page.read(cx).monitoring.as_ref().unwrap().dirty);
        assert!(window.find("metrics-mode-row-hint").visible());
    })
    .unwrap();
    assert!(!cx.did_prompt_for_paths());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("freshkube.yaml")).unwrap(),
        SETUP
    );
    assert!(!directory.path().join("monitoring.json").exists());
}
