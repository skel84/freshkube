use super::*;
use freshkube_resources::forwards::{self, State};

fn owner(cx: &mut TestAppContext) -> Entity<forwards::ForwardView> {
    cx.update(|cx| {
        forwards::list(cx)
            .read(cx)
            .find("invented-connection", "acme", "results", 8443, cx)
            .unwrap()
    })
}

#[gpui_kit::test]
async fn forwarding_keeps_the_builds_selection_inspector_and_list_rows(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _case = PORT_CASE.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let setup = configured(dir.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = fake::Server::new(runtime.handle());
    server.hold_start.store(true, Ordering::SeqCst);
    server.hold.store(true, Ordering::SeqCst);
    let (window, app, _reads) = mount_access(
        cx,
        &runtime,
        &setup,
        None,
        false,
        Arc::default(),
        Some(server.access.clone()),
    );
    wait_list(cx, window, &app).await;
    let changes = cx.read(|cx| list(&app, cx));
    let rows = cx.read(|cx| changes.read(cx).lines_text());
    let change = open_change(cx, window, &app);
    cx.wait_for_io(window, |_, cx| change.read(cx).answered())
        .await;
    let key = cx.read(|cx| change.read(cx).change.hops.iter().find(|hop| matches!(&hop.shows, freshkube_delivery::change::Shows::Hop(detail) if detail.kind == "Builds" || detail.kind == "PipelineRun")).unwrap().key.clone());
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click(
            gpui_kit::SharedString::from(format!("change-hop-{key}")),
            cx,
        );
        window.render_frame(cx);
        window.click("change-forward-results", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let owner = owner(cx);
    let id = cx.read(|cx| owner.read(cx).id);
    cx.wait_for_io(window, |_, cx| {
        matches!(owner.read(cx).snapshot().state, State::Starting)
    })
    .await;
    // Neither Starting nor Listening may replace the shown page with unread data.
    for listening in [false, true] {
        if listening {
            server.hold_start.store(false, Ordering::SeqCst);
            server.release_start.notify_waiters();
            cx.wait_for_io(window, |_, _| server.results.load(Ordering::SeqCst) > 0)
                .await;
        }
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            assert_eq!(
                change.read(cx).selected().map(|key| key.as_ref()),
                Some(key.as_str())
            );
            assert!(change.read(cx).loader.data().is_some());
            assert_eq!(changes.read(cx).lines_text(), rows);
            window.render_frame(cx);
            assert!(window.try_find("change-detail").is_some());
            assert_eq!(
                window.find("change-forward-results").label(),
                Some("Forward")
            );
            // A repeated explicit press while Starting/Listening cannot add an owner.
            window.click("change-forward-results", cx);
        })
        .unwrap();
        assert_eq!(cx.read(forwards::running), 1);
    }
    server.hold.store(false, Ordering::SeqCst);
    server.release.notify_waiters();
    cx.wait_for_io(window, |_, cx| archived(change.read(cx)))
        .await;
    cx.wait_for_io(window, |_, _| server.ready.receiver_count() > 0)
        .await;
    server.ready.send(false).unwrap();
    let waiting = freshkube_delivery::entry::pool::ResultsWait::NoReadyPod.words();
    cx.wait_for_io(window, |_, cx| {
        matches!(owner.read(cx).snapshot().state, State::NoReadyPod)
            && change
                .read(cx)
                .change
                .missing
                .iter()
                .any(|missing| missing.text.contains(waiting))
    })
    .await;
    cx.read(|cx| {
        assert!(changes.read(cx).status.text(None).contains(waiting));
        assert!(!changes.read(cx).display.rows.is_empty());
    });
    server.hold.store(true, Ordering::SeqCst);
    let reads = server.results.load(Ordering::SeqCst);
    server.ready.send(true).unwrap();
    cx.wait_for_io(window, |_, _| server.results.load(Ordering::SeqCst) > reads)
        .await;
    cx.read(|cx| {
        assert!(change.read(cx).loader.data().is_some());
        assert!(change.read(cx).loader.is_stale());
        assert!(!changes.read(cx).display.rows.is_empty());
    });
    cx.update(|cx| forwards::list(cx).update(cx, |list, cx| list.stop(id, cx)));
}

