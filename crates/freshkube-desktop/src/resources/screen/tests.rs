use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Bounds, Entity, Pixels, TestAppContext, px, size};
use tokio::runtime::Runtime;

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use freshkube_core::resources::ResourceKind;

use super::{
    KEYBOARD_PAUSE, KubeAccess, KubeSource, ListView, NotServed, ResourcesScreen, WATCH_COALESCE,
    row_id,
};
use crate::resources::example;
use crate::resources::model::{ReadState, ResourceIdentity, ResourceRow};
use crate::resources::store::ResourceEvent;

/// A built-in or example custom kind by its kubectl key.
fn kind(key: &str) -> ResourceKind {
    example::kind(key).unwrap()
}

fn source(context: &str) -> KubeSource {
    KubeSource {
        id: example::connection(context),
        context: context.into(),
        access: KubeAccess::Example,
    }
}

/// The page as the shell shows it: example data for `context`, visible.
fn mount(
    cx: &mut TestAppContext,
    context: Option<&str>,
) -> (Runtime, Entity<ResourcesScreen>, AnyWindowHandle) {
    mount_sized(cx, context, 1280.)
}

fn mount_sized(
    cx: &mut TestAppContext,
    context: Option<&str>,
    width: f32,
) -> (Runtime, Entity<ResourcesScreen>, AnyWindowHandle) {
    mount_window(cx, context, width, 800.)
}

fn mount_window(
    cx: &mut TestAppContext,
    context: Option<&str>,
    width: f32,
    height: f32,
) -> (Runtime, Entity<ResourcesScreen>, AnyWindowHandle) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        cx.set_reduce_motion(true);
    });
    let runtime = Runtime::new().unwrap();
    let source = context.map(source);
    let mut screen = None;
    let handle = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = ResourcesScreen::new(runtime.handle().clone(), window, cx);
            view.set_source(source, window, cx);
            view.set_visible(true, window, cx);
            // Most tests read the rows in one sorted list; the problems
            // view has tests of its own.
            view.set_list_view(ListView::All, cx);
            view
        });
        screen = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, screen.unwrap(), handle.into())
}

fn identity_at(
    screen: &Entity<ResourcesScreen>,
    ix: usize,
    cx: &gpui_kit::App,
) -> ResourceIdentity {
    let screen = screen.read(cx);
    screen
        .projection
        .row(&screen.store, ix)
        .unwrap()
        .identity
        .clone()
}

fn selected(screen: &Entity<ResourcesScreen>, cx: &gpui_kit::App) -> Option<ResourceIdentity> {
    screen.read(cx).projection.selected().cloned()
}

fn shown(screen: &Entity<ResourcesScreen>, cx: &gpui_kit::App) -> Option<ResourceIdentity> {
    screen.read(cx).detail.read(cx).target_identity().cloned()
}

/// Applies events as if the current read delivered them.
fn deliver(screen: &Entity<ResourcesScreen>, events: Vec<ResourceEvent>, cx: &mut gpui_kit::App) {
    screen.update(cx, |screen, cx| {
        let epoch = screen.store.epoch();
        screen.apply(epoch, vec![events], cx);
    });
}

