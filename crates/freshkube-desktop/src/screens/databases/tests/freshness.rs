use super::*;

#[test]
fn a_failed_operator_keeps_stale_rows_that_say_so_and_never_read_healthy() {
    let previous = databases::example::inventory(at());
    let mut fresh = databases::example::inventory(at());
    fresh.engines[1].1 = Coverage::Refused;
    fresh.databases.items.remove(1);
    fresh.databases.coverage = Coverage::Refused;
    fresh.retain(&previous);
    let display = display_of(&fresh, Mode::Clusters);
    let ledger = display
        .rows
        .iter()
        .find(|row| row.identity.name == "ledger-db")
        .expect("kept");
    assert_eq!(ledger.cells[4], "Unknown");
    assert_eq!(ledger.chip, None);
    let orders = display
        .rows
        .iter()
        .find(|row| row.identity.name == "orders-db")
        .unwrap();
    assert_eq!(orders.cells[4], "Healthy");
    let notice = display.notice.unwrap();
    assert!(
        notice.contains("MariaDB · MariaDB operator: Not permitted · last read retained"),
        "{notice}"
    );
}

#[test]
fn retained_rows_stay_unknown_with_the_banner_after_two_failed_refreshes_in_every_view() {
    let previous = databases::example::inventory(at());
    let refused = |previous: &Inventory| {
        let mut fresh = databases::example::inventory(at());
        fresh.engines[1].1 = Coverage::Refused;
        fresh.databases.items.remove(1);
        fresh
            .backups
            .items
            .retain(|b| b.identity.group != "k8s.mariadb.com");
        fresh
            .schedules
            .items
            .retain(|s| s.identity.group != "k8s.mariadb.com");
        fresh.databases.coverage = Coverage::Refused;
        fresh.retain(previous);
        fresh
    };
    let once = refused(&previous);
    let twice = refused(&once);
    for mode in [Mode::Clusters, Mode::Backups, Mode::Schedules] {
        let display = display_of(&twice, mode);
        let kept = display
            .rows
            .iter()
            .find(|row| row.identity.group == "k8s.mariadb.com")
            .unwrap_or_else(|| panic!("{}", mode.label()));
        assert_eq!(kept.tone, crate::ui::Tone::Unknown, "{}", mode.label());
        if mode != Mode::Clusters {
            assert!(kept.verdict.headline.contains("last read retained"));
        }
        assert!(
            display
                .notice
                .as_ref()
                .unwrap()
                .contains("last read retained")
        );
    }
}

#[test]
fn stale_and_retained_rows_are_marked_in_every_view_and_the_summary_counts() {
    // The whole read failed: everything kept is stale.
    let inv = databases::example::shaped(at(), "stale");
    for mode in [Mode::Clusters, Mode::Backups, Mode::Schedules] {
        let display = display_of(&inv, mode);
        assert!(
            display
                .rows
                .iter()
                .all(|row| row.tone == crate::ui::Tone::Unknown),
            "{}",
            mode.label()
        );
        assert!(display.notice.is_some(), "{}", mode.label());
        assert!(
            display
                .summary
                .starts_with(&format!("{} ", display.rows.len())),
            "{}",
            display.summary
        );
    }
    // One operator refused after a good read: its rows are kept and marked.
    let inv = databases::example::shaped(at(), "engine-refused");
    assert_eq!(inv.retained, vec![databases::Engine::MariaDb]);
    let display = display_of(&inv, Mode::Clusters);
    assert_eq!(display.rows.len(), 2);
    assert!(
        display
            .notice
            .as_ref()
            .unwrap()
            .contains("last read retained")
    );
    // The kept row's Inspector says so too, not Healthy.
    let ledger = display
        .rows
        .iter()
        .find(|row| row.identity.name == "ledger-db")
        .unwrap();
    assert_eq!(ledger.verdict.tone, crate::ui::Tone::Unknown);
    assert!(ledger.verdict.headline.starts_with("Unknown"));
    assert_eq!(ledger.verdict.source, None);
    // A backup whose database could not be checked is not a pass.
    let archive = display_of(&inv, Mode::Backups);
    let archive = archive
        .rows
        .iter()
        .find(|row| row.identity.name == "archive-old")
        .unwrap();
    assert_eq!(archive.tone, crate::ui::Tone::Unknown);
}

