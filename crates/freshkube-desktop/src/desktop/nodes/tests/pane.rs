use super::*;

#[gpui_kit::test]
fn joined_fixture_pane_preserves_selection_tab_and_target(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let rows = pilot.read(cx).node_workspace.rows.clone();
        assert_eq!(rows.len(), 6);
        let bad = rows
            .iter()
            .find(|row| row.name == "talos-wk-fra1-02")
            .unwrap();
        assert_eq!(bad.ready, "NotReady");
        assert_eq!(bad.tone, Tone::Crit);
        assert!(
            bad.problems
                .iter()
                .any(|problem| problem == "kubelet unhealthy")
        );
        let silent = rows
            .iter()
            .find(|row| row.name == "talos-wk-fra1-03")
            .unwrap();
        assert_eq!(silent.talos_state, "No response");
        window.click(bad.id.clone(), cx);
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).selected_node.as_deref(),
            Some("talos-wk-fra1-02")
        );
        // Its unhealthy kubelet marks the Services tab with the warning glyph.
        assert!(window.try_find("node-services-problem").is_some());
        let selected = pilot.read(cx).node_workspace.selected.clone();
        window.click("node-tab-processes", cx);
        window.render_frame(cx);
        window.click("node-expand", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.expanded);
        assert!(window.try_find("nodes-cards").is_none());
        window.click("node-expand", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.selected, selected);
        // Beside the pane, the table folds its healthy rows as it does
        // without one (#339).
        crate::desktop::tests::expand_healthy_nodes(window, cx);
        window.click("node-talos-cp-fra1-01", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.tab, NodeTab::Processes);
        assert_eq!(
            pilot.read(cx).selected_node.as_deref(),
            Some("talos-cp-fra1-01")
        );
        assert!(window.try_find("node-services-problem").is_none());
        window.click("node-close", cx);
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).selected_node.as_deref(),
            Some("talos-cp-fra1-01")
        );
        assert_eq!(pilot.read(cx).page, Page::Nodes);
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubernetes_node_has_only_its_supported_tabs_and_stacks_under_the_table_when_narrow(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{point, px};
    let (_runtime, handle, pilot) = fixture(cx, 760., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.nodes.clear();
            pilot.rebuild_joined_nodes(cx);
            pilot.navigate(Page::Nodes, window, cx);
        });
        window.render_frame(cx);
        crate::desktop::tests::expand_healthy_nodes(window, cx);
        pilot.update(cx, |pilot, cx| {
            let key = pilot.node_workspace.rows[0].key.clone();
            pilot.open_node(key, window, cx);
        });
        window.render_frame(cx);
        // Narrow, the inspector opens under the table, which stays.
        assert!(window.try_find("node-back").is_none());
        let table = window.find("nodes-table-scroll").bounds();
        let pane = window.find("node-inspector").bounds();
        assert!(
            pane.top() >= table.bottom() - px(1.),
            "{pane:?} under {table:?}"
        );
        assert!(window.find("node-tab-pods").visible());
        assert!(window.try_find("node-tab-processes").is_none());
        window.click("node-tab-yaml", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.tab, NodeTab::Yaml);

        // Another row above opens its node on the same tab.
        pilot
            .read(cx)
            .node_workspace
            .page_scroll
            .set_offset(point(px(0.), px(0.)));
        window.render_frame(cx);
        let rows: Vec<_> = pilot.read(cx).node_workspace.rows.iter().cloned().collect();
        let other = rows[1..]
            .iter()
            .find(|row| {
                window
                    .try_find(row.id.clone())
                    .is_some_and(|row| row.visible())
            })
            .expect("another row shows above the inspector");
        window.click(other.id.clone(), cx);
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace;
        assert_eq!(nodes.selected.as_ref(), Some(&other.key));
        assert!(nodes.open);
        assert_eq!(nodes.tab, NodeTab::Yaml);

        window.click("node-close", cx);
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.open);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_context_change_closes_the_old_node_document_and_replaces_rows(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            let key = pilot.node_workspace.rows[0].key.clone();
            pilot.open_node(key, window, cx);
            pilot.show_node_tab(NodeTab::Yaml, window, cx);
        });
        window.render_frame(cx);
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .document
                .read(cx)
                .target_identity()
                .is_some()
        );
        pilot.update(cx, |pilot, cx| {
            pilot.select_context("homelab".into(), window, cx)
        });
        window.render_frame(cx);
        let read = pilot.read(cx);
        assert!(!read.node_workspace.open);
        assert!(
            read.node_workspace
                .document
                .read(cx)
                .target_identity()
                .is_none()
        );
        assert_eq!(read.node_workspace.rows.len(), 1);
        assert_eq!(read.node_workspace.rows[0].name, "talos-home");
    })
    .unwrap();
}

