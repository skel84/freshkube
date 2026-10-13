use super::*;

#[gpui_kit::test]
fn header_chips_filter_health_without_changing_the_talos_target(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let target = pilot.read(cx).selected_node.clone();
        let nodes = &pilot.read(cx).node_workspace;
        let not_ready = nodes
            .rows
            .iter()
            .filter(|row| row.ready == "NotReady")
            .count();
        let ready = nodes
            .rows
            .iter()
            .filter(|row| row.tone == Tone::Good)
            .count();
        let unknown = nodes
            .rows
            .iter()
            .filter(|row| row.tone == Tone::Unknown)
            .count();
        assert_eq!(
            window.find("nodes-tally-failing").label(),
            Some(format!("{not_ready} failing nodes").as_str())
        );
        assert_eq!(
            window.find("nodes-tally-healthy").label(),
            Some(format!("{ready} healthy nodes").as_str())
        );
        assert_eq!(
            window.find("nodes-tally-unknown").label(),
            Some(format!("{unknown} unknown nodes").as_str())
        );
        window.click("nodes-tally-failing", cx);
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace;
        assert_eq!(nodes.lines.len(), not_ready);
        assert!(
            nodes
                .lines
                .iter()
                .all(|ix| nodes.rows[*ix].ready == "NotReady")
        );
        assert_eq!(pilot.read(cx).selected_node, target);
        assert!(!nodes.open);
        window.click("nodes-tally-failing", cx);
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace;
        assert_eq!(nodes.lines.len(), nodes.rows.len());
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).node_workspace.view,
            super::super::NodeView::Cards
        );
        assert!(window.find("nodes-title").visible());
        assert_eq!(pilot.read(cx).selected_node, target);
    })
    .unwrap();
}

/// With a node's pane open, the table follows the filter as it does
/// without one, and its tallies count the rows it shows (#339). A filter
/// that hides the open node keeps its pane and its selection.
#[gpui_kit::test]
fn the_filter_narrows_the_table_beside_an_open_pane(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    let type_filter = |cx: &mut TestAppContext, text: &str| {
        cx.update_window(handle, |_, window, cx| {
            let query = pilot.read(cx).node_workspace.query.clone();
            query.update(cx, |input, cx| input.set_value("", window, cx));
            window.focus(&query.read(cx).focus_handle(cx), cx);
            window.input(text, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    // The rows drawn, by name, and the tallies' total.
    let shown = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let nodes = &pilot.read(cx).node_workspace;
            let names: Vec<String> = nodes
                .rows
                .iter()
                .filter(|row| window.try_find(row.id.clone()).is_some())
                .map(|row| row.name.to_string())
                .collect();
            (names, nodes.counts.iter().sum::<usize>())
        })
        .unwrap()
    };
    let key = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("nav-nodes", cx);
            window.render_frame(cx);
            window.click("node-talos-wk-fra1-01", cx);
            window.render_frame(cx);
            assert!(pilot.read(cx).node_workspace.open);
            pilot.read(cx).node_workspace.selected.clone().unwrap()
        })
        .unwrap();

    type_filter(cx, "wk");
    let (names, total) = shown(cx);
    assert!(names.iter().all(|name| name.contains("wk")), "{names:?}");
    assert_eq!(names.len(), 3);
    assert_eq!(total, 3);

    // A filter that hides the open node keeps its pane and selection.
    type_filter(cx, "cp");
    let (names, total) = shown(cx);
    assert!(names.iter().all(|name| name.contains("cp")), "{names:?}");
    assert_eq!(names.len(), 3);
    assert_eq!(total, 3);
    let nodes = pilot.read_with(cx, |pilot, _| {
        (
            pilot.node_workspace.open,
            pilot.node_workspace.selected.clone(),
        )
    });
    assert_eq!(nodes, (true, Some(key)));
}

