use super::*;
use crate::DebugOptions;

/// Enter on a node, a click on a pane tab, or a click in the pane moves
/// the keyboard into the node's pane, as Enter does on Resources (#340).
/// Escape then hands it back to the table with the pane open, and a second
/// Escape closes the pane.
#[gpui_kit::test]
fn enter_and_pane_clicks_move_the_keyboard_into_the_node_pane(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    let in_pane = |pilot: &gpui_kit::Entity<crate::desktop::Pilot>,
                   window: &gpui_kit::Window,
                   cx: &gpui_kit::App| {
        pilot
            .read(cx)
            .node_workspace
            .read(cx)
            .pane_focus
            .contains_focused(window, cx)
    };
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| {
            window.focus(&pilot.node_workspace.read(cx).focus.clone(), cx)
        });
        window.press("down", cx);
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.read(cx).open);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.read(cx).open);
        assert_eq!(
            pilot.read(cx).node_workspace.read(cx).tab,
            NodeTab::Overview
        );
        assert!(in_pane(&pilot, window, cx));
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.read(cx).open);
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .read(cx)
                .focus
                .is_focused(window)
        );
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.read(cx).open);

        // A click on a row opens the pane and leaves the keyboard on the
        // table; a click on a tab moves it into the tab's list.
        crate::desktop::tests::expand_healthy_nodes(window, cx);
        window.click("node-talos-cp-fra1-01", cx);
        window.render_frame(cx);
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .read(cx)
                .focus
                .is_focused(window)
        );
        window.click("node-tab-pods", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.read(cx).tab, NodeTab::Pods);
        assert!(in_pane(&pilot, window, cx));

        // A click on the pane's content takes it from the table too.
        window.click("node-tab-overview", cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| {
            window.focus(&pilot.node_workspace.read(cx).focus.clone(), cx)
        });
        window.click("node-pane", cx);
        window.render_frame(cx);
        assert!(in_pane(&pilot, window, cx));

        // From the filter, a click on a tab whose screen takes no keys
        // moves the keyboard to the tab strip, not leaving it in the filter.
        let query = pilot.read(cx).node_workspace.read(cx).query.clone();
        window.focus(&query.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
        window.click("node-tab-overview", cx);
        window.render_frame(cx);
        assert!(in_pane(&pilot, window, cx));
        assert!(!query.read(cx).focus_handle(cx).is_focused(window));
        // The strip's mouse-down took it there; asked straight from the
        // filter, the pane takes it too, rather than leaving it in the
        // filter.
        window.focus(&query.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| pilot.focus_node_pane(window, cx));
        window.render_frame(cx);
        assert!(in_pane(&pilot, window, cx));
    })
    .unwrap();
}

/// Leaving a tab whose list or log holds the keyboard for one whose screen
/// takes no keys gives the keyboard to the tab strip, not to the previous
/// tab's handle, which isn't drawn any more; Escape then still reaches the
/// pane and hands the keyboard to the table (#343 review). A headless click
/// on a tab focuses the strip on its mouse-down first, so the switch is
/// asked straight, as More's items and Overview's links ask it.
#[gpui_kit::test]
fn a_tab_without_keys_takes_the_keyboard_from_the_previous_tab(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        crate::desktop::tests::expand_healthy_nodes(window, cx);
        window.click("node-talos-cp-fra1-01", cx);
        window.render_frame(cx);
    })
    .unwrap();
    for from in [NodeTab::Pods, NodeTab::Logs] {
        for to in [NodeTab::Overview, NodeTab::Services] {
            cx.update_window(handle, |_, window, cx| {
                pilot.update(cx, |pilot, cx| {
                    pilot.show_node_tab(from, window, cx);
                    pilot.focus_node_pane(window, cx);
                });
                window.render_frame(cx);
                let tab_strip = pilot.read(cx).node_workspace.read(cx).tab_focus.clone();
                let pane = pilot.read(cx).node_workspace.read(cx).pane_focus.clone();
                assert!(pane.contains_focused(window, cx), "{from:?}");
                assert!(!tab_strip.is_focused(window), "{from:?}");
                pilot.update(cx, |pilot, cx| {
                    pilot.show_node_tab(to, window, cx);
                    pilot.focus_node_pane(window, cx);
                });
                window.render_frame(cx);
                assert_eq!(pilot.read(cx).node_workspace.read(cx).tab, to);
                assert!(tab_strip.is_focused(window), "{from:?} → {to:?}");
                window.press("escape", cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(
                    pilot.read(cx).node_workspace.read(cx).open,
                    "{from:?} → {to:?}"
                );
                assert!(
                    pilot
                        .read(cx)
                        .node_workspace
                        .read(cx)
                        .focus
                        .is_focused(window),
                    "{from:?} → {to:?}"
                );
            })
            .unwrap();
        }
    }
}

