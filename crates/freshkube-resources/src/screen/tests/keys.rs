use super::*;

#[gpui_kit::test]
fn escape_in_the_filter_clears_it_then_returns_to_the_list(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    step(cx, &|window, cx| {
        screen.update(cx, |screen, cx| screen.focus(window, cx));
    });
    // Command-F reaches the filter as the slash does.
    step(cx, &|window, cx| window.press("secondary-f", cx));
    step(cx, &|window, cx| window.input("coredns", cx));
    step(cx, &|window, cx| {
        assert_eq!(screen.read(cx).projection.len(), 2);
        window.press("escape", cx);
    });
    step(cx, &|window, cx| {
        // The first Escape clears the text and stays in the filter.
        assert_eq!(screen.read(cx).projection.len(), 22);
        assert!(screen.read(cx).query.read(cx).value().is_empty());
        assert_eq!(window.find("resource-body").focused(), Some(false));
        window.press("escape", cx);
    });
    step(cx, &|window, cx| {
        // The second hands the keyboard back to the list.
        assert_eq!(window.find("resource-body").focused(), Some(true));
        window.press("down", cx);
        assert_eq!(selected(&screen, cx), Some(identity_at(&screen, 0, cx)));
    });
}

#[gpui_kit::test]
fn the_namespace_picker_hands_the_keyboard_back_to_the_list(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    // Escape closes the menu without a choice.
    step(cx, &|window, cx| window.click("resource-namespace", cx));
    step(cx, &|window, cx| {
        assert_eq!(window.find("resource-body").focused(), Some(false));
        window.press("escape", cx);
    });
    step(cx, &|window, cx| {
        assert_eq!(window.find("resource-body").focused(), Some(true));
        assert_eq!(screen.read(cx).namespace, None);
    });
    // N on the list opens it too, and a choice applies; the arrows then
    // move through the list again.
    step(cx, &|window, cx| {
        screen.update(cx, |screen, cx| screen.focus(window, cx));
    });
    step(cx, &|window, cx| window.press("n", cx));
    // Past `argocd`, which holds no pods, to `batch`.
    step(cx, &|window, cx| window.press("down", cx));
    step(cx, &|window, cx| window.press("down", cx));
    step(cx, &|window, cx| window.press("enter", cx));
    step(cx, &|window, cx| {
        let chosen = screen.read(cx).namespace.clone();
        assert!(chosen.is_some());
        assert_eq!(window.find("resource-body").focused(), Some(true));
        window.press("down", cx);
        let first = identity_at(&screen, 0, cx);
        assert_eq!(Some(first.namespace.clone()), chosen);
        assert_eq!(selected(&screen, cx), Some(first));
    });
}

#[gpui_kit::test]
fn the_page_keeps_its_keys_while_no_rows_show(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("configmaps"), window, cx);
            screen.focus(window, cx);
        });
        window.render_frame(cx);
        assert!(window.find("resource-not-in-example").visible());
        assert_eq!(window.find("resource-body").focused(), Some(true));
        // The list's keys still answer: slash goes to the filter.
        window.press("/", cx);
        window.render_frame(cx);
        assert_eq!(window.find("resource-body").focused(), Some(false));
    })
    .unwrap();
}

