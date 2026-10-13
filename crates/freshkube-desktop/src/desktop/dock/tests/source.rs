use super::*;

#[gpui_kit::test]
fn another_context_closes_every_tab(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    let view = pod_view(&dock, 0, cx).downgrade();
    let other = cx.update(|cx| {
        let pilot = pilot.read(cx);
        pilot
            .contexts
            .iter()
            .find(|context| Some(*context) != pilot.applied.context.as_ref())
            .cloned()
            .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.select_context(other, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(titles(&dock, cx).is_empty());
    assert!(view.upgrade().is_none());
}

#[gpui_kit::test]
fn another_talos_context_closes_every_tab_while_it_loads(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    let view = pod_view(&dock, 0, cx).downgrade();
    let other = cx.update(|cx| {
        let pilot = pilot.read(cx);
        pilot
            .contexts
            .iter()
            .find(|context| Some(*context) != pilot.applied.context.as_ref())
            .cloned()
            .unwrap()
    });
    while_loading(handle, &pilot, cx, |pilot, window, cx| {
        pilot.select_context(other, window, cx);
        assert!(pilot.source().kube.is_none());
    });
    assert!(titles(&dock, cx).is_empty());
    assert!(view.upgrade().is_none());
}

#[gpui_kit::test]
fn another_kubeconfig_closes_every_tab_while_it_loads(cx: &mut TestAppContext) {
    use freshkube_core::cluster_overview::KubeconfigSelection;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config");
    std::fs::write(&path, "apiVersion: v1\nkind: Config\n").unwrap();
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    let view = pod_view(&dock, 0, cx).downgrade();
    while_loading(handle, &pilot, cx, |pilot, window, cx| {
        let selection = KubeconfigSelection::File {
            path: path.clone(),
            context: None,
        };
        pilot.apply_kubeconfig(selection, window, cx);
        assert!(pilot.source().kube.is_none());
    });
    assert!(titles(&dock, cx).is_empty());
    assert!(view.upgrade().is_none());
}

#[gpui_kit::test]
fn losing_the_source_closes_every_tab_and_its_return_reopens_none(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    let source = cx.update(|cx| pilot.read(cx).source().kube);
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| {
            dock.set_source(None, None, window, cx);
            assert!(dock.tabs.is_empty());
            // The same connection back opens nothing it had.
            dock.set_source(source.clone(), None, window, cx);
            assert!(dock.tabs.is_empty());
        })
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_source_missing_for_a_while_keeps_the_tabs_and_their_shells(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    start_shell(handle, &pilot, &pods[1], cx);
    let source = cx.update(|cx| pilot.read(cx).source().kube).unwrap();
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| {
            // As Talos without its overview or client: the same access.
            dock.set_source(None, Some(&source.id), window, cx);
            assert_eq!(dock.tabs.len(), 2);
            dock.set_source(Some(source.clone()), Some(&source.id), window, cx);
            assert_eq!(dock.tabs.len(), 2);
        })
    })
    .unwrap();
    assert_eq!(cx.read(shell::running_anywhere).len(), 1);
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| {
            // Another identity closes them, its source loaded or not.
            dock.set_source(None, Some("access:another"), window, cx);
            assert!(dock.tabs.is_empty());
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.read(shell::running_anywhere).is_empty());
}
