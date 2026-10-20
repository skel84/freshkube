use super::*;

#[gpui_kit::test]
fn nothing_is_read_until_the_tab_shows_then_every_pod_is_followed(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let selector = selector(&api).unwrap();
    assert_eq!(selector, "app=api");
    let expected = pods(&api, &selector);
    assert!(expected.len() > 3, "{}", expected.len());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.show_workload(Some(api.clone()), Some(ClusterAccess::EXAMPLE), cx);
            view.set_selector(Some(selector.clone()), cx);
        });
        window.render_frame(cx);
        // The pane passed the workload and its selector, but the tab hasn't
        // shown yet.
        assert!(!view.read(cx).reading());
        assert_eq!(status(window), "Idle");
        assert_eq!(lines(&view, cx), 0);

        view.update(cx, |view, cx| view.want(cx));
        window.render_frame(cx);
        assert!(view.read(cx).reading());
        let count = expected.len();
        assert_eq!(
            status(window),
            format!("Following: {count} pods · {count} containers")
        );
        // One chip per container, tagged pod/container.
        let mut creating = 0;
        for pod in &expected {
            let tag = format!("{}/api", pod.name);
            let label = window
                .find(format!("workload-logs-stream-{tag}"))
                .label()
                .unwrap()
                .to_owned();
            if !label.starts_with(&format!("{tag}: Streaming")) {
                assert!(
                    label.starts_with(&format!("{tag}: Waiting (ContainerCreating)")),
                    "{label}"
                );
                creating += 1;
            }
        }
        assert_eq!(creating, 1);
        // A container still being created has written nothing yet.
        assert_eq!(tags(&view, cx).len(), count - 1);
        assert!(window.try_find("logs-empty").is_none());
        assert!(window.try_find("workload-logs-capped").is_none());
        // Lines from every pod interleave by time.
        let keys: Vec<i64> = view
            .read(cx)
            .retained()
            .iter()
            .map(|entry| entry.timestamp.as_ref().map_or(0, |time| time.sort_key))
            .collect();
        assert!(keys.windows(2).all(|pair| pair[0] <= pair[1]));
    })
    .unwrap();

    // Running containers write on.
    let before = cx.update(|cx| lines(&view, cx));
    cx.executor().advance_clock(EXAMPLE_INTERVAL * 3);
    cx.run_until_parked();
    assert!(cx.update(|cx| lines(&view, cx)) > before);
}

#[gpui_kit::test]
fn finding_pods_no_selector_and_no_pods_each_say_so(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    cx.update_window(handle, |_, window, cx| {
        // Wanted before the document is read.
        show_fed(&view, &api, cx);
        window.render_frame(cx);
        assert_eq!(status(window), "Finding pods");
        assert_eq!(
            window.find("logs-empty").label(),
            Some("Finding the workload's pods")
        );

        view.update(cx, |view, cx| view.set_selector(None, cx));
        window.render_frame(cx);
        assert_eq!(status(window), "No selector");
        assert_eq!(
            window.find("logs-empty").label(),
            Some("This workload selects no pods.")
        );

        view.update(cx, |view, cx| {
            view.set_selector(Some("app=nothing-runs".into()), cx)
        });
        window.render_frame(cx);
        assert_eq!(status(window), "No pods: Pods show here as they start.");
        assert_eq!(
            window.find("logs-empty").label(),
            Some("The workload runs no pods.")
        );
        assert!(window.try_find("workload-logs-streams").is_none());
    })
    .unwrap();
}