#[gpui_kit::test]
fn header_controls_share_the_title_row_at_both_widths(cx: &mut TestAppContext) {
    for width in [1280., 760.] {
        let (_runtime, _screen, handle) = mount_sized(cx, Some("homelab"), width);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let title = window.find("resource-title").bounds();
            let namespace = window.find("resource-namespace").bounds();
            let filter = window.find("resource-filter").bounds();
            let refresh = window.find("resource-refresh").bounds();
            for control in [namespace, filter, refresh] {
                assert!(control.left() >= px(0.), "{width}: {control:?}");
                assert!(control.right() <= px(width), "{width}: {control:?}");
            }
            assert!(title.right() <= filter.left());
            assert!(namespace.right() <= refresh.left());
            // Centred on one line, to layout's rounding.
            let level = |a: Bounds<Pixels>, b: Bounds<Pixels>| {
                (a.center().y - b.center().y).abs() < px(0.5)
            };
            assert!(level(namespace, refresh), "{namespace:?} {refresh:?}");
            assert!(filter.size.width >= px(120.), "{width}: {filter:?}");
            assert!(level(namespace, filter), "{namespace:?} {filter:?}");
            assert!(filter.right() <= namespace.left());
            assert!(window.find("resource-list").bounds().top() > refresh.bottom());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn pods_list_every_namespace_and_select_by_identity(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(screen.read(cx).store.len(), 22);
        assert!(window.find("resource-list").visible());
        // The glyph first, which doesn't sort; then the name, after its
        // namespace when listing every one, and its Logs button, which
        // doesn't sort either; then the containers, readiness, use,
        // restarts, owner, the node and age. IP stays left out.
        assert!(window.try_find(("resource-sort", 0usize)).is_none());
        assert!(window.try_find(("resource-sort", 2usize)).is_none());
        let labels: Vec<String> = (1..11usize)
            .filter(|ix| *ix != 2)
            .map(|ix| {
                window
                    .find(("resource-sort", ix))
                    .label()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(
            labels,
            [
                "Name",
                "Containers",
                "Ready",
                "CPU",
                "Memory",
                "Restarts",
                "Owner",
                "Node",
                "Age"
            ]
        );
        assert!(window.try_find(("resource-sort", 11usize)).is_none());

        let third = identity_at(&screen, 2, cx);
        window.within(row_id(&third)).click("name", cx);
        window.render_frame(cx);
        assert_eq!(selected(&screen, cx), Some(third.clone()));
        assert_eq!(window.find(row_id(&third)).selected(), Some(true));
        // A click focuses the list, so arrows move on from the row.
        window.press("down", cx);
        window.render_frame(cx);
        let fourth = identity_at(&screen, 3, cx);
        assert_eq!(selected(&screen, cx), Some(fourth.clone()));

        // Sorting moves rows, not the selection.
        window.click(("resource-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("resource-sort", 1usize)).label(),
            Some("Name, sorted ascending")
        );
        assert_eq!(selected(&screen, cx), Some(fourth.clone()));
        assert_eq!(window.find(row_id(&fourth)).selected(), Some(true));
        assert_ne!(identity_at(&screen, 3, cx), fourth);
        window.click(("resource-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("resource-sort", 1usize)).label(),
            Some("Name, sorted descending")
        );

        // Refresh runs ⌘R's action, which the shell handles by listing
        // again; mounted alone, the screen lists again itself.
        let refreshed = Rc::new(Cell::new(0));
        let count = refreshed.clone();
        cx.on_action(move |_: &freshkube_ui::page::Refresh, _| count.set(count.get() + 1));
        window.click("resource-refresh", cx);
        assert_eq!(refreshed.get(), 1);
        // Listing again finds the same object and selects it again.
        screen.update(cx, |screen, cx| screen.refresh(window, cx));
        window.render_frame(cx);
        assert_eq!(selected(&screen, cx), Some(fourth.clone()));
        assert_eq!(window.find(row_id(&fourth)).selected(), Some(true));

        window.press("end", cx);
        window.render_frame(cx);
        assert_eq!(selected(&screen, cx), Some(identity_at(&screen, 21, cx)));
        window.press("home", cx);
        window.render_frame(cx);
        assert_eq!(selected(&screen, cx), Some(identity_at(&screen, 0, cx)));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_namespace_narrows_namespaced_kinds_and_persists_across_kinds(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("resource-namespace").click("input", cx);
        window.render_frame(cx);
        window.input("payments", cx);
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let view = screen.read(cx);
        assert_eq!(view.namespace.as_deref(), Some("payments"));
        assert_eq!(view.store.len(), 6);
        assert!(
            view.store
                .entries()
                .iter()
                .all(|entry| entry.row().identity.namespace == "payments")
        );
        // One namespace isn't repeated before every name.
        assert!(!view.layout.namespaced);
        assert_eq!(
            window.find(("resource-sort", 3usize)).label(),
            Some("Containers")
        );

        // Cluster-scoped kinds have no namespace picker and list all.
        screen.update(cx, |screen, cx| screen.set_kind(kind("nodes"), window, cx));
        window.render_frame(cx);
        assert!(window.try_find("resource-namespace").is_none());
        assert!(!screen.read(cx).store.is_empty());
        assert!(window.find("resource-list").visible());

        // Back on a namespaced kind, the namespace still applies.
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("deployments.apps"), window, cx)
        });
        window.render_frame(cx);
        assert!(window.find("resource-namespace").visible());
        assert_eq!(screen.read(cx).store.len(), 3);
        // A glyph from the replicas ready, then the name without its
        // namespace.
        assert!(!screen.read(cx).layout.namespaced);
        assert_eq!(window.find(("resource-sort", 1usize)).label(), Some("Name"));
        assert_eq!(
            window.find(("resource-sort", 2usize)).label(),
            Some("Ready")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_filter_narrows_rows_and_escape_clears_it(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    // Typing reaches the screen through the input's change event, which
    // is delivered once the update ends.
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    step(cx, &|window, cx| {
        screen.update(cx, |screen, cx| screen.focus(window, cx));
    });
    step(cx, &|window, cx| window.press("/", cx));
    step(cx, &|window, cx| window.input("coredns", cx));
    step(cx, &|window, cx| {
        assert_eq!(screen.read(cx).projection.len(), 2);
        assert_eq!(screen.read(cx).store.len(), 22);
        window.input("-nothing-", cx);
    });
    step(cx, &|window, cx| {
        assert!(window.find("resource-empty").visible());
        // Enter hands the keyboard back to the list; Escape clears.
        window.press("enter", cx);
    });
    step(cx, &|window, cx| window.press("escape", cx));
    step(cx, &|window, cx| {
        assert_eq!(screen.read(cx).projection.len(), 22);
        assert!(window.try_find("resource-empty").is_none());
    });
}

#[gpui_kit::test]
fn reads_show_refusals_failures_and_stale_rows(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // A broken watch keeps the rows it had and says so.
        deliver(
            &screen,
            vec![ResourceEvent::Read(ReadState::Failed(
                "Unreachable · connection reset".into(),
            ))],
            cx,
        );
        window.render_frame(cx);
        assert!(window.find("resource-stale").visible());
        assert!(window.find("resource-list").visible());
        window.click("resource-stale-retry", cx);
        window.render_frame(cx);
        assert!(window.try_find("resource-stale").is_none());
        assert_eq!(screen.read(cx).store.len(), 22);

        // Kinds the example lacks say so rather than listing nothing.
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("configmaps"), window, cx)
        });
        window.render_frame(cx);
        assert!(window.find("resource-not-in-example").visible());

        // Without rows, a refusal and a failure each replace the table.
        deliver(
            &screen,
            vec![ResourceEvent::Read(ReadState::Refused(
                "configmaps is forbidden".into(),
            ))],
            cx,
        );
        window.render_frame(cx);
        assert!(window.find("resource-refused").visible());
        assert!(window.try_find("resource-list").is_none());
        deliver(
            &screen,
            vec![ResourceEvent::Read(ReadState::Failed("Timeout".into()))],
            cx,
        );
        window.render_frame(cx);
        assert!(window.find("resource-failed").visible());
        // A batch from an earlier read changes nothing.
        screen.update(cx, |screen, cx| {
            let epoch = screen.store.epoch() - 1;
            screen.apply(
                epoch,
                vec![vec![ResourceEvent::Read(ReadState::Loaded)]],
                cx,
            );
        });
        window.render_frame(cx);
        assert!(window.find("resource-failed").visible());
        window.click("resource-retry", cx);
        window.render_frame(cx);
        assert!(window.find("resource-not-in-example").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_kind_no_longer_served_says_so_and_closes_its_details(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let missing = Rc::new(RefCell::new(Vec::new()));
    let sink = missing.clone();
    cx.update(|cx| {
        cx.subscribe(&screen, move |_, NotServed(kind), _| {
            sink.borrow_mut().push(kind.key())
        })
        .detach()
    });
    cx.update_window(handle, |_, window, cx| {
        // A custom kind lists through the same table.
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("certificates.cert-manager.io"), window, cx)
        });
        window.render_frame(cx);
        assert_eq!(screen.read(cx).title(), "Certificate");
        assert_eq!(screen.read(cx).noun(), "certificates");
        assert_eq!(screen.read(cx).store.len(), 5);
        let first = identity_at(&screen, 0, cx);
        window.within(row_id(&first)).click("name", cx);
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), Some(first));
        assert!(window.find("resource-drawer").visible());
    })
    .unwrap();

    // The definition was removed while the page watched it.
    let gone = || {
        vec![ResourceEvent::Read(ReadState::Missing(
            "the server could not find the requested resource".into(),
        ))]
    };
    cx.update_window(handle, |_, window, cx| {
        deliver(&screen, gone(), cx);
        window.render_frame(cx);
        assert!(window.find("resource-not-served").visible());
        assert!(window.try_find("resource-list").is_none());
        assert!(window.try_find("resource-detail").is_none());
        assert_eq!(shown(&screen, cx), None);
    })
    .unwrap();
    // Events reach subscribers once the update is done.
    assert_eq!(*missing.borrow(), ["certificates.cert-manager.io"]);
    // Saying it again isn't news.
    cx.update(|cx| deliver(&screen, gone(), cx));
    assert_eq!(missing.borrow().len(), 1);

    cx.update_window(handle, |_, window, cx| {
        // Retry reads again; here the example still serves it.
        window.click("resource-missing-retry", cx);
        window.render_frame(cx);
        assert!(window.try_find("resource-not-served").is_none());
        assert_eq!(screen.read(cx).store.len(), 5);
        assert_eq!(
            screen.read(cx).not_served("homelab"),
            "The API server of homelab doesn't serve certificates at cert-manager.io/v1. \
             Its definition may have been removed, or that version is no longer served."
        );

        // A built-in kind has no definition to remove.
        let bindings = "validatingadmissionpolicybindings.admissionregistration.k8s.io";
        screen.update(cx, |screen, cx| screen.set_kind(kind(bindings), window, cx));
        deliver(&screen, gone(), cx);
        window.render_frame(cx);
        assert!(window.find("resource-not-served").visible());
        assert_eq!(
            screen.read(cx).not_served("homelab"),
            "The API server of homelab doesn't serve validating admission policy bindings \
             at admissionregistration.k8s.io/v1. The server may predate that version, or \
             no longer serve it."
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_watch_burst_applies_at_most_once_per_coalesce_window(cx: &mut TestAppContext) {
    let (_runtime, screen, _handle) = mount(cx, Some("homelab"));
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    let _task = screen.update(cx, |screen, cx| {
        let epoch = screen.store.epoch();
        screen.receive(epoch, receiver, cx)
    });
    let rows = |cx: &mut TestAppContext| screen.read_with(cx, |screen, _| screen.store.len());
    let upsert = |n| vec![ResourceEvent::Upsert(example::inserted_pod(n))];
    let listed = rows(cx);

    // After a quiet spell a change shows at once.
    sender.try_send(upsert(1)).unwrap();
    cx.run_until_parked();
    assert_eq!(rows(cx), listed + 1);

    // More within the window wait for it, then apply together.
    sender.try_send(upsert(2)).unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(WATCH_COALESCE / 2);
    sender.try_send(upsert(3)).unwrap();
    cx.run_until_parked();
    assert_eq!(rows(cx), listed + 1);
    cx.executor().advance_clock(WATCH_COALESCE / 2);
    cx.run_until_parked();
    assert_eq!(rows(cx), listed + 3);

    cx.executor().advance_clock(WATCH_COALESCE * 2);
    sender.try_send(upsert(4)).unwrap();
    cx.run_until_parked();
    assert_eq!(rows(cx), listed + 4);
}

#[gpui_kit::test]
fn only_a_visible_connected_page_reads(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("resource-disconnected").visible());

        screen.update(cx, |screen, cx| {
            screen.set_visible(false, window, cx);
            screen.set_source(Some(source("homelab")), window, cx);
        });
        window.render_frame(cx);
        assert!(screen.read(cx).store.is_empty());
        // The table's loading rows, under the header the kind will keep.
        assert!(
            window
                .within("resource-list")
                .find("resource-loading")
                .visible()
        );
        window.find(("resource-sort", 1usize));

        screen.update(cx, |screen, cx| screen.set_visible(true, window, cx));
        window.render_frame(cx);
        assert_eq!(screen.read(cx).store.len(), 22);
        assert!(window.try_find("resource-loading").is_none());
        let first = identity_at(&screen, 0, cx);
        window.within(row_id(&first)).click("name", cx);

        // Another connection keeps nothing from the last one.
        screen.update(cx, |screen, cx| {
            screen.set_source(Some(source("staging-eu")), window, cx)
        });
        window.render_frame(cx);
        assert_eq!(screen.read(cx).store.len(), 48);
        assert_eq!(selected(&screen, cx), None);
        assert!(window.try_find(row_id(&first)).is_none());
    })
    .unwrap();
}

/// How many sortable header cells the list draws.
fn header_cells(window: &gpui_kit::Window) -> usize {
    (0usize..40)
        .filter(|ix| window.try_find(("resource-sort", *ix)).is_some())
        .count()
}

#[gpui_kit::test]
fn the_loading_rows_give_way_to_a_refusal_or_a_failure(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, None);
    cx.update_window(handle, |_, window, cx| {
        for (connection, state, shown) in [
            (
                "homelab",
                ReadState::Refused("pods is forbidden".into()),
                "resource-refused",
            ),
            (
                "staging-eu",
                ReadState::Failed("Timeout".into()),
                "resource-failed",
            ),
        ] {
            // Hidden, the page waits for its first list.
            screen.update(cx, |screen, cx| {
                screen.set_visible(false, window, cx);
                screen.set_source(Some(source(connection)), window, cx);
            });
            window.render_frame(cx);
            assert!(window.find("resource-loading").visible());

            deliver(&screen, vec![ResourceEvent::Read(state)], cx);
            window.render_frame(cx);
            assert!(window.find(shown).visible(), "{shown}");
            assert!(window.try_find("resource-loading").is_none(), "{shown}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_relist_of_the_same_kind_keeps_its_header_over_the_loading_rows(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("deployments.apps"), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(screen.read(cx).kind, kind("deployments.apps"));
        assert!(!screen.read(cx).store.is_empty());
        let listed = header_cells(window);
        assert!(listed > 2, "{listed}");

        // Refresh and a namespace change start again from no rows; the
        // header stays the kind's while they wait.
        screen.update(cx, |screen, cx| {
            screen.set_visible(false, window, cx);
            screen.restart(window, cx);
        });
        window.render_frame(cx);
        assert!(window.find("resource-loading").visible());
        assert_eq!(header_cells(window), listed);

        // Another kind waits under Name and Age.
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("statefulsets.apps"), window, cx)
        });
        window.render_frame(cx);
        assert!(window.find("resource-loading").visible());
        assert!(header_cells(window) < listed);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_clicked_row_shows_its_details_in_the_drawer_over_the_list(cx: &mut TestAppContext) {
    // Here the page is the whole window: 1280 has room for the drawer
    // beside 280 of list, and the drawer takes a page under 600 whole.
    for (width, full) in [(1280., false), (560., true)] {
        let (_runtime, screen, handle) = mount_sized(cx, Some("homelab"), width);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("resource-drawer").is_none());
            let third = identity_at(&screen, 2, cx);
            window.within(row_id(&third)).click("name", cx);
            window.render_frame(cx);
            assert_eq!(shown(&screen, cx), Some(third.clone()));
            assert_eq!(
                window.find("detail-title").label(),
                Some(third.address().as_str())
            );
            // The list keeps its whole width under the drawer.
            let body = window.find("resource-split").bounds();
            let list = window.find("resource-list-area").bounds();
            assert_eq!(list.size.width, body.size.width);
            let drawn = crate::desktop::layout_check::assert_drawer(
                window,
                cx,
                "resource-split",
                "resource-drawer",
                "detail-inspector",
                "detail-title",
            );
            let page = body.size.width / crate::ui::dp_px(1., window);
            let fit = freshkube_ui::drawer::fit(freshkube_ui::drawer::WIDTH, page);
            assert_eq!(fit.full, full, "{width}: a page {page} wide");
            assert!(
                (drawn - fit.width).abs() <= 1.,
                "{width}: {drawn} for {fit:?}"
            );

            // With no filter to clear, Escape closes the drawer and drops
            // the selection it showed.
            window.press("escape", cx);
            window.render_frame(cx);
            assert!(window.try_find("resource-drawer").is_none());
            assert_eq!(shown(&screen, cx), None);
            assert_eq!(selected(&screen, cx), None);
        })
        .unwrap();
    }
}

/// A click on the list that isn't on a row closes the drawer; one on
/// another row swaps its object in the same drawer, without closing it.
#[gpui_kit::test]
fn a_click_beside_the_rows_closes_the_drawer_and_a_row_swaps_it(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount_window(cx, Some("homelab"), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first(&screen, window, cx);
        let pane = screen.read(cx).detail.clone();
        let second = identity_at(&screen, 1, cx);
        let closes = Rc::new(RefCell::new(0));
        let _closed = {
            let closes = closes.clone();
            cx.observe(&pane, move |pane, cx| {
                if pane.read(cx).target_identity().is_none() {
                    *closes.borrow_mut() += 1;
                }
            })
        };
        window.within(row_id(&second)).click("name", cx);
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), Some(second.clone()));
        assert_eq!(*closes.borrow(), 0, "the swap closed the drawer");
        assert!(window.try_find("resource-drawer").is_some());

        // A click inside the drawer leaves it open.
        window.click("detail-title", cx);
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), Some(second.clone()));

        // Under the last row, the list is empty space: a click there
        // closes the drawer, keeping the row selected for Enter.
        let list = window.find("resource-list-area").bounds();
        window.click_at(
            "resource-list-area",
            gpui_kit::point(px(40.), list.size.height - px(10.)),
            cx,
        );
        window.render_frame(cx);
        assert!(window.try_find("resource-drawer").is_none());
        assert_eq!(shown(&screen, cx), None);
        assert_eq!(selected(&screen, cx), Some(second.clone()));
        window.press("enter", cx);
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), Some(second));
    })
    .unwrap();
}

