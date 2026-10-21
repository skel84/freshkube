use super::*;

#[gpui_kit::test]
fn applications_show_all_and_columns_keep_the_same_projection(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let problems = page.read(cx).shown_apps;
        assert!(
            window
                .within("obs-applications-footer")
                .find("obs-applications-collapsed")
                .visible()
        );
        let total = page.read(cx).counts[1];
        assert!(total > problems);
        assert_eq!(
            window.find("obs-show-all").label(),
            Some(format!("Show all {total}").as_str()),
            "Show all names the count, as Pods' does"
        );
        window.click("obs-show-all", cx);
        assert!(page.read(cx).shown_apps > problems);
        window.render_frame(cx);
        assert!(window.try_find("obs-applications-collapsed").is_none());
        let shown = page.read(cx).shown_apps;
        let width = page.read(cx).application_width;
        window.click("obs-columns", cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert!(
            page.read(cx)
                .hidden_application_columns
                .contains(&super::application_columns::ColumnKind::Type)
        );
        assert!(page.read(cx).application_width < width);
        assert_eq!(page.read(cx).shown_apps, shown);
        window.render_frame(cx);
        let table_id = gpui_kit::ElementId::from("obs-applications-table");
        assert!(
            !gpui_kit::base::test_support::snapshots(window)
                .iter()
                .any(|element| {
                    element.path().contains(&table_id)
                        && element.role() == Some(gpui_kit::Role::ColumnHeader)
                        && element.label() == Some("Type")
                })
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn applications_sideways_scroll_reaches_the_last_report_and_opens_it(cx: &mut TestAppContext) {
    // This harness mounts only the page: subtract the shell's rail and column
    // to give its table the same viewport as the real 1280-wide desktop.
    let width = 1280. - freshkube_ui::page::RAIL_WIDTH - freshkube_ui::page::COLUMN_WIDTH;
    let (_runtime, handle, page) = mount_size(cx, true, width, 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let card = window.find("obs-applications-table-scroll").bounds();
        assert!(
            window
                .find("obs-check-fixture:payments:Deployment:worker-logs")
                .bounds()
                .right()
                > card.right(),
            "the last report must begin clipped, so the scroll proves reachability"
        );
        window.scroll(
            "obs-applications-table-scroll",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(-2000.), px(0.))),
            cx,
        );
        let report = window.find("obs-check-fixture:payments:Deployment:worker-logs");
        let card = window.find("obs-applications-table-scroll").bounds();
        assert!(report.visible());
        assert!(report.bounds().left() >= card.left() && report.bounds().right() <= card.right());
        window.click("obs-check-fixture:payments:Deployment:worker-logs", cx);
        assert_eq!(page.read(cx).destination, Destination::Application);
        assert_eq!(page.read(cx).report, Report::Logs);
    })
    .unwrap();
}
