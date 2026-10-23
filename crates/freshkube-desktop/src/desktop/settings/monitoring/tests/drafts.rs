use super::*;

#[gpui_kit::test]
async fn monitoring_keeps_its_draft_after_staying_and_a_workspace_save(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    let (_runtime, handle, pilot, page) = live(cx, directory.path(), "cluster-a").await;
    cx.update_window(handle, |_, window, cx| {
        replace(window, cx, "metrics-port", "9091")
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Clusters, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-unsaved-stay", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        assert_eq!(page.read(cx).section(), SettingsSection::Monitoring);
        // Another setup group can save while this section keeps its draft.
        page.update(cx, |page, cx| {
            let mut next = page.workspace.clone();
            let mut entry = next.clusters[0].clone();
            entry.id = "cluster-b".into();
            entry.context = "cluster-b".into();
            next.clusters.push(entry);
            page.commit(next, "Added cluster-b.".into(), cx);
        });
    })
    .unwrap();
    cx.wait_for_io(handle, |_, cx| page.read(cx).saving.is_none())
        .await;
    cx.run_until_parked();
    open(cx, handle, &pilot);
    cx.update_window(handle, |_, window, cx| {
        let draft = page.read(cx).monitoring.as_ref().unwrap();
        assert!(draft.error.is_none());
        assert!(draft.dirty);
        assert_eq!(draft.port.read(cx).value(), "9091");
        assert_eq!(draft.seen, draft.file.read(cx).seen);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    let setup = setup::parse(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(setup.workspace.clusters.len(), 2);
    assert_eq!(setup.metrics.store.unwrap().service.unwrap().port, 9091);
}

#[gpui_kit::test]
async fn monitoring_escape_closes_the_picker_then_reverts_only_the_focused_group(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("freshkube.yaml"), SETUP).unwrap();
    let (_runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-outside").await;
    cx.update_window(handle, |_, window, cx| {
        replace(window, cx, "metrics-port", "9091")
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-entry", cx);
        window.render_frame(cx);
        assert_eq!(window.find("metrics-entry").expanded(), Some(true));
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("metrics-entry").expanded(), Some(false));
        assert_eq!(
            page.read(cx)
                .monitoring
                .as_ref()
                .unwrap()
                .port
                .read(cx)
                .value(),
            "9091"
        );
        window.click("monitoring-source-url", cx);
        window.render_frame(cx);
        replace(
            window,
            cx,
            "monitoring-source-address",
            "https://context-metrics.example.test",
        );
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("monitoring-source-address").is_none());
        assert_eq!(
            page.read(cx)
                .monitoring
                .as_ref()
                .unwrap()
                .port
                .read(cx)
                .value(),
            "9091"
        );
        window.click("monitoring-folder-choose", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.press("escape", cx);
        assert_eq!(
            page.read(cx)
                .monitoring
                .as_ref()
                .unwrap()
                .port
                .read(cx)
                .value(),
            "9091"
        );
        window.click("metrics-port", cx);
        window.press("escape", cx);
        assert_eq!(
            page.read(cx)
                .monitoring
                .as_ref()
                .unwrap()
                .port
                .read(cx)
                .value(),
            "9090"
        );
    })
    .unwrap();
}
