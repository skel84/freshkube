use super::*;

#[gpui_kit::test]
async fn monitoring_saves_only_metrics_and_keeps_every_unknown_key(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    let (_runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    let mut expected = setup::parse(SETUP.as_bytes()).unwrap();
    expected
        .metrics
        .store
        .as_mut()
        .unwrap()
        .service
        .as_mut()
        .unwrap()
        .port = 9091;
    cx.update_window(handle, |_, window, cx| {
        replace(window, cx, "metrics-port", "9091");
        window.press("enter", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    assert_eq!(
        setup::parse(&std::fs::read(&file).unwrap()).unwrap(),
        expected
    );
    assert_eq!(
        std::fs::read_to_string(file.with_extension("yaml.bak")).unwrap(),
        SETUP
    );
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).monitoring.as_ref().unwrap().error.is_none());
        assert!(!page.read(cx).monitoring.as_ref().unwrap().dirty);
        replace(window, cx, "metrics-label", "example_cluster");
        window.press("escape", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx)
                .monitoring
                .as_ref()
                .unwrap()
                .label
                .read(cx)
                .value(),
            "cluster"
        );
        assert!(!page.read(cx).monitoring.as_ref().unwrap().dirty);
    })
    .unwrap();
}

#[gpui_kit::test]
async fn monitoring_fields_report_port_label_and_missing_entry_errors(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    let (_runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    cx.update_window(handle, |_, window, cx| {
        replace(window, cx, "metrics-port", "wrong");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.click("metrics-label", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("metrics-port-row-error").visible());
        window.click("metrics-save", cx);
        window.render_frame(cx);
        assert!(
            page.read(cx)
                .monitoring
                .as_ref()
                .unwrap()
                .port
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        replace(window, cx, "metrics-port", "9090");
        replace(window, cx, "metrics-label", "not a label");
        window.click("metrics-save", cx);
        window.render_frame(cx);
        assert!(window.find("metrics-label-row-error").visible());
        assert_eq!(
            page.read(cx).monitoring.as_ref().unwrap().errors.get(&Field::Label).map(String::as_str),
            Some("Use a Prometheus label name: letters, digits and underscores, not starting with a digit.")
        );
        window.click("metrics-revert", cx);
        window.render_frame(cx);
        page.update(cx, |page, cx| {
            let draft = page.monitoring.as_mut().unwrap();
            draft
                .entry
                .update(cx, |state, cx| state.set_selected_index(None, window, cx));
            draft.changed(cx);
        });
        window.render_frame(cx);
        window.click("metrics-save", cx);
        window.render_frame(cx);
        assert!(window.find("metrics-entry-row-error").visible());
        page.update(cx, |page, cx| {
            let draft = page.monitoring.as_mut().unwrap();
            draft.missing_entry = Some("cluster-gone".into());
            draft.changed(cx);
        });
        window.render_frame(cx);
        window.click("metrics-save", cx);
        window.render_frame(cx);
        assert!(
            page.read(cx).monitoring.as_ref().unwrap().errors[&Field::Entry]
                .contains("in the workspace")
        );
        assert!(page.read(cx).monitoring.as_ref().unwrap().saving.is_none());
    })
    .unwrap();
    assert_eq!(std::fs::read_to_string(file).unwrap(), SETUP);
}

#[gpui_kit::test]
async fn monitoring_refuses_broken_and_changed_files_and_revert_reads_the_latest(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    let (_runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    cx.update_window(handle, |_, window, cx| {
        replace(window, cx, "metrics-port", "9091")
    })
    .unwrap();
    std::fs::write(&file, "version: [").unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "version: [");
    cx.read(|cx| {
        assert_eq!(
            page.read(cx).monitoring.as_ref().unwrap().error.as_deref(),
            Some(
                "freshkube.yaml isn’t usable. Nothing was written."
                    .replace('’', "'")
                    .as_str()
            )
        )
    });
    let changed = SETUP.replace("port: 9090", "port: 9092");
    std::fs::write(&file, &changed).unwrap();
    reload(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &page).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), changed);
    cx.read(|cx| {
        assert_eq!(
            page.read(cx).monitoring.as_ref().unwrap().error.as_deref(),
            Some(crate::setup_file::SaveError::Changed.to_string().as_str())
        )
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-revert", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx)
                .monitoring
                .as_ref()
                .unwrap()
                .port
                .read(cx)
                .value(),
            "9092"
        );
    })
    .unwrap();
}
