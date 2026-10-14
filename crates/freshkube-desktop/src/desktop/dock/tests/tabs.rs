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

/// Open either trigger and read the same menu, including its separator.
fn open_tab_menu(
    handle: AnyWindowHandle,
    id: u64,
    dropdown: bool,
    cx: &mut TestAppContext,
) -> Vec<Option<String>> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        if dropdown {
            window.click("dock-menu", cx);
        } else {
            window.right_click(format!("dock-tab-{id}"), cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menu = window.within("popup-menu");
        (0usize..)
            .map_while(|ix| menu.try_find(ix))
            .map(|item| item.label().map(str::to_owned))
            .collect()
    })
    .unwrap()
}

fn pick_tab_menu(handle: AnyWindowHandle, ix: usize, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.within("popup-menu").click(ix, cx)
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
}

#[gpui_kit::test]
fn a_tab_right_click_selects_it_and_shares_the_dropdown_menu(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    for pod in &pods[..3] {
        open_logs(handle, &pilot, "pods", pod, cx);
    }
    let items = open_tab_menu(handle, 0, false, cx);
    assert_eq!(cx.read(|cx| dock.read(cx).selected()), Some(0));
    let shortcut = if cfg!(target_os = "macos") {
        "kbd:cmd-w"
    } else {
        "kbd:ctrl-shift-w"
    };
    let hint = gpui_kit::VisualTestContext::from_window(handle, cx)
        .debug_bounds(shortcut)
        .expect("Close draws the platform shortcut");
    cx.update_window(handle, |_, window, _| {
        assert!(
            window
                .within("popup-menu")
                .find(0usize)
                .bounds()
                .contains(&hint.center())
        );
    })
    .unwrap();
    assert_eq!(
        items,
        [
            Some("Close".into()),
            Some("Close others".into()),
            Some("Close to the right".into()),
            None,
            Some("Close all".into()),
        ]
    );
    cx.update_window(handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("popup-menu").is_none());
        assert_eq!(window.find("logs-viewport").focused(), Some(true));
    })
    .unwrap();
    assert_eq!(open_tab_menu(handle, 0, true, cx), items);
    cx.update_window(handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    // The key after dismissal runs on the selected tab, as Close does.
    cx.update_window(handle, |_, window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-w"
            } else {
                "ctrl-shift-w"
            },
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        titles(&dock, cx),
        [
            format!("Pod {}", pods[1].name),
            format!("Pod {}", pods[2].name)
        ]
    );
}

