use super::*;

fn confirmation(
    current: Rc<Cell<u64>>,
    confirmed: Rc<Cell<usize>>,
) -> crate::mutation::Confirmation {
    crate::mutation::Confirmation {
        title: "Reboot talos-wk-fra1-02".into(),
        summary: "Drains the node, then reboots it.".into(),
        facts: vec![("Pods to evict".into(), "12".into())],
        warnings: Vec::new(),
        confirm_label: "Reboot".into(),
        destructive: true,
        fingerprint: 1,
        current: Rc::new(move |_| Some(current.get())),
        on_confirm: Rc::new(move |_, _| confirmed.set(confirmed.get() + 1)),
    }
}

/// Opens a confirmation and lets its open animation settle, so clicks land
/// where the dialog finally is.
fn open_confirmation(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    current: &Rc<Cell<u64>>,
    confirmed: &Rc<Cell<usize>>,
) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        crate::mutation::confirm(confirmation(current.clone(), confirmed.clone()), window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

#[gpui_kit::test]
fn safety_g10_confirmation_runs_only_against_the_preview_shown(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    let current = Rc::new(Cell::new(1));
    let confirmed = Rc::new(Cell::new(0));
    open_confirmation(cx, handle, &current, &confirmed);
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("confirm-dialog").visible());
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert_eq!(confirmed.get(), 1);
        assert!(window.try_find("confirm-dialog").is_none());
    })
    .unwrap();

    // The cluster changed while the dialog was open: refuse, explain, and
    // don't let a second click through either.
    open_confirmation(cx, handle, &current, &confirmed);
    current.set(2);
    cx.update_window(handle, |_, window, cx| {
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert_eq!(confirmed.get(), 1);
        assert!(window.find("confirm-blocked").visible());
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert_eq!(confirmed.get(), 1);
        window.click("confirm-cancel", cx);
        window.render_frame(cx);
        assert!(window.try_find("confirm-dialog").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn one_operation_at_a_time_with_status_cancel_and_close_guard(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    let operations = cx.update(crate::mutation::Operations::global);
    let ticket = operations
        .update(cx, |operations, cx| {
            operations.begin("Reboot talos-wk-fra1-02", cx)
        })
        .ok()
        .unwrap();
    assert!(
        operations
            .update(cx, |operations, cx| operations
                .begin("Drain talos-wk-fra1-01", cx))
            .is_err()
    );
    let cancelled = ticket.cancellation();
    let current = Rc::new(Cell::new(1));
    let confirmed = Rc::new(Cell::new(0));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("operation-status")
                .label()
                .unwrap()
                .starts_with("Reboot talos-wk-fra1-02")
        );
    })
    .unwrap();
    // A second mutation can't be confirmed while one runs.
    open_confirmation(cx, handle, &current, &confirmed);
    cx.update_window(handle, |_, window, cx| {
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert_eq!(confirmed.get(), 0);
        assert!(window.find("confirm-blocked").visible());
        window.click("confirm-cancel", cx);
        window.render_frame(cx);
        // Closing is refused with an explanation.
        assert!(!crate::mutation::may_close(window, cx));
    })
    .unwrap();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Keep running");
    cx.update_window(handle, |_, window, cx| {
        window.click("operation-cancel", cx);
        window.render_frame(cx);
        assert!(cancelled());
        assert!(
            window
                .find("operation-status")
                .label()
                .unwrap()
                .contains("stopping")
        );
    })
    .unwrap();
    operations.update(cx, |operations, cx| operations.finish(ticket, cx));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("operation-status").is_none());
        assert!(crate::mutation::may_close(window, cx));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_dropped_ticket_frees_the_slot(cx: &mut TestAppContext) {
    let operations = cx.update(crate::mutation::Operations::global);
    let ticket = operations
        .update(cx, |operations, cx| operations.begin("Drain", cx))
        .ok()
        .unwrap();
    drop(ticket);
    assert!(cx.read(crate::mutation::Operations::current).is_none());
    assert!(
        operations
            .update(cx, |operations, cx| operations.begin("Drain", cx))
            .is_ok()
    );
}