/// A control in the list, such as a group's Expand or the footer's Show
/// all, is no click beside the rows: the drawer stays open on its object.
#[gpui_kit::test]
fn the_lists_own_buttons_leave_the_drawer_open(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount_window(cx, Some("homelab"), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        problems(&screen, cx);
        open_first(&screen, window, cx);
        let first = shown(&screen, cx).expect("a row opened");
        let drawer = window.find("resource-drawer").bounds();
        for button in [
            "resource-group-healthy-toggle",
            "resource-group-healthy-toggle",
            "resource-show-all",
        ] {
            assert!(
                window.find(button).bounds().right() < drawer.left(),
                "{button}"
            );
            window.click(button, cx);
            window.render_frame(cx);
            assert_eq!(shown(&screen, cx), Some(first.clone()), "{button}");
        }
        // Show all did what it does.
        assert_eq!(screen.read(cx).list_view, ListView::All);
        // A column header sorts, and leaves the drawer open too.
        window.click(("resource-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), Some(first));
    })
    .unwrap();
}

/// Closed, the drawer keeps its row selected: the arrows move the
/// selection without opening it, Escape clears it, and only Enter or a
/// click opens it again.
#[gpui_kit::test]
fn a_closed_drawer_stays_closed_while_the_selection_moves(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount_window(cx, Some("homelab"), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first(&screen, window, cx);
        // A click under the rows closes the drawer and keeps the row.
        let list = window.find("resource-list-area").bounds();
        window.click_at(
            "resource-list-area",
            gpui_kit::point(px(40.), list.size.height - px(10.)),
            cx,
        );
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), None);
        assert_eq!(selected(&screen, cx), Some(identity_at(&screen, 0, cx)));
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(selected(&screen, cx), Some(identity_at(&screen, 1, cx)));
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(selected(&screen, cx), Some(identity_at(&screen, 2, cx)));
    })
    .unwrap();
    // The keyboard pause passes; nothing opens.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), None);
        assert!(window.try_find("resource-drawer").is_none());
        window.press("escape", cx);
        window.render_frame(cx);
        assert_eq!(selected(&screen, cx), None);
        window.press("down", cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), Some(identity_at(&screen, 0, cx)));
    })
    .unwrap();
}

/// The drawer's bounds come from the list's width as last drawn. When the
/// window narrows, the list measures itself while drawing and asks for one
/// more frame, which fits the drawer to it without any input, and then
/// asks for no more.
#[gpui_kit::test]
fn the_measured_width_settles_without_input(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount_window(cx, Some("homelab"), 1280., 880.);
    let left = |window: &mut gpui_kit::Window| {
        let body = window.find("resource-split").bounds();
        let drawer = window.find("resource-drawer").bounds();
        (drawer.left() - body.left()) / crate::ui::dp_px(1., window)
    };
    cx.update_window(handle, |_, window, cx| {
        open_first(&screen, window, cx);
        while window.simulate_next_frame(cx) > 0 {
            window.render_frame(cx);
        }
    })
    .unwrap();
    cx.simulate_window_resize(handle, size(px(800.), px(880.)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The list measured its new width and asked for a frame.
        assert_eq!(window.simulate_next_frame(cx), 1, "no frame asked for");
        window.render_frame(cx);
        assert!(
            left(window) >= freshkube_ui::drawer::LIST_KEEPS - 1.,
            "{}",
            left(window)
        );
        // Settled: the next draw asks for nothing.
        assert_eq!(window.simulate_next_frame(cx), 0);
    })
    .unwrap();
}

/// The page's toolbar is above the drawer, not under it: typing a filter
/// keeps the drawer open.
#[gpui_kit::test]
fn the_drawer_starts_under_the_page_toolbar(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount_window(cx, Some("homelab"), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first(&screen, window, cx);
        let drawer = window.find("resource-drawer").bounds();
        let filter = window.find("resource-filter").bounds();
        assert!(drawer.top() >= filter.bottom(), "{filter:?} {drawer:?}");
        window.click("resource-filter", cx);
        window.render_frame(cx);
        assert!(window.try_find("resource-drawer").is_some());
    })
    .unwrap();
}

fn open_first(
    screen: &Entity<ResourcesScreen>,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
) {
    window.render_frame(cx);
    let first = identity_at(screen, 0, cx);
    window.within(row_id(&first)).click("name", cx);
    window.render_frame(cx);
}

/// The drawer's width is the page's, for every kind: a drag of its left
/// edge sets it within its bounds, and it comes back when the app opens
/// again.
#[gpui_kit::test]
fn the_drawer_width_follows_its_edge_and_survives_reopening(cx: &mut TestAppContext) {
    use crate::navigation_file::NavigationFile;
    use freshkube_ui::split_size::SizeStore as _;
    let directory = std::env::temp_dir().join(format!(
        "freshkube-resources-width-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let preferences = directory.join("preferences.json");
    cx.update(|cx| NavigationFile::open(Some(&preferences)).install(cx));
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        open_first(&screen, window, cx);
        let drawer = window.find("resource-drawer").bounds();
        // Drag the left edge so the drawer is 560 dp wide.
        let from = gpui_kit::point(drawer.left() + px(2.), drawer.center().y);
        let to = drawer.right() - crate::ui::dp_px(560., window);
        window.drag(from, gpui_kit::point(to, from.y), cx);
        window.render_frame(cx);
        let width = window.find("resource-drawer").bounds().size.width;
        let expected = crate::ui::dp_px(560., window);
        assert!(
            (width - expected).abs() <= px(2.),
            "{width:?}, {expected:?}"
        );
        // Dragged past its bounds, it stops at them.
        screen.update(cx, |screen, cx| screen.resize_drawer(50., window, cx));
        assert_eq!(
            screen.read(cx).drawer.size(),
            freshkube_ui::drawer::MIN_WIDTH
        );
        screen.update(cx, |screen, cx| screen.resize_drawer(560., window, cx));
    })
    .unwrap();
    cx.executor()
        .advance_clock(freshkube_ui::split_size::SAVE_DELAY * 2);
    cx.run_until_parked();
    let reopened = NavigationFile::open(Some(&preferences));
    assert_eq!(
        reopened.size(crate::navigation_file::DRAWER_WIDTH),
        Some(560.)
    );
    assert_eq!(
        reopened.size(freshkube_ui::inspector::width_key("resources")),
        None
    );

    cx.update(|cx| reopened.install(cx));
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        open_first(&screen, window, cx);
        let width = window.find("resource-drawer").bounds().size.width;
        let expected = crate::ui::dp_px(560., window);
        assert!(
            (width - expected).abs() <= px(1.),
            "{width:?}, {expected:?}"
        );
    })
    .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

/// A window too narrow for the width the user gave shows the drawer
/// narrower and writes nothing: a wider window gets the width back.
#[gpui_kit::test]
fn a_narrow_window_bounds_the_drawer_without_saving(cx: &mut TestAppContext) {
    use freshkube_ui::split_size::{MemorySizes, SizeStore as _, set_store};
    let sizes = std::rc::Rc::new(MemorySizes::with(
        crate::navigation_file::DRAWER_WIDTH,
        900.,
    ));
    cx.update(|cx| set_store(sizes.clone(), cx));
    let (_runtime, screen, handle) = mount_window(cx, Some("homelab"), 900., 700.);
    cx.update_window(handle, |_, window, cx| {
        open_first(&screen, window, cx);
        let width = window.find("resource-drawer").bounds().size.width;
        assert!(width < crate::ui::dp_px(900., window), "{width:?}");
    })
    .unwrap();
    cx.executor()
        .advance_clock(freshkube_ui::split_size::SAVE_DELAY * 2);
    cx.run_until_parked();
    assert_eq!(sizes.saves(), 0);
    assert_eq!(sizes.size(crate::navigation_file::DRAWER_WIDTH), Some(900.));
    cx.update(|cx| assert_eq!(screen.read(cx).drawer.size(), 900.));
}

#[gpui_kit::test]
fn arrow_keys_show_the_next_row_once_the_keyboard_pauses(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let first = identity_at(&screen, 0, cx);
        window.within(row_id(&first)).click("name", cx);
        window.render_frame(cx);
        assert!(window.try_find("detail-state").is_none());

        window.press("down", cx);
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), Some(identity_at(&screen, 2, cx)));
        assert_eq!(window.find("detail-state").label(), Some("Reading"));
        assert!(window.find("resource-drawer").visible());
    })
    .unwrap();
    cx.executor().advance_clock(KEYBOARD_PAUSE);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("detail-state").is_none());
        assert!(window.find("detail-overview").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_deleted_row_marks_its_details_and_offers_the_new_object(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let successor = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let first = identity_at(&screen, 0, cx);
            let row = screen.read(cx).store.get(&first).cloned().unwrap();
            window.within(row_id(&first)).click("name", cx);
            deliver(&screen, vec![ResourceEvent::Delete(first.clone())], cx);
            window.render_frame(cx);
            assert!(window.find("detail-deleted").visible());
            assert_eq!(window.find("detail-state").label(), Some("Deleted"));
            assert!(window.try_find("detail-open-recreated").is_none());

            // The same name again, with a new UID: another object.
            let successor = ResourceIdentity {
                uid: "recreated".into(),
                ..first.clone()
            };
            let recreated = ResourceRow {
                identity: successor.clone(),
                ..row
            };
            deliver(&screen, vec![ResourceEvent::Upsert(recreated)], cx);
            window.render_frame(cx);
            assert_eq!(shown(&screen, cx), Some(first));
            window.click("detail-open-recreated", cx);
            successor
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        assert_eq!(selected(&screen, cx), Some(successor.clone()));
        assert_eq!(shown(&screen, cx), Some(successor));
    })
    .unwrap();
}

