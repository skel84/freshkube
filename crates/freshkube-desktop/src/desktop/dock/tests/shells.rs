use super::*;

#[gpui_kit::test]
fn a_pick_from_the_shell_menu_starts_a_shell_in_a_tab_of_its_own(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                builtin("pods").unwrap(),
                pods[0].clone().into(),
                Tab::Overview,
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    // Opening the menu runs nothing.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("detail-open-shell", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(titles(&dock, cx).is_empty());
    assert!(cx.read(shell::running_anywhere).is_empty());
    cx.update_window(handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();

    pick_shell(handle, &pilot, &pods[0], cx);
    assert_eq!(titles(&dock, cx), [format!("Shell {}", pods[0].name)]);
    assert_eq!(shell_state(&dock, 0, cx), ShellState::Running);
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("terminal").focused(), Some(true));
        assert_eq!(window.find("dock-toggle").label(), Some("1 shell"));
    })
    .unwrap();

    // Another pod's shell takes another tab, and a log tab counts apart.
    pick_shell(handle, &pilot, &pods[1], cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    assert_eq!(
        titles(&dock, cx),
        [
            format!("Shell {}", pods[0].name),
            format!("Shell {}", pods[1].name),
            format!("Pod {}", pods[0].name),
        ]
    );
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("dock-toggle").label(), Some("1 log · 2 shells"));
    })
    .unwrap();
    assert_eq!(cx.read(shell::running_anywhere).len(), 2);

    // The same container again selects its tab, and starts again a shell
    // that ended.
    let first = shell_view(&dock, 0, cx);
    cx.update(|cx| first.update(cx, |view, cx| view.end(cx)));
    assert_eq!(shell_state(&dock, 0, cx), ShellState::Ended);
    pick_shell(handle, &pilot, &pods[0], cx);
    assert_eq!(titles(&dock, cx).len(), 3);
    assert_eq!(cx.update(|cx| dock.read(cx).selected()), Some(0));
    assert_eq!(shell_state(&dock, 0, cx), ShellState::Running);
}