/// With the open node hidden by the filter, Down and Up move the pane to the
/// first and last rows the table shows (#339's selection rule).
#[gpui_kit::test]
fn arrows_from_a_hidden_open_node_move_to_the_visible_rows(cx: &mut TestAppContext) {
    use freshkube_ui::table::{Line, TableSource};
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    let hidden = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("nav-nodes", cx);
            window.render_frame(cx);
            crate::desktop::tests::expand_healthy_nodes(window, cx);
            window.click("node-talos-cp-fra1-01", cx);
            window.render_frame(cx);
            pilot
                .read(cx)
                .node_workspace
                .read(cx)
                .selected
                .clone()
                .unwrap()
        })
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        let query = pilot.read(cx).node_workspace.read(cx).query.clone();
        window.focus(&query.read(cx).focus_handle(cx), cx);
        window.input("wk", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let visible: Vec<_> = {
            let pilot = pilot.read(cx).node_workspace.read(cx);
            (0..pilot.line_count())
                .filter_map(|line| match pilot.line(line, cx)? {
                    Line::Row(row) => Some(row.key),
                    Line::Group(_) => None,
                })
                .collect()
        };
        assert!(
            visible.len() > 1 && !visible.contains(&hidden),
            "{visible:?}"
        );
        for (key, first) in [("down", true), ("up", false)] {
            pilot.update(cx, |pilot, cx| {
                pilot.open_node(hidden.clone(), window, cx);
                window.focus(&pilot.node_workspace.read(cx).focus.clone(), cx);
            });
            window.render_frame(cx);
            assert_eq!(
                pilot.read(cx).node_workspace.read(cx).selected.as_ref(),
                Some(&hidden)
            );
            window.press(key, cx);
            window.render_frame(cx);
            let expected = if first {
                visible.first()
            } else {
                visible.last()
            };
            let nodes = &pilot.read(cx).node_workspace.read(cx);
            assert!(nodes.open, "{key}");
            assert_eq!(nodes.selected.as_ref(), expected, "{key}");
        }
    })
    .unwrap();
}