#[gpui_kit::test]
fn another_kind_namespace_or_connection_closes_the_details(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        let open_first = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            window.render_frame(cx);
            window
                .within(row_id(&identity_at(&screen, 0, cx)))
                .click("name", cx);
            window.render_frame(cx);
            assert!(window.find("resource-detail").visible());
        };
        open_first(window, cx);
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("deployments.apps"), window, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("resource-detail").is_none());
        assert_eq!(shown(&screen, cx), None);

        open_first(window, cx);
        screen.update(cx, |screen, cx| {
            screen.set_namespace(Some("payments".into()), window, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("resource-detail").is_none());

        // Another snapshot of the same access session keeps it open.
        open_first(window, cx);
        screen.update(cx, |screen, cx| {
            screen.set_source(Some(source("homelab")), window, cx)
        });
        window.render_frame(cx);
        assert!(window.find("resource-detail").visible());
        screen.update(cx, |screen, cx| {
            screen.set_source(Some(source("staging-eu")), window, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("resource-detail").is_none());
        assert_eq!(shown(&screen, cx), None);
    })
    .unwrap();
}

#[gpui_kit::test]
fn access_replacement_rejects_old_batches_with_the_same_object_name_and_uid(
    cx: &mut TestAppContext,
) {
    use crate::resources::direct::DirectAccess;
    let (_runtime, screen, handle) = mount(cx, None);
    let source = || {
        let access = DirectAccess::new(
            Vec::new(),
            "homelab".into(),
            freshkube_core::ConfigurationRevision::default(),
        );
        KubeSource {
            id: access.id(),
            context: "homelab".into(),
            access: KubeAccess::Direct(access),
        }
    };
    let previous = source();
    let current = source();
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            // Feed controlled completions without making network requests.
            screen.set_visible(false, window, cx);
            screen.set_source(Some(previous.clone()), window, cx);
            let (columns, mut rows) =
                example::read("homelab", "pods", None, super::live::now()).unwrap();
            rows.truncate(1);
            rows[0].identity.connection = previous.id.clone();
            let old_identity = rows[0].identity.clone();
            let old_epoch = screen.store.epoch();
            screen.apply(
                old_epoch,
                vec![vec![ResourceEvent::reset(columns.clone(), rows.clone())]],
                cx,
            );
            screen
                .projection
                .select_identity(&screen.store, &old_identity);
            screen.set_source(Some(previous.clone()), window, cx);
            assert_eq!(screen.store.epoch(), old_epoch);
            assert_eq!(screen.projection.selected(), Some(&old_identity));
            screen.set_source(Some(current.clone()), window, cx);
            let epoch = screen.store.epoch();
            assert_ne!(epoch, old_epoch);
            assert!(screen.store.is_empty());
            screen.apply(
                old_epoch,
                vec![vec![ResourceEvent::reset(columns.clone(), rows.clone())]],
                cx,
            );
            assert!(
                screen.store.is_empty(),
                "a late reset must not repopulate the next session"
            );
            assert!(screen.projection.selected().is_none());
            rows[0].identity.connection = current.id.clone();
            let new_identity = rows[0].identity.clone();
            assert_eq!(old_identity.name, new_identity.name);
            assert_eq!(old_identity.uid, new_identity.uid);
            assert_ne!(old_identity, new_identity);
            screen.apply(epoch, vec![vec![ResourceEvent::reset(columns, rows)]], cx);
            assert_eq!(screen.store.len(), 1);
            assert!(screen.store.get(&old_identity).is_none());
            assert!(screen.store.get(&new_identity).is_some());
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn escape_in_the_filter_clears_it_then_returns_to_the_list(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    step(cx, &|window, cx| {
        screen.update(cx, |screen, cx| screen.focus(window, cx));
    });
    // Command-F reaches the filter as the slash does.
    step(cx, &|window, cx| window.press("secondary-f", cx));
    step(cx, &|window, cx| window.input("coredns", cx));
    step(cx, &|window, cx| {
        assert_eq!(screen.read(cx).projection.len(), 2);
        window.press("escape", cx);
    });
    step(cx, &|window, cx| {
        // The first Escape clears the text and stays in the filter.
        assert_eq!(screen.read(cx).projection.len(), 22);
        assert!(screen.read(cx).query.read(cx).value().is_empty());
        assert_eq!(window.find("resource-body").focused(), Some(false));
        window.press("escape", cx);
    });
    step(cx, &|window, cx| {
        // The second hands the keyboard back to the list.
        assert_eq!(window.find("resource-body").focused(), Some(true));
        window.press("down", cx);
        assert_eq!(selected(&screen, cx), Some(identity_at(&screen, 0, cx)));
    });
}

#[gpui_kit::test]
fn the_namespace_picker_hands_the_keyboard_back_to_the_list(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    // Escape closes the menu without a choice.
    step(cx, &|window, cx| window.click("resource-namespace", cx));
    step(cx, &|window, cx| {
        assert_eq!(window.find("resource-body").focused(), Some(false));
        window.press("escape", cx);
    });
    step(cx, &|window, cx| {
        assert_eq!(window.find("resource-body").focused(), Some(true));
        assert_eq!(screen.read(cx).namespace, None);
    });
    // N on the list opens it too, and a choice applies; the arrows then
    // move through the list again.
    step(cx, &|window, cx| {
        screen.update(cx, |screen, cx| screen.focus(window, cx));
    });
    step(cx, &|window, cx| window.press("n", cx));
    // Past `argocd`, which holds no pods, to `batch`.
    step(cx, &|window, cx| window.press("down", cx));
    step(cx, &|window, cx| window.press("down", cx));
    step(cx, &|window, cx| window.press("enter", cx));
    step(cx, &|window, cx| {
        let chosen = screen.read(cx).namespace.clone();
        assert!(chosen.is_some());
        assert_eq!(window.find("resource-body").focused(), Some(true));
        window.press("down", cx);
        let first = identity_at(&screen, 0, cx);
        assert_eq!(Some(first.namespace.clone()), chosen);
        assert_eq!(selected(&screen, cx), Some(first));
    });
}

#[gpui_kit::test]
fn the_page_keeps_its_keys_while_no_rows_show(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("configmaps"), window, cx);
            screen.focus(window, cx);
        });
        window.render_frame(cx);
        assert!(window.find("resource-not-in-example").visible());
        assert_eq!(window.find("resource-body").focused(), Some(true));
        // The list's keys still answer: slash goes to the filter.
        window.press("/", cx);
        window.render_frame(cx);
        assert_eq!(window.find("resource-body").focused(), Some(false));
    })
    .unwrap();
}

#[gpui_kit::test]
fn enter_opens_a_row_at_once_and_escape_steps_back_one_level(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let focused = |window: &mut gpui_kit::Window, id: &'static str| window.find(id).focused();
    step(cx, &|window, cx| {
        screen.update(cx, |screen, cx| screen.focus(window, cx));
    });
    step(cx, &|window, cx| {
        // Enter on the list with nothing selected leaves it be.
        window.press("enter", cx);
        assert_eq!(shown(&screen, cx), None);
        window.press("down", cx);
        window.press("enter", cx);
    });
    step(cx, &|window, cx| {
        // Read at once, without waiting for the keys to pause, and the
        // keyboard is on the pane.
        let first = identity_at(&screen, 0, cx);
        assert_eq!(shown(&screen, cx), Some(first));
        assert!(window.try_find("detail-state").is_none());
        assert_eq!(focused(window, "resource-detail"), Some(true));
        window.press("secondary-}", cx);
    });
    step(cx, &|window, cx| {
        assert_eq!(window.find("detail-tab-yaml").selected(), Some(true));
        assert_eq!(focused(window, "resource-detail"), Some(true));
        // Escape in the pane steps back to the list; the pane stays.
        window.press("escape", cx);
    });
    step(cx, &|window, cx| {
        assert_eq!(focused(window, "resource-body"), Some(true));
        assert!(shown(&screen, cx).is_some());
        // From the list, the brackets switch the tab and keep the keyboard
        // on the list. The two tabs wrap round.
        window.press("secondary-}", cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-tab-details").selected(), Some(true));
        window.press("secondary-{", cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-tab-yaml").selected(), Some(true));
        assert_eq!(focused(window, "resource-body"), Some(true));
        // Enter on the open row hands the keyboard to the pane.
        window.press("enter", cx);
        window.render_frame(cx);
        assert_eq!(focused(window, "resource-detail"), Some(true));
        // Escape in the pane steps back to the list.
        window.press("escape", cx);
    });
    step(cx, &|window, cx| {
        assert_eq!(focused(window, "resource-body"), Some(true));
        assert!(shown(&screen, cx).is_some());
        // Escape in the list, with no filter, closes the pane.
        window.press("escape", cx);
    });
    step(cx, &|window, cx| {
        assert!(window.try_find("resource-detail").is_none());
        assert_eq!(shown(&screen, cx), None);
        assert_eq!(focused(window, "resource-body"), Some(true));
    });
}

#[gpui_kit::test]
fn n_does_nothing_for_a_kind_without_namespaces(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("nodes"), window, cx);
            screen.focus(window, cx);
        });
        window.render_frame(cx);
        assert!(window.try_find("resource-namespace").is_none());
        window.press("n", cx);
        window.render_frame(cx);
        assert_eq!(window.find("resource-body").focused(), Some(true));
    })
    .unwrap();
}

/// The index of the first row whose pod runs.
fn running_row(screen: &Entity<ResourcesScreen>, cx: &gpui_kit::App) -> usize {
    (0usize..)
        .find(|&ix| {
            let identity = identity_at(screen, ix, cx);
            screen.read(cx).store.get(&identity).unwrap().cells[2] == "Running"
        })
        .unwrap()
}

#[gpui_kit::test]
fn a_forward_runs_on_through_another_context_and_names_its_own(cx: &mut TestAppContext) {
    // The forward listens on Tokio, which wakes GPUI from its own threads.
    cx.executor().allow_parking();
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    step(cx, &|window, cx| {
        let ix = running_row(&screen, cx);
        window
            .within(row_id(&identity_at(&screen, ix, cx)))
            .click("name", cx);
    });
    step(cx, &|window, cx| window.click("detail-jump-ports", cx));
    step(cx, &|window, cx| window.click("ports-forward-8080", cx));
    step(cx, &|window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_source(Some(source("staging-eu")), window, cx)
        });
    });
    let list = cx.update(crate::forwards::list);
    cx.read(|cx| {
        assert_eq!(shown(&screen, cx), None, "another context closes the pane");
        let items = &list.read(cx).items;
        assert_eq!(items.len(), 1);
        let forward = items[0].read(cx);
        assert!(forward.running(), "the forward keeps its connection");
        assert_eq!(forward.display.context.as_ref(), "homelab");
    });
}