#[gpui_kit::test]
fn the_status_segment_counts_rows_and_names_incomplete_coverage_for_a_stale_read(
    cx: &mut TestAppContext,
) {
    let (_runtime, page, window) = mount(cx, 1280.);
    cx.update_window(window.into(), |_, window, cx| {
        for mode in [Mode::Clusters, Mode::Backups, Mode::Schedules] {
            page.update(cx, |page, cx| {
                page.loader.resolve(
                    page.identity().unwrap(),
                    Ok(databases::example::shaped(at(), "stale")),
                );
                page.mode(mode, cx);
            });
            window.render_frame(cx);
            let text = page.update(cx, |page, cx| {
                page.status(cx).unwrap().text(None).to_string()
            });
            assert!(
                text.contains("coverage incomplete"),
                "{}: {text}",
                mode.label()
            );
            assert!(
                text.contains(&format!("{} ", page.read(cx).display.rows.len())),
                "{text}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn every_narrower_status_line_keeps_the_count_with_the_coverage_warning(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280.);
    cx.update_window(window.into(), |_, window, cx| {
        for shape in ["stale", "engine-refused"] {
            for mode in [Mode::Clusters, Mode::Backups, Mode::Schedules] {
                page.update(cx, |page, cx| {
                    page.loader.resolve(
                        page.identity().unwrap(),
                        Ok(databases::example::shaped(at(), shape)),
                    );
                    page.mode(mode, cx);
                });
                window.render_frame(cx);
                let (lines, count) = page.update(cx, |page, cx| {
                    let lines = page.status(cx).unwrap().lines(None);
                    (lines, page.display.summary.clone())
                });
                // The bar draws one of these lines, however narrow it is.
                for line in lines {
                    assert!(
                        line.contains(&format!("{count} · coverage incomplete")),
                        "{shape} {}: {line}",
                        mode.label()
                    );
                }
            }
        }
    })
    .unwrap();
}

#[test]
fn kept_rows_say_so_in_the_row_and_the_inspector() {
    for shape in ["stale", "engine-refused"] {
        let inv = databases::example::shaped(at(), shape);
        let clusters = display_of(&inv, Mode::Clusters);
        let ledger = clusters
            .rows
            .iter()
            .find(|row| row.identity.name == "ledger-db")
            .unwrap();
        assert_eq!(ledger.verdict.tone, crate::ui::Tone::Unknown, "{shape}");
        assert_eq!(
            ledger.verdict.headline, "Unknown · last read retained",
            "{shape}"
        );
        let kept = |mode, name: &str, cell: usize| {
            let display = display_of(&inv, mode);
            let row = display
                .rows
                .iter()
                .find(|row| row.identity.name == name)
                .unwrap();
            row.cells[cell].to_string()
        };
        assert!(
            kept(Mode::Backups, "ledger-manual", 2).ends_with(" · last read retained"),
            "{shape}"
        );
        assert!(
            kept(Mode::Schedules, "ledger-nightly", 3).ends_with(" · last read retained"),
            "{shape}"
        );
    }
    // The cluster read was refused: a backup cannot be checked against it.
    let inv = databases::example::shaped(at(), "engine-refused");
    let backups = display_of(&inv, Mode::Backups);
    let archive = backups
        .rows
        .iter()
        .find(|row| row.identity.name == "archive-old")
        .unwrap();
    assert_eq!(archive.tone, crate::ui::Tone::Unknown);
    assert!(archive.verdict.headline.contains("could not be checked"));
}