/// Selects `service` on the Services page and opens its restart confirmation.
fn open_restart(cx: &mut TestAppContext, handle: AnyWindowHandle, service: &'static str) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        open_services_with_room(window, cx);
        window.render_frame(cx);
        window.within("services-region").click(service, cx);
        window.render_frame(cx);
        window.click("service-restart", cx);
    })
    .unwrap();
    crate::mutation::settle_confirmation(cx, handle);
}

/// The inspector as a node first opens, beside the table: Services stacks its
/// detail under the list, and Restart scrolls into view there.
#[gpui_kit::test]
fn restart_is_reachable_in_the_unexpanded_inspector(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        open_node_tab(window, cx, super::super::nodes::NodeTab::Services);
        window.render_frame(cx);
        assert!(!view.read(cx).pages.node_workspace.read(cx).expanded);
        assert!(window.try_find("nodes-table").is_some());
        window.within("services-region").click("containerd", cx);
        window.render_frame(cx);
        let list = window.find("services-region").bounds();
        let detail = window.find("selected-service").bounds();
        assert!(
            detail.top() >= list.bottom() - px(0.5),
            "the detail {detail:?} isn't under the list {list:?}"
        );
        // The Services page fills the inspector's content and scrolls in it.
        let shown = window.find("node-inspector-content").bounds();
        for _ in 0..60 {
            let restart = window.find("service-restart").bounds();
            if restart.top() >= shown.top() && restart.bottom() <= shown.bottom() {
                assert_eq!(
                    window.find("service-restart").label(),
                    Some("Restart containerd")
                );
                return;
            }
            use gpui_kit::InputEvent as _;
            window.dispatch_event(
                gpui_kit::ScrollWheelEvent {
                    position: shown.center(),
                    delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-40.))),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
        }
        panic!("Restart never scrolled into {shown:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn restart_is_offered_only_for_a_selected_service_on_an_answering_node(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        open_services_with_room(window, cx);
        window.render_frame(cx);
        // Nothing selected yet: there is nothing to restart.
        assert!(window.try_find("service-restart").is_none());
        window.within("services-region").click("containerd", cx);
        window.render_frame(cx);
        assert!(window.find("service-restart").visible());
        assert_eq!(
            window.find("service-restart").label(),
            Some("Restart containerd")
        );
    })
    .unwrap();
    // While another operation holds the slot, the button is off.
    let operations = cx.update(crate::mutation::Operations::global);
    let ticket = operations
        .update(cx, |operations, cx| {
            operations.begin("Drain talos-wk-fra1-01", cx)
        })
        .ok()
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Pressing it does nothing: no dialog opens.
        window.click("service-restart", cx);
        window.render_frame(cx);
        assert!(window.try_find("confirm-dialog").is_none());
    })
    .unwrap();
    cx.run_until_parked();
    operations.update(cx, |operations, cx| operations.finish(ticket, cx));
    // A node that doesn't answer has no services to restart.
    cx.update_window(handle, |_, window, cx| {
        pick_target(window, cx, 5);
        window.render_frame(cx);
        assert!(window.try_find("service-restart").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn confirming_a_restart_in_example_mode_simulates_it_and_frees_the_slot(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    open_restart(cx, handle, "containerd");
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            window.find("confirm-dialog").label(),
            Some("Restart containerd?")
        );
        assert!(window.try_find("service-restart-result").is_none());
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert!(window.try_find("confirm-dialog").is_none());
        assert!(window.try_find("confirm-blocked").is_none());
        // Run and finished within the click: the slot is free again.
        assert!(crate::mutation::Operations::current(cx).is_none());
        let result = window
            .find("service-restart-result")
            .label()
            .map(str::to_owned)
            .unwrap();
        assert!(result.contains("Example data"), "{result}");
        assert!(result.contains("containerd"), "{result}");
        assert_eq!(
            view.read(cx).pages.selected_service(cx).as_deref(),
            Some("containerd")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn dangerous_services_are_confirmed_by_name(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    for service in ["etcd", "apid"] {
        open_restart(cx, handle, service);
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(
                window.find("confirm-dialog").label().map(str::to_owned),
                Some(format!("Restart {service}?"))
            );
            window.click("confirm-cancel", cx);
            window.render_frame(cx);
            assert!(window.try_find("confirm-dialog").is_none());
            assert!(crate::mutation::Operations::current(cx).is_none());
            // Cancelling ran nothing.
            assert!(window.try_find("service-restart-result").is_none());
        })
        .unwrap();
        cx.run_until_parked();
    }
    assert!(crate::actions::critical_warning("etcd").is_some());
    assert!(crate::actions::critical_warning("machined").is_some());
    assert!(crate::actions::critical_warning("containerd").is_none());
}

#[gpui_kit::test]
fn safety_g10_a_stale_restart_preview_is_refused_and_runs_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    open_restart(cx, handle, "containerd");
    // The target moved on while the dialog was open.
    view.update(cx, |view, _| view.epoch = view.epoch.wrapping_add(1));
    cx.update_window(handle, |_, window, cx| {
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert!(window.find("confirm-blocked").visible());
        assert!(window.find("confirm-dialog").visible());
        assert!(crate::mutation::Operations::current(cx).is_none());
        assert!(window.try_find("service-restart-result").is_none());
        window.click("confirm-cancel", cx);
    })
    .unwrap();
    // Switching to another node also invalidates a preview.
    cx.run_until_parked();
    open_restart(cx, handle, "containerd");
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.select_node(Some(DEGRADED_NODE.to_owned()), window, cx)
        });
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert!(window.find("confirm-blocked").visible());
        assert!(window.try_find("service-restart-result").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_restart_is_refused_while_another_operation_runs(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    open_restart(cx, handle, "containerd");
    let operations = cx.update(crate::mutation::Operations::global);
    let ticket = operations
        .update(cx, |operations, cx| {
            operations.begin("Reboot talos-wk-fra1-02", cx)
        })
        .ok()
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert!(window.find("confirm-blocked").visible());
        assert!(window.try_find("service-restart-result").is_none());
        // The operation that holds the slot is untouched.
        assert!(
            crate::mutation::Operations::current(cx)
                .is_some_and(|running| running.label.starts_with("Reboot"))
        );
    })
    .unwrap();
    operations.update(cx, |operations, cx| operations.finish(ticket, cx));
}