#[gpui_kit::test]
fn node_pods_are_filtered_across_namespaces_and_do_not_open_a_nested_pane(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("prod-fra"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_node(Some("talos-wk-fra1-02"), window, cx)
        });
        window.render_frame(cx);
        let read = screen.read(cx);
        assert!(!read.store.is_empty());
        let column = read
            .store
            .columns()
            .iter()
            .position(|column| column.name == "Node")
            .unwrap();
        assert!(
            read.store
                .entries()
                .iter()
                .all(|entry| entry.row().cells[column] == "talos-wk-fra1-02")
        );
        let old_epoch = read.store.epoch();
        let old_rows = read
            .store
            .entries()
            .iter()
            .map(|entry| entry.row().clone())
            .collect::<Vec<_>>();
        assert!(window.try_find("resource-namespace").is_none());
        screen.update(cx, |screen, cx| screen.focus(window, cx));
        window.press("down", cx);
        window.render_frame(cx);
        assert!(shown(&screen, cx).is_none());
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(shown(&screen, cx).is_none());
        screen.update(cx, |screen, cx| {
            screen.set_node(Some("talos-wk-fra1-01"), window, cx);
            screen.apply(
                old_epoch,
                vec![vec![ResourceEvent::reset(
                    screen.store.columns().to_vec(),
                    old_rows,
                )]],
                cx,
            );
        });
        assert!(
            screen
                .read(cx)
                .store
                .entries()
                .iter()
                .all(|entry| entry.row().cells[column] == "talos-wk-fra1-01")
        );
    })
    .unwrap();
}

/// A node's Pods list has no Logs buttons, since L does nothing there
/// (#337), and each of its cells still starts under its column's label.
#[gpui_kit::test]
fn node_pods_have_no_logs_column_and_keep_their_header_aligned(cx: &mut TestAppContext) {
    use super::layout::ColumnSource;
    let (_runtime, screen, handle) = mount(cx, Some("prod-fra"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_node(Some("talos-wk-fra1-02"), window, cx)
        });
        window.render_frame(cx);
        let columns = screen.read(cx).layout.columns.clone();
        assert!(
            columns
                .iter()
                .all(|column| column.source != ColumnSource::Logs)
        );
        let first = identity_at(&screen, 0, cx);
        assert!(
            window
                .try_find(gpui_kit::SharedString::from(format!(
                    "pod-row-logs-{}",
                    first.uid
                )))
                .is_none()
        );
        let row = row_id(&first);
        for (source, cell) in [
            (ColumnSource::Containers, "containers"),
            (ColumnSource::Restarts, "restarts"),
        ] {
            let ix = columns
                .iter()
                .position(|column| column.source == source)
                .unwrap();
            let header = window.find(("resource-sort", ix)).bounds().left();
            let left = window.within(row.clone()).find(cell).bounds().left();
            assert!(
                (left - header).abs() <= px(1.5),
                "{cell}: {left:?} under {header:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn object_links_keep_matching_namespace_and_filter_but_reveal_hidden_objects(
    cx: &mut TestAppContext,
) {
    let (_runtime, screen, handle) = mount(cx, Some("prod-fra"));
    cx.update_window(handle, |_, window, cx| {
        let identity = screen
            .read(cx)
            .identity_named("payments", "worker-5d7c9-7rr9b")
            .unwrap_or_else(|| {
                screen
                    .read(cx)
                    .store
                    .entries()
                    .iter()
                    .find(|entry| entry.row().identity.namespace == "payments")
                    .unwrap()
                    .row()
                    .identity
                    .clone()
            });
        screen.update(cx, |screen, cx| {
            screen.set_namespace(Some("payments".into()), window, cx);
            screen.set_filter(&identity.name, window, cx);
            screen.open_identity_on(
                identity.clone(),
                crate::resources::Tab::Overview,
                window,
                cx,
            );
        });
        assert_eq!(screen.read(cx).namespace.as_deref(), Some("payments"));
        assert_eq!(screen.read(cx).query.read(cx).value(), identity.name);
        assert_eq!(shown(&screen, cx), Some(identity.clone()));
        screen.update(cx, |screen, cx| {
            screen.set_namespace(Some("batch".into()), window, cx);
            screen.set_filter("does-not-match", window, cx);
            screen.open_identity_on(identity.clone(), crate::resources::Tab::Logs, window, cx);
        });
        assert!(screen.read(cx).namespace.is_none());
        assert!(screen.read(cx).query.read(cx).value().is_empty());
        assert_eq!(shown(&screen, cx), Some(identity));
        // Logs open in the dock; the pane stays on its tab.
        assert_eq!(
            screen.read(cx).detail.read(cx).tab(),
            crate::resources::Tab::Overview
        );
        let missing = ResourceIdentity {
            name: "absent-from-list".into(),
            uid: "missing-uid".into(),
            ..identity_at(&screen, 0, cx)
        };
        screen.update(cx, |screen, cx| {
            screen.open_identity_on(missing.clone(), crate::resources::Tab::Overview, window, cx)
        });
        assert_eq!(shown(&screen, cx), Some(missing));
    })
    .unwrap();
}

/// A group row's buttons follow its count, within the 280 dp a drawer
/// leaves the list, and the Healthy row offers no fold while a filter keeps
/// every healthy pod in sight (#321).
#[gpui_kit::test]
fn a_group_rows_buttons_follow_its_count_and_a_filter_offers_no_fold(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        problems(&screen, cx);
        window.render_frame(cx);
        let row = window.find("resource-group-healthy").bounds();
        let toggle = window.find("resource-group-healthy-toggle").bounds();
        assert!(
            toggle.right() - row.left() <= gpui_kit::px(280.),
            "{row:?} {toggle:?}"
        );

        // The healthy chip shows every healthy pod: nothing to fold.
        window.click("resource-tally-healthy", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).projection.len(), 20);
        assert!(window.try_find("resource-group-healthy-toggle").is_none());
        window.click("resource-tally-healthy", cx);
        window.render_frame(cx);
        assert!(window.try_find("resource-group-healthy-toggle").is_some());

        // Neither does a typed filter, which reaches the screen through
        // the input's change event once the update ends.
        window.click("resource-filter", cx);
        window.render_frame(cx);
        window.input("coredns", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(screen.read(cx).projection.len(), 2);
        assert!(window.find("resource-group-healthy").visible());
        assert!(window.try_find("resource-group-healthy-toggle").is_none());
    })
    .unwrap();
}

/// The pods page as it opens: problems first.
fn problems(screen: &Entity<ResourcesScreen>, cx: &mut gpui_kit::App) {
    screen.update(cx, |screen, cx| {
        screen.set_list_view(ListView::Problems, cx)
    });
}

#[gpui_kit::test]
fn pods_show_problems_first_and_fold_healthy_ones(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        problems(&screen, cx);
        window.render_frame(cx);
        // One pod crashes and one waits to start; the rest are folded.
        let tally = screen.read(cx).projection.tally();
        assert_eq!((tally.failing, tally.waiting, tally.total()), (1, 1, 22));
        assert_eq!(tally.problems(), 2);
        assert_eq!(screen.read(cx).projection.len(), 2);
        assert!(window.find("resource-group-failing").visible());
        assert!(window.find("resource-group-pending").visible());
        let healthy = window
            .find("resource-group-healthy")
            .label()
            .unwrap()
            .to_owned();
        assert!(healthy.starts_with("Healthy · 20 pods · "), "{healthy}");
        assert!(healthy.ends_with("· collapsed"), "{healthy}");
        // The count sits in the table's footer, under the rows.
        let collapsed = window.within("resource-footer").find("resource-collapsed");
        assert!(collapsed.visible());
        assert!(collapsed.bounds().top() >= window.find("resource-table-scroll").bounds().bottom());
        assert_eq!(
            window.find("resource-tally-failing").label(),
            Some("1 failing")
        );
        // The crashing pod says why after its name.
        let failing = screen.read(cx).projection.row(&screen.read(cx).store, 0);
        let failing = failing.unwrap().identity.clone();
        assert_eq!(
            screen
                .read(cx)
                .store
                .get(&failing)
                .unwrap()
                .pod
                .as_ref()
                .unwrap()
                .reason,
            "CrashLoopBackOff · exit 1"
        );

        window.click("resource-group-healthy-toggle", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).projection.len(), 22);
        assert!(window.try_find("resource-collapsed").is_none());
        window.click("resource-group-healthy-toggle", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).projection.len(), 2);

        // Show all lists every pod in one sorted list.
        window.click("resource-show-all", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).list_view, ListView::All);
        assert_eq!(screen.read(cx).projection.len(), 22);
        assert!(window.try_find("resource-group-failing").is_none());
        window.click("resource-view-problems", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).projection.len(), 2);
    })
    .unwrap();
}

/// A pod's glyph by its printed status, on a ready node.
fn pod_glyph(status: &str) -> crate::ui::Tone {
    let cells = ["p", "0/1", status, "0", "", "10.0.0.1", "wk-1"].map(str::to_owned);
    let pod = crate::resources::rows::pod_row(&example::pod_columns(), &cells, None, false);
    super::cells::pod_tone(&pod, true)
}

#[test]
fn a_container_that_ran_and_stopped_died_and_one_that_cannot_start_is_critical() {
    use crate::ui::Tone;
    for status in [
        "CrashLoopBackOff",
        "Init:CrashLoopBackOff",
        "Error",
        "OOMKilled",
    ] {
        assert_eq!(pod_glyph(status), Tone::Died, "{status}");
    }
    for status in ["ImagePullBackOff", "ErrImagePull", "Evicted"] {
        assert_eq!(pod_glyph(status), Tone::Crit, "{status}");
    }
    // An unschedulable pod prints Pending: it waits, as before.
    assert_eq!(pod_glyph("Pending"), Tone::Unknown);
    assert_eq!(pod_glyph("Running"), Tone::Warn);
}

#[gpui_kit::test]
fn a_pod_that_died_counts_with_the_failing_ones(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        problems(&screen, cx);
        window.render_frame(cx);
        let view = screen.read(cx);
        let crashing = view.projection.row(&view.store, 0).unwrap();
        let pod = crashing.pod.as_ref().unwrap();
        assert_eq!(pod.status, "CrashLoopBackOff");
        assert_eq!(super::cells::pod_tone(pod, true), crate::ui::Tone::Died);
        // The chip and the group count it as failing, under the critical
        // glyph.
        assert_eq!(view.projection.tally().failing, 1);
        assert_eq!(
            window.find("resource-tally-failing").label(),
            Some("1 failing")
        );
        assert!(window.find("resource-group-failing").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn opening_a_folded_pod_unfolds_the_healthy_ones(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        let healthy = {
            let view = screen.read(cx);
            view.store
                .entries()
                .iter()
                .map(|entry| entry.row())
                .find(|row| row.pod.as_ref().unwrap().reason.is_empty())
                .unwrap()
                .identity
                .clone()
        };
        problems(&screen, cx);
        screen.update(cx, |screen, cx| {
            screen.open_identity_on(healthy.clone(), crate::resources::Tab::Overview, window, cx)
        });
        window.render_frame(cx);
        assert_eq!(selected(&screen, cx), Some(healthy.clone()));
        assert!(window.find(row_id(&healthy)).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn pods_on_a_node_that_isnt_ready_group_under_it(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let links = Rc::new(RefCell::new(Vec::new()));
    let sink = links.clone();
    cx.update(|cx| {
        cx.subscribe(
            &screen,
            move |_, link: &crate::resources::ResourceLink, _| {
                if let crate::resources::ResourceLink::Node(node, tab) = link {
                    sink.borrow_mut().push((node.clone(), *tab));
                }
            },
        )
        .detach()
    });
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.not_ready = [("talos-home".to_owned(), None)].into();
            screen.set_list_view(ListView::Problems, cx);
        });
        window.render_frame(cx);
        let group = window
            .find("resource-group-node:talos-home")
            .label()
            .unwrap()
            .to_owned();
        assert!(group.starts_with("Node not ready · "), "{group}");
        assert!(group.contains("metrics are last known"), "{group}");
        // The crashing pod stays failing; the waiting ones and the
        // running ones can't be confirmed.
        let tally = screen.read(cx).projection.tally();
        assert_eq!(tally.failing, 1);
        assert!(tally.warning > 10, "{tally:?}");
    })
    .unwrap();
    // The group's menu offers its node, as O does.
    choose(
        cx,
        handle,
        |window, cx| window.right_click("resource-group-node:talos-home-line", cx),
        "Open node talos-home",
    );
    assert_eq!(
        links.borrow().as_slice(),
        [(
            "talos-home".to_owned(),
            crate::desktop::nodes::NodeTab::Overview
        )]
    );
}

