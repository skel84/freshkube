use super::*;

/// The logs the pane asked the dock for, in order.
fn asked_logs(emitted: &Emitted) -> Vec<LogsRequest> {
    emitted
        .borrow()
        .iter()
        .filter_map(|event| match event {
            DetailEvent::Link(ResourceLink::Logs(request)) => Some(request.clone()),
            _ => None,
        })
        .collect()
}

#[gpui_kit::test]
fn pods_and_what_runs_them_ask_the_dock_for_their_logs(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, emitted) = mount(cx);
    let (pod, _) = running_pod();
    let (deployment, _) = target("deployments.apps", |_, name| name == "api");
    let (secret, _) = target("secrets", |_, _| true);
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &secret, Duration::ZERO, cx);
        window.render_frame(cx);
        assert!(window.try_find("detail-open-logs").is_none());
        assert!(window.try_find("detail-tab-logs").is_none());

        // Logs is a button in the header, not a tab: the pane stays where
        // it was and asks for the logs in the dock.
        open(&pane, &pod, Duration::ZERO, cx);
        window.render_frame(cx);
        assert!(window.try_find("detail-tab-logs").is_none());
        window.click("detail-open-logs", cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-tab-details").selected(), Some(true));

        open(&pane, &deployment, Duration::ZERO, cx);
        window.render_frame(cx);
        window.click("detail-open-logs", cx);
        // Asking for the Logs tab from outside asks the dock too.
        pane.update(cx, |pane, cx| pane.set_tab(super::Tab::Logs, cx));
        assert_eq!(pane.read(cx).tab(), super::Tab::Overview);
    })
    .unwrap();
    let asked = asked_logs(&emitted);
    let targets: Vec<_> = asked.iter().map(|request| &request.target).collect();
    assert_eq!(targets, [&pod, &deployment, &deployment]);
    // A pod's opens on the container Overview picks; nothing asks for a
    // previous instance.
    assert!(asked[0].at.as_ref().is_some_and(|at| !at.previous));
    assert!(asked[1..].iter().all(|request| request.at.is_none()));
}

#[gpui_kit::test]
fn a_crashing_pod_opens_on_why_with_its_restarts_and_relations(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, emitted) = mount(cx);
    let (pod, _) = crashing_pod();
    let container = cx
        .update_window(handle, |_, window, cx| {
            open(&pane, &pod, Duration::ZERO, cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("pod-cause").label(),
                Some("Why it's failing · CrashLoopBackOff")
            );
            assert!(window.find("pod-timeline").visible());
            assert!(window.try_find("pod-relations").is_some());
            let links = &pane.read(cx).cross_links;
            let facts: Vec<&str> = links
                .facts
                .iter()
                .map(|(label, _)| label.as_ref())
                .collect();
            assert_eq!(facts, ["ServiceAccount", "Pod IP", "QoS class"]);
            let card = links.cause.as_ref().unwrap();
            assert!(card.previous);
            let container = card.container.clone().unwrap();

            // The crashed instance's logs are one click away, in the dock.
            window.click("pod-cause-previous", cx);
            window.render_frame(cx);
            assert_eq!(pane.read(cx).tab(), super::Tab::Overview);
            container
        })
        .unwrap();
    let asked = asked_logs(&emitted);
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].target, pod);
    assert_eq!(
        asked[0].at,
        Some(LogsAt {
            container,
            previous: true,
            all: false,
        })
    );
}

#[gpui_kit::test]
fn a_healthy_pod_has_no_cause_and_logs_is_one_button(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, emitted) = mount(cx);
    let (pod, _) = running_pod();
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        window.render_frame(cx);
        assert!(window.try_find("pod-cause").is_none());
        // The header's Logs is the pane's only one; the Overview has none
        // of its own.
        assert!(window.try_find("pod-open-logs").is_none());
        assert!(window.find("detail-open-logs").visible());
        window.click("detail-open-logs", cx);
        window.render_frame(cx);
        assert_eq!(pane.read(cx).tab(), super::Tab::Overview);
    })
    .unwrap();
    let asked = asked_logs(&emitted);
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].target, pod);
    assert!(asked[0].at.as_ref().is_none_or(|at| !at.previous));
}

/// A pod with a healthy default container and a crash-looping sidecar:
/// the header's Logs opens the sidecar, the one at fault.
#[gpui_kit::test]
fn logs_opens_the_container_at_fault_rather_than_the_default(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, emitted) = mount(cx);
    let (pod, _) = crashing_pod();
    let blamed = cx.update(|cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        pane.update(cx, |pane, _| {
            let view = pane.detail.as_mut().unwrap().view.as_mut().unwrap();
            let mut document = view.document.clone();
            let overview = &mut document.overview;
            // A healthy first container, which is also the default.
            let status = overview.pod_status.as_mut().unwrap();
            let blamed = status.containers[0].name.clone();
            let mut healthy = status.containers[0].clone();
            healthy.name = "proxy".into();
            healthy.ready = true;
            healthy.restarts = 0;
            healthy.waiting = None;
            healthy.last = None;
            healthy.running_since = Some(chrono::Utc::now());
            status.containers.insert(0, healthy);
            let containers = overview.pod.as_mut().unwrap();
            let mut proxy = containers.containers[0].clone();
            proxy.name = "proxy".into();
            containers.containers.insert(0, proxy);
            containers.default = Some("proxy".into());
            *view = std::sync::Arc::new(DocumentView::new(document));
            pane.rebuild_links();
            blamed
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            pane.read(cx).cross_links.logs_tip.as_deref(),
            Some(format!("Opens {blamed}'s log in the dock (L from the list)").as_str())
        );
        window.click("detail-open-logs", cx);
    })
    .unwrap();
    let asked = asked_logs(&emitted);
    assert_eq!(asked.len(), 1);
    assert_eq!(
        asked[0].at,
        Some(LogsAt {
            container: blamed,
            previous: false,
            all: false,
        })
    );
}
