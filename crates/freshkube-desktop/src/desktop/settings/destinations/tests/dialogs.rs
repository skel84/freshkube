use super::*;

#[gpui_kit::test]
fn small_window_destination_dialogs_keep_save_outside_the_scroll(cx: &mut TestAppContext) {
    use crate::{DebugOptions, GpuiOptions};
    let options = GpuiOptions::fixture().with_debug(DebugOptions {
        page: Some("settings-destinations".into()),
        text_size: Some(20.),
        ..DebugOptions::default()
    });
    let (_runtime, handle, pilot) = crate::desktop::testing::mount(cx, options, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        let page = pilot.read(cx).settings_page.clone();
        let list = page.read(cx).destinations.clone();
        for editing in [false, true] {
            let edit = editing.then(|| super::form::Editing {
                at: 0, key: Key::Server("https://a-very-long-cluster-name.region-one.zone-two.example.test:6443/a-long-path".into()),
                entry: "prod".into(), listed: true,
            });
            super::form::open(page.clone(), list.downgrade(), edit, vec!["prod".into()], window, cx);
            super::super::small_window_tests::assert_dialog(window, cx, "settings-destination-save");
            let surface = window.within("dialog").find(0usize).bounds();
            assert!(surface.top() >= crate::ui::dp_px(freshkube_ui::page::APP_HEADER_HEIGHT, window), "{surface:?}");
            let before = window.find("settings-destination-save").bounds();
            window.scroll("settings-destination-form", gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-4000.))), cx);
            window.render_frame(cx);
            assert_eq!(window.find("settings-destination-save").bounds(), before);
            window.click("settings-destination-cancel", cx);
            window.render_frame(cx);
        }
    }).unwrap();
}

#[gpui_kit::test]
async fn small_window_remove_dialogs_fit_without_writing(cx: &mut TestAppContext) {
    use crate::{DebugOptions, GpuiOptions};
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    write(directory.path(), ONE);
    let options = GpuiOptions::new(Some(directory.path().join("missing")), None, 100)
        .with_preferences(Some(directory.path().join("preferences.json")))
        .with_debug(DebugOptions {
            text_size: Some(20.),
            ..DebugOptions::default()
        });
    let (_runtime, handle, pilot) = crate::desktop::testing::mount(cx, options, 760., 560.);
    let before = std::fs::read(directory.path().join("freshkube.yaml")).unwrap();
    open_section(cx, handle, &pilot, SettingsSection::Clusters).await;
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        window.click("settings-cluster-prod", cx);
        window.press("backspace", cx);
        super::super::small_window_tests::assert_dialog(window, cx, "settings-remove-confirm");
        window.click("settings-remove-cancel", cx);
    })
    .unwrap();
    open_section(cx, handle, &pilot, SettingsSection::Destinations).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destination-0", cx);
        window.press("backspace", cx);
        super::super::small_window_tests::assert_dialog(
            window,
            cx,
            "settings-destination-remove-confirm",
        );
        window.click("settings-destination-remove-cancel", cx);
    })
    .unwrap();
    assert_eq!(
        std::fs::read(directory.path().join("freshkube.yaml")).unwrap(),
        before
    );
}

// ---- Find suggestions ------------------------------------------------------