#[gpui_kit::test]
fn enter_opens_a_row_at_once_and_escape_steps_back_one_level(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let focused = |window: &mut gpui_kit::Window, id: &'static str| window.find(id).focused();
    step(cx, &|window, cx| {
        screen.update(cx, |screen, cx| screen.focus(window, cx));
    });
    step(cx, &|window, cx| {
        // Enter on the list with nothing selected leaves it be.
        window.press("enter", cx);
        assert_eq!(shown(&screen, cx), None);
        window.press("down", cx);
        window.press("enter", cx);
    });
    step(cx, &|window, cx| {
        // Read at once, without waiting for the keys to pause, and the
        // keyboard is on the pane.
        let first = identity_at(&screen, 0, cx);
        assert_eq!(shown(&screen, cx), Some(first));
        assert!(window.try_find("detail-state").is_none());
        assert_eq!(focused(window, "resource-detail"), Some(true));
        window.press("secondary-alt-]", cx);
    });
    step(cx, &|window, cx| {
        assert_eq!(window.find("detail-tab-yaml").selected(), Some(true));
        assert_eq!(focused(window, "resource-detail"), Some(true));
        // Escape in the pane steps back to the list; the pane stays.
        window.press("escape", cx);
    });
    step(cx, &|window, cx| {
        assert_eq!(focused(window, "resource-body"), Some(true));
        assert!(shown(&screen, cx).is_some());
        // From the list, the brackets switch the tab and keep the keyboard
        // on the list. The two tabs wrap round.
        window.press("secondary-alt-]", cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-tab-details").selected(), Some(true));
        window.press("secondary-alt-[", cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-tab-yaml").selected(), Some(true));
        assert_eq!(focused(window, "resource-body"), Some(true));
        // Enter on the open row hands the keyboard to the pane.
        window.press("enter", cx);
        window.render_frame(cx);
        assert_eq!(focused(window, "resource-detail"), Some(true));
        // Escape in the pane steps back to the list.
        window.press("escape", cx);
    });
    step(cx, &|window, cx| {
        assert_eq!(focused(window, "resource-body"), Some(true));
        assert!(shown(&screen, cx).is_some());
        // Escape in the list, with no filter, closes the pane.
        window.press("escape", cx);
    });
    step(cx, &|window, cx| {
        assert!(window.try_find("resource-detail").is_none());
        assert_eq!(shown(&screen, cx), None);
        assert_eq!(focused(window, "resource-body"), Some(true));
    });
}

#[gpui_kit::test]
fn n_does_nothing_for_a_kind_without_namespaces(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("nodes"), window, cx);
            screen.focus(window, cx);
        });
        window.render_frame(cx);
        assert!(window.try_find("resource-namespace").is_none());
        window.press("n", cx);
        window.render_frame(cx);
        assert_eq!(window.find("resource-body").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn x_marks_rows_and_a_group_selects_all_of_its_own(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let first = identity_at(&screen, 0, cx);
        window.within(row_id(&first)).click("name", cx);
        window.press("x", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).marked.contains(&first));
        assert!(
            window
                .within("resource-footer")
                .find("resource-marks")
                .visible()
        );
        window.press("x", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).marked.is_empty());
        assert!(window.try_find("resource-marks").is_none());

        // Shift-X marks the selected row's group: here each has one pod.
        problems(&screen, cx);
        window.render_frame(cx);
        let first = identity_at(&screen, 0, cx);
        window.within(row_id(&first)).click("name", cx);
        window.press("shift-x", cx);
        window.press("down", cx);
        window.press("shift-x", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).marked.len(), 2);
        window.click("resource-marks-copy", cx);
    })
    .unwrap();
    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    assert_eq!(copied.lines().count(), 2);
    assert!(copied.lines().all(|line| line.contains('/')), "{copied}");
    cx.update_window(handle, |_, window, cx| {
        window.click("resource-marks-clear", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).marked.is_empty());
        // Listing again starts without marks.
        screen.update(cx, |screen, cx| {
            screen.marked.insert(identity_at_now(screen));
            screen.refresh(window, cx);
        });
        assert!(screen.read(cx).marked.is_empty());
    })
    .unwrap();
}

fn identity_at_now(screen: &ResourcesScreen) -> ResourceIdentity {
    screen
        .projection
        .row(&screen.store, 0)
        .unwrap()
        .identity
        .clone()
}

