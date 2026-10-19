use super::*;

#[gpui_kit::test]
fn keys_switch_close_and_minimize_tabs_and_escape_hands_the_keyboard_back(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    for pod in &pods[..3] {
        open_logs(handle, &pilot, "pods", pod, cx);
    }
    let press = |key: &str, cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
        draw(handle, cx);
    };
    let selected = |cx: &mut TestAppContext| cx.update(|cx| dock.read(cx).selected());
    assert_eq!(selected(cx), Some(2));
    press("ctrl-.", cx);
    assert_eq!(selected(cx), Some(0), "wraps round");
    press("ctrl-,", cx);
    assert_eq!(selected(cx), Some(2));
    press("ctrl-,", cx);
    assert_eq!(selected(cx), Some(1));
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("logs-viewport").focused(), Some(true));
    })
    .unwrap();

    // Command-W closes the selected tab; the next one takes its place.
    press(
        if cfg!(target_os = "macos") {
            "cmd-w"
        } else {
            "ctrl-shift-w"
        },
        cx,
    );
    assert_eq!(titles(&dock, cx).len(), 2);
    assert_eq!(selected(cx), Some(2));

    // Escape with nothing to clear hands the keyboard back to the list.
    press("escape", cx);
    cx.update_window(handle, |_, window, cx| {
        assert!(!dock.read(cx).has_focus(window, cx));
        assert_eq!(window.find("resource-body").focused(), Some(true));
    })
    .unwrap();

    // Shift-Escape minimizes to the tabs, from anywhere.
    press("shift-escape", cx);
    assert!(cx.update(|cx| !dock.read(cx).is_open()));
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("dock-bar").visible());
        assert!(window.try_find("dock-body").is_none());
    })
    .unwrap();
    // Control-. brings it back on the tab it had.
    press("ctrl-.", cx);
    assert!(cx.update(|cx| dock.read(cx).is_open()));
    assert_eq!(selected(cx), Some(2));
}

#[gpui_kit::test]
fn command_f_searches_the_logs_in_the_dock_and_the_yaml_in_the_pane(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-f", cx);
        window.render_frame(cx);
        assert!(window.find("logs-search").focused().unwrap_or(false));
        assert_eq!(
            pilot.read(cx).pages.resources.read(cx).detail_tab(cx),
            Tab::Overview
        );
        // From the pane, it finds in the YAML. Logs closed the drawer; the
        // row opens it again.
        window.click_at(
            crate::resources::row_id(&pods[0]),
            gpui_kit::point(px(40.), px(8.)),
            cx,
        );
        window.render_frame(cx);
        window.click("detail-tab-details", cx);
        window.render_frame(cx);
        window.press("secondary-f", cx);
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).pages.resources.read(cx).detail_tab(cx),
            Tab::Yaml
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn escape_in_the_dock_clears_the_search_leaves_it_then_returns_to_the_list(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    let step = |cx: &mut TestAppContext, key: &str, typed: Option<&str>| {
        cx.update_window(handle, |_, window, cx| {
            window.press(key, cx);
            if let Some(text) = typed {
                window.input(text, cx);
            }
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let focused = |cx: &mut TestAppContext, id: &'static str| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).focused().unwrap_or(false)
        })
        .unwrap()
    };
    step(cx, "secondary-f", Some("GET"));
    assert!(focused(cx, "logs-search"));
    // The first Escape clears the search and stays in it.
    step(cx, "escape", None);
    assert!(focused(cx, "logs-search"));
    // The next leaves it for the lines.
    step(cx, "escape", None);
    assert!(focused(cx, "logs-viewport"));
    // With nothing selected, the keyboard goes back to the list.
    step(cx, "escape", None);
    assert!(focused(cx, "resource-body"));
    assert!(cx.update(|cx| pilot.read(cx).pages.dock.read(cx).has_tabs()));
}

#[gpui_kit::test]
fn without_tabs_the_docks_keys_go_on(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let handled = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            ["ctrl-.", "ctrl-,", "shift-escape"]
                .map(|key| window.dispatch_keystroke(gpui_kit::Keystroke::parse(key).unwrap(), cx))
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(handled, [false; 3], "nothing took them");
    assert!(
        cx.update(|cx| dock.read(cx).is_open()),
        "nothing was minimized"
    );
}

#[gpui_kit::test]
fn leaving_the_dock_after_another_page_puts_the_keyboard_on_that_page(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    // The list had the keyboard when the dock took it.
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.open_builtin("pods", window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| pilot.focus_page(window, cx));
        window.render_frame(cx);
        assert_eq!(window.find("resource-body").focused(), Some(true));
        let target = crate::resources::detail::DetailTarget {
            identity: pods[0].clone(),
            kind: builtin("pods").unwrap(),
        };
        dock.update(cx, |dock, cx| {
            dock.open_logs(
                crate::resources::LogsRequest { target, at: None },
                window,
                cx,
            )
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(dock.read(cx).has_focus(window, cx));
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Overview, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    cx.update_window(handle, |_, window, cx| {
        // A click in the lines takes the keyboard without the dock
        // remembering anew: it still holds the list.
        dock.update(cx, |dock, cx| dock.tabs[0].focus_lines(window, cx));
        window.render_frame(cx);
        assert!(dock.read(cx).has_focus(window, cx));
        dock.update(cx, |dock, cx| dock.leave(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Not the list, which Overview doesn't draw: the page.
        assert!(window.try_find("resource-body").is_none());
        assert!(pilot.read(cx).focus.contains_focused(window, cx));
        assert!(!dock.read(cx).has_focus(window, cx));
    })
    .unwrap();
}