/// DESIGN.md's inspector: beside the table at the default text size, under
/// it at 20, where the page is narrower than 900 dp; the table's toolbar
/// stays, and the tabs are the inspector's, 28 high.
#[gpui_kit::test]
fn a_node_opens_in_the_inspector_beside_the_table_or_under_it(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first_node(window, cx);
        assert!(pilot.read(cx).node_workspace.open);
        for text in [13., 20.] {
            crate::text_size::set(text, cx);
            window.render_frame(cx);
            crate::desktop::layout_check::assert_inspector(
                window,
                cx,
                "nodes-split",
                "nodes-table",
                "node-inspector",
                "node-pane-title",
            );
            let pane = window.find("node-inspector").bounds();
            let table = window.find("nodes-table-scroll").bounds();
            assert_eq!(
                pane.left() > table.left(),
                text == 13.,
                "{pane:?} by {table:?}"
            );
            assert!(window.find("nodes-title").visible());
            let tall = crate::ui::dp_px(freshkube_ui::inspector::TAB_HEIGHT, window);
            for tab in ["node-tab-overview", "node-tab-pods", "node-tab-logs"] {
                let height = window.find(tab).bounds().size.height;
                assert!((height - tall).abs() <= px(1.), "{tab} is {height:?} high");
            }
        }
    })
    .unwrap();
}

/// At the least window the inspector stacks under the table and shrinks it;
/// the opened row is revealed in the shrunk table, not the table it left.
#[gpui_kit::test]
fn the_opened_row_shows_whole_above_the_stacked_inspector(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        // Each node in turn, from the table as it was before: the rows
        // under the stacked inspector's least table must scroll to show.
        let rows: Vec<_> = (pilot.read(cx).node_workspace.rows.iter())
            .map(|row| (row.key.clone(), row.id.clone()))
            .collect();
        for (key, id) in rows {
            pilot.update(cx, |pilot, cx| pilot.open_node(key, window, cx));
            // As the app's frames run: the next-frame callbacks, then the
            // draw.
            for _ in 0..8 {
                let ran = window.simulate_next_frame(cx);
                window.render_frame(cx);
                if ran == 0 {
                    break;
                }
            }
            let pane = window.find("node-inspector").bounds();
            let list = window.find("nodes-list").bounds();
            assert!(pane.top() >= list.bottom() - px(0.5), "not stacked");
            let row = window.find(id.clone()).bounds();
            assert!(
                row.top() >= list.top() - px(0.5) && row.bottom() <= list.bottom() + px(0.5),
                "{id} at {row:?} in {list:?}"
            );
            pilot.update(cx, |pilot, cx| pilot.close_node(window, cx));
            window.render_frame(cx);
        }
    })
    .unwrap();
}

/// Beside the table the inspector is too narrow for a node's chips on one
/// line, so they wrap inside it rather than cut one mid-text.
#[gpui_kit::test]
fn the_node_chips_wrap_inside_a_narrow_inspector(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (_runtime, handle, _pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first_node(window, cx);
        window.render_frame(cx);
        let banner = window.find("node-inspector-banner").bounds();
        let chips = window.find("node-chips").bounds();
        assert!(
            chips.right() <= banner.right() + px(0.5),
            "{chips:?} past {banner:?}"
        );
        // More than one line of 20 dp chips.
        let line = crate::ui::dp_px(20., window);
        assert!(chips.size.height > line * 1.5, "one line: {chips:?}");
        let tabs = window.find("node-inspector-tabs").bounds();
        assert!(tabs.top() >= chips.bottom(), "{tabs:?} over {chips:?}");
    })
    .unwrap();
}

