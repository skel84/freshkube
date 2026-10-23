use super::*;
use gpui_kit::InputEvent as _;

fn yaml_menu_fixture(
    cx: &mut TestAppContext,
    pane: &Entity<DetailPane>,
    handle: AnyWindowHandle,
    node: bool,
) {
    let (target, _) = if node {
        target("nodes", |_, _| true)
    } else {
        crashing_pod()
    };
    cx.update_window(handle, |_, window, cx| {
        open(pane, &target, Duration::ZERO, cx);
        pane.update(cx, |pane, cx| {
            pane.selection = None;
            if node {
                pane.embed_node(super::Tab::Yaml, cx);
            } else {
                pane.set_tab(super::Tab::Yaml, cx);
            }
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn open_yaml_menu(cx: &mut TestAppContext, handle: AnyWindowHandle, line: usize) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(("detail-yaml-line", line), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("popup-menu").visible());
    })
    .unwrap();
}

fn pick_yaml_menu(cx: &mut TestAppContext, handle: AnyWindowHandle, item: usize) {
    cx.update_window(handle, |_, window, cx| {
        window.within("popup-menu").click(item, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn yaml_menu_selection(cx: &mut TestAppContext, node: bool) {
    let (_runtime, pane, handle, _) = mount(cx);
    yaml_menu_fixture(cx, &pane, handle, node);
    let yaml = pane.read_with(cx, |pane, _| pane.view().unwrap().document.yaml.clone());
    let lines: Vec<_> = yaml.lines().collect();
    cx.update_window(handle, |_, window, cx| {
        window.click(("detail-yaml-line", 3usize), cx);
        pane.update(cx, |pane, cx| pane.select_line(1, true, cx));
        // Search had focus before right mouse-down; Escape must return to
        // the document rather than back to the search input.
        pane.update(cx, |pane, cx| pane.focus_find(window, cx));
    })
    .unwrap();
    open_yaml_menu(cx, handle, 2);
    let selection = pane.read_with(cx, |pane, _| pane.selection.unwrap());
    assert_eq!((selection.anchor, selection.cursor), (3, 1));
    cx.update_window(handle, |_, window, cx| {
        let menu = window.within("popup-menu");
        assert_eq!(menu.find(0usize).label(), Some("Copy"));
        assert_eq!(menu.find(1usize).label(), Some("Select all"));
        assert_eq!(menu.find(2usize).label(), None);
        assert_eq!(menu.find(3usize).label(), Some("Find"));
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(pane.read(cx).focus.is_focused(window));
        window.press("secondary-c", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            lines[1..4].join("\n")
        );
    })
    .unwrap();
    open_yaml_menu(cx, handle, 2);
    pick_yaml_menu(cx, handle, 0);
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().unwrap(),
        lines[1..4].join("\n")
    );
    open_yaml_menu(cx, handle, 5);
    let selection = pane.read_with(cx, |pane, _| pane.selection.unwrap());
    assert_eq!((selection.anchor, selection.cursor), (5, 5));
    pick_yaml_menu(cx, handle, 0);
    assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), lines[5]);
    open_yaml_menu(cx, handle, 5);
    pick_yaml_menu(cx, handle, 1);
    assert_eq!(
        pane.read_with(cx, |pane, _| pane.selection.unwrap().range()),
        0..lines.len()
    );
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-c", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            lines.join("\n")
        );
        if !node {
            window.click("detail-copy-yaml", cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), yaml);
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn yaml_menu_keeps_the_range_inside_and_selects_one_line_outside(cx: &mut TestAppContext) {
    yaml_menu_selection(cx, false);
}

#[gpui_kit::test]
fn node_yaml_menu_keeps_the_range_inside_and_selects_one_line_outside(cx: &mut TestAppContext) {
    yaml_menu_selection(cx, true);
}

fn yaml_menu_find(cx: &mut TestAppContext, node: bool) {
    let (_runtime, pane, handle, _) = mount(cx);
    yaml_menu_fixture(cx, &pane, handle, node);
    open_yaml_menu(cx, handle, 1);
    let hints = match freshkube_ui::platform::Platform::current() {
        freshkube_ui::platform::Platform::MacOs => ["kbd:cmd-c", "kbd:cmd-a", "kbd:cmd-f"],
        _ => ["kbd:ctrl-c", "kbd:ctrl-a", "kbd:ctrl-f"],
    };
    for (item, hint) in [0usize, 1, 3].into_iter().zip(hints) {
        let bounds = gpui_kit::VisualTestContext::from_window(handle, cx)
            .debug_bounds(hint)
            .expect("the menu draws the document's platform binding");
        cx.update_window(handle, |_, window, _| {
            assert!(
                window
                    .within("popup-menu")
                    .find(item)
                    .bounds()
                    .contains(&bounds.center())
            );
        })
        .unwrap();
    }
    pick_yaml_menu(cx, handle, 3);
    cx.update_window(handle, |_, window, cx| {
        assert!(window.try_find("popup-menu").is_none());
        assert!(
            pane.read(cx)
                .find
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        window.input("metadata", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(pane.read(cx).query, "metadata");
        assert!(!pane.read(cx).matches.is_empty());
        window.press("escape", cx);
        assert!(pane.read(cx).query.is_empty());
        window.press("escape", cx);
        assert!(pane.read(cx).focus.is_focused(window));
        window.press("secondary-a", cx);
        let pane = pane.read(cx);
        assert_eq!(
            pane.selection.unwrap().range(),
            0..pane.view().unwrap().lines.len()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn yaml_menu_find_keeps_search_focused_and_escape_returns(cx: &mut TestAppContext) {
    yaml_menu_find(cx, false);
}

#[gpui_kit::test]
fn node_yaml_menu_find_keeps_search_focused_and_escape_returns(cx: &mut TestAppContext) {
    yaml_menu_find(cx, true);
}

fn yaml_placeholder_menu(cx: &mut TestAppContext, node: bool) {
    let (_runtime, pane, handle, _) = mount(cx);
    yaml_menu_fixture(cx, &pane, handle, node);
    for state in [
        DocumentRead::Loading,
        DocumentRead::Failed("Example read failed".into()),
        DocumentRead::Refused("Example read refused".into()),
        DocumentRead::Deleted,
    ] {
        cx.update_window(handle, |_, window, cx| {
            pane.update(cx, |pane, cx| {
                let detail = pane.detail.as_mut().unwrap();
                detail.view = None;
                detail.read = state;
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.try_find("detail-yaml").is_none());
            window.right_click("resource-detail", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("popup-menu").is_none());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn yaml_placeholders_open_no_document_menu(cx: &mut TestAppContext) {
    yaml_placeholder_menu(cx, false);
}

#[gpui_kit::test]
fn node_yaml_placeholders_open_no_document_menu(cx: &mut TestAppContext) {
    yaml_placeholder_menu(cx, true);
}

fn stale_yaml_menu(cx: &mut TestAppContext, node: bool, change: &str) {
    let (_runtime, pane, handle, _) = mount(cx);
    for item in [0usize, 1, 3] {
        yaml_menu_fixture(cx, &pane, handle, node);
        open_yaml_menu(cx, handle, 1);
        let selected = cx
            .update_window(handle, |_, window, cx| {
                let position = window.within("popup-menu").find(item).bounds().center();
                window.dispatch_event(
                    gpui_kit::MouseDownEvent {
                        button: gpui_kit::MouseButton::Left,
                        position,
                        click_count: 1,
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("unchanged".into()));
                pane.update(cx, |pane, cx| {
                    match change {
                        "document" => {
                            let (replacement, _) = target("deployments.apps", |_, _| true);
                            pane.open(replacement, ClusterAccess::EXAMPLE, "1", Duration::ZERO, cx);
                            pane.select_line(1, false, cx);
                        }
                        "refresh" => {
                            let detail = pane.detail.as_mut().unwrap();
                            // Even identical content in a freshly read document
                            // invalidates the old menu's subject.
                            detail.view = Some(std::sync::Arc::new(example_view(&detail.target)));
                        }
                        "selection" => pane.select_line(3, true, cx),
                        _ => unreachable!(),
                    }
                });
                let selected = pane.read(cx).selection;
                window.dispatch_event(
                    gpui_kit::MouseUpEvent {
                        button: gpui_kit::MouseButton::Left,
                        position,
                        click_count: 1,
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
                assert_eq!(pane.read(cx).selection, selected);
                assert_eq!(
                    cx.read_from_clipboard().unwrap().text().unwrap(),
                    "unchanged"
                );
                selected
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(pane.read(cx).selection, selected);
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                "unchanged"
            );
            assert!(
                !pane
                    .read(cx)
                    .find
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_new_document_makes_the_yaml_menu_inert(cx: &mut TestAppContext) {
    stale_yaml_menu(cx, false, "document");
}

#[gpui_kit::test]
fn a_new_document_makes_the_node_yaml_menu_inert(cx: &mut TestAppContext) {
    stale_yaml_menu(cx, true, "document");
}

#[gpui_kit::test]
fn a_refresh_makes_the_yaml_menu_inert(cx: &mut TestAppContext) {
    stale_yaml_menu(cx, false, "refresh");
}

#[gpui_kit::test]
fn a_refresh_makes_the_node_yaml_menu_inert(cx: &mut TestAppContext) {
    stale_yaml_menu(cx, true, "refresh");
}

#[gpui_kit::test]
fn a_changed_selection_makes_the_yaml_menu_inert(cx: &mut TestAppContext) {
    stale_yaml_menu(cx, false, "selection");
}

#[gpui_kit::test]
fn a_changed_selection_makes_the_node_yaml_menu_inert(cx: &mut TestAppContext) {
    stale_yaml_menu(cx, true, "selection");
}
