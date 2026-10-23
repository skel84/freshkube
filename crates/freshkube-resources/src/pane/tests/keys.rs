use super::*;

#[gpui_kit::test]
fn tabs_take_the_keyboard_and_command_brackets_switch_them(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = running_pod();
    let (deployment, _) = target("deployments.apps", |_, _| true);
    let (secret, _) = target("secrets", |_, _| true);
    let tab = |window: &mut gpui_kit::Window, id: &'static str| {
        let tab = window.find(id);
        (tab.selected(), tab.focused())
    };
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        window.render_frame(cx);
        // A click focuses the tab, and the arrows move along the tabs.
        window.click("detail-tab-details", cx);
        window.render_frame(cx);
        assert_eq!(tab(window, "detail-tab-details"), (Some(true), Some(true)));
        window.press("right", cx);
        window.render_frame(cx);
        assert_eq!(tab(window, "detail-tab-yaml"), (Some(true), Some(true)));
        // Two tabs, which wrap round.
        window.press("right", cx);
        window.render_frame(cx);
        assert_eq!(tab(window, "detail-tab-details"), (Some(true), Some(true)));
        window.press("left", cx);
        window.render_frame(cx);
        assert_eq!(tab(window, "detail-tab-yaml"), (Some(true), Some(true)));

        // From the pane, Option-Command-] and [ switch the tab and hand the
        // keyboard to the pane.
        pane.update(cx, |pane, cx| pane.set_tab(super::Tab::Overview, cx));
        let focus = pane.read(cx).focus.clone();
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("secondary-alt-[", cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-tab-yaml").selected(), Some(true));
        assert_eq!(window.find("resource-detail").focused(), Some(true));
        window.press("secondary-alt-]", cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-tab-details").selected(), Some(true));
        assert_eq!(window.find("resource-detail").focused(), Some(true));

        // Every kind has the same two tabs; Details holds a pod's and a
        // workload's ports, and every kind's events.
        for (target, ports) in [(&pod, true), (&deployment, true), (&secret, false)] {
            open(&pane, target, Duration::ZERO, cx);
            window.render_frame(cx);
            assert!(window.find("detail-tab-yaml").visible());
            assert!(window.find("detail-jump-events").visible());
            assert_eq!(window.try_find("detail-jump-ports").is_some(), ports);
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn find_keys_follow_the_tab_shown(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = running_pod();
    let step = |cx: &mut TestAppContext, act: &dyn Fn(&mut gpui_kit::Window, &mut App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let count = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find("detail-find-count")
                .and_then(|count| count.label().map(str::to_owned))
        })
        .unwrap()
    };
    let focus_pane = |window: &mut gpui_kit::Window, cx: &mut App| {
        let focus = pane.read(cx).focus.clone();
        window.focus(&focus, cx);
    };
    step(cx, &|window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        window.render_frame(cx);
        focus_pane(window, cx);
        window.press("secondary-f", cx);
    });
    step(cx, &|window, cx| window.input("CONTAINER", cx));
    let found = pane.read_with(cx, |pane, _| pane.matches.len());
    assert!(found > 2, "{found}");
    // Command-G and F3 step through the YAML matches from the pane.
    step(cx, &|window, cx| {
        focus_pane(window, cx);
        window.press("secondary-g", cx);
    });
    assert_eq!(count(cx), Some(format!("2 of {found}")));
    step(cx, &|window, cx| window.press("f3", cx));
    assert_eq!(count(cx), Some(format!("3 of {found}")));
    step(cx, &|window, cx| window.press("secondary-shift-g", cx));
    step(cx, &|window, cx| window.press("shift-f3", cx));
    assert_eq!(count(cx), Some(format!("1 of {found}")));
}
