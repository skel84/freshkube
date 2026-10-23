use super::*;

#[gpui_kit::test]
fn not_installed_refused_empty_partial_and_stale_are_distinct(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280.);
    for (shape, words) in [
        ("not-installed", "No database operator is installed"),
        ("refused", "Not permitted"),
        ("empty", "No clusters"),
        ("unsupported", "Unsupported"),
    ] {
        cx.update_window(window.into(), |_, window, cx| {
            page.update(cx, |page, cx| {
                let identity = page.identity().unwrap();
                page.loader
                    .resolve(identity, Ok(databases::example::shaped(at(), shape)));
                cx.notify();
            });
            window.render_frame(cx);
            assert!(
                window
                    .find("databases-empty-state")
                    .label()
                    .unwrap()
                    .contains(words)
            );
        })
        .unwrap();
    }
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| {
            let id = page.identity().unwrap();
            page.loader.resolve(
                id.clone(),
                Ok(databases::example::shaped(at(), "backup-refused")),
            );
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(page.read(cx).display.rows.len(), 2);
        assert!(
            window
                .find("databases-coverage")
                .label()
                .unwrap()
                .contains("Backups: Not permitted")
        );
        assert!(
            page.read(cx)
                .display
                .rows
                .iter()
                .all(|row| row.cells[3] == "Unknown")
        );
        page.update(cx, |page, cx| {
            let id = page.identity().unwrap();
            let request = page.loader.start(id);
            page.loader
                .land(&request, Err("Invented read failed".into()), cx);
            cx.notify();
        });
        window.render_frame(cx);
        window.find("databases-stale");
        assert_eq!(page.read(cx).display.rows.len(), 2);
    })
    .unwrap();
}
#[gpui_kit::test]
fn source_replacement_hide_and_new_scope_reject_old_answers(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280.);
    let mut scope_ticket = None;
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| {
            let old = page.loader.start(page.identity().unwrap());
            page.set_visible(false, window, cx);
            assert!(
                !page
                    .loader
                    .land(&old, Ok(databases::example::inventory(at())), cx)
            );
            page.set_page_source(source("replacement-access"), window, cx);
            assert!(page.loader.data().is_none());
            assert!(
                !page
                    .loader
                    .land(&old, Ok(databases::example::inventory(at())), cx)
            );
            page.set_visible(true, window, cx);
        });
        window.render_frame(cx);
        scope_ticket = Some(page.update(cx, |page, _| page.loader.start(page.identity().unwrap())));
        window.click("databases-namespace", cx);
        window.input("shop", cx);
        window.press("enter", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            assert_eq!(page.namespace.as_deref(), Some("shop"));
            let mut late = databases::example::inventory(at());
            late.databases.items[0].identity.name = "late-old-scope".into();
            assert!(!page.loader.land(&scope_ticket.unwrap(), Ok(late), cx));
            assert!(
                page.loader
                    .data()
                    .unwrap()
                    .databases
                    .items
                    .iter()
                    .all(|db| db.identity.name != "late-old-scope")
            );
        })
    });
}

#[gpui_kit::test]
fn initial_failures_offer_retry_without_claiming_retained_data(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280.);
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| {
            page.loader.reset();
            page.loader.resolve(
                page.identity().unwrap(),
                Err("Invented connection failure".into()),
            );
            let status = page.status(cx).unwrap();
            assert!(!status.text(None).contains("retained"));
            cx.notify();
        });
        window.render_frame(cx);
        window.find("databases-retry");
        window.click("databases-retry", cx);
        window.render_frame(cx);
        assert!(!page.read(cx).display.rows.is_empty());
        page.update(cx, |page, cx| {
            page.loader.resolve(
                page.identity().unwrap(),
                Ok(Inventory::unavailable(Coverage::Failed, at())),
            );
            cx.notify();
        });
        window.render_frame(cx);
        window.find("databases-retry");
        window.click("databases-retry", cx);
        window.render_frame(cx);
        assert!(!page.read(cx).display.rows.is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn read_failure_retry_is_button_sized_at_both_text_sizes(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280.);
    for text in [13., 20.] {
        for shape in [
            "not-installed",
            "refused",
            "unsupported",
            "failed",
            "backup-refused",
        ] {
            cx.update_window(window.into(), |_, window, cx| {
                crate::text_size::set(text, cx);
                page.update(cx, |page, cx| {
                    page.loader.resolve(
                        page.identity().unwrap(),
                        Ok(databases::example::shaped(at(), shape)),
                    );
                    page.mode(
                        if shape == "backup-refused" {
                            Mode::Backups
                        } else {
                            Mode::Clusters
                        },
                        cx,
                    );
                    cx.notify();
                });
                window.render_frame(cx);
                let retry = window.find("databases-retry").bounds();
                assert!(
                    retry.size.width < px(140. * text / 13.),
                    "{shape}: Retry must not stretch"
                );
                assert!(
                    retry.size.height > px(0.) && retry.size.height <= px(40. * text / 13.),
                    "{shape}: shared button height {retry:?}"
                );
                window.click("databases-retry", cx);
                window.render_frame(cx);
                assert!(
                    !page.read(cx).display.rows.is_empty(),
                    "Retry reads the fixture"
                );
            })
            .unwrap();
        }
    }
}

#[test]
fn both_operators_failing_differently_say_one_line_each() {
    let mut inv = databases::example::inventory(at());
    inv.engines = vec![
        (databases::Engine::CloudNativePg, Coverage::Complete),
        (databases::Engine::MariaDb, Coverage::Refused),
    ];
    inv.backups.coverage = Coverage::Partial;
    let display = display_of(&inv, Mode::Clusters);
    let notice = display.notice.unwrap();
    assert!(
        notice.contains("MariaDB · MariaDB operator: Not permitted"),
        "{notice}"
    );
    assert!(notice.contains("Backups: Partial / capped"), "{notice}");
    assert!(!notice.contains("PostgreSQL"), "{notice}");
}
