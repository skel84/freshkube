use super::{DatabasesScreen, Mode, PageSource, ScreenPanel};
use crate::desktop::layout_check;
use chrono::{TimeZone, Utc};
use freshkube_core::{
    cluster_source::{ClusterAccess, ClusterSource},
    databases::{self, Coverage, Inventory},
};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, InputEvent, TestAppContext, WindowHandle, px, size};
fn source(id: &str) -> PageSource {
    PageSource {
        kube: Some(ClusterSource {
            id: id.into(),
            context: "invented-db-context".into(),
            access: ClusterAccess::EXAMPLE,
        }),
        ..PageSource::default()
    }
}
fn at() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap()
}
/// The id of the row for database `name` in the current view.
fn row_id(
    page: &Entity<DatabasesScreen>,
    name: &str,
    cx: &gpui_kit::App,
) -> gpui_kit::SharedString {
    page.read(cx)
        .display
        .rows
        .iter()
        .find(|row| row.identity.name == name)
        .unwrap_or_else(|| panic!("no row {name}"))
        .id
        .clone()
}
pub(super) fn mount(
    cx: &mut TestAppContext,
    width: f32,
) -> (
    tokio::runtime::Runtime,
    Entity<DatabasesScreen>,
    WindowHandle<Root>,
) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        crate::theme::install(cx)
    });
    let mut page = None;
    let window = cx.open_window(size(px(width), px(880.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = DatabasesScreen::new(runtime.handle().clone(), window, cx);
            view.set_page_source(source("invented-access"), window, cx);
            view.set_visible(true, window, cx);
            view.activate(window, cx);
            view
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, page.unwrap(), window)
}
#[gpui_kit::test]
fn inventory_views_keyboard_inspector_and_resource_link_use_object_identity(
    cx: &mut TestAppContext,
) {
    let (_runtime, page, window) = mount(cx, 1280.);
    let links = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = links.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(
            &page,
            move |_, event: &crate::resources::ResourceLink, _| {
                seen.borrow_mut().push(event.clone())
            },
        )
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let id = row_id(&page, "orders-db", cx);
        window.click(id, cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("databases-detail-title").label(),
            Some("orders-db")
        );
        window.press("o", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let links = links.borrow();
    let crate::resources::ResourceLink::Object(kind, object, _) = &links[0] else {
        panic!("object link")
    };
    assert_eq!(kind.group, "postgresql.cnpg.io");
    assert_eq!(object.uid, "invented-orders-db");
    assert_eq!(object.connection.as_deref(), Some("invented-access"));
    cx.update_window(window.into(), |_, window, cx| {
        window.click("databases-view-backups", cx);
        window.render_frame(cx);
        assert_eq!(page.read(cx).display.rows.len(), 4);
        window.press("down", cx);
        window.render_frame(cx);
        assert!(page.read(cx).selected.is_some());
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("databases-detail").is_none());
        window.click("databases-view-schedules", cx);
        window.render_frame(cx);
        assert_eq!(page.read(cx).display.rows.len(), 2);
    })
    .unwrap();
}
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
fn loading_and_not_connected_keep_the_page_key_context(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280.);
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| {
            page.loader.reset();
            page.loader.start(page.identity().unwrap());
            cx.notify();
        });
        window.render_frame(cx);
        window.find("databases-list");
        page.update(cx, |page, cx| {
            page.set_page_source(PageSource::default(), window, cx)
        });
        window.render_frame(cx);
        window.find("databases-title");
    })
    .unwrap();
}
#[gpui_kit::test]
fn table_and_inspector_follow_shared_layout_at_wide_narrow_and_large_text(cx: &mut TestAppContext) {
    for width in [1280., 760.] {
        let (_runtime, page, window) = mount(cx, width);
        for text in [13., 20.] {
            cx.update_window(window.into(), |_, window, cx| {
                crate::text_size::set(text, cx);
                window.render_frame(cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                layout_check::settle_header(window, cx);
                window.render_frame(cx);
                layout_check::assert_table_page(
                    window,
                    cx,
                    &layout_check::TablePage {
                        page: "databases-page",
                        title: "databases-title",
                        title_text: "Databases",
                        table: "databases-table-scroll",
                        list: "databases-list",
                    },
                );
                let id = row_id(&page, "orders-db", cx);
                window.click(id, cx);
                window.render_frame(cx);
                layout_check::assert_inspector(
                    window,
                    cx,
                    "databases-split",
                    "databases-table",
                    "databases-detail",
                    "databases-detail-title",
                );
                window.press("escape", cx);
                window.render_frame(cx);
            })
            .unwrap();
        }
    }
}
fn display_of(inv: &Inventory, mode: Mode) -> super::display::Display {
    super::display::Display::new(
        Some(inv),
        &databases::coroot::Evidence::default(),
        mode,
        at(),
    )
}
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
fn shell_shortcut_opens_databases_in_the_storage_column(cx: &mut TestAppContext) {
    let (_runtime, window, _pilot) = crate::desktop::tests::fixture(cx, 1280., 880.);
    cx.update_window(window, |_, window, cx| {
        window.press("secondary-shift-d", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.find("databases-title");
        window.find("nav-databases");
    })
    .unwrap();
}

#[test]
fn a_large_cluster_keeps_the_inspector_bounded_and_discloses_omitted_evidence() {
    let mut inventory = databases::example::inventory(at());
    inventory.databases.items[0].instances = (0..500)
        .map(|index| databases::Instance::named(format!("orders-{index}")))
        .collect();
    let display = display_of(&inventory, Mode::Clusters);
    let row = display
        .rows
        .iter()
        .find(|row| row.identity.name == "orders-db")
        .unwrap();
    let total: usize = row.sections.iter().map(|s| s.fields.len()).sum();
    assert!(total <= 129, "{total}");
    let last = row.sections.last().unwrap();
    assert_eq!(last.title, "More evidence");
    assert!(
        last.fields[0]
            .value
            .contains("additional fields are not shown")
    );
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
fn escape_passes_on_only_when_no_inspector_is_selected(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280.);
    let count = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen = count.clone();
    cx.update(|cx| {
        cx.on_action(move |_: &super::Close, _| seen.set(seen.get() + 1));
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let id = row_id(&page, "orders-db", cx);
        window.click(id, cx);
        page.update(cx, |page, cx| page.focus(window, cx));
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(count.get(), 0);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(count.get(), 1);
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
fn field<'a>(
    display: &'a super::display::Display,
    name: &str,
    label: &str,
) -> &'a super::display::Field {
    display
        .rows
        .iter()
        .find(|row| row.identity.name == name)
        .unwrap()
        .sections
        .iter()
        .flat_map(|section| &section.fields)
        .find(|field| field.label == label)
        .unwrap_or_else(|| panic!("no field {label}"))
}
#[test]
fn inspector_reports_unknown_schedule_coverage_configuration_and_plain_generations() {
    let mut inv = databases::example::inventory(at());
    inv.schedules = databases::Read::empty(Coverage::Refused, at());
    inv.databases.items[0].backup_configuration = databases::Fact::Known(false);
    inv.databases.items[0].conditions[0].observed_generation = None;
    let display = display_of(&inv, Mode::Clusters);
    assert!(
        field(&display, "orders-db", "Schedules")
            .value
            .starts_with("Unknown")
    );
    assert!(
        field(&display, "orders-db", "Backup configuration")
            .value
            .starts_with("No backup")
    );
    let ready = &field(&display, "orders-db", "Ready").value;
    assert!(ready.contains("generation Not reported"), "{ready}");
    assert!(!ready.contains("Some("));
    assert!(
        field(&display, "orders-db", "Read")
            .value
            .contains(&crate::ui::clock(at().into()))
    );
}

#[gpui_kit::test]
fn collapsed_storage_keeps_databases_reachable_and_selected(cx: &mut TestAppContext) {
    for (width, text) in [(760., 13.), (1280., 20.)] {
        let (_runtime, window, _pilot) = crate::desktop::tests::fixture(cx, width, 880.);
        cx.update_window(window, |_, window, cx| {
            crate::text_size::set(text, cx);
            window.press("secondary-shift-d", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let column = window.find("nav-column").bounds();
            assert!(column.size.width < px(90.), "Storage is collapsed");
            assert_eq!(window.find("nav-databases").checked(), Some(true));
            window.click("nav-k8s-persistentvolumeclaims", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_ne!(window.find("nav-databases").checked(), Some(true));
            window.click("nav-databases", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.find("databases-title");
            assert_eq!(window.find("nav-databases").checked(), Some(true));
        })
        .unwrap();
    }
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
fn cluster_inspector_distinguishes_schedule_states_and_uses_prose() {
    for (shape, expected) in [
        ("", "Active · Daily at 02:00"),
        ("suspended", "Suspended · Daily at 02:00"),
    ] {
        let inv = databases::example::shaped(at(), shape);
        let display = display_of(&inv, Mode::Clusters);
        assert_eq!(field(&display, "orders-db", "orders-daily").value, expected);
        // The expression is the operator's, in its own dialect.
        let expression = field(&display, "orders-db", "Expression");
        assert_eq!(expression.value, "0 0 2 * * *");
        assert!(expression.monospace);
        for label in ["Freshness", "Restore prerequisites", "Backup configuration"] {
            assert!(
                !field(&display, "orders-db", label).monospace,
                "{label} is prose"
            );
        }
        assert!(field(&display, "orders-db", "Primary").monospace);
        assert!(field(&display, "orders-db", "Read").monospace);
    }
    // MariaDB's schedule has five fields.
    let inv = databases::example::inventory(at());
    let display = display_of(&inv, Mode::Clusters);
    assert_eq!(
        field(&display, "ledger-db", "ledger-nightly").value,
        "Active · Daily at 03:00"
    );
    assert_eq!(
        field(&display, "ledger-db", "Expression").value,
        "0 3 * * *"
    );
    let mut inv = databases::example::inventory(at());
    inv.schedules.items[0].suspended = databases::Fact::Unknown;
    inv.databases.items[0].current_primary = databases::Fact::Unknown;
    let display = display_of(&inv, Mode::Clusters);
    assert!(
        field(&display, "orders-db", "orders-daily")
            .value
            .starts_with("Suspension unknown")
    );
    assert!(!field(&display, "orders-db", "Primary").monospace);
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
fn every_coroot_state_keeps_the_operators_answer_and_says_so(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280.);
    for (shape, words) in [
        ("coroot-none", "names no Coroot cluster"),
        ("coroot-not-found", "Coroot lists no application"),
        (
            "coroot-unreachable",
            "The last answer is dimmed and not used",
        ),
    ] {
        cx.update_window(window.into(), |_, window, cx| {
            page.update(cx, |page, cx| {
                page.shape = shape.into();
                page.loader.reset();
                page.read(cx);
            });
            window.render_frame(cx);
            let rows = &page.read(cx).display.rows;
            // The operator calls both databases ready, and nothing worsens that.
            assert!(
                rows.iter().all(|row| row.cells[4] == "Healthy"),
                "{shape}: {:?}",
                rows.iter().map(|r| r.cells[4].clone()).collect::<Vec<_>>()
            );
            assert!(
                rows.iter()
                    .all(|row| row.chip == Some("Operator") || row.chip == Some("Both"))
            );
            let coroot = rows[0].sections.iter().find(|s| s.id == "coroot").unwrap();
            assert!(
                coroot.fields[0].value.contains(words),
                "{shape}: {}",
                coroot.fields[0].value
            );
            if shape == "coroot-unreachable" {
                // What it said before is shown, dimmed, and not used.
                assert!(coroot.fields.iter().skip(1).all(|f| f.dim));
                assert!(coroot.fields.len() > 1);
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_engine_moves_into_the_inspector_header_when_the_table_has_no_room(cx: &mut TestAppContext) {
    for (width, in_table) in [(1280., false), (1800., true)] {
        let (_runtime, page, window) = mount(cx, width);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            // Without a selection the table has the room and shows it.
            assert!(page.read(cx).display.engine_shown());
            let id = row_id(&page, "orders-db", cx);
            window.click(id, cx);
            window.render_frame(cx);
            assert_eq!(page.read(cx).display.engine_shown(), in_table, "{width}");
            let columns = &page.read(cx).display.columns.0;
            assert_eq!(columns.iter().any(|c| c.label_text() == "Engine"), in_table);
            assert_eq!(
                window.try_find("databases-detail-engine").is_some(),
                !in_table
            );
            if !in_table {
                assert_eq!(
                    window.find("databases-detail-engine").label(),
                    Some("PostgreSQL 16")
                );
            }
        })
        .unwrap();
    }
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

#[test]
fn a_coroot_app_without_a_report_is_not_called_missing() {
    use databases::coroot::{CorootState, Evidence};
    let inv = databases::example::inventory(at());
    let evidence = Evidence {
        states: inv
            .databases
            .items
            .iter()
            .map(|db| (db.identity.uid.clone(), CorootState::NoReport))
            .collect(),
    };
    let display = super::display::Display::new(Some(&inv), &evidence, Mode::Clusters, at());
    let row = display
        .rows
        .iter()
        .find(|row| row.identity.name == "orders-db")
        .unwrap();
    let value = &row
        .sections
        .iter()
        .find(|s| s.id == "coroot")
        .unwrap()
        .fields[0]
        .value;
    assert!(value.contains("has no Postgres report"), "{value}");
    assert!(!value.contains("lists no application"));
}

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
