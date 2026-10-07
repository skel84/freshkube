use super::Reading;
use crate::desktop::{Page, layout_check, tests::fixture};
use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};

const SYSTEM_SERVICES: layout_check::TablePage = layout_check::TablePage {
    page: "system-services-page",
    title: "system-services-title",
    title_text: "System services",
    table: "system-services-table-scroll",
    list: "system-services-list",
};

const UNHEALTHY: &str = "system-service-talos-wk-fra1-02-kubelet";
const NOT_REPORTED: &str = "system-service-talos-cp-fra1-01-auditd";
const HEALTHY: &str = "system-service-talos-cp-fra1-01-apid";

#[gpui_kit::test]
fn system_services_is_a_table_page_at_every_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            window.press("secondary-7", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(view.read(cx).page, Page::SystemServices);
            // The unhealthy kubelet puts the rows under a header per health.
            let layout = layout_check::assert_table_page(window, cx, &SYSTEM_SERVICES);
            assert!(layout.group.is_some(), "{layout:#?}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn status_chips_filter_the_services_and_clear(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-7", cx);
        window.render_frame(cx);
        assert!(window.find("system-services-group-unhealthy").visible());
        assert_eq!(
            window.find("system-services-tally-unhealthy").label(),
            Some("1 unhealthy")
        );

        window.click("system-services-tally-unhealthy", cx);
        window.render_frame(cx);
        assert!(window.find(UNHEALTHY).visible());
        assert!(window.try_find(HEALTHY).is_none());
        // One status shows, so it has no group header.
        assert!(window.try_find("system-services-group-unhealthy").is_none());

        window.click("system-services-tally-not-reported", cx);
        window.render_frame(cx);
        assert!(window.find(NOT_REPORTED).visible());
        assert!(window.try_find(UNHEALTHY).is_none());

        window.click("system-services-tally-not-reported", cx);
        window.render_frame(cx);
        assert!(window.find(UNHEALTHY).visible());
        assert!(window.find("system-services-group-unhealthy").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_table_waits_for_the_talos_overview_until_it_answers_or_fails(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-7", cx);
        view.update(cx, |pilot, cx| {
            pilot.overview = crate::state::Snapshot::default();
            pilot.nodes.clear();
            pilot
                .system_services
                .update(cx, |services, cx| services.set_nodes(&[], cx));
            pilot.rebuild_joined_nodes(cx);
        });
        window.render_frame(cx);
        // The table's loading rows, under its header.
        assert!(
            window
                .within("system-services-list")
                .find("system-services-loading")
                .visible()
        );
        window.find(("system-services-sort", 1usize));
        assert!(window.try_find("system-services-empty").is_none());

        // A failed read waits no more, and says why.
        view.update(cx, |pilot, cx| pilot.simulate_failure(cx));
        window.render_frame(cx);
        assert!(window.try_find("system-services-loading").is_none());
        assert!(window.find("system-services-empty").visible());
        let reading = view.read(cx).system_services.read(cx).reading().clone();
        assert!(
            matches!(&reading, Reading::Failed(reason) if reason.contains("didn't answer within 10 s")),
            "{reading:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_loading_rows_give_way_to_the_services_when_the_overview_answers(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-7", cx);
        let nodes = view.update(cx, |pilot, cx| {
            let nodes = std::mem::take(&mut pilot.nodes);
            let answer = pilot.overview.data().cloned();
            pilot.overview = crate::state::Snapshot::default();
            pilot
                .system_services
                .update(cx, |services, cx| services.set_nodes(&[], cx));
            pilot.rebuild_joined_nodes(cx);
            (nodes, answer)
        });
        window.render_frame(cx);
        assert!(window.find("system-services-loading").visible());

        view.update(cx, |pilot, cx| {
            let (nodes, answer) = nodes;
            let request = pilot.overview.begin(pilot.applied.clone());
            pilot
                .overview
                .apply(&request, Ok(answer.expect("the example answered")));
            pilot.nodes = nodes;
            pilot.rebuild_joined_nodes(cx);
            let summaries = pilot.nodes.clone();
            pilot
                .system_services
                .update(cx, |services, cx| services.set_nodes(&summaries, cx));
        });
        window.render_frame(cx);
        assert!(window.try_find("system-services-loading").is_none());
        assert!(window.find(UNHEALTHY).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_filter_with_no_match_says_so(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-7", cx);
        window.render_frame(cx);
        window.click("system-service-filter", cx);
        window.input("no-such-service", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("system-services-empty").visible());
        assert!(window.try_find(UNHEALTHY).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn narrow_window_keeps_the_header_and_scrolls_the_table(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-7", cx);
        crate::desktop::tests::settle_header(window, cx);
        let page = window.find("system-services-page").bounds();
        for id in [
            "system-services-title",
            "system-service-filter",
            "system-services-tally",
            "system-service-node",
        ] {
            let bounds = window.find(id).bounds();
            assert!(
                bounds.right() <= page.right() && bounds.left() >= page.left(),
                "{id} at {bounds:?} leaves the page at {page:?}"
            );
        }
        assert!(window.find(UNHEALTHY).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_refresh_keeps_the_filter_and_the_chosen_status(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-7", cx);
        window.render_frame(cx);
        window.click("system-services-tally-not-reported", cx);
        window.click("system-service-filter", cx);
        window.input("cp-fra1-01", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find(NOT_REPORTED).visible());
        assert!(window.try_find(HEALTHY).is_none());
        assert!(
            window
                .try_find("system-service-talos-wk-fra1-01-auditd")
                .is_none()
        );
        window.press("secondary-r", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find(NOT_REPORTED).visible());
        assert!(window.try_find(HEALTHY).is_none());
        assert!(
            window
                .try_find("system-service-talos-wk-fra1-01-auditd")
                .is_none()
        );
        assert_eq!(
            window.find("system-service-filter").value(),
            Some("cp-fra1-01")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_node_that_leaves_falls_back_to_all_nodes(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-7", cx);
        window.render_frame(cx);
        let services = view.read(cx).system_services.clone();
        services.update(cx, |services, cx| {
            services.node = Some("talos-wk-fra1-02".into());
            services.rebuild(cx);
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("system-service-node").label(),
            Some("talos-wk-fra1-02")
        );
        assert!(window.try_find(HEALTHY).is_none());

        let nodes: Vec<_> = view
            .read(cx)
            .nodes
            .iter()
            .filter(|node| node.name != "talos-wk-fra1-02")
            .cloned()
            .collect();
        services.update(cx, |services, cx| services.set_nodes(&nodes, cx));
        window.render_frame(cx);
        assert_eq!(
            window.find("system-service-node").label(),
            Some("All nodes")
        );
        assert!(window.find(HEALTHY).visible());
    })
    .unwrap();
}

/// The selected row's id, read from the page.
fn selected(view: &gpui_kit::Entity<crate::desktop::Pilot>, cx: &gpui_kit::App) -> Option<String> {
    view.read(cx)
        .system_services
        .read(cx)
        .selected
        .as_ref()
        .map(|key| key.to_string())
}

#[gpui_kit::test]
fn the_rows_carry_no_actions_and_the_toolbar_waits_for_a_selection(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    for size in [crate::ui::BASE_TEXT, 14.] {
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(size, cx);
            window.press("secondary-7", cx);
            window.render_frame(cx);
            for row in [UNHEALTHY, HEALTHY] {
                for action in ["logs", "open"] {
                    assert!(
                        window.within(row).try_find(action).is_none(),
                        "{row} still has {action} at {size} px"
                    );
                }
            }
            // Health check is the last column.
            assert!(window.try_find(("system-services-sort", 5usize)).is_none());
            // With nothing selected, the toolbar's actions do nothing.
            window.click("system-service-open", cx);
            window.click("system-service-logs", cx);
            window.press("enter", cx);
            window.press("l", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).page, Page::SystemServices);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_arrows_select_and_escape_clears(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-7", cx);
        window.render_frame(cx);
        // The page gives the list the keyboard; down starts at the first row.
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(selected(&view, cx).as_deref(), Some(UNHEALTHY));
        assert_eq!(window.find(UNHEALTHY).selected(), Some(true));
        window.press("down", cx);
        window.press("up", cx);
        assert_eq!(selected(&view, cx).as_deref(), Some(UNHEALTHY));
        window.press("escape", cx);
        window.render_frame(cx);
        assert_eq!(selected(&view, cx), None);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_filter_types_the_lists_keys(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-7", cx);
        window.render_frame(cx);
        window.click(UNHEALTHY, cx);
        window.click("system-service-filter", cx);
        window.input("talos-wk-fra1-02", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // l and o went into the filter, not to the selected row.
        assert_eq!(view.read(cx).page, Page::SystemServices);
        assert_eq!(selected(&view, cx).as_deref(), Some(UNHEALTHY));
        assert!(window.try_find(HEALTHY).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_filter_that_hides_the_selected_row_clears_it(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-7", cx);
        window.render_frame(cx);
        window.click(UNHEALTHY, cx);
        assert_eq!(selected(&view, cx).as_deref(), Some(UNHEALTHY));
        window.click("system-services-tally-healthy", cx);
        window.render_frame(cx);
        assert_eq!(selected(&view, cx), None);
        window.click("system-service-open", cx);
        assert_eq!(view.read(cx).page, Page::SystemServices);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_right_click_selects_its_row_and_its_menu_opens_the_node(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-7", cx);
        window.render_frame(cx);
        // Another row is selected: the menu takes its own row.
        window.click(UNHEALTHY, cx);
        window.render_frame(cx);
        window.right_click(HEALTHY, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(selected(&view, cx).as_deref(), Some(HEALTHY));
        let menu = window.within("popup-menu");
        let labels: Vec<_> = (0..2usize)
            .map(|ix| menu.find(ix).label().map(str::to_owned))
            .collect();
        assert!(
            labels[0]
                .as_deref()
                .is_some_and(|label| label.starts_with("Logs"))
                && labels[1]
                    .as_deref()
                    .is_some_and(|label| label.starts_with("Open node")),
            "{labels:?}"
        );
        window.within("popup-menu").click(1usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.page, Page::Nodes);
        assert_eq!(pilot.selected_node.as_deref(), Some("talos-cp-fra1-01"));
        assert_eq!(pilot.selected_service.as_deref(), Some("apid"));
        assert_eq!(
            pilot.node_workspace.tab,
            crate::desktop::nodes::NodeTab::Services
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_empty_state_has_no_menu(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-7", cx);
        window.render_frame(cx);
        window.click("system-service-filter", cx);
        window.input("no-such-service", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click("system-services-empty", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
        assert_eq!(selected(&view, cx), None);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_folded_node_picker_picks_a_node_as_the_picker_does(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-7", cx);
        crate::text_size::set(20., cx);
        crate::desktop::tests::settle_header(window, cx);
        assert!(
            window.try_find("system-service-node").is_none(),
            "not folded"
        );
        let services = view.read(cx).system_services.clone();
        let first = services.read(cx).nodes[0].clone();
        assert!(window.try_find("system-services-more-dot").is_none());
        window.click("system-services-more", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
        window.render_frame(cx);
        // All nodes, then each node.
        window
            .within("submenu")
            .within("popup-menu")
            .click(1usize, cx);
        window.render_frame(cx);
        assert_eq!(services.read(cx).node.as_deref(), Some(first.as_str()));
        // The "…" says the folded picker narrows the list.
        window.find("system-services-more-dot");
        assert_eq!(
            window.find("system-services-more").label(),
            Some(format!("More · Node {first}").as_str())
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_short_window_scrolls_the_frame_and_keeps_the_list_usable(cx: &mut TestAppContext) {
    use freshkube_ui::page::SHORT_LIST_HEIGHT;
    use gpui_kit::px;
    // At 760 × 560 the header and a usable list fit; at 480 high they don't.
    for (width, height, text, short) in [(760., 480., 20., true), (1280., 880., 13., false)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(text, cx);
            window.press("secondary-7", cx);
            crate::desktop::tests::settle_header(window, cx);
            let scroll = view.read(cx).system_services.read(cx).page_scroll.clone();
            let page = window.find(SYSTEM_SERVICES.page).bounds();
            let table = window.find(SYSTEM_SERVICES.table).bounds();
            // The table runs to the end of the scrolled page, less the
            // hairline under it.
            let end = page.bottom() + scroll.max_offset().y;
            assert!(end - table.bottom() <= px(1.5), "{table:?} in {page:?}");
            if short {
                assert!(scroll.max_offset().y > px(0.), "the frame doesn't scroll");
                let least = crate::ui::dp_px(SHORT_LIST_HEIGHT, window) - px(2.);
                assert!(
                    table.size.height >= least,
                    "the table is squeezed to {table:?}"
                );
            } else {
                assert_eq!(scroll.max_offset().y, px(0.));
            }
        })
        .unwrap();
    }
}
