use super::{DatabasesScreen, Mode, PageSource, ScreenPanel};
use crate::desktop::layout_check;
use chrono::{TimeZone, Utc};
use freshkube_core::{
    cluster_source::{ClusterAccess, ClusterSource},
    databases::{self, Coverage, Inventory},
};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
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
        window.click("databases-namespace", cx);
        window.input("shop", cx);
        window.press("enter", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(page.read(cx).namespace.as_deref(), Some("shop")));
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
            .1
            .contains("additional fields are not shown")
    );
}
