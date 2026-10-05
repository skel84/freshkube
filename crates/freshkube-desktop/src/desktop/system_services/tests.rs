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
        window.within(UNHEALTHY).find("open");
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

/// The rows the fit checks read: the unhealthy one, and one of the
/// baremetal node's, whose long name is the widest and truncates.
const FIT_ROWS: [&str; 2] = [
    UNHEALTHY,
    "system-service-talos-cp-fra1-03-baremetal-rack-b7-auditd",
];

/// Health check fills the line up to the actions: its right edge sits no
/// further from Logs than the actions cell's padding and a little slack.
fn assert_health_check_reaches_the_actions(
    window: &mut gpui_kit::Window,
    rows: &[&'static str],
    size: f32,
) {
    let health = window.find(("system-services-sort", 4usize)).bounds();
    let dp = gpui_kit::px(size / crate::ui::BASE_TEXT);
    for &row in rows {
        let logs = window.within(row).find("logs").bounds();
        assert!(
            logs.left() >= health.right() && logs.left() - health.right() <= dp * 16.,
            "{row}: Health check ends at {:?}, Logs starts at {:?} at {size} px",
            health.right(),
            logs.left()
        );
    }
}

#[gpui_kit::test]
fn the_row_actions_fit_at_1280_at_the_default_text_size_and_one_up(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 880.);
    for size in [crate::ui::BASE_TEXT, 14.] {
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(size, cx);
            window.press("secondary-7", cx);
            window.render_frame(cx);
            let table = window.find("system-services-table-scroll").bounds();
            for row in FIT_ROWS {
                for action in ["logs", "open"] {
                    let bounds = window.within(row).find(action).bounds();
                    assert!(
                        bounds.left() >= table.left() && bounds.right() <= table.right(),
                        "{row} {action} at {bounds:?} leaves the table at {table:?} at {size} px"
                    );
                }
            }
            assert_health_check_reaches_the_actions(window, &FIT_ROWS, size);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn health_check_reaches_the_actions_when_the_table_scrolls(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.press("secondary-7", cx);
        window.render_frame(cx);
        // Only the first rows are drawn in this short window.
        assert_health_check_reaches_the_actions(window, &[UNHEALTHY], 20.);
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
    })
    .unwrap();
}