#[gpui_kit::test]
fn x_marks_rows_and_a_group_selects_all_of_its_own(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let first = identity_at(&screen, 0, cx);
        window.within(row_id(&first)).click("name", cx);
        window.press("x", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).marked.contains(&first));
        assert!(
            window
                .within("resource-footer")
                .find("resource-marks")
                .visible()
        );
        window.press("x", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).marked.is_empty());
        assert!(window.try_find("resource-marks").is_none());

        // Shift-X marks the selected row's group: here each has one pod.
        problems(&screen, cx);
        window.render_frame(cx);
        let first = identity_at(&screen, 0, cx);
        window.within(row_id(&first)).click("name", cx);
        window.press("shift-x", cx);
        window.press("down", cx);
        window.press("shift-x", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).marked.len(), 2);
        window.click("resource-marks-copy", cx);
    })
    .unwrap();
    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    assert_eq!(copied.lines().count(), 2);
    assert!(copied.lines().all(|line| line.contains('/')), "{copied}");
    cx.update_window(handle, |_, window, cx| {
        window.click("resource-marks-clear", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).marked.is_empty());
        // Listing again starts without marks.
        screen.update(cx, |screen, cx| {
            screen.marked.insert(identity_at_now(screen));
            screen.refresh(window, cx);
        });
        assert!(screen.read(cx).marked.is_empty());
    })
    .unwrap();
}

fn identity_at_now(screen: &ResourcesScreen) -> ResourceIdentity {
    screen
        .projection
        .row(&screen.store, 0)
        .unwrap()
        .identity
        .clone()
}

#[gpui_kit::test]
fn l_asks_for_the_selected_pods_logs(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let asked = Rc::new(RefCell::new(Vec::new()));
    let sink = asked.clone();
    cx.update(|cx| {
        cx.subscribe(
            &screen,
            move |_, event: &crate::resources::ResourceLink, _| {
                if let crate::resources::ResourceLink::Logs(request) = event {
                    sink.borrow_mut().push(request.target.identity.clone());
                }
            },
        )
        .detach()
    });
    let first = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let first = identity_at(&screen, 0, cx);
            window.within(row_id(&first)).click("name", cx);
            window.render_frame(cx);
            window.press("l", cx);
            window.render_frame(cx);
            // The drawer steps aside for the dock and the row stays
            // selected, so Enter opens it again.
            assert_eq!(shown(&screen, cx), None);
            assert!(window.try_find("resource-drawer").is_none());
            assert_eq!(screen.read(cx).selected_row(), Some(&first));
            first
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(*asked.borrow(), [first]);
}

/// Each pod row has a Logs button after its name, named for its pod, that
/// opens that pod's logs without selecting the row or opening the drawer
/// (#291).
#[gpui_kit::test]
fn a_rows_logs_button_asks_for_its_pods_logs(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let asked = Rc::new(RefCell::new(Vec::new()));
    let sink = asked.clone();
    cx.update(|cx| {
        cx.subscribe(
            &screen,
            move |_, event: &crate::resources::ResourceLink, _| {
                if let crate::resources::ResourceLink::Logs(request) = event {
                    sink.borrow_mut().push(request.target.identity.clone());
                }
            },
        )
        .detach()
    });
    let second = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let button = |identity: &ResourceIdentity| {
                gpui_kit::SharedString::from(format!("pod-row-logs-{}", identity.uid))
            };
            // Every pod row has one, inside the row and named for its pod.
            for line in 0..3 {
                let identity = identity_at(&screen, line, cx);
                let row = window.find(row_id(&identity)).bounds();
                let found = window.find(button(&identity));
                assert!(row.contains(&found.bounds().center()), "{row:?}");
                assert_eq!(
                    found.label(),
                    Some(format!("Logs for {}", identity.name).as_str())
                );
            }
            let second = identity_at(&screen, 1, cx);
            window.click(button(&second), cx);
            window.render_frame(cx);
            assert_eq!(shown(&screen, cx), None);
            assert!(window.try_find("resource-drawer").is_none());
            assert_eq!(screen.read(cx).selected_row(), None);
            second
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(*asked.borrow(), [second]);
}

#[gpui_kit::test]
fn a_sideways_scroll_keeps_each_name_in_view_once(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount_sized(cx, Some("homelab"), 640.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let rows: Vec<_> = (0..3)
            .map(|ix| row_id(&identity_at(&screen, ix, cx)))
            .collect();
        let left = |window: &mut gpui_kit::Window, row: &gpui_kit::ElementId| {
            window.within(row.clone()).find("name").bounds().left()
        };
        let names: Vec<_> = rows.iter().map(|row| left(window, row)).collect();
        let first = identity_at(&screen, 0, cx);
        let logs = gpui_kit::SharedString::from(format!("pod-row-logs-{}", first.uid));
        let button = window.find(logs.clone()).bounds().left();
        let header = window.find(("resource-sort", 1usize)).bounds().left();
        // Column 3 starts at the pinned run's edge (glyph, name, Logs),
        // where its label stays once scrolled; column 5 starts clear of
        // the scroll's 120.
        let edge = window.find(("resource-sort", 3usize)).bounds().left();
        let later = window.find(("resource-sort", 5usize)).bounds().left();
        assert!(later - edge > px(120.), "{:?}", later - edge);
        window.scroll(
            "resource-table-scroll",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(-120.), px(0.))),
            cx,
        );
        window.render_frame(cx);
        let moved = later - window.find(("resource-sort", 5usize)).bounds().left();
        assert!((f32::from(moved) - 120.).abs() <= 1.5, "{moved:?}");
        // `find` fails on an id that resolves twice.
        assert!((window.find(("resource-sort", 1usize)).bounds().left() - header).abs() <= px(1.5));
        for (row, name) in rows.iter().zip(names) {
            assert!((left(window, row) - name).abs() <= px(1.5));
        }
        // A pod's Logs button is pinned with its name (#291).
        assert!((window.find(logs).bounds().left() - button).abs() <= px(1.5));
    })
    .unwrap();
}

/// Ready sorts the least ready pods first and Restarts the most restarted
/// last; each pod's restarts have a cell of their own (#291).
#[gpui_kit::test]
fn ready_and_restarts_sort_on_their_own(cx: &mut TestAppContext) {
    use crate::resources::{model::SortKey, rows::PodRow};
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| screen.set_list_view(ListView::All, cx));
        window.render_frame(cx);
        let column =
            |screen: &Entity<ResourcesScreen>, cx: &gpui_kit::App, pick: fn(&PodRow) -> f64| {
                let view = screen.read(cx);
                (0..view.projection.len())
                    .map(|ix| {
                        pick(
                            view.projection
                                .row(&view.store, ix)
                                .unwrap()
                                .pod
                                .as_ref()
                                .unwrap(),
                        )
                    })
                    .collect::<Vec<f64>>()
            };
        let share = |pod: &PodRow| {
            let (up, all) = pod.ready.split_once('/').unwrap();
            up.parse::<f64>().unwrap() / all.parse::<f64>().unwrap()
        };
        screen.update(cx, |screen, cx| screen.sort_by(SortKey::Ready, cx));
        let ready = column(&screen, cx, share);
        assert!(ready.is_sorted(), "{ready:?}");
        assert!(ready[0] < 1., "{ready:?}");

        screen.update(cx, |screen, cx| screen.sort_by(SortKey::Restarts, cx));
        let restarts = column(&screen, cx, |pod| pod.restarts as f64);
        assert!(restarts.is_sorted(), "{restarts:?}");
        assert!(*restarts.last().unwrap() > 0., "{restarts:?}");

        // Descending, the most restarted pod leads, its count in its cell.
        screen.update(cx, |screen, cx| screen.sort_by(SortKey::Restarts, cx));
        window.render_frame(cx);
        let view = screen.read(cx);
        let first = view.projection.row(&view.store, 0).unwrap();
        let (restarted, count) = (first.identity.clone(), first.pod.as_ref().unwrap().restarts);
        assert_eq!(count as f64, *restarts.last().unwrap());
        let cell = window.within(row_id(&restarted)).find("restarts");
        assert_eq!(cell.label(), Some(format!("{count} restarts").as_str()));
    })
    .unwrap();
}

/// Containers sorts the pod with the worst container first, and each
/// row's squares name every container, init containers first (#291).
#[gpui_kit::test]
fn containers_sort_the_worst_first_and_name_each_container(cx: &mut TestAppContext) {
    use crate::resources::model::SortKey;
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_list_view(ListView::All, cx);
            screen.sort_by(SortKey::Containers, cx);
        });
        window.render_frame(cx);
        let view = screen.read(cx);
        let rows: Vec<_> = (0..view.projection.len())
            .map(|ix| view.projection.row(&view.store, ix).unwrap())
            .collect();
        let worst: Vec<u8> = rows
            .iter()
            .map(|row| row.pod.as_ref().unwrap().worst)
            .collect();
        assert!(worst.is_sorted(), "{worst:?}");
        assert_eq!(worst[0], 0, "a failing container leads");
        let first = rows[0].identity.clone();
        let pod = rows[0].pod.as_ref().unwrap();
        let cell = window.within(row_id(&first)).find("containers");
        assert_eq!(cell.label(), Some(pod.containers_label.as_str()));
        for container in &pod.containers {
            assert!(
                pod.containers_label.contains(&container.name),
                "{}",
                pod.containers_label
            );
        }
        // Ledger migrates first: its init container leads its squares,
        // dimmed, and its line.
        let ledger = rows
            .iter()
            .find(|row| row.identity.name.starts_with("ledger-"))
            .unwrap()
            .pod
            .as_ref()
            .unwrap();
        assert!(ledger.squares[0].dim);
        assert!(
            ledger
                .containers_label
                .starts_with("migrate (init): exited 0"),
            "{}",
            ledger.containers_label
        );
    })
    .unwrap();
}

