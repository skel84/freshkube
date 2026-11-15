use super::*;

#[gpui_kit::test]
fn search_marks_matches_steps_through_them_and_escape_backs_out(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, emitted) = mount(cx);
    let (pod, _) = crashing_pod();
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
    step(cx, &|window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        window.render_frame(cx);
        window.click("detail-details", cx);
    });
    step(cx, &|window, cx| window.press("secondary-f", cx));
    step(cx, &|window, cx| {
        assert_eq!(window.find("detail-tab-yaml").selected(), Some(true));
        window.input("CONTAINER", cx);
    });
    let found = pane.read_with(cx, |pane, _| pane.matches.len());
    assert!(found > 2, "{found}");
    assert_eq!(count(cx), Some(format!("1 of {found}")));
    step(cx, &|window, cx| window.press("enter", cx));
    assert_eq!(count(cx), Some(format!("2 of {found}")));
    step(cx, &|window, cx| window.press("shift-enter", cx));
    step(cx, &|window, cx| window.press("shift-enter", cx));
    assert_eq!(count(cx), Some(format!("{found} of {found}")));
    step(cx, &|window, cx| window.click("detail-find-next", cx));
    assert_eq!(count(cx), Some(format!("1 of {found}")));

    // Escape clears the search, then leaves it, then steps back to the
    // list with the pane still open.
    step(cx, &|window, cx| {
        let focus = pane.read(cx).find.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    step(cx, &|window, cx| window.press("escape", cx));
    assert_eq!(count(cx), None);
    assert!(pane.read_with(cx, |pane, _| pane.matches.is_empty()));
    step(cx, &|window, cx| window.press("escape", cx));
    assert!(emitted.borrow().is_empty());
    step(cx, &|window, cx| window.press("escape", cx));
    assert_eq!(*emitted.borrow(), [DetailEvent::Leave]);
    assert!(pane.read_with(cx, |pane, _| pane.detail.is_some()));
}

#[gpui_kit::test]
fn lines_select_and_copy_and_copy_yaml_takes_the_whole_document(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (deployment, _) = target("deployments.apps", |_, _| true);
    let clipboard = |cx: &mut TestAppContext| {
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default()
    };
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &deployment, Duration::ZERO, cx);
        window.render_frame(cx);
        window.click("detail-tab-yaml", cx);
        window.render_frame(cx);
        window.click(("detail-yaml-line", 1usize), cx);
        pane.update(cx, |pane, cx| pane.select_line(3, true, cx));
        window.render_frame(cx);
        assert_eq!(
            window.find(("detail-yaml-line", 2usize)).selected(),
            Some(true)
        );
        assert_eq!(
            window.find(("detail-yaml-line", 0usize)).selected(),
            Some(false)
        );
        assert_eq!(
            window.find("detail-copy-lines").label(),
            Some("Copy 3 lines")
        );
        window.press("secondary-c", cx);
    })
    .unwrap();
    let yaml = pane.read_with(cx, |pane, _| pane.view().unwrap().document.yaml.clone());
    let lines: Vec<&str> = yaml.lines().collect();
    assert_eq!(clipboard(cx), lines[1..4].join("\n"));

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("detail-feedback").label(),
            Some("Copied 3 lines")
        );
        window.press("secondary-a", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("detail-copy-lines").label(),
            Some(format!("Copy {} lines", lines.len()).as_str())
        );
        window.click("detail-copy-yaml", cx);
    })
    .unwrap();
    assert_eq!(clipboard(cx), yaml);

    // The button beside the title copies the name alone, as kubectl
    // takes it.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("detail-copy-name", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("detail-feedback").label(),
            Some("Copied the name")
        );
    })
    .unwrap();
    let name = pane.read_with(cx, |pane, _| {
        pane.detail.as_ref().unwrap().target.identity.name.clone()
    });
    assert!(!name.contains('/'));
    assert_eq!(clipboard(cx), name);
}

/// A click on the YAML tab switches to it, at every pane width and text size
/// (#1056, which reported two clicks doing nothing). The strip's fade over a
/// cut end only dims the tab under it; just its chevron takes a click, so a
/// tab is pressed at its centre unless the chevron is what covers it.
#[gpui_kit::test]
fn a_click_on_the_yaml_tab_switches_to_it_at_every_width_and_text_size(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = running_pod();
    cx.update(|cx| crate::text_size::install(None, cx));
    let mut failed = vec![];
    for text in [13., 16., 20.] {
        cx.update(|cx| crate::text_size::set(text, cx));
        for width in (220..=700).step_by(20) {
            cx.simulate_window_resize(handle, size(px(width as f32), px(820.)));
            cx.update_window(handle, |_, window, cx| {
                open(&pane, &pod, Duration::ZERO, cx);
                pane.update(cx, |pane, cx| pane.set_tab(Tab::Overview, cx));
                settle(window, cx);
                window.click("detail-tab-yaml", cx);
                settle(window, cx);
                if window.find("detail-tab-yaml").selected() != Some(true) {
                    failed.push((text, width));
                }
            })
            .unwrap();
        }
    }
    assert!(failed.is_empty(), "{failed:?}");
}