#[gpui_kit::test]
fn nodes_empty_states_keep_the_shared_header_and_keyboard_context(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| {
            pilot.node_workspace.rows = std::sync::Arc::new(Vec::new());
            pilot.node_workspace.empty = Some(super::super::Empty::Loading);
            pilot.node_workspace.rebuild_lines();
            window.focus(&pilot.node_focus, cx);
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("nodes-title").visible());
        // The table's loading rows, under its header.
        assert!(window.within("nodes-list").find("nodes-loading").visible());
        window.find(("nodes-sort", 1usize));
        window.press("down", cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.open);
        for (state, id, title) in [
            (
                super::super::Empty::Loaded,
                "nodes-empty",
                "No nodes reported",
            ),
            (
                super::super::Empty::Failed("Read result".into()),
                "nodes-failed",
                "Nodes unavailable · Read result",
            ),
        ] {
            pilot.update(cx, |pilot, cx| {
                pilot.node_workspace.empty = Some(state.clone());
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.find(id).visible());
            assert_eq!(window.find(id).label(), Some(title));
            assert!(window.try_find("nodes-table-scroll").is_none());
            assert!(window.find("nodes-refresh").visible());
            if id == "nodes-failed" {
                assert!(window.find("nodes-retry").visible());
            }
        }
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        assert!(window.find("nodes-title").visible());
        // The cards, not a table, wait in a card.
        pilot.update(cx, |pilot, cx| {
            pilot.node_workspace.empty = Some(super::super::Empty::Loading);
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("nodes-loading").visible());
        assert!(window.try_find("nodes-list").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_folded_nodes_controls_do_what_the_controls_do(cx: &mut TestAppContext) {
    // At 760 the chips' own row leaves room for every control; at 600 the
    // last two fold.
    let (_runtime, handle, pilot) = fixture(cx, 600., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        crate::text_size::set(20., cx);
        crate::desktop::tests::settle_header(window, cx);
        // Columns at the page's default, and Refresh never marks the menu.
        assert!(window.try_find("nodes-more-dot").is_none());
        // Refresh reads again, as its button does.
        let tick = pilot.read(cx).fixture_tick;
        window.click("nodes-more", cx);
        window.render_frame(cx);
        // Columns, Refresh.
        window.within("popup-menu").click(1usize, cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).fixture_tick, tick + 1);
    })
    .unwrap();
    // The first menu finishes closing.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        // A column from the menu hides as the Columns menu hides it.
        let first = pilot.read(cx).node_workspace.menu_columns[0].0;
        window.click("nodes-more", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
        window.render_frame(cx);
        window
            .within("submenu")
            .within("popup-menu")
            .click(0usize, cx);
        window.render_frame(cx);
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .hidden_columns
                .contains(&first)
        );
        // Off the page's default, the "…" counts the hidden columns.
        window.find("nodes-more-dot");
        let hidden = pilot.read(cx).node_workspace.hidden_columns.len();
        assert_eq!(hidden, 2);
        assert_eq!(
            window.find("nodes-more").label(),
            Some("More · 2 columns hidden")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn groups_tallies_and_glyphs_agree_and_arrows_skip_folded_healthy_rows(cx: &mut TestAppContext) {
    use super::super::projection::{Item, Status};
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace;
        assert_eq!(nodes.counts, [2, 1, 0, 3]);
        assert_eq!(nodes.group_counts, nodes.counts);
        assert_eq!(nodes.items.first(), Some(&Item::Group(Status::Failing)));
        assert!(nodes.healthy_collapsed());
        assert_eq!(
            window.find("nodes-tally-warning").label(),
            Some("1 warning nodes")
        );
        assert_eq!(
            window.find("nodes-group-warning").label(),
            Some("Warning · 1 node")
        );
        assert_eq!(
            window.find("nodes-group-failing").label(),
            Some("Failing · 2 nodes")
        );
        assert!(window.try_find("node-talos-cp-fra1-01").is_none());
        pilot.update(cx, |pilot, cx| window.focus(&pilot.node_focus, cx));
        for name in ["talos-wk-fra1-02", "talos-wk-fra1-03", "talos-wk-fra1-01"] {
            window.press("down", cx);
            window.render_frame(cx);
            let nodes = &pilot.read(cx).node_workspace;
            assert_eq!(nodes.row().unwrap().name, name);
            assert!(nodes.row().unwrap().tone != Tone::Good);
            assert!(window.find(nodes.row().unwrap().id.clone()).visible());
        }
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).node_workspace.row().unwrap().name,
            "talos-wk-fra1-01"
        );
        window.press("up", cx);
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).node_workspace.row().unwrap().name,
            "talos-wk-fra1-03"
        );
        window.click("nodes-tally-warning", cx);
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace;
        assert_eq!(nodes.lines.len(), 1);
        assert_eq!(nodes.rows[nodes.lines[0]].tone, Tone::Warn);
        assert_eq!(nodes.group_counts, [0, 1, 0, 0]);
        assert_eq!(nodes.counts, [2, 1, 0, 3]);
    })
    .unwrap();
}

