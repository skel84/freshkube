use super::*;

#[gpui_kit::test]
fn logs_open_in_a_tab_and_opening_them_again_selects_it(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    let api = objects(&pilot, "deployments.apps", |_, name| name == "api", cx);
    draw(handle, cx);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("dock").is_none(), "no tabs, no dock");
    })
    .unwrap();

    open_logs(handle, &pilot, "pods", &pods[0], cx);
    assert_eq!(titles(&dock, cx), [format!("Pod {}", pods[0].name)]);
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(window.find("dock-tab-0").selected(), Some(true));
        assert_eq!(window.find("pod-logs-status").label(), Some("Streaming"));
        // The tab takes the keyboard, and the status bar counts it.
        assert_eq!(window.find("logs-viewport").focused(), Some(true));
        assert_eq!(window.find("dock-toggle").label(), Some("1 log"));
        let _ = cx;
    })
    .unwrap();

    // A workload follows every pod it runs, in a tab of its own.
    open_logs(handle, &pilot, "deployments.apps", &api[0], cx);
    assert_eq!(
        titles(&dock, cx),
        [format!("Pod {}", pods[0].name), "Deployment api".into()]
    );
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("dock-tab-1").selected(), Some(true));
        let status = window.find("workload-logs-status");
        let status = status.label().unwrap();
        assert!(status.starts_with("Following: "), "{status}");
    })
    .unwrap();

    // The same pod again selects its tab.
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    assert_eq!(titles(&dock, cx).len(), 2);
    assert_eq!(cx.update(|cx| dock.read(cx).selected()), Some(0));
}

#[gpui_kit::test]
fn a_ninth_log_tab_asks_to_close_the_oldest(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = objects(&pilot, "pods", |_, _| true, cx);
    assert!(pods.len() > MAX_LOG_TABS + 1);
    for pod in &pods[..MAX_LOG_TABS] {
        open_logs(handle, &pilot, "pods", pod, cx);
    }
    assert_eq!(titles(&dock, cx).len(), MAX_LOG_TABS);
    assert!(!cx.has_pending_prompt());

    // Cancel opens nothing.
    open_logs(handle, &pilot, "pods", &pods[MAX_LOG_TABS], cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(titles(&dock, cx)[0], format!("Pod {}", pods[0].name));

    // Agreeing closes the oldest and opens the new one.
    open_logs(handle, &pilot, "pods", &pods[MAX_LOG_TABS], cx);
    cx.simulate_prompt_answer("Close it");
    cx.run_until_parked();
    let titles = titles(&dock, cx);
    assert_eq!(titles.len(), MAX_LOG_TABS);
    assert_eq!(titles[0], format!("Pod {}", pods[1].name));
    assert_eq!(
        titles.last().unwrap(),
        &format!("Pod {}", pods[MAX_LOG_TABS].name)
    );
}

#[gpui_kit::test]
fn the_chromes_close_closes_every_tab(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    open_logs(handle, &pilot, "pods", &pods[1], cx);
    let views = [
        pod_view(&dock, 0, cx).downgrade(),
        pod_view(&dock, 1, cx).downgrade(),
    ];
    cx.update_window(handle, |_, window, cx| window.click("dock-close-all", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(titles(&dock, cx).is_empty());
    assert!(views.iter().all(|view| view.upgrade().is_none()));
    draw(handle, cx);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("dock").is_none());
        assert!(window.try_find("dock-toggle").is_none());
        // The keyboard is back on the page.
        assert_eq!(window.find("resource-body").focused(), Some(true));
    })
    .unwrap();
}
