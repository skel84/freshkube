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
        let id = page.read(cx).display.rows[0].id.clone();
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
        assert_eq!(page.read(cx).display.rows.len(), 3);
        window.press("down", cx);
        window.render_frame(cx);
        assert!(page.read(cx).selected.is_some());
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("databases-detail").is_none());
        window.click("databases-view-schedules", cx);
        window.render_frame(cx);
        assert_eq!(page.read(cx).display.rows.len(), 1);
    })
    .unwrap();
}
#[gpui_kit::test]
fn not_installed_refused_empty_partial_and_stale_are_distinct(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280.);
    for (shape, words) in [
        ("not-installed", "not installed"),
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
        assert_eq!(page.read(cx).display.rows.len(), 1);
        assert!(
            window
                .find("databases-coverage")
                .label()
                .unwrap()
                .contains("Backups: Not permitted")
        );
        assert!(page.read(cx).display.rows[0].cells[5].contains("unknown"));
        page.update(cx, |page, cx| {
            let id = page.identity().unwrap();
            let request = page.loader.start(id);
            page.loader
                .land(&request, Err("Invented read failed".into()), cx);
            cx.notify();
        });
        window.render_frame(cx);
        window.find("databases-stale");
        assert_eq!(page.read(cx).display.rows.len(), 1);
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
                let id = page.read(cx).display.rows[0].id.clone();
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
#[test]
fn display_uses_both_engines_and_keeps_orphans_visible() {
    let inventory = databases::example::shaped(at(), "mariadb-model");
    let display = super::display::Display::new(Some(&inventory), Mode::Clusters);
    assert_eq!(display.rows.len(), 2);
    assert!(display.rows.iter().any(|row| row.cells[1] == "MariaDB"));
    let backups = super::display::Display::new(Some(&inventory), Mode::Backups);
    assert!(
        backups
            .rows
            .iter()
            .any(|row| row.cells[5].contains("missing"))
    );
    let partial = Inventory::unavailable(Coverage::Refused, at());
    assert!(
        !super::display::Display::new(Some(&partial), Mode::Backups)
            .empty
            .starts_with("No ")
    );
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
        .map(|index| databases::Instance {
            name: format!("orders-{index}"),
            role: databases::Fact::Unknown,
            state: databases::Fact::Unknown,
            replication: databases::Fact::Unknown,
        })
        .collect();
    let display = super::display::Display::new(Some(&inventory), Mode::Clusters);
    assert!(display.rows[0].fields.len() <= 129);
    assert!(
        display.rows[0]
            .fields
            .last()
            .unwrap()
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
        let id = page.read(cx).display.rows[0].id.clone();
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
                Some(row.cells[3].as_ref())
            );
        }
    })
    .unwrap();
}
#[test]
fn inspector_reports_unknown_schedule_coverage_configuration_and_plain_generations() {
    let mut inv = databases::example::inventory(at());
    inv.schedules = databases::Read::empty(Coverage::Refused, at());
    inv.databases.items[0].backup_configuration = databases::Fact::Known(false);
    inv.databases.items[0].conditions[0].observed_generation = None;
    let display = super::display::Display::new(Some(&inv), Mode::Clusters);
    let fields = &display.rows[0].fields;
    let field = |name: &str| {
        fields
            .iter()
            .find(|field| field.label == name)
            .unwrap()
            .value
            .as_str()
    };
    assert!(field("Schedules").starts_with("Unknown"));
    assert!(field("Backup configuration").starts_with("No backup"));
    assert!(field("Ready").contains("generation Not reported"));
    assert!(!field("Ready").contains("Some("));
    assert!(field("Observed").contains(&crate::ui::clock(at().into())));
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
    for (shape, expected) in [("", "1 active"), ("suspended", "1 suspended")] {
        let inv = databases::example::shaped(at(), shape);
        let display = super::display::Display::new(Some(&inv), Mode::Clusters);
        let field = |name: &str| {
            display.rows[0]
                .fields
                .iter()
                .find(|f| f.label == name)
                .unwrap()
        };
        assert!(field("Schedules").value.starts_with(expected));
        for label in [
            "Phase",
            "Backup configuration",
            "Schedules",
            "Freshness",
            "Replication",
            "Restore prerequisites",
        ] {
            assert!(!field(label).monospace, "{label} is prose");
        }
        assert!(field("Current primary").monospace);
        assert!(field("Observed").monospace);
    }
    let mut inv = databases::example::inventory(at());
    inv.schedules.items[0].suspended = databases::Fact::Unknown;
    inv.databases.items[0].current_primary = databases::Fact::Unknown;
    let display = super::display::Display::new(Some(&inv), Mode::Clusters);
    assert!(
        display.rows[0]
            .fields
            .iter()
            .any(|f| f.label == "Schedules" && f.value.starts_with("1 suspension unknown"))
    );
    assert!(
        !display.rows[0]
            .fields
            .iter()
            .find(|f| f.label == "Current primary")
            .unwrap()
            .monospace
    );
}

#[gpui_kit::test]
fn inspector_narrows_schedule_header_with_ellipsis_and_full_tooltip(cx: &mut TestAppContext) {
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
        window.click(
            "databases-row-ScheduledBackup-shop-invented-orders-daily",
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let header = window.find((gpui_kit::SharedString::from("databases-sort"), 5_usize));
        assert_eq!(header.label(), Some("Suspended"));
        let text = freshkube_ui::table::word_cut_shown("databases-sort-5-label").unwrap();
        assert!(
            text.ends_with('…'),
            "header must end with ellipsis: {text:?}"
        );
        let visible = header
            .bounds()
            .intersect(&window.find("databases-table").bounds());
        assert!(visible.size.width > px(0.));
        window.dispatch_event(
            gpui_kit::MouseMoveEvent {
                position: visible.center(),
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
        assert!(freshkube_ui::tooltip::drawn("Suspended") > 0);
    })
    .unwrap();
}