#[gpui_kit::test]
fn closing_a_running_shells_tab_asks_first(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    pick_shell(handle, &pilot, &pods[0], cx);
    pick_shell(handle, &pilot, &pods[1], cx);
    let click = |id: &'static str, cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };

    // The tab's ×: Cancel keeps it and its shell.
    click("dock-tab-1-close", cx);
    let (message, _) = cx.pending_prompt().unwrap();
    assert_eq!(message, format!("End the shell in {}?", pods[1].name));
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(titles(&dock, cx).len(), 2);
    assert_eq!(cx.read(shell::running_anywhere).len(), 2);

    // The chrome's × asks once for both.
    click("dock-close-all", cx);
    let (message, _) = cx.pending_prompt().unwrap();
    assert_eq!(message, "End 2 shells?");
    cx.simulate_prompt_answer("End the shells");
    cx.run_until_parked();
    assert!(titles(&dock, cx).is_empty());
    assert!(cx.read(shell::running_anywhere).is_empty());

    // A shell that has ended closes without asking.
    pick_shell(handle, &pilot, &pods[0], cx);
    let view = shell_view(&dock, 0, cx);
    cx.update(|cx| view.update(cx, |view, cx| view.end(cx)));
    let id = cx.update(|cx| dock.read(cx).tabs[0].id);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(format!("dock-tab-{id}-close"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert!(titles(&dock, cx).is_empty());
}

#[gpui_kit::test]
fn a_ninth_shell_tab_asks_to_close_the_oldest(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    assert!(pods.len() > MAX_SHELL_TABS);
    // Log tabs don't count toward the shells' cap.
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    for pod in &pods[..MAX_SHELL_TABS] {
        start_shell(handle, &pilot, pod, cx);
    }
    assert_eq!(titles(&dock, cx).len(), MAX_SHELL_TABS + 1);
    assert_eq!(cx.read(shell::running_anywhere).len(), MAX_SHELL_TABS);
    assert!(!cx.has_pending_prompt());

    // Every shell runs: the oldest asks first, and Cancel opens nothing.
    start_shell(handle, &pilot, &pods[MAX_SHELL_TABS], cx);
    let (message, _) = cx.pending_prompt().unwrap();
    assert_eq!(message, format!("End the shell in {}?", pods[0].name));
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(titles(&dock, cx).len(), MAX_SHELL_TABS + 1);
    assert_eq!(cx.read(shell::running_anywhere).len(), MAX_SHELL_TABS);

    // Agreeing ends the oldest shell and starts the new one.
    start_shell(handle, &pilot, &pods[MAX_SHELL_TABS], cx);
    cx.simulate_prompt_answer("End the shell");
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    let shown = titles(&dock, cx);
    assert_eq!(shown.len(), MAX_SHELL_TABS + 1);
    assert_eq!(shown[0], format!("Pod {}", pods[0].name));
    assert_eq!(shown[1], format!("Shell {}", pods[1].name));
    assert_eq!(
        shown.last().unwrap(),
        &format!("Shell {}", pods[MAX_SHELL_TABS].name)
    );
    let running = cx.read(shell::running_anywhere);
    assert_eq!(running.len(), MAX_SHELL_TABS);
    assert!(!running.iter().any(|pod| pod.as_ref() == pods[0].name));

    // A shell that ended goes first, before older running ones, unasked.
    let ended = shell_view(&dock, 4, cx);
    let ended_pod = cx.read(|cx| ended.read(cx).pod().unwrap().name.clone());
    cx.update(|cx| ended.update(cx, |view, cx| view.end(cx)));
    cx.run_until_parked();
    start_shell(handle, &pilot, &pods[0], cx);
    assert!(!cx.has_pending_prompt());
    let shown = titles(&dock, cx);
    assert_eq!(shown.len(), MAX_SHELL_TABS + 1);
    assert!(!shown.contains(&format!("Shell {ended_pod}")));
    assert_eq!(shown[1], format!("Shell {}", pods[1].name));
    assert_eq!(shown.last().unwrap(), &format!("Shell {}", pods[0].name));
}

#[gpui_kit::test]
fn every_way_of_closing_a_running_shells_tab_asks(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    pick_shell(handle, &pilot, &pods[1], cx);
    pick_shell(handle, &pilot, &pods[2], cx);
    let ids: Vec<u64> = cx.update(|cx| dock.read(cx).tabs.iter().map(|tab| tab.id).collect());
    let asked = |expected: String, cx: &mut TestAppContext| {
        let (message, _) = cx.pending_prompt().unwrap();
        assert_eq!(message, expected);
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert_eq!(titles(&dock, cx).len(), 3);
        assert_eq!(cx.read(shell::running_anywhere).len(), 2);
    };

    // A middle-click.
    cx.update_window(handle, |_, window, cx| {
        use gpui_kit::InputEvent as _;
        window.render_frame(cx);
        let position = window
            .find(format!("dock-tab-{}", ids[1]))
            .bounds()
            .center();
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Middle,
                position,
                click_count: 1,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    asked(format!("End the shell in {}?", pods[1].name), cx);

    // Command-W from the selected shell's terminal.
    let close = if cfg!(target_os = "macos") {
        "cmd-w"
    } else {
        "ctrl-shift-w"
    };
    cx.update_window(handle, |_, window, cx| window.press(close, cx))
        .unwrap();
    cx.run_until_parked();
    asked(format!("End the shell in {}?", pods[2].name), cx);

    // Close others and Close to the right, from the log tab.
    for close in [Dock::close_others, Dock::close_to_right] {
        cx.update_window(handle, |_, window, cx| {
            dock.update(cx, |dock, cx| close(dock, ids[0], window, cx))
        })
        .unwrap();
        cx.run_until_parked();
        asked("End 2 shells?".into(), cx);
    }
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| dock.close_to_right(ids[0], window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    cx.simulate_prompt_answer("End the shells");
    cx.run_until_parked();
    assert_eq!(titles(&dock, cx), [format!("Pod {}", pods[0].name)]);
    assert!(cx.read(shell::running_anywhere).is_empty());
}

#[gpui_kit::test]
fn a_gone_pods_shell_tab_starts_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pod = running_pods(&pilot, cx)[0].clone();
    pick_shell(handle, &pilot, &pod, cx);
    let view = shell_view(&dock, 0, cx);
    cx.update(|cx| view.update(cx, |view, cx| view.end(cx)));
    cx.run_until_parked();
    let ended = shell_state(&dock, 0, cx);
    assert!(cx.read(|cx| view.read(cx).can_start()));
    // Its watch finds the pod gone.
    let id = cx.update(|cx| dock.read(cx).tabs[0].id);
    let gone = freshkube_core::resources::WorkloadPods {
        listed: true,
        ..Default::default()
    };
    cx.update(|cx| dock.update(cx, |dock, cx| dock.apply_pod(id, gone, cx)));
    assert!(!cx.read(|cx| view.read(cx).can_start()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("pod-shell-start", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(shell_state(&dock, 0, cx), ended);
    assert!(cx.read(shell::running_anywhere).is_empty());
    // Nor does a pick from the menu.
    pick_shell(handle, &pilot, &pod, cx);
    assert_eq!(shell_state(&dock, 0, cx), ended);
    assert!(cx.read(shell::running_anywhere).is_empty());
}

#[gpui_kit::test]
fn the_terminal_keeps_control_period_and_comma(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    pick_shell(handle, &pilot, &pods[1], cx);
    assert_eq!(cx.update(|cx| dock.read(cx).selected()), Some(1));
    for key in ["ctrl-.", "ctrl-,"] {
        cx.update_window(handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
        // The shell has them; the tab stays.
        assert_eq!(cx.update(|cx| dock.read(cx).selected()), Some(1));
    }
    // Command-Escape hands the keyboard back, and the keys step the tabs.
    let leave = if cfg!(target_os = "macos") {
        "secondary-escape"
    } else {
        "ctrl-shift-q"
    };
    cx.update_window(handle, |_, window, cx| window.press(leave, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.press("ctrl-.", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| dock.read(cx).selected()), Some(0));
    assert_eq!(shell_state(&dock, 1, cx), ShellState::Running);
}

#[gpui_kit::test]
fn a_shell_tab_asked_for_by_name_starts_on_the_pod_its_watch_finds(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    let pod = pods[0].clone();
    assert!(!pod.uid.is_empty());
    // As a restore on a cluster: the pod by name alone, no UID yet.
    let by_name = ResourceIdentity {
        uid: String::new(),
        ..pod.clone()
    };
    let target = crate::resources::detail::DetailTarget {
        identity: by_name.clone(),
        kind: builtin("pods").unwrap(),
    };
    let container = watched(&pod).pods[0].containers.default.clone().unwrap();
    let id = cx
        .update_window(handle, |_, window, cx| {
            dock.update(cx, |dock, cx| {
                let id = dock.add_shell(target, container.clone(), window, cx);
                let view = shell_view_of(dock, 0);
                let access = dock.source.as_ref().unwrap().access.clone();
                view.update(cx, |view, cx| {
                    view.show_pod(Some(by_name.clone()), Some(access), cx);
                    view.choose_container(container.clone(), cx);
                });
                id
            })
        })
        .unwrap();
    let view = cx.update(|cx| shell_view_of(dock.read(cx), 0));
    assert!(!cx.read(|cx| view.read(cx).can_start()), "no UID, no start");

    // The watch finds the pod: the view takes its UID and keeps its container.
    cx.update(|cx| dock.update(cx, |dock, cx| dock.apply_pod(id, watched(&pod), cx)));
    cx.read(|cx| {
        let view = view.read(cx);
        assert_eq!(view.pod().unwrap().uid, pod.uid);
        assert_eq!(view.container(), Some(container.as_str()));
        assert!(view.can_start());
    });
    // The menu picks the same pod: the tab, not another.
    pick_shell(handle, &pilot, &pod, cx);
    assert_eq!(titles(&dock, cx), [format!("Shell {}", pod.name)]);
    assert_eq!(shell_state(&dock, 0, cx), ShellState::Running);
}

#[gpui_kit::test]
fn a_shell_tab_asked_for_by_name_takes_the_picked_pods_uid(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pod = running_pods(&pilot, cx)[0].clone();
    let by_name = ResourceIdentity {
        uid: String::new(),
        ..pod.clone()
    };
    let container = watched(&pod).pods[0].containers.default.clone().unwrap();
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| {
            let target = crate::resources::detail::DetailTarget {
                identity: by_name.clone(),
                kind: builtin("pods").unwrap(),
            };
            dock.add_shell(target, container.clone(), window, cx);
            let view = shell_view_of(dock, 0);
            let access = dock.source.as_ref().unwrap().access.clone();
            view.update(cx, |view, cx| {
                view.show_pod(Some(by_name.clone()), Some(access), cx);
                view.choose_container(container.clone(), cx);
            });
        })
    })
    .unwrap();
    // Before its watch answers, the menu's pick settles the tab's pod.
    pick_shell(handle, &pilot, &pod, cx);
    assert_eq!(titles(&dock, cx), [format!("Shell {}", pod.name)]);
    let view = cx.update(|cx| shell_view_of(dock.read(cx), 0));
    assert_eq!(
        cx.read(|cx| view.read(cx).pod().unwrap().uid.clone()),
        pod.uid
    );
    assert_eq!(shell_state(&dock, 0, cx), ShellState::Running);
}

#[gpui_kit::test]
fn end_then_tab_close_keeps_the_closing_worker_after_the_view_drops(cx: &mut TestAppContext) {
    end_then_remove(false, cx);
}

#[gpui_kit::test]
fn end_then_tab_close_then_quit_waits_without_a_view_or_another_prompt(cx: &mut TestAppContext) {
    end_then_remove(true, cx);
}

fn end_then_remove(quit: bool, cx: &mut TestAppContext) {
    use std::{cell::Cell, rc::Rc};
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    pick_shell(handle, &pilot, &pods[0], cx);
    let id = cx.update(|cx| dock.read(cx).tabs[0].id);
    let view = shell_view(&dock, 0, cx);
    let weak = view.downgrade();
    let mut transport = shell::ClosingTransport::install(&view, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("pod-shell-end", cx);
    })
    .unwrap();
    cx.run_until_parked();
    transport.wait_for_interrupt(cx);
    drop(view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(format!("dock-tab-{id}-close"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert!(titles(&dock, cx).is_empty());
    assert!(weak.upgrade().is_none(), "the tab must release its view");
    assert!(!cx.has_pending_prompt(), "End already gave consent");
    let closed = Rc::new(Cell::new(false));
    if quit {
        let flag = closed.clone();
        let immediate = cx
            .update_window(handle, |_, window, cx| {
                shell::may_close(window, cx, move |_, _| flag.set(true))
            })
            .unwrap();
        assert!(!immediate, "closing work must postpone quit without a view");
        cx.run_until_parked();
        assert!(!closed.get());
        assert!(!cx.has_pending_prompt());
    }
    transport.release();
    transport.wait_for_eof(cx);
    transport.exit();
    transport.wait_for_completion(cx);
    if quit {
        freshkube_ui::real_io::until_io(cx, "quit after tab removal", |_| closed.get());
    }
    assert!(!cx.has_pending_prompt());
}

#[gpui_kit::test]
fn direct_tab_close_can_be_cancelled_then_sends_one_eof(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    pick_shell(handle, &pilot, &pods[0], cx);
    let id = cx.update(|cx| dock.read(cx).tabs[0].id);
    let view = shell_view(&dock, 0, cx);
    let weak = view.downgrade();
    let mut transport = shell::ClosingTransport::install(&view, cx);
    drop(view);
    let close = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(format!("dock-tab-{id}-close"), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(cx.has_pending_prompt());
    };
    close(cx);
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(titles(&dock, cx).len(), 1);
    assert_eq!(shell_state(&dock, 0, cx), ShellState::Running);
    close(cx);
    cx.simulate_prompt_answer("End the shell");
    cx.run_until_parked();
    draw(handle, cx);
    assert!(weak.upgrade().is_none());
    transport.wait_for_interrupt(cx);
    transport.release();
    transport.wait_for_eof(cx);
    transport.exit();
    transport.wait_for_completion(cx);
    assert!(!cx.has_pending_prompt());
}