/// The inspector's width is Nodes' own and comes back when the app opens
/// again.
#[gpui_kit::test]
fn the_node_inspector_width_survives_reopening(cx: &mut TestAppContext) {
    use crate::navigation_file::NavigationFile;
    use freshkube_ui::split_size::SizeStore as _;
    use gpui_kit::px;
    let directory = std::env::temp_dir().join(format!(
        "freshkube-nodes-width-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let preferences = directory.join("preferences.json");
    let options = || crate::GpuiOptions::fixture().with_preferences(Some(preferences.clone()));
    let (_runtime, handle, pilot) = crate::desktop::tests::mount(cx, options(), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first_node(window, cx);
        let state = pilot.read(cx).node_workspace.split.beside_state().clone();
        state.update(cx, |state, cx| {
            state.resize_panel(1, crate::ui::dp_px(560., window), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    let reopened = NavigationFile::open(Some(&preferences));
    assert_eq!(
        reopened.size(freshkube_ui::inspector::width_key("nodes")),
        Some(560.)
    );
    assert_eq!(
        reopened.size(freshkube_ui::inspector::width_key("resources")),
        None
    );

    let (_runtime, handle, _pilot) = crate::desktop::tests::mount(cx, options(), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first_node(window, cx);
        let width = window.find("node-inspector").bounds().size.width;
        let expected = crate::ui::dp_px(560., window);
        assert!(
            (width - expected).abs() <= px(1.),
            "{width:?}, not {expected:?}"
        );
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&directory);
}

/// From the cards, a node opens with the roster, one to a row, beside it.
#[gpui_kit::test]
fn a_node_opened_from_the_cards_keeps_the_roster_beside_its_inspector(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        let key = pilot.read(cx).node_workspace.rows[0].key.clone();
        pilot.update(cx, |pilot, cx| pilot.open_node(key, window, cx));
        window.render_frame(cx);
        assert!(window.try_find("nodes-table-scroll").is_none());
        let roster = window.find("nodes-cards").bounds();
        let pane = window.find("node-inspector").bounds();
        assert!(pane.left() >= roster.right(), "{pane:?} beside {roster:?}");
        assert!(
            window
                .find("nodes-split")
                .bounds()
                .contains(&roster.center())
        );

        // The opened node's row is marked, and the mark follows another.
        let rows = pilot.read(cx).node_workspace.rows.clone();
        let marked = |window: &mut gpui_kit::Window| {
            let mark = window.find("nodes-roster-selected").bounds();
            rows.iter()
                .filter(|row| {
                    let element = window.find(row.id.clone());
                    // Inside the row's bottom hairline.
                    element.selected() == Some(true) && element.bounds().contains(&mark.center())
                })
                .map(|row| row.key.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(marked(window), vec![rows[0].key.clone()]);
        window.click(rows[1].id.clone(), cx);
        window.render_frame(cx);
        assert_eq!(marked(window), vec![rows[1].key.clone()]);
        assert_eq!(window.find(rows[0].id.clone()).selected(), Some(false));
    })
    .unwrap();
}

/// The node's Events and YAML draw in the inspector's content, in no card
/// of their own.
#[gpui_kit::test]
fn node_events_and_yaml_draw_without_a_card(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first_node(window, cx);
    })
    .unwrap();
    for tab in [NodeTab::Yaml, NodeTab::Events] {
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| pilot.show_node_tab(tab, window, cx));
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(pilot.read(cx).node_workspace.tab, tab);
            let content = window.find("node-inspector-content").bounds();
            let detail = window.find("resource-detail").bounds();
            assert!(
                content.contains(&detail.center()),
                "{detail:?} in {content:?}"
            );
            let content = content.scale(window.scale_factor());
            let card = window.painted_quads().into_iter().find(|quad| {
                let (b, w) = (quad.bounds, quad.border_widths);
                w.left.0 > 0.
                    && w.right.0 > 0.
                    && quad.corner_radii.top_left.0 > 0.
                    && b.size.width.0 >= content.size.width.0 * 0.9
                    && b.size.height.0 >= content.size.height.0 * 0.5
            });
            assert!(card.is_none(), "{tab:?} draws in a card: {card:#?}");
        })
        .unwrap();
    }
}

/// Kit rescales the inspector when the text size or the window changes and
/// says nothing, so the node's screens lay out by the width it has, not the
/// width it was last dragged to.
#[gpui_kit::test]
fn the_node_screens_follow_the_inspector_after_a_text_size_change(cx: &mut TestAppContext) {
    use crate::ui::dp_px;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first_node(window, cx);
        let state = pilot.read(cx).node_workspace.split.beside_state().clone();
        state.update(cx, |state, cx| {
            state.resize_panel(1, dp_px(800., window), window, cx)
        });
        let check = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, what: &str| {
            for _ in 0..4 {
                window.render_frame(cx);
                if window.simulate_next_frame(cx) == 0 {
                    break;
                }
            }
            window.render_frame(cx);
            let pane = window.find("node-inspector").bounds().size.width / dp_px(1., window);
            let seen =
                crate::screens::embedded_width(window) + freshkube_ui::page::PANE_PADDING * 2.;
            assert!(
                (pane - seen).abs() <= 1.,
                "{what}: the inspector is {pane} dp, its screens lay out by {seen}"
            );
        };
        check(window, cx, "after the drag");
        crate::text_size::set(20., cx);
        check(window, cx, "at 20 px");
        crate::text_size::set(13., cx);
        check(window, cx, "back at 13 px");
    })
    .unwrap();
}

/// Expanded, stepping to another node keeps the frame at the top, so the
/// node's name and Collapse stay in view in a short window.
#[gpui_kit::test]
fn stepping_nodes_while_expanded_keeps_the_heading_in_view(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let key = pilot.read(cx).node_workspace.rows[0].key.clone();
        pilot.update(cx, |pilot, cx| pilot.open_node(key, window, cx));
        window.render_frame(cx);
        window.click("node-expand", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.expanded);
        let frame = pilot.read(cx).node_workspace.page_scroll.clone();
        let first = pilot.read(cx).node_workspace.selected.clone();
        pilot.update(cx, |pilot, cx| pilot.step_joined_node(1, window, cx));
        for _ in 0..3 {
            window.render_frame(cx);
        }
        assert_ne!(pilot.read(cx).node_workspace.selected, first, "no step");
        assert_eq!(frame.offset().y, px(0.));
        let page = window.find("nodes-page").bounds();
        let title = window.find("node-pane-title").bounds();
        assert!(title.top() >= page.top(), "{title:?} above {page:?}");
        assert!(window.try_find("node-expand").is_some());
    })
    .unwrap();
}