/// The Processes find in the node pane (#347): Escape clears it, then
/// hands the keyboard to the list, then steps back to the table with the
/// pane left open.
#[gpui_kit::test]
fn escape_steps_back_from_the_processes_find_in_the_node_pane(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        crate::desktop::tests::open_node_tab(window, cx, NodeTab::Processes);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| {
            pilot.active_screen(cx).unwrap().focus(window, cx)
        });
        window.press("/", cx);
        for key in ["e", "t", "c", "d"] {
            window.press(key, cx);
        }
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    // Init, PID 1, is listed unless the find hides it.
    let init_shown = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        window.render_frame(cx);
        window.try_find(("process", 1usize)).is_some()
    };
    let escape = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| window.press("escape", cx))
            .unwrap();
        cx.run_until_parked();
    };
    cx.update_window(handle, |_, window, cx| {
        assert!(!init_shown(window, cx), "etcd hides init");
    })
    .unwrap();
    escape(cx);
    cx.update_window(handle, |_, window, cx| {
        assert!(init_shown(window, cx), "the find is clear");
        let nodes = &pilot.read(cx).node_workspace.read(cx);
        assert!(nodes.open && nodes.tab == NodeTab::Processes);
        assert!(
            !pilot
                .read(cx)
                .node_workspace
                .read(cx)
                .focus
                .is_focused(window)
        );
    })
    .unwrap();
    escape(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .read(cx)
                .pane_focus
                .contains_focused(window, cx)
        );
        assert!(
            !pilot
                .read(cx)
                .node_workspace
                .read(cx)
                .focus
                .is_focused(window)
        );
    })
    .unwrap();
    escape(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.read(cx).open);
        assert_eq!(
            pilot.read(cx).node_workspace.read(cx).tab,
            NodeTab::Processes
        );
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .read(cx)
                .focus
                .is_focused(window)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn node_keys_expand_switch_tabs_and_step_back(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        crate::desktop::tests::expand_healthy_nodes(window, cx);
        window.click("node-talos-cp-fra1-01", cx);
        window.render_frame(cx);
        window.press("secondary-shift-enter", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.read(cx).expanded);
        window.press("secondary-shift-enter", cx);
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.read(cx).expanded);
        window.click("node-tab-processes", cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| {
            window.focus(&pilot.node_workspace.read(cx).tab_focus.clone(), cx)
        });
        window.press("right", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.read(cx).tab, NodeTab::Storage);
        window.press("secondary-{", cx);
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).node_workspace.read(cx).tab,
            NodeTab::Processes
        );
        pilot.update(cx, |pilot, cx| {
            pilot.active_screen(cx).unwrap().focus(window, cx)
        });
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        // Escape steps back as on Resources (#334): the pane hands the
        // keyboard to the table and stays open on its tab.
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace.read(cx);
        assert!(nodes.open);
        assert_eq!(nodes.tab, NodeTab::Processes);
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .read(cx)
                .focus
                .is_focused(window)
        );
        // Expanded from the table, the pane first gives the table its room
        // back.
        window.press("secondary-shift-enter", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.read(cx).expanded);
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .read(cx)
                .focus
                .is_focused(window)
        );
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace.read(cx);
        assert!(nodes.open && !nodes.expanded);
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .read(cx)
                .focus
                .is_focused(window)
        );
        window.focus(
            &pilot
                .read(cx)
                .node_workspace
                .read(cx)
                .query
                .read(cx)
                .focus_handle(cx),
            cx,
        );
        window.input("fra1", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.read(cx).query_text, "fra1");
        pilot.update(cx, |pilot, cx| {
            window.focus(&pilot.node_workspace.read(cx).focus.clone(), cx)
        });
        // The table clears its filter, then closes the pane.
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.read(cx).query_text.is_empty());
        assert!(pilot.read(cx).node_workspace.read(cx).open);
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.read(cx).open);
    })
    .unwrap();
}

