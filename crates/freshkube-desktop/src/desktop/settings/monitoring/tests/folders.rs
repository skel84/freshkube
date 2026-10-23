use super::*;

#[gpui_kit::test]
async fn monitoring_folder_changes_save_at_once_and_keep_other_preferences(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("freshkube.yaml"), SETUP).unwrap();
    let file = directory.path().join("monitoring.json");
    let mut expected = serde_json::json!({"dashboards": null, "prometheus": {}, "sources": {}, "future": {"keep": [true, null, 7]}});
    std::fs::write(&file, serde_json::to_vec(&expected).unwrap()).unwrap();
    let folder = directory.path().join("dashboards");
    std::fs::create_dir(&folder).unwrap();
    let (_runtime, handle, _pilot, _page) = live(cx, directory.path(), "cluster-outside").await;
    cx.update_window(handle, |_, window, cx| {
        window.click("monitoring-folder-choose", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.directories);
        assert!(!options.files);
        Some(vec![folder.clone()])
    });
    cx.wait_for_io(handle, |_, _| {
        std::fs::read_to_string(&file).is_ok_and(|text| text.contains("dashboards\""))
    })
    .await;
    cx.wait_for_io(handle, |_, _| {
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&file).unwrap())
            .is_ok_and(|value| value["dashboards"].is_string())
    })
    .await;
    expected["dashboards"] = serde_json::to_value(&folder).unwrap();
    let read =
        || serde_json::from_slice::<serde_json::Value>(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(read(), expected);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("monitoring-folder-forget", cx);
    })
    .unwrap();
    cx.wait_for_io(handle, |_, _| read()["dashboards"].is_null())
        .await;
    expected["dashboards"] = serde_json::Value::Null;
    assert_eq!(read(), expected);
}

#[gpui_kit::test]
async fn monitoring_columns_folder_action_opens_the_monitoring_section(cx: &mut TestAppContext) {
    for (width, height, text_size) in [(1280., 880., 13.), (760., 560., 20.)] {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("freshkube.yaml"), SETUP).unwrap();
        let (_runtime, handle, pilot, page) = live(cx, directory.path(), "cluster-a").await;
        cx.simulate_window_resize(handle, gpui_kit::size(px(width), px(height)));
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(text_size, cx);
            window.render_frame(cx);
            if height == 560. {
                assert!(
                    window.find("monitoring-dashboards").bounds().bottom()
                        > window.find("settings-page").bounds().bottom()
                );
            }
        })
        .unwrap();
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| {
                pilot.navigate(crate::desktop::Page::Monitoring, window, cx)
            });
            // Exercise a cold form as well as returning to an existing one.
            if text_size == 20. {
                page.update(cx, |page, _| page.monitoring = None);
            }
            window.render_frame(cx);
            if window.try_find("monitoring-add-folder").is_none() {
                window.click("nav-dashboard-list", cx);
                window.render_frame(cx);
            }
            window.click("monitoring-add-folder", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                pilot.read(cx).navigation.page,
                crate::desktop::Page::Settings
            );
            assert_eq!(
                pilot.read(cx).settings_page.read(cx).section(),
                SettingsSection::Monitoring
            );
            assert!(!pilot.read(cx).navigation.settings_open);
            assert!(
                window.find("monitoring-folder-choose").visible(),
                "{width} x {height}, text {text_size}: folder {:?}, viewport {:?}, scroll {:?}",
                window.find("monitoring-dashboards").bounds(),
                window.find("settings-page").bounds(),
                page.read(cx).monitoring_scroll.offset()
            );
            let group = window.find("monitoring-dashboards").bounds();
            let viewport = window.find("settings-page").bounds();
            assert!(group.top() >= viewport.top());
            assert!(group.bottom() <= viewport.bottom());
            window.click("monitoring-folder-choose", cx);
        })
        .unwrap();
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
    }
}