/// A pod with more containers than the column draws counts the rest
/// whole, inside the cell, at the default and the largest text size
/// (#329 review).
#[gpui_kit::test]
fn a_crowded_pods_count_fits_its_cell(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    for size in [freshkube_ui::ui::BASE_TEXT, 20.] {
        cx.update_window(handle, |_, window, cx| {
            window.set_rem_size(gpui_kit::px(size));
            screen.update(cx, |screen, cx| {
                screen.set_list_view(ListView::All, cx);
                screen.set_filter("gateway", window, cx);
            });
            window.render_frame(cx);
            let gateway = identity_at(&screen, 0, cx);
            assert!(gateway.name.starts_with("gateway-"), "{gateway:?}");
            let view = screen.read(cx);
            let pod = view
                .projection
                .row(&view.store, 0)
                .unwrap()
                .pod
                .clone()
                .unwrap();
            assert_eq!(pod.squares.len(), 10);
            assert_eq!(pod.squares.more().map(|more| more.as_ref()), Some("+2"));
            let row = window.within(row_id(&gateway));
            let cell = row.find("containers").bounds();
            let more = row.find("squares-more");
            let pad = freshkube_ui::ui::dp_px(freshkube_ui::table::CELL_PAD, window);
            assert!(
                more.bounds().right() <= cell.right() - pad + gpui_kit::px(0.5),
                "at {size}: {:?} in {cell:?}",
                more.bounds()
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn pod_rows_show_owner_readiness_use_and_node(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let view = screen.read(cx);
        // Example use arrives with the rows, so the first frame has it.
        assert_eq!(view.usage_state, super::UsageState::Known);
        let running = view
            .store
            .entries()
            .iter()
            .find(|entry| entry.usage().is_some())
            .unwrap()
            .row()
            .identity
            .clone();
        let row = view.store.get(&running).unwrap();
        assert_eq!(row.owner.as_ref().unwrap().short, "deploy");
        let label = window.find(row_id(&running)).label().unwrap().to_owned();
        assert!(label.contains(" · deploy/"), "{label}");
        // Use sorts by value with unknown use below any known: busiest
        // first, then the pods with no use.
        let pod = row.pod.clone().unwrap();
        assert!(!pod.node.is_empty() && !pod.containers.is_empty());
        screen.update(cx, |screen, cx| {
            screen.sort_by(crate::resources::model::SortKey::Cpu, cx);
            screen.sort_by(crate::resources::model::SortKey::Cpu, cx);
        });
        let view = screen.read(cx);
        let cpu: Vec<Option<f64>> = (0..view.projection.len())
            .map(|ix| {
                view.projection
                    .entry(&view.store, ix)
                    .unwrap()
                    .usage()
                    .and_then(|usage| usage.cpu_millis)
            })
            .collect();
        let known = cpu.iter().take_while(|cpu| cpu.is_some()).count();
        assert!(
            known > 0 && cpu[known..].iter().all(Option::is_none),
            "{cpu:?}"
        );
        assert!(
            cpu[..known].windows(2).all(|pair| pair[0] >= pair[1]),
            "{cpu:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn workloads_take_their_glyph_from_replicas_ready(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("deployments.apps"), window, cx)
        });
        window.render_frame(cx);
        let layout = &screen.read(cx).layout;
        assert_eq!(layout.columns[0].source, super::layout::ColumnSource::Glyph);
        assert!(matches!(
            layout.tone_from,
            Some(super::layout::ToneSource::Ready(_))
        ));
        assert!(screen.read(cx).projection.grouping().is_none());
        assert!(window.try_find("resource-view").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn fog_glyph_filters_and_column_choices_change_the_table(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("example"));
    let before = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            for control in ["resource-columns", "resource-refresh"] {
                assert!(window.find(control).visible(), "{control}");
                assert!(
                    window.find(control).bounds().bottom()
                        <= window.find("resource-list").bounds().top()
                );
            }
            window.click("resource-tally-healthy", cx);
            assert_eq!(
                screen.read(cx).projection.pod_filter(),
                Some(crate::resources::projection::PodFilter::Healthy)
            );
            assert_eq!(
                screen.read(cx).projection.len(),
                screen.read(cx).projection.tally().healthy
            );
            window.click("resource-view-all", cx);
            assert!(screen.read(cx).projection.pod_filter().is_none());
            assert_eq!(
                screen.read(cx).projection.len(),
                screen.read(cx).store.len()
            );
            let before = screen.read(cx).layout.width;
            window.click("resource-columns", cx);
            window.render_frame(cx);
            // The first optional column is Containers; the Name and glyph
            // stay fixed.
            window.within("popup-menu").click(0usize, cx);
            assert!(
                screen
                    .read(cx)
                    .hidden_columns
                    .contains(&super::layout::ColumnSource::Containers)
            );
            assert!(screen.read(cx).layout.width < before);
            before
        })
        .unwrap();
    // Let the menu's deferred dismiss finish before opening it again.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.click("resource-columns", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
        assert_eq!(screen.read(cx).layout.width, before);
    })
    .unwrap();
}

#[gpui_kit::test]
fn namespaces_survive_a_kind_change_but_reject_a_source_round_trip(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            let generation = screen.namespace_generation;
            screen.set_kind(kind("deployments.apps"), window, cx);
            assert!(screen.finish_namespaces(generation, Ok(vec!["current".into()]), window, cx));
            assert_eq!(screen.namespaces, ["current"]);
            screen.set_source(Some(source("prod-fra")), window, cx);
            screen.set_source(Some(source("homelab")), window, cx);
            let current = screen.namespaces.clone();
            assert!(!screen.finish_namespaces(generation, Ok(vec!["obsolete".into()]), window, cx));
            assert_eq!(screen.namespaces, current);
            assert!(screen.finish_namespaces(
                screen.namespace_generation,
                Ok(vec!["fresh".into()]),
                window,
                cx
            ));
            assert_eq!(screen.namespaces, ["fresh"]);
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn old_metrics_cannot_change_the_new_access_or_a_disconnected_page(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            let generation = screen.usage_generation;
            screen.set_source(Some(source("prod-fra")), window, cx);
            assert!(!screen.apply_usage(generation, Err("late failure".into()), cx));
            assert_eq!(screen.usage_state, super::UsageState::Known);
            let generation = screen.usage_generation;
            assert!(screen.apply_usage(generation, Err("current failure".into()), cx));
            assert_eq!(screen.usage_state, super::UsageState::Stale);
            screen.set_source(None, window, cx);
            assert!(screen.usage.is_none());
            assert!(!screen.apply_usage(generation, Ok(Vec::new()), cx));
            assert_eq!(screen.usage_state, super::UsageState::Unknown);
        });
    })
    .unwrap();
}

/// Draws until the header stops asking for frames: it folds its controls
/// from what its parts measured on the frame before.
fn settle(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    for _ in 0..4 {
        cx.run_until_parked();
        let asked = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.simulate_next_frame(cx)
            })
            .unwrap();
        if asked == 0 {
            return;
        }
    }
    panic!("the header keeps moving");
}