#[gpui_kit::test]
fn text_filter_and_cards_empty_state_preserve_selection_and_target(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let target = pilot.read_with(cx, |pilot, _| pilot.selected_node.clone());
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        assert!(window.find("nodes-filter").visible());
        let scope = window.find("nodes-scope");
        assert!(scope.visible());
        assert!(
            scope
                .path()
                .contains(&gpui_kit::ElementId::from("status-bar")),
            "Nodes' line is in the status bar"
        );
        let status = scope.label().unwrap_or_default().to_owned();
        for part in ["Example data", "Talos", "Kubernetes"] {
            assert!(status.contains(part), "{part} is missing from {status}");
        }
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        window.focus(
            &pilot
                .read(cx)
                .node_workspace
                .query
                .read(cx)
                .focus_handle(cx),
            cx,
        );
        window.input("not-an-observed-node", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.lines.len(), 0);
        assert_eq!(pilot.read(cx).node_workspace.counts, [0; 4]);
        assert_eq!(
            window.find("nodes-empty").label(),
            Some("No matching nodes")
        );
        assert!(window.try_find("nodes-cards").is_none());
        assert_eq!(pilot.read(cx).selected_node, target);
        assert!(pilot.read(cx).node_workspace.selected.is_none());
        // Escape clears the filter, keeping the keyboard there; a second
        // hands it to the cards.
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.query_text.is_empty());
        assert!(window.find("nodes-cards").visible());
        assert!(!pilot.read(cx).node_focus.is_focused(window));
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_focus.is_focused(window));
        window.focus(
            &pilot
                .read(cx)
                .node_workspace
                .query
                .read(cx)
                .focus_handle(cx),
            cx,
        );
        window.input("WoRkEr", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace;
        assert_eq!(nodes.lines.len(), 3);
        assert_eq!(nodes.counts, [2, 1, 0, 0]);
        assert!(
            nodes
                .lines
                .iter()
                .all(|ix| nodes.rows[*ix].role == presentation::Role::Worker)
        );
        assert_eq!(pilot.read(cx).selected_node, target);
    })
    .unwrap();
}

#[gpui_kit::test]
fn leaving_kubernetes_only_restores_talos_columns_without_waiting_for_a_summary(
    cx: &mut TestAppContext,
) {
    use freshkube_ui::table::TableColumn;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.kubernetes_only = Some(crate::desktop::kubernetes_only::KubernetesOnly::new(
                None, None,
            ));
            pilot.rebuild_joined_nodes(cx);
            let labels = |pilot: &super::super::Pilot| {
                pilot
                    .node_workspace
                    .columns
                    .iter()
                    .map(|column| column.label().to_string())
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                labels(pilot),
                ["", "Name", "Role", "Kubernetes", "CPU", "Memory", "Pods"]
            );
            pilot.leave_kubernetes_only(window, cx);
            assert_eq!(
                labels(pilot),
                [
                    "",
                    "Name",
                    "Role",
                    "Kubernetes",
                    "Talos",
                    "CPU",
                    "Memory",
                    "Pods",
                    "Services"
                ]
            );
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn opening_a_folded_healthy_node_from_overview_expands_and_reveals_it_on_return(
    cx: &mut TestAppContext,
) {
    use freshkube_ui::table::TableSource;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 560.);
    cx.update_window(handle, |_, window, cx| {
        let key = pilot
            .read(cx)
            .node_workspace
            .rows
            .iter()
            .find(|row| row.name == "talos-cp-fra1-01")
            .unwrap()
            .key
            .clone();
        assert!(pilot.read(cx).node_workspace.healthy_collapsed());
        assert!(pilot.read(cx).line_of(&key).is_none());
        pilot.update(cx, |pilot, cx| {
            pilot.open_destination(
                crate::presentation::attention::Destination::Node(key.clone(), NodeTab::Overview),
                window,
                cx,
            )
        });
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.healthy_collapsed());
        assert_eq!(pilot.read(cx).node_workspace.selected.as_ref(), Some(&key));
        // Simulate a newer summary folding its now-healthy selected row while the pane is open.
        pilot.update(cx, |pilot, _| {
            pilot.node_workspace.healthy_open = false;
            pilot.node_workspace.rebuild_lines();
            pilot
                .node_workspace
                .table
                .reveal(0, gpui_kit::ScrollStrategy::Top);
        });
        assert!(pilot.read(cx).node_workspace.healthy_collapsed());
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.open);
        assert!(!pilot.read(cx).node_workspace.healthy_collapsed());
        assert_eq!(pilot.read(cx).node_workspace.selected.as_ref(), Some(&key));
        assert!(pilot.read(cx).line_of(&key).is_some());
        assert!(window.find("node-talos-cp-fra1-01").visible());
        assert_eq!(
            pilot.read(cx).selected_node.as_deref(),
            Some("talos-cp-fra1-01")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn node_logs_startup_opens_a_folded_node_without_a_table_click(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        assert!(pilot.read(cx).node_workspace.healthy_collapsed());
        pilot.update(cx, |pilot, cx| {
            pilot.startup_selection(Some("node-logs"), None, None, window, cx);
        });
        window.render_frame(cx);
        let view = pilot.read(cx);
        assert_eq!(view.page, Page::Nodes);
        assert!(view.node_workspace.open);
        assert_eq!(view.node_workspace.tab, NodeTab::Logs);
        assert_eq!(view.node_workspace.row().unwrap().name, "talos-cp-fra1-01");
        assert!(!view.node_workspace.healthy_collapsed());
        assert!(window.find("logs-viewport").visible());
        window.click("node-close", cx);
        window.render_frame(cx);
        assert!(window.find("node-talos-cp-fra1-01").visible());
    })
    .unwrap();
}
