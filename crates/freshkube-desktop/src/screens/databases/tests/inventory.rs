use super::*;

#[test]
fn display_uses_both_engines_and_keeps_orphans_visible() {
    let inventory = databases::example::inventory(at());
    let display = display_of(&inventory, Mode::Clusters);
    assert_eq!(display.rows.len(), 2);
    let engines: Vec<&str> = display
        .rows
        .iter()
        .map(|row| row.cells[1].as_ref())
        .collect();
    assert!(engines.contains(&"MariaDB 11.4"), "{engines:?}");
    assert!(engines.contains(&"PostgreSQL 16"), "{engines:?}");
    let backups = display_of(&inventory, Mode::Backups);
    let orphan = backups
        .rows
        .iter()
        .find(|row| row.identity.name == "archive-old")
        .unwrap();
    assert_eq!(
        orphan.tone,
        crate::ui::Tone::Warn,
        "an orphan is not a pass"
    );
    assert!(orphan.verdict.headline.contains("archive-db"));
    assert!(orphan.cells[2] == "Completed");
    let partial = Inventory::unavailable(Coverage::Refused, at());
    assert!(!display_of(&partial, Mode::Backups).empty.starts_with("No "));
}

#[gpui_kit::test]
fn cluster_glyph_names_ready_instances_and_backup_glyph_names_phase(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        for mode in [Mode::Clusters, Mode::Backups] {
            page.update(cx, |page, cx| page.mode(mode, cx));
            window.render_frame(cx);
            let row = &page.read(cx).display.rows[0];
            assert_eq!(
                window
                    .find(gpui_kit::SharedString::from(format!("{}-state", row.id)))
                    .label(),
                Some(row.cells[if mode == Mode::Clusters { 4 } else { 2 }].as_ref())
            );
        }
    })
    .unwrap();
}

#[test]
fn schedules_say_active_or_suspended_and_keep_the_expression_in_the_tooltip() {
    let inv = databases::example::inventory(at());
    let display = display_of(&inv, Mode::Schedules);
    let cnpg = display
        .rows
        .iter()
        .find(|row| row.identity.name == "orders-daily")
        .unwrap();
    assert_eq!(cnpg.cells[2], "Daily at 02:00");
    assert_eq!(cnpg.cells[3], "Active");
    assert_eq!(cnpg.tips[2].as_deref(), Some("0 0 2 * * *"));
    let maria = display
        .rows
        .iter()
        .find(|row| row.identity.name == "ledger-nightly")
        .unwrap();
    assert_eq!(maria.cells[2], "Daily at 03:00");
    assert_eq!(maria.tips[2].as_deref(), Some("0 3 * * *"));
    // A dialect mismatch is shown, not reinterpreted.
    let mut bad = inv.clone();
    bad.schedules.items[0].schedule = databases::Fact::Known("0 2 * * *".into());
    let display = display_of(&bad, Mode::Schedules);
    let row = display
        .rows
        .iter()
        .find(|row| row.identity.name == "orders-daily")
        .unwrap();
    assert_eq!(row.cells[2], "0 2 * * *");
    assert!(
        row.tips[2]
            .as_deref()
            .unwrap()
            .contains("5 fields where PostgreSQL takes 6")
    );
}

#[gpui_kit::test]
fn the_cron_expression_is_the_tooltip_of_the_when_cell(cx: &mut TestAppContext) {
    let (_runtime, window, _pilot) = crate::desktop::tests::fixture(cx, 1280., 880.);
    cx.update_window(window, |_, window, cx| {
        window.press("secondary-shift-d", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("databases-view-schedules", cx);
        window.render_frame(cx);
        let cell = window
            .within("databases-row-ScheduledBackup-shop-invented-orders-daily")
            .find("databases-row-ScheduledBackup-shop-invented-orders-daily-text-3-cell")
            .bounds();
        window.dispatch_event(
            gpui_kit::MouseMoveEvent {
                position: cell.center(),
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.find("follow-tooltip-text");
        assert!(freshkube_ui::tooltip::drawn("0 0 2 * * *") > 0);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_default_fixture_groups_rows_that_need_a_look_before_the_healthy_ones(
    cx: &mut TestAppContext,
) {
    let (_runtime, page, window) = mount(cx, 1280.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find("databases-group-needs-a-look");
        window.find("databases-group-healthy");
        let labels: Vec<_> = page
            .read(cx)
            .display
            .groups
            .iter()
            .map(|group| (group.label, group.count))
            .collect();
        assert_eq!(labels, [("Needs a look", 1), ("Healthy", 1)]);
        // Coroot sees the slow MariaDB the operator calls ready.
        let rows = &page.read(cx).display.rows;
        assert_eq!(rows[0].identity.name, "ledger-db");
        assert_eq!(rows[0].chip, Some("Coroot"));
        assert_eq!(rows[1].identity.name, "orders-db");
        assert_eq!(rows[1].chip, Some("Both"));
        assert_eq!(
            rows[0].cells.iter().map(|c| c.as_ref()).collect::<Vec<_>>()[..2],
            ["ledger-db", "MariaDB 11.4"]
        );
        assert_eq!(rows[0].cells[4], "Warning");
        assert_eq!(rows[1].cells[4], "Healthy");
        window.find("databases-row-MariaDB-shop-invented-ledger-db-text-5-chip");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_filter_finds_rows_by_namespace_primary_and_engine(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        for (text, found) in [
            ("shop", 2),
            ("ledger-db-0", 1),
            ("orders-db-1", 1),
            ("mariadb", 1),
            ("postgres", 1),
            ("warning", 1),
            ("nothing-here", 0),
            ("", 2),
        ] {
            window.click("databases-filter", cx);
            page.update(cx, |page, cx| {
                page.filter_input.update(cx, |input, cx| {
                    input.set_value(text.to_owned(), window, cx);
                });
                page.display.set_filter(text);
                cx.notify();
            });
            window.render_frame(cx);
            let rows = page
                .read(cx)
                .display
                .lines
                .iter()
                .filter(|line| matches!(line, super::display::Entry::Row(_)))
                .count();
            assert_eq!(rows, found, "{text}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn counts_and_the_namespace_label_read_naturally(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280.);
    cx.update_window(window.into(), |_, _, cx| {
        let text = page.update(cx, |page, cx| {
            page.status(cx).unwrap().text(None).to_string()
        });
        assert!(text.contains("2 clusters"), "{text}");
        assert!(text.contains("1 need a look"), "{text}");
        assert!(text.contains("All namespaces"), "{text}");
        assert!(!text.contains("read-only"), "{text}");
        assert!(!text.contains("/ "), "{text}");
        assert_eq!(super::NAMESPACE_PLACEHOLDER, "Namespace · all");
    })
    .unwrap();
    let inv = databases::example::shaped(at(), "mariadb");
    let one = display_of(&inv, Mode::Clusters);
    assert!(one.summary.starts_with("1 cluster"), "{}", one.summary);
    let schedules = display_of(&inv, Mode::Schedules);
    assert!(
        schedules.summary.starts_with("1 schedule"),
        "{}",
        schedules.summary
    );
    let backups = display_of(&inv, Mode::Backups);
    assert!(
        backups.summary.starts_with("1 backup"),
        "{}",
        backups.summary
    );
}