#[gpui_kit::test]
fn each_tab_menu_entry_closes_the_clicked_tabs_scope(cx: &mut TestAppContext) {
    for (ix, remaining) in [(0, vec![0, 2]), (1, vec![1]), (2, vec![0, 1]), (4, vec![])] {
        let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
        let dock = dock(&pilot, cx);
        let pods = running_pods(&pilot, cx);
        for pod in &pods[..3] {
            open_logs(handle, &pilot, "pods", pod, cx);
        }
        open_tab_menu(handle, 1, false, cx);
        assert_eq!(cx.read(|cx| dock.read(cx).selected()), Some(1));
        pick_tab_menu(handle, ix, cx);
        let expected: Vec<_> = remaining
            .iter()
            .map(|&at| format!("Pod {}", pods[at].name))
            .collect();
        assert_eq!(titles(&dock, cx), expected, "menu entry {ix}");
        cx.update_window(handle, |_, window, _| {
            assert!(window.try_find("popup-menu").is_none());
            let focus = if remaining.is_empty() {
                "resource-body"
            } else {
                "logs-viewport"
            };
            assert_eq!(window.find(focus).focused(), Some(true));
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn every_tab_menu_entry_is_inert_after_another_tab_is_selected(cx: &mut TestAppContext) {
    for dropdown in [false, true] {
        for ix in [0, 1, 2, 4] {
            let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
            let dock = dock(&pilot, cx);
            let pods = running_pods(&pilot, cx);
            for pod in &pods[..3] {
                open_logs(handle, &pilot, "pods", pod, cx);
            }
            open_tab_menu(handle, 2, dropdown, cx);
            cx.update_window(handle, |_, window, cx| {
                // Keep the menu open, as a source update can change the
                // subject without first dismissing it.
                dock.update(cx, |dock, cx| dock.select(0, false, window, cx));
                window.render_frame(cx);
            })
            .unwrap();
            pick_tab_menu(handle, ix, cx);
            assert_eq!(titles(&dock, cx).len(), 3, "{dropdown}: entry {ix}");
            assert_eq!(cx.read(|cx| dock.read(cx).selected()), Some(0));
        }
    }
}

#[gpui_kit::test]
fn removing_the_clicked_tab_removes_its_menu_and_keeps_the_other_tabs(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    for pod in &pods[..3] {
        open_logs(handle, &pilot, "pods", pod, cx);
    }
    open_tab_menu(handle, 1, false, cx);
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| dock.close_tab(1, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("popup-menu").is_none());
    })
    .unwrap();
    assert_eq!(
        titles(&dock, cx),
        [
            format!("Pod {}", pods[0].name),
            format!("Pod {}", pods[2].name)
        ]
    );
}

#[gpui_kit::test]
fn right_clicking_a_tabs_close_button_opens_its_menu_without_closing_it(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    for pod in &pods[..2] {
        open_logs(handle, &pilot, "pods", pod, cx);
    }
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click("dock-tab-0-close", cx);
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert_eq!(titles(&dock, cx).len(), 2);
    assert_eq!(cx.read(|cx| dock.read(cx).selected()), Some(0));
    cx.update_window(handle, |_, window, cx| {
        assert!(window.try_find("popup-menu").is_some());
        // Clicking outside dismisses without closing a tab.
        window.click("resource-body", cx);
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert_eq!(titles(&dock, cx).len(), 2);
}

#[gpui_kit::test]
fn a_minimized_tabs_menu_keeps_the_bar_in_place(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    for pod in &pods[..2] {
        open_logs(handle, &pilot, "pods", pod, cx);
    }
    cx.update_window(handle, |_, window, cx| window.press("shift-escape", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert!(!cx.read(|cx| dock.read(cx).is_open()));
    open_tab_menu(handle, 0, false, cx);
    assert!(!cx.read(|cx| dock.read(cx).is_open()));
    assert_eq!(cx.read(|cx| dock.read(cx).selected()), Some(0));
    cx.update_window(handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    cx.update_window(handle, |_, window, cx| {
        assert!(dock.read(cx).has_focus(window, cx));
    })
    .unwrap();
    open_tab_menu(handle, 0, false, cx);
    pick_tab_menu(handle, 0, cx);
    assert_eq!(titles(&dock, cx), [format!("Pod {}", pods[1].name)]);
    assert!(!cx.read(|cx| dock.read(cx).is_open()));
    cx.update_window(handle, |_, window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-w"
            } else {
                "ctrl-shift-w"
            },
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert!(titles(&dock, cx).is_empty());
}

#[gpui_kit::test]
fn a_running_shells_context_menu_keeps_the_existing_close_confirmation(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    pick_shell(handle, &pilot, &pods[0], cx);
    open_logs(handle, &pilot, "pods", &pods[1], cx);
    open_tab_menu(handle, 0, false, cx);
    pick_tab_menu(handle, 0, cx);
    assert!(cx.has_pending_prompt());
    assert_eq!(shell_state(&dock, 0, cx), ShellState::Running);
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(titles(&dock, cx).len(), 2);
    open_tab_menu(handle, 0, false, cx);
    pick_tab_menu(handle, 0, cx);
    cx.simulate_prompt_answer("End the shell");
    cx.run_until_parked();
    assert_eq!(titles(&dock, cx), [format!("Pod {}", pods[1].name)]);
    assert!(cx.read(shell::running_anywhere).is_empty());
}

/// Actual draws preserve the dock's cache; render_frame would hide a
/// dismissed menu's stale dispatch path by invalidating every view.
fn draw_cached_tabs(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    for _ in 0..3 {
        cx.update_window(handle, |_, window, cx| {
            window.draw(cx).clear(cx);
            window.simulate_next_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn dismissing_a_cached_tab_menu_redraws_the_dock_and_keeps_its_close_key(cx: &mut TestAppContext) {
    for minimized in [false, true] {
        let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
        let dock = dock(&pilot, cx);
        let pods = running_pods(&pilot, cx);
        for pod in &pods[..2] {
            open_logs(handle, &pilot, "pods", pod, cx);
        }
        if minimized {
            cx.update_window(handle, |_, window, cx| window.press("shift-escape", cx))
                .unwrap();
            cx.run_until_parked();
        }
        draw_cached_tabs(handle, cx);
        cx.update_window(handle, |_, window, cx| window.right_click("dock-tab-0", cx))
            .unwrap();
        cx.run_until_parked();
        draw_cached_tabs(handle, cx);
        cx.update_window(handle, |_, window, _| {
            assert!(window.try_find("popup-menu").is_some());
        })
        .unwrap();
        let before = freshkube_probe::probe::count("dock");
        cx.update_window(handle, |_, window, cx| window.press("escape", cx))
            .unwrap();
        cx.run_until_parked();
        draw_cached_tabs(handle, cx);
        assert!(freshkube_probe::probe::count("dock") > before);
        cx.update_window(handle, |_, window, cx| {
            assert!(window.try_find("popup-menu").is_none());
            assert!(dock.read(cx).has_focus(window, cx));
            window.press(
                if cfg!(target_os = "macos") {
                    "cmd-w"
                } else {
                    "ctrl-shift-w"
                },
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        draw_cached_tabs(handle, cx);
        assert_eq!(titles(&dock, cx), [format!("Pod {}", pods[1].name)]);
    }
}