#[gpui_kit::test]
async fn list_forwarding_keeps_rows_and_uses_an_action_label(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _case = PORT_CASE.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let setup = configured(dir.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = fake::Server::new(runtime.handle());
    server.hold_start.store(true, Ordering::SeqCst);
    server.hold.store(true, Ordering::SeqCst);
    let (window, app, _reads) = mount_access(
        cx,
        &runtime,
        &setup,
        None,
        false,
        Arc::default(),
        Some(server.access.clone()),
    );
    wait_list(cx, window, &app).await;
    let changes = cx.read(|cx| list(&app, cx));
    let rows = cx.read(|cx| changes.read(cx).lines_text());
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("changes-forward-results", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let owner = owner(cx);
    for listening in [false, true] {
        if listening {
            server.hold_start.store(false, Ordering::SeqCst);
            server.release_start.notify_waiters();
            cx.wait_for_io(window, |_, _| server.results.load(Ordering::SeqCst) > 0)
                .await;
        }
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            assert_eq!(changes.read(cx).lines_text(), rows);
            window.render_frame(cx);
            assert_eq!(window.find("changes-forward-results").label(), Some("Forward"));
            let key = changes.read(cx).display.rows[0].key.clone();
            let menu = changes.update(cx, |page, cx| freshkube_ui::table::TableSource::row_menu(page, &key, window, cx));
            assert!(menu.iter().any(|item| matches!(item, freshkube_ui::menu::MenuAction::Item {label, enabled:false, ..} if label.as_ref() == "Forward")));
        }).unwrap();
    }
    let id = cx.read(|cx| owner.read(cx).id);
    cx.update(|cx| forwards::list(cx).update(cx, |list, cx| list.stop(id, cx)));
}

#[gpui_kit::test]
async fn two_pages_share_the_removed_owner_and_a_second_press_starts_nothing(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let _case = PORT_CASE.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let setup = configured(dir.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = fake::Server::new(runtime.handle());
    let (window, app, _reads) = mount_access(
        cx,
        &runtime,
        &setup,
        None,
        false,
        Arc::default(),
        Some(server.access.clone()),
    );
    wait_list(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("changes-forward-results", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.wait_for_io(window, |_, _| server.results.load(Ordering::SeqCst) >= 3)
        .await;
    wait_list(cx, window, &app).await;
    let change = open_change(cx, window, &app);
    cx.wait_for_io(window, |_, cx| archived(change.read(cx)))
        .await;
    let first = owner(cx);
    let id = cx.read(|cx| first.read(cx).id);
    let ports = cx.open_window(size(px(600.), px(500.)), |window, cx| {
        let view = cx.new(forwards::ForwardsIndicator::new);
        Root::new(view, window, cx)
    });
    cx.update_window(ports.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("forwards", cx);
        window.render_frame(cx);
        window.click(
            gpui_kit::SharedString::from(format!("forward-{id}-stop")),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(ports.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(
            gpui_kit::SharedString::from(format!("forward-{id}-remove")),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(!forwards::list(cx).read(cx).contains(id, cx)));
    cx.wait_for_io(window, |_, cx| {
        change.read(cx).answered()
            && change
                .read(cx)
                .change
                .missing
                .iter()
                .any(|missing| missing.text.contains("stopped"))
    })
    .await;
    cx.read(|cx| {
        assert!(
            list(&app, cx)
                .read(cx)
                .status
                .text(None)
                .contains("stopped")
        )
    });
    server.hold_start.store(true, Ordering::SeqCst);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let key = change.read(cx).change.hops.iter().find(|hop| matches!(&hop.shows, freshkube_delivery::change::Shows::Hop(detail) if detail.kind == "Builds" || detail.kind == "PipelineRun")).unwrap().key.clone();
        window.click(gpui_kit::SharedString::from(format!("change-hop-{key}")), cx);
        window.render_frame(cx);
        window.click("change-forward-results", cx);
        let changes = list(&app, cx);
        changes.update(cx, |page, cx| page.close_change(window, cx));
        window.render_frame(cx);
        window.click("changes-forward-results", cx);
    }).unwrap();
    cx.run_until_parked();
    assert_eq!(cx.read(forwards::running), 1);
    let second = owner(cx);
    let second_id = cx.read(|cx| second.read(cx).id);
    assert_ne!(id, second_id);
    cx.update(|cx| forwards::list(cx).update(cx, |list, cx| list.stop(second_id, cx)));
}

#[gpui_kit::test]
async fn unconfigured_results_adds_no_forward_action_or_status(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let setup = setup(None, None, false);
    let (window, app, _reads) = mount(cx, &runtime, &setup, None, false, Arc::default());
    wait_list(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("changes-forward-results").is_none());
        let status = list(&app, cx).read(cx).status.text(None);
        assert!(!status.contains("forward"));
        assert!(!status.contains("Tekton Results"));
    })
    .unwrap();
}