#[gpui_kit::test]
fn l_asks_for_the_selected_pods_logs(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let asked = Rc::new(RefCell::new(Vec::new()));
    let sink = asked.clone();
    cx.update(|cx| {
        cx.subscribe(&screen, move |_, event: &crate::ResourceLink, _| {
            if let crate::ResourceLink::Logs(request) = event {
                sink.borrow_mut().push(request.target.identity.clone());
            }
        })
        .detach()
    });
    let first = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let first = identity_at(&screen, 0, cx);
            window.within(row_id(&first)).click("name", cx);
            window.render_frame(cx);
            window.press("l", cx);
            window.render_frame(cx);
            // The drawer steps aside for the dock and the row stays
            // selected, so Enter opens it again.
            assert_eq!(shown(&screen, cx), None);
            assert!(window.try_find("resource-drawer").is_none());
            assert_eq!(screen.read(cx).selected_row(), Some(&first));
            first
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(*asked.borrow(), [first]);
}

/// Each pod row has a Logs button after its name, named for its pod, that
/// opens that pod's logs without selecting the row or opening the drawer
/// (#291).
#[gpui_kit::test]
fn a_rows_logs_button_asks_for_its_pods_logs(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let asked = Rc::new(RefCell::new(Vec::new()));
    let sink = asked.clone();
    cx.update(|cx| {
        cx.subscribe(&screen, move |_, event: &crate::ResourceLink, _| {
            if let crate::ResourceLink::Logs(request) = event {
                sink.borrow_mut().push(request.target.identity.clone());
            }
        })
        .detach()
    });
    let second = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let button = |identity: &ResourceIdentity| {
                gpui_kit::SharedString::from(format!("pod-row-logs-{}", identity.uid))
            };
            // Every pod row has one, inside the row and named for its pod.
            for line in 0..3 {
                let identity = identity_at(&screen, line, cx);
                let row = window.find(row_id(&identity)).bounds();
                let found = window.find(button(&identity));
                assert!(row.contains(&found.bounds().center()), "{row:?}");
                assert_eq!(
                    found.label(),
                    Some(format!("Logs for {}", identity.name).as_str())
                );
            }
            let second = identity_at(&screen, 1, cx);
            window.click(button(&second), cx);
            window.render_frame(cx);
            assert_eq!(shown(&screen, cx), None);
            assert!(window.try_find("resource-drawer").is_none());
            assert_eq!(screen.read(cx).selected_row(), None);
            second
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(*asked.borrow(), [second]);
}

#[gpui_kit::test]
fn h_folds_the_healthy_pods_and_o_opens_the_selected_pods_node(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let links = Rc::new(RefCell::new(Vec::new()));
    let sink = links.clone();
    cx.update(|cx| {
        cx.subscribe(&screen, move |_, link: &crate::ResourceLink, _| {
            if let crate::ResourceLink::Node(node, _) = link {
                sink.borrow_mut().push(node.clone());
            }
        })
        .detach()
    });
    cx.update_window(handle, |_, window, cx| {
        problems(&screen, cx);
        window.render_frame(cx);
        let folded = screen.read(cx).projection.len();
        let first = identity_at(&screen, 0, cx);
        window.within(row_id(&first)).click("name", cx);
        window.press("h", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).projection.len() > folded);
        window.press("h", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).projection.len(), folded);
        window.press("o", cx);
    })
    .unwrap();
    let node = cx.read(|cx| screen.read(cx).selected_node());
    assert!(node.is_some());
    assert_eq!(links.borrow().as_slice(), [node.unwrap()]);
}

#[gpui_kit::test]
fn the_filter_types_the_lists_keys(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        problems(&screen, cx);
        window.render_frame(cx);
        window.click("resource-filter", cx);
        window.render_frame(cx);
        for key in ["o", "h", "x", "shift-x"] {
            window.press(key, cx);
        }
        window.render_frame(cx);
        assert_eq!(screen.read(cx).query.read(cx).value(), "ohxX");
        assert!(screen.read(cx).marked.is_empty());
    })
    .unwrap();
}