/// Escape steps back from each kind of tab as on Resources (#334): the
/// tab's own levels first, then the keyboard to the table with the pane
/// left open, then the pane closes, keeping its node selected.
#[gpui_kit::test]
fn escape_steps_back_from_the_pods_yaml_and_logs_tabs(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    let press = |cx: &mut TestAppContext, key: &str| {
        cx.update_window(handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let pilot = pilot.read(cx);
            (
                pilot.node_workspace.read(cx).open,
                pilot.node_workspace.read(cx).focus.is_focused(window),
            )
        })
        .unwrap()
    };
    let show = |cx: &mut TestAppContext, tab: NodeTab| {
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| pilot.show_node_tab(tab, window, cx));
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let key = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("nav-nodes", cx);
            window.render_frame(cx);
            crate::desktop::tests::expand_healthy_nodes(window, cx);
            window.click("node-talos-cp-fra1-01", cx);
            window.render_frame(cx);
            pilot
                .read(cx)
                .node_workspace
                .read(cx)
                .selected
                .clone()
                .unwrap()
        })
        .unwrap();

    // Pods: the filter clears, then leaves; the list hands the keyboard
    // to the table.
    show(cx, NodeTab::Pods);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot
                .node_workspace
                .read(cx)
                .pods
                .clone()
                .update(cx, |pods, cx| pods.focus(window, cx))
        });
        window.press("/", cx);
        window.input("zz", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(press(cx, "escape"), (true, false), "the filter clears");
    assert_eq!(press(cx, "escape"), (true, false), "the filter leaves");
    assert_eq!(press(cx, "escape"), (true, true), "the list steps back");
    assert_eq!(
        pilot.read_with(cx, |p, cx| p.node_workspace.read(cx).tab),
        NodeTab::Pods
    );

    // YAML: find clears, then leaves; the document hands the keyboard to
    // the table.
    show(cx, NodeTab::Yaml);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot
                .node_workspace
                .read(cx)
                .document
                .clone()
                .update(cx, |pane, cx| pane.focus(window, cx))
        });
        window.press("secondary-f", cx);
        window.input("kind", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(press(cx, "escape"), (true, false), "find clears");
    assert_eq!(press(cx, "escape"), (true, false), "find leaves");
    assert_eq!(press(cx, "escape"), (true, true), "the document steps back");
    assert_eq!(
        pilot.read_with(cx, |p, cx| p.node_workspace.read(cx).tab),
        NodeTab::Yaml
    );

    // Logs: the log hands the keyboard to the table.
    show(cx, NodeTab::Logs);
    assert!(
        !cx.update_window(handle, |_, window, cx| {
            pilot
                .read(cx)
                .node_workspace
                .read(cx)
                .focus
                .is_focused(window)
        })
        .unwrap()
    );
    assert_eq!(press(cx, "escape"), (true, true), "the log steps back");

    // The table closes the pane and keeps the node selected.
    assert_eq!(press(cx, "escape"), (false, true));
    pilot.read_with(cx, |pilot, cx| {
        assert_eq!(pilot.node_workspace.read(cx).selected.as_ref(), Some(&key));
        assert_eq!(pilot.selected_node.as_deref(), Some("talos-cp-fra1-01"));
    });
}

#[gpui_kit::test]
fn filtered_table_keys_select_visible_nodes_and_keep_the_pane_tabs(cx: &mut TestAppContext) {
    use freshkube_ui::table::TableSource;
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let target = pilot.read(cx).selected_node.clone();
        window.click("nodes-tally-failing", cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| {
            window.focus(&pilot.node_workspace.read(cx).focus.clone(), cx)
        });
        window.press("down", cx);
        window.render_frame(cx);
        let row = pilot.read(cx).node_workspace.read(cx).row().unwrap();
        assert_eq!(row.ready, "NotReady");
        assert_eq!(pilot.read(cx).selected_node, target);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.read(cx).open);
        // The table beside the pane keeps its filter (#339): the failing
        // group's heading and its two rows.
        assert_eq!(pilot.read(cx).node_workspace.read(cx).line_count(), 3);
        assert_eq!(pilot.read(cx).node_workspace.read(cx).lines.len(), 2);
        assert_eq!(
            pilot.read(cx).selected_node.as_deref(),
            Some("talos-wk-fra1-02")
        );
        window.click("node-tab-processes", cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| pilot.step_joined_node(1, window, cx));
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).node_workspace.read(cx).tab,
            NodeTab::Processes
        );
        assert_eq!(
            pilot.read(cx).selected_node.as_deref(),
            Some("talos-wk-fra1-03")
        );
        window.click("node-close", cx);
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).node_workspace.read(cx).filter,
            Some(super::super::projection::Status::Failing)
        );
        assert_eq!(pilot.read(cx).node_workspace.read(cx).lines.len(), 2);
        assert!(window.find("node-talos-wk-fra1-03").visible());
    })
    .unwrap();
}