/// A read-only cluster (#852) turns Restart off with the reason: pressing it
/// opens no confirmation and takes no slot.
#[gpui_kit::test]
fn safety_g10_a_read_only_cluster_refuses_a_restart_and_takes_no_slot(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        open_services_with_room(window, cx);
        window.render_frame(cx);
        window.within("services-region").click("containerd", cx);
        window.render_frame(cx);
        crate::mutation::set_protection(freshkube_core::write::Protection::ReadOnly, cx);
        window.render_frame(cx);
        window.click("service-restart", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("confirm-dialog").is_none());
        assert!(window.try_find("service-restart-result").is_none());
        assert!(crate::mutation::Operations::current(cx).is_none());
        let why = window
            .find("service-restart-blocked")
            .label()
            .map(str::to_owned);
        assert_eq!(why.as_deref(), Some("This cluster is read-only."));
        crate::mutation::set_protection(freshkube_core::write::Protection::Guarded, cx);
        window.render_frame(cx);
        window.click("service-restart", cx);
    })
    .unwrap();
    crate::mutation::settle_confirmation(cx, handle);
    cx.update_window(handle, |_, window, _| {
        assert_eq!(
            window.find("confirm-dialog").label(),
            Some("Restart containerd?")
        );
    })
    .unwrap();
}
