use super::*;

#[gpui_kit::test]
fn an_object_reads_at_once_with_its_overview_yaml_and_events(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = crashing_pod();
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("detail-title").label(),
            Some(pod.identity.address().as_str())
        );
        assert!(window.try_find("detail-state").is_none());
        assert_eq!(window.find("detail-tab-details").selected(), Some(true));
        assert!(window.find("detail-overview").visible());
        // A crashing pod opens on why; its conditions follow further down.
        assert!(window.find("pod-cause").visible());
        assert!(window.try_find(("detail-condition", 0usize)).is_some());

        window.click("detail-tab-yaml", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("detail-yaml-line", 0usize)).label(),
            Some("apiVersion: v1")
        );

        // Scheduled, Pulled, Created and Started, then the newest: a
        // warning that it keeps crashing. Details' index jumps to them.
        window.click("detail-tab-details", cx);
        window.render_frame(cx);
        window.click("detail-jump-events", cx);
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-jump-events").selected(), Some(true));
        assert!(window.find("detail-section-events").visible());
        assert_eq!(window.find("detail-jump-events").label(), Some("Events 5"));
        let newest = window.find(("detail-event", 0usize));
        assert!(
            newest.label().unwrap().starts_with("BackOff: "),
            "{newest:?}"
        );
        assert!(window.try_find(("detail-event", 5usize)).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn reads_for_another_object_or_an_older_request_are_dropped(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = crashing_pod();
    let (other, _) = target("pods", |cells, _| cells[2] == "Running");
    cx.update_window(handle, |_, window, cx| {
        // A keyboard pause holds the read back, so nothing has landed.
        open(&pane, &pod, Duration::from_secs(1), cx);
        window.render_frame(cx);
        assert_eq!(read(&pane, cx), DocumentRead::Loading);
        assert!(window.find("detail-loading").visible());
        assert_eq!(window.find("detail-state").label(), Some("Reading"));

        let seq = pane.read(cx).read_seq;
        pane.update(cx, |pane, cx| {
            pane.finish_read(&other.identity, seq, Ok(example_view(&other)), cx);
            pane.finish_read(&pod.identity, seq - 1, Ok(example_view(&pod)), cx);
        });
        assert_eq!(read(&pane, cx), DocumentRead::Loading);
        pane.update(cx, |pane, cx| {
            pane.finish_read(&pod.identity, seq, Ok(example_view(&pod)), cx)
        });
        assert_eq!(read(&pane, cx), DocumentRead::Loaded);
    })
    .unwrap();
}

#[gpui_kit::test]
fn late_detail_results_cannot_cross_access_sessions_for_the_same_object(cx: &mut TestAppContext) {
    use freshkube_core::{AccessIdentity, AccessSessionId, ConfigurationRevision};
    let (_runtime, pane, handle, _) = mount(cx);
    let (example, _) = crashing_pod();
    let mut previous = example.clone();
    previous.identity.connection =
        AccessIdentity::new(AccessSessionId::new(), ConfigurationRevision::default()).key();
    let mut current = previous.clone();
    current.identity.connection =
        AccessIdentity::new(AccessSessionId::new(), ConfigurationRevision::default()).key();
    cx.update_window(handle, |_, _, cx| {
        open(&pane, &previous, Duration::from_secs(1), cx);
        let old_sequence = pane.read(cx).read_seq;
        open(&pane, &current, Duration::from_secs(1), cx);
        pane.update(cx, |pane, cx| {
            let sequence = pane.read_seq;
            assert_ne!(sequence, old_sequence);
            pane.finish_read(&previous.identity, sequence, Ok(example_view(&example)), cx);
            pane.finish_read(
                &current.identity,
                old_sequence,
                Ok(example_view(&example)),
                cx,
            );
            assert_eq!(pane.detail.as_ref().unwrap().read, DocumentRead::Loading);
            pane.finish_read(&current.identity, sequence, Ok(example_view(&example)), cx);
            assert_eq!(pane.detail.as_ref().unwrap().read, DocumentRead::Loaded);
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn refusals_failures_and_stale_reads_each_say_so(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = crashing_pod();
    let finish = |result, cx: &mut App| {
        pane.update(cx, |pane, cx| {
            let seq = pane.read_seq;
            pane.finish_read(&pod.identity, seq, result, cx)
        })
    };
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::from_secs(1), cx);
        finish(
            Err(Failure::new(FailureKind::Forbidden, "pods is forbidden")),
            cx,
        );
        window.render_frame(cx);
        assert!(window.find("detail-refused").visible());
        assert_eq!(window.find("detail-state").label(), Some("Not permitted"));

        finish(Err(Failure::new(FailureKind::Unreachable, "down")), cx);
        window.render_frame(cx);
        assert!(window.find("detail-failed").visible());
        window.click("detail-retry", cx);
        window.render_frame(cx);
        assert_eq!(read(&pane, cx), DocumentRead::Loaded);
        assert!(window.find("detail-overview").visible());

        // With a document shown, a failed read keeps it and says so.
        finish(Err(Failure::new(FailureKind::Timeout, "slow")), cx);
        window.render_frame(cx);
        assert!(window.find("detail-stale").visible());
        assert!(window.find("detail-overview").visible());
        assert_eq!(window.find("detail-state").label(), Some("Stale"));
        window.click("detail-stale-retry", cx);
        window.render_frame(cx);
        assert!(window.try_find("detail-stale").is_none());
        assert!(window.try_find("detail-state").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn new_versions_are_read_once_each_and_at_most_once_a_second(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = crashing_pod();
    let reads = |cx: &mut TestAppContext| pane.read_with(cx, |pane, _| pane.read_seq);
    let observe = |version: &'static str, cx: &mut TestAppContext| {
        pane.update(cx, |pane, cx| pane.observed(version, cx))
    };
    cx.update_window(handle, |_, _, cx| open(&pane, &pod, Duration::ZERO, cx))
        .unwrap();
    let first = reads(cx);
    // The example always reads as version 1; the list has moved on.
    observe("2", cx);
    assert_eq!(reads(cx), first, "read again within a second");
    cx.executor().advance_clock(FOLLOW_INTERVAL);
    cx.run_until_parked();
    assert_eq!(reads(cx), first + 1);
    // Version 2 was asked for; the older answer doesn't ask again.
    observe("2", cx);
    cx.executor().advance_clock(FOLLOW_INTERVAL * 3);
    cx.run_until_parked();
    assert_eq!(reads(cx), first + 1);
    observe("3", cx);
    cx.run_until_parked();
    assert_eq!(reads(cx), first + 2, "a second has passed, so at once");
}

#[gpui_kit::test]
fn safety_g09_a_hidden_pane_reads_nothing_and_hides_revealed_values(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (secret, _) = target("secrets", |_, name| name == "ledger-database");
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &secret, Duration::ZERO, cx);
        pane.update(cx, |pane, cx| pane.reveal("username".into(), cx));
        assert_eq!(pane.read(cx).detail.as_ref().unwrap().reveals.len(), 1);
        pane.update(cx, |pane, cx| pane.set_active(false, cx));
        assert!(pane.read(cx).detail.as_ref().unwrap().reveals.is_empty());

        let (pod, _) = crashing_pod();
        open(&pane, &pod, Duration::ZERO, cx);
        window.render_frame(cx);
        assert_eq!(read(&pane, cx), DocumentRead::Loading);
        pane.update(cx, |pane, cx| pane.observed("9", cx));
        assert_eq!(read(&pane, cx), DocumentRead::Loading);
        pane.update(cx, |pane, cx| pane.set_active(true, cx));
        assert_eq!(read(&pane, cx), DocumentRead::Loaded);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_deleted_object_stays_deleted_and_offers_its_successor(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, emitted) = mount(cx);
    let (pod, _) = crashing_pod();
    let successor = ResourceIdentity {
        uid: "another-incarnation".into(),
        ..pod.identity.clone()
    };
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        pane.update(cx, |pane, cx| pane.gone(None, cx));
        window.render_frame(cx);
        assert!(window.find("detail-deleted").visible());
        assert_eq!(window.find("detail-state").label(), Some("Deleted"));
        // The last read still shows, and nothing read later undoes it.
        assert!(window.find("detail-overview").visible());
        pane.update(cx, |pane, cx| pane.refresh(cx));
        assert_eq!(read(&pane, cx), DocumentRead::Deleted);

        pane.update(cx, |pane, cx| pane.gone(Some(successor.clone()), cx));
        window.render_frame(cx);
        window.click("detail-open-recreated", cx);
    })
    .unwrap();
    assert_eq!(*emitted.borrow(), [DetailEvent::Open(successor)]);
}
