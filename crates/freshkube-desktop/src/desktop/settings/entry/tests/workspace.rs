use super::*;

#[gpui_kit::test]
async fn the_workspace_kubeconfig_is_chosen_and_cleared_at_once(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-kubeconfig-path").visible());
        assert!(window.find("settings-kubeconfig-browse").visible());
        assert!(window.try_find("settings-kubeconfig-clear").is_none());
        view.read(cx).settings_page.clone().update(cx, |page, cx| {
            page.set_kubeconfig(Some(abs("kubeconfig").into()), cx)
        });
    })
    .unwrap();
    saved(cx, handle, &view).await;
    assert_eq!(
        read_workspace(guard.path()).kubeconfig.as_deref(),
        Some(std::path::Path::new(&abs("kubeconfig")))
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // A chosen file can be given up again.
        assert!(window.find("settings-kubeconfig-clear").visible());
        window.click("settings-kubeconfig-clear", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    assert_eq!(read_workspace(guard.path()).kubeconfig, None);
    // The clusters stayed as they were.
    assert_eq!(read_workspace(guard.path()).clusters.len(), 2);
}