/// Enter in a node log's search steps to the next match and Shift-Enter
/// to the previous one, with the keyboard kept in the search (#316): the
/// node workspace's own Enter doesn't take it.
#[gpui_kit::test]
fn enter_in_a_node_logs_search_steps_through_its_matches(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.startup_selection(&DebugOptions::default().with_page("node-logs"), window, cx);
        });
        for _ in 0..3 {
            window.render_frame(cx);
        }
        window.click("logs-search", cx);
        window.input("cilium", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let mut seen = Vec::new();
    for key in ["enter", "enter", "shift-enter"] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let current = pilot.read(cx).logs.read(cx).current_match();
            assert_eq!(
                window.find("logs-search").focused(),
                Some(true),
                "{key} keeps the keyboard in the search"
            );
            seen.push(current.expect("a current match"));
        })
        .unwrap();
    }
    // Enter moves on, and Shift-Enter comes back.
    assert_ne!(seen[0], seen[1], "{seen:?}");
    assert_eq!(seen[2], seen[0], "{seen:?}");
}

#[gpui_kit::test]
fn enter_opens_a_node_from_the_list_and_presses_a_focused_button(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        // On the list, Enter opens the selected node.
        pilot.update(cx, |pilot, cx| {
            window.focus(&pilot.node_workspace.read(cx).focus.clone(), cx)
        });
        window.press("down", cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.read(cx).open);
        let node = pilot.read(cx).selected_node.clone();
        assert!(node.is_some());
        pilot.update(cx, |pilot, cx| {
            pilot.show_node_tab(NodeTab::Logs, window, cx)
        });
        window.render_frame(cx);
        // On a focused button in the inspector, Enter presses the button.
        window.click("logs-search", cx);
        window.press("tab", cx);
        window.render_frame(cx);
        assert_eq!(window.find("logs-wrap").focused(), Some(true));
        let wrapped = window.find("logs-wrap").checked();
        assert!(wrapped.is_some());
        window.press("enter", cx);
        window.render_frame(cx);
        assert_ne!(window.find("logs-wrap").checked(), wrapped);
        assert_eq!(window.find("logs-wrap").focused(), Some(true));
        assert_eq!(pilot.read(cx).selected_node, node);
        assert!(pilot.read(cx).node_workspace.read(cx).open);
    })
    .unwrap();
}

/// Enter in the filter hands the keyboard to the list without opening a
/// node, as the Resources filter does; Enter on the list then opens one.
#[gpui_kit::test]
fn enter_in_the_filter_moves_to_the_list_and_opens_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        window.click("nodes-filter", cx);
        window.input("wk", cx);
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .read(cx)
                .focus
                .is_focused(window)
        );
        assert!(!pilot.read(cx).node_workspace.read(cx).open);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.read(cx).open);
        let node = pilot.read(cx).selected_node.clone().unwrap_or_default();
        assert!(node.contains("-wk-"), "{node} isn't a filtered node");
    })
    .unwrap();
}

#[gpui_kit::test]
fn h_folds_the_healthy_nodes_from_the_list_and_the_filter_types_it(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.read(cx).healthy_collapsed());
        pilot.update(cx, |pilot, cx| {
            window.focus(&pilot.node_workspace.read(cx).focus.clone(), cx)
        });
        window.press("h", cx);
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.read(cx).healthy_collapsed());
        assert!(window.find("node-talos-cp-fra1-01").visible());
        window.press("h", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.read(cx).healthy_collapsed());

        window.click("nodes-filter", cx);
        window.render_frame(cx);
        window.press("h", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(pilot.read(cx).node_workspace.read(cx).query_text, "h"));
}

#[gpui_kit::test]
fn h_leaves_the_fold_alone_on_the_cards(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).node_workspace.read(cx).view,
            super::super::NodeView::Cards
        );
        assert!(pilot.read(cx).node_workspace.read(cx).healthy_collapsed());
        pilot.update(cx, |pilot, cx| {
            window.focus(&pilot.node_workspace.read(cx).focus.clone(), cx)
        });
        window.press("h", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.read(cx).healthy_collapsed());
    })
    .unwrap();
}