#[gpui_kit::test]
fn the_folded_controls_do_what_the_controls_do(cx: &mut TestAppContext) {
    // 316 dp inside the toolbar's insets: the title, filter and "…" fit,
    // and nothing else.
    let (_runtime, screen, handle) = mount_sized(cx, Some("homelab"), 340.);
    settle(handle, cx);
    let payments = cx
        .update_window(handle, |_, window, cx| {
            assert!(
                window.try_find("resource-namespace").is_none(),
                "not folded"
            );
            assert!(window.try_find("resource-refresh").is_none());
            // Every folded control at its default: no dot.
            assert!(window.try_find("resource-more-dot").is_none());
            assert_eq!(window.find("resource-more").label(), Some("More"));
            let names = &screen.read(cx).namespaces;
            // All namespaces comes first.
            names.iter().position(|name| name == "payments").unwrap() + 1
        })
        .unwrap();
    // Refresh runs ⌘R's action, which the shell handles; mounted alone,
    // the screen has no shell, so the app catches it.
    let refreshed = Rc::new(Cell::new(0));
    let count = refreshed.clone();
    cx.update(|cx| {
        cx.on_action(move |_: &freshkube_ui::page::Refresh, _| count.set(count.get() + 1));
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("resource-more", cx);
        window.render_frame(cx);
        // Namespace, Columns, Refresh.
        window.within("popup-menu").click(2usize, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(refreshed.get(), 1);
    // A namespace from the menu narrows the list as the picker does.
    cx.update_window(handle, |_, window, cx| {
        window.click("resource-more", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
        window.render_frame(cx);
        window
            .within("submenu")
            .within("popup-menu")
            .click(payments, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let view = screen.read(cx);
        assert_eq!(view.namespace.as_deref(), Some("payments"));
        assert_eq!(view.store.len(), 6);
        assert_eq!(
            view.namespace_select.read(cx).selected_value(),
            Some(&Some("payments".to_owned()))
        );
        // The "…" says the folded picker narrows the list.
        window.find("resource-more-dot");
        assert_eq!(
            window.find("resource-more").label(),
            Some("More · Namespace payments")
        );
    })
    .unwrap();
    // A column from the menu hides as the Columns menu hides it.
    let hidden = screen.read_with(cx, |screen, _| screen.hidden_columns.len());
    cx.update_window(handle, |_, window, cx| {
        window.click("resource-more", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(1usize, cx);
        window.render_frame(cx);
        window
            .within("submenu")
            .within("popup-menu")
            .click(0usize, cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_ne!(
        screen.read_with(cx, |screen, _| screen.hidden_columns.len()),
        hidden
    );
    cx.update_window(handle, |_, window, _| {
        assert_eq!(
            window.find("resource-more").label(),
            Some("More · Namespace payments · 1 column hidden")
        );
    })
    .unwrap();
    // Both back to their defaults: All namespaces, every column.
    for (control, item) in [(0usize, 0usize), (1, 0)] {
        cx.update_window(handle, |_, window, cx| {
            window.click("resource-more", cx);
            window.render_frame(cx);
            window.within("popup-menu").click(control, cx);
            window.render_frame(cx);
            window
                .within("submenu")
                .within("popup-menu")
                .click(item, cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(screen.read(cx).namespace, None);
        assert!(window.try_find("resource-more-dot").is_none());
        assert_eq!(window.find("resource-more").label(), Some("More"));
    })
    .unwrap();
}

/// Opens a context menu with `press` and picks the item labelled `label`.
fn choose(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    press: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
    label: &str,
) {
    let items = open_menu(cx, handle, press);
    let ix = items
        .iter()
        .position(|item| item.as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no {label:?} in {items:?}"));
    cx.update_window(handle, |_, window, cx| {
        window.within("popup-menu").click(ix, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// Opens a context menu with `press`, a right-click, and reads its items'
/// labels, a separator as `None`.
fn open_menu(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    press: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
) -> Vec<Option<String>> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        press(window, cx);
    })
    .unwrap();
    // The menu builds on the next frame.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menu = window.within("popup-menu");
        (0usize..)
            .map_while(|ix| menu.try_find(ix))
            .map(|item| item.label().map(str::to_owned))
            .collect()
    })
    .unwrap()
}

/// A row's tooltip, such as its name's, doesn't draw over the menu the
/// row opened, and shows again once the menu is gone.
#[gpui_kit::test]
fn no_row_tooltip_draws_while_its_menu_is_open(cx: &mut TestAppContext) {
    use freshkube_ui::tooltip::drawn_total;
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let row = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let row = row_id(&identity_at(&screen, 0, cx));
            window.within(row.clone()).hover("name", cx);
            row
        })
        .unwrap();
    let shown = |cx: &mut TestAppContext| {
        // Past the tooltip's show delay.
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(600));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let before = drawn_total();
            window.render_frame(cx);
            drawn_total() > before
        })
        .unwrap()
    };
    assert!(shown(cx), "the name's tooltip never showed");
    let items = open_menu(cx, handle, |window, cx| {
        window.within(row.clone()).right_click("name", cx)
    });
    assert!(!items.is_empty());
    // The pointer rests on the name, left of the menu that opened at its
    // middle.
    cx.update_window(handle, |_, window, cx| {
        use gpui_kit::{InputEvent, MouseMoveEvent, point, px};
        let name = window.within(row.clone()).find("name").bounds();
        let position = point(name.left() + px(4.), name.center().y);
        window.dispatch_event(
            MouseMoveEvent {
                position,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
    assert!(!shown(cx), "a tooltip drew over the open menu");
    // A click outside closes the menu; the pointer comes back to the name.
    cx.update_window(handle, |_, window, cx| {
        window.click("resource-filter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none(), "the menu stayed");
        window.within(row).hover("name", cx);
    })
    .unwrap();
    assert!(shown(cx), "the tooltip stayed hidden after the menu closed");
}

#[gpui_kit::test]
fn a_right_click_selects_its_row_before_the_menu_acts(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let (first, third) = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let first = identity_at(&screen, 0, cx);
            window.within(row_id(&first)).click("name", cx);
            (first, identity_at(&screen, 2, cx))
        })
        .unwrap();
    assert_eq!(cx.read(|cx| selected(&screen, cx)), Some(first.clone()));
    // Its name, clear of the drawer the first click opened.
    let row = row_id(&third);
    let items = open_menu(cx, handle, |window, cx| {
        window.within(row).right_click("name", cx)
    });
    // The menu is the row's own: it is selected as the menu opens.
    assert_eq!(cx.read(|cx| selected(&screen, cx)), Some(third.clone()));
    assert_eq!(
        items.first().cloned().flatten().as_deref(),
        Some("Open"),
        "{items:?}"
    );
    assert!(items.contains(&Some("Logs".into())), "{items:?}");
    cx.update_window(handle, |_, window, cx| {
        let ix = items
            .iter()
            .position(|item| item.as_deref() == Some("Mark"));
        window.within("popup-menu").click(ix.unwrap(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        let marked = &screen.read(cx).marked;
        assert!(marked.contains(&third) && !marked.contains(&first));
    });
}

#[gpui_kit::test]
fn h_folds_the_healthy_pods_and_o_opens_the_selected_pods_node(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let links = Rc::new(RefCell::new(Vec::new()));
    let sink = links.clone();
    cx.update(|cx| {
        cx.subscribe(
            &screen,
            move |_, link: &crate::resources::ResourceLink, _| {
                if let crate::resources::ResourceLink::Node(node, _) = link {
                    sink.borrow_mut().push(node.clone());
                }
            },
        )
        .detach()
    });
    cx.update_window(handle, |_, window, cx| {
        problems(&screen, cx);
        window.render_frame(cx);
        let folded = screen.read(cx).projection.len();
        let first = identity_at(&screen, 0, cx);
        window.within(row_id(&first)).click("name", cx);
        window.press("h", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).projection.len() > folded);
        window.press("h", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).projection.len(), folded);
        window.press("o", cx);
    })
    .unwrap();
    let node = cx.read(|cx| screen.read(cx).selected_node());
    assert!(node.is_some());
    assert_eq!(links.borrow().as_slice(), [node.unwrap()]);
}

#[gpui_kit::test]
fn the_filter_types_the_lists_keys(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        problems(&screen, cx);
        window.render_frame(cx);
        window.click("resource-filter", cx);
        window.render_frame(cx);
        for key in ["o", "h", "x", "shift-x"] {
            window.press(key, cx);
        }
        window.render_frame(cx);
        assert_eq!(screen.read(cx).query.read(cx).value(), "ohxX");
        assert!(screen.read(cx).marked.is_empty());
    })
    .unwrap();
}

/// Moves the selection to the first row, as a list change under an open
/// menu could.
fn select_first(screen: &Entity<ResourcesScreen>, cx: &mut gpui_kit::App) {
    screen.update(cx, |screen, cx| {
        screen.projection.select(&screen.store, Some(0));
        cx.notify();
    });
}

/// Picks `label` from the open menu.
fn pick(
    window: &mut gpui_kit::Window,
    items: &[Option<String>],
    label: &str,
    cx: &mut gpui_kit::App,
) {
    let ix = items.iter().position(|item| item.as_deref() == Some(label));
    window.within("popup-menu").click(ix.unwrap(), cx);
}

#[gpui_kit::test]
fn a_menu_acts_only_on_the_row_it_was_opened_for(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let third = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            identity_at(&screen, 2, cx)
        })
        .unwrap();
    let row = row_id(&third);
    let items = open_menu(cx, handle, |window, cx| {
        window.within(row.clone()).right_click("name", cx)
    });
    // Another row is selected while the menu is open: Mark marks neither.
    cx.update_window(handle, |_, window, cx| {
        select_first(&screen, cx);
        window.render_frame(cx);
        pick(window, &items, "Mark", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert!(screen.read(cx).marked.is_empty()));

    // A row's menu goes with its row.
    open_menu(cx, handle, |window, cx| {
        window.within(row).right_click("name", cx)
    });
    cx.update_window(handle, |_, window, cx| {
        deliver(&screen, vec![ResourceEvent::Delete(third.clone())], cx);
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
    })
    .unwrap();
    cx.run_until_parked();

    // A group's menu outlives the row it selected, and acts only on it.
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.not_ready = [("talos-home".to_owned(), None)].into();
            screen.set_list_view(ListView::Problems, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    let group = "resource-group-node:talos-home-line";
    let items = open_menu(cx, handle, |window, cx| window.right_click(group, cx));
    let first = cx.read(|cx| selected(&screen, cx)).unwrap();
    cx.update_window(handle, |_, window, cx| {
        select_first(&screen, cx);
        assert_ne!(selected(&screen, cx), Some(first.clone()));
        window.render_frame(cx);
        pick(window, &items, "Mark", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert!(screen.read(cx).marked.is_empty()));

    // With its row still selected, the same pick marks it, so the checks
    // above can't pass on a missed click.
    let first = cx.read(|cx| selected(&screen, cx)).unwrap();
    let row = row_id(&first);
    let items = open_menu(cx, handle, |window, cx| {
        window.within(row).right_click("name", cx)
    });
    cx.update_window(handle, |_, window, cx| pick(window, &items, "Mark", cx))
        .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert!(screen.read(cx).marked.contains(&first)));
}

/// A watch's change to a pod's state flashes its row; a relist forgets
/// every flash and flashes nothing of its own.
#[gpui_kit::test]
fn a_changed_pod_flashes_and_a_relist_forgets_it(cx: &mut TestAppContext) {
    let (_runtime, screen, _window) = mount(cx, Some("demo"));
    cx.update(|cx| cx.set_reduce_motion(false));
    let (changed, columns, rows) = cx.update(|cx| {
        let screen = screen.read(cx);
        let rows: Vec<_> = screen
            .store
            .entries()
            .iter()
            .map(|e| e.row().clone())
            .collect();
        (screen.restarted(0), screen.store.columns().to_vec(), rows)
    });
    let id = changed.identity.clone();
    cx.update(|cx| deliver(&screen, vec![ResourceEvent::Upsert(changed)], cx));
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(screen.read(cx).flashing(cx), vec![id.clone()]));
    // Fades over FADE, then the layer forgets it.
    cx.background_executor
        .advance_clock(freshkube_ui::motion::FADE);
    cx.run_until_parked();
    cx.update(|cx| assert!(screen.read(cx).flashing(cx).is_empty()));
    // Another change flashes; a relist forgets it and flashes nothing.
    let again = cx.update(|cx| screen.read(cx).restarted(1));
    cx.update(|cx| deliver(&screen, vec![ResourceEvent::Upsert(again)], cx));
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(screen.read(cx).flashing(cx).len(), 1));
    cx.update(|cx| deliver(&screen, vec![ResourceEvent::reset(columns, rows)], cx));
    cx.run_until_parked();
    cx.update(|cx| assert!(screen.read(cx).flashing(cx).is_empty()));
}

/// A sort moves a fading row's tint with it before the next frame draws.
#[gpui_kit::test]
fn a_sort_moves_the_flash_with_its_row(cx: &mut TestAppContext) {
    use crate::resources::model::SortKey;
    use freshkube_ui::table::TableSource;
    let (_runtime, screen, _window) = mount(cx, Some("demo"));
    cx.update(|cx| cx.set_reduce_motion(false));
    let changed = cx.update(|cx| screen.read(cx).restarted(0));
    let id = changed.identity.clone();
    cx.update(|cx| deliver(&screen, vec![ResourceEvent::Upsert(changed)], cx));
    cx.run_until_parked();
    let line = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            let screen = screen.read(cx);
            let drawn = screen.flash.read(cx).line(&id);
            (drawn, TableSource::line_of(screen, &id))
        })
    };
    let (before, _) = line(cx);
    assert!(before.is_some());
    for key in [SortKey::Restarts, SortKey::Ready] {
        // No frame between the sort and the check.
        cx.update(|cx| screen.update(cx, |screen, cx| screen.sort_by(key, cx)));
        let (drawn, now) = line(cx);
        assert_eq!(drawn, now, "{key:?}");
    }
    assert_ne!(line(cx).0, before, "the sorts moved the row");
}

/// With reduced motion, nothing flashes.
#[gpui_kit::test]
fn reduced_motion_flashes_nothing(cx: &mut TestAppContext) {
    let (_runtime, screen, _window) = mount(cx, Some("demo"));
    let changed = cx.update(|cx| screen.read(cx).restarted(0));
    cx.update(|cx| deliver(&screen, vec![ResourceEvent::Upsert(changed)], cx));
    cx.run_until_parked();
    cx.update(|cx| assert!(screen.read(cx).flashing(cx).is_empty()));
}
