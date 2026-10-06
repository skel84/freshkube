use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Bounds, Entity, Pixels, TestAppContext, px, size};
use tokio::runtime::Runtime;

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use std::cell::RefCell;
use std::rc::Rc;

use freshkube_core::resources::ResourceKind;

use super::{
    KEYBOARD_PAUSE, KubeAccess, KubeSource, ListView, NotServed, ResourcesScreen, WATCH_COALESCE,
    row_id,
};
use crate::resources::example;
use crate::resources::model::{ReadState, ResourceIdentity, ResourceRow};
use crate::resources::store::{ResourceBatch, ResourceEvent};

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
        screen.apply(ResourceBatch { epoch, events }, cx);
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
        // namespace when listing every one, the owner, readiness with
        // restarts, use, the node and age. IP stays left out.
        assert!(window.try_find(("resource-sort", 0usize)).is_none());
        let labels: Vec<String> = (1..8usize)
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
            ["Name", "Owner", "Ready", "CPU", "Memory", "Node", "Age"]
        );
        assert!(window.try_find(("resource-sort", 8usize)).is_none());

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

        // Listing again finds the same object and selects it again.
        window.click("resource-refresh", cx);
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
            window.find(("resource-sort", 2usize)).label(),
            Some("Owner")
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
                ResourceBatch {
                    epoch,
                    events: vec![ResourceEvent::Read(ReadState::Loaded)],
                },
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
        assert!(window.find("resource-loading").visible());

        screen.update(cx, |screen, cx| screen.set_visible(true, window, cx));
        window.render_frame(cx);
        assert_eq!(screen.read(cx).store.len(), 22);
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

#[gpui_kit::test]
fn a_clicked_row_shows_its_details_beside_the_list_or_below_it(cx: &mut TestAppContext) {
    // 1280 leaves the page 1000 wide, room for both; 760 leaves 480.
    for (width, beside) in [(1280., true), (760., false)] {
        let (_runtime, screen, handle) = mount_sized(cx, Some("homelab"), width);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("resource-detail").is_none());
            let third = identity_at(&screen, 2, cx);
            window.within(row_id(&third)).click("name", cx);
            window.render_frame(cx);
            assert_eq!(shown(&screen, cx), Some(third.clone()));
            assert_eq!(
                window.find("detail-title").label(),
                Some(third.address().as_str())
            );
            // The table scrolls sideways in what the pane leaves it.
            let list = window.find("resource-table-scroll").bounds();
            let pane = window.find("resource-detail").bounds();
            if beside {
                assert!(pane.left() >= list.right(), "{list:?} {pane:?}");
                assert!(pane.size.width >= px(320.), "{pane:?}");
                assert!(list.size.width >= px(320.), "{list:?}");
            } else {
                assert!(pane.top() >= list.bottom(), "{list:?} {pane:?}");
                assert!(pane.size.height >= px(220.), "{pane:?}");
            }
            assert!(pane.right() <= px(width), "{width}: {pane:?}");
            crate::desktop::layout_check::assert_inspector(
                window,
                cx,
                "resource-split",
                "resource-body",
                "detail-inspector",
                "detail-title",
            );

            // With no filter to clear, Escape closes the pane and drops
            // the selection it showed.
            window.press("escape", cx);
            window.render_frame(cx);
            assert!(window.try_find("resource-detail").is_none());
            assert_eq!(shown(&screen, cx), None);
            assert_eq!(selected(&screen, cx), None);
        })
        .unwrap();
    }
}

/// Stacked, the pane takes two thirds of the height, as it did before the
/// inspector.
#[gpui_kit::test]
fn the_stacked_pane_takes_two_thirds_of_the_height(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount_window(cx, Some("homelab"), 760., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let first = identity_at(&screen, 0, cx);
        window.within(row_id(&first)).click("name", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let list = window.find("resource-body").bounds();
        let pane = window.find("detail-inspector").bounds();
        let split = window.find("resource-split").bounds();
        assert!(
            (pane.size.height - list.size.height * 2.).abs() <= px(2.),
            "{list:?} {pane:?}"
        );
        assert_eq!(pane.bottom(), split.bottom());
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

/// The pane's width is the page's, for every kind, and comes back when
/// the app opens again.
#[gpui_kit::test]
fn the_pane_width_survives_reopening(cx: &mut TestAppContext) {
    use crate::navigation_file::NavigationFile;
    let directory = std::env::temp_dir().join(format!(
        "freshkube-resources-width-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
    ));
    let preferences = directory.join("preferences.json");
    cx.update(|cx| cx.set_global(NavigationFile::open(Some(&preferences))));
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        open_first(&screen, window, cx);
        let state = screen.read(cx).split.beside_state().clone();
        state.update(cx, |state, cx| {
            state.resize_panel(1, crate::ui::dp_px(560., window), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    let reopened = NavigationFile::open(Some(&preferences));
    assert_eq!(reopened.inspector_width("resources"), Some(560.));

    cx.update(|cx| cx.set_global(reopened));
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        open_first(&screen, window, cx);
        let width = window.find("detail-inspector").bounds().size.width;
        let expected = crate::ui::dp_px(560., window);
        assert!(
            (width - expected).abs() <= px(1.),
            "{width:?}, {expected:?}"
        );
    })
    .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
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
                ResourceBatch {
                    epoch: old_epoch,
                    events: vec![ResourceEvent::reset(columns.clone(), rows.clone())],
                },
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
                ResourceBatch {
                    epoch: old_epoch,
                    events: vec![ResourceEvent::reset(columns.clone(), rows.clone())],
                },
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
            screen.apply(
                ResourceBatch {
                    epoch,
                    events: vec![ResourceEvent::reset(columns, rows)],
                },
                cx,
            );
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
        // on the list. A pod's tabs wrap round to Ports, then Shell.
        window.press("secondary-}", cx);
        assert_eq!(window.find("detail-tab-events").selected(), Some(true));
        window.press("secondary-{", cx);
        window.press("secondary-{", cx);
        window.press("secondary-{", cx);
        window.press("secondary-{", cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-tab-shell").selected(), Some(true));
        assert_eq!(focused(window, "resource-body"), Some(true));
        // Enter on the open row hands the keyboard to the pane, which has
        // no session to give it to.
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
    (0..)
        .find(|&ix| {
            let identity = identity_at(screen, ix, cx);
            screen.read(cx).store.get(&identity).unwrap().cells[2] == "Running"
        })
        .unwrap()
}

/// Dragging the split resizes the shell's terminal, so a program in it
/// draws at the pane's new width.
#[gpui_kit::test]
fn dragging_the_split_resizes_the_shell(cx: &mut TestAppContext) {
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
        let pod = identity_at(&screen, running_row(&screen, cx), cx);
        window.within(row_id(&pod)).click("name", cx);
    });
    step(cx, &|window, cx| window.click("detail-tab-shell", cx));
    step(cx, &|window, cx| window.click("pod-shell-start", cx));
    step(cx, &|_, _| {});
    let columns = |cx: &mut TestAppContext| {
        cx.read(|cx| screen.read(cx).detail.read(cx).terminal_size(cx).columns)
    };
    let before = columns(cx);
    step(cx, &|window, cx| {
        let pane = window.find("detail-inspector").bounds().size.width;
        let state = screen.read(cx).split.beside_state().clone();
        state.update(cx, |state, cx| {
            state.resize_panel(1, pane + px(200.), window, cx)
        });
    });
    // The terminal resizes at most every 100 ms.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    step(cx, &|_, _| {});
    let after = columns(cx);
    assert!(after > before + 10, "{before} → {after}");
}

#[gpui_kit::test]
fn a_running_shell_pins_the_pane_until_the_user_ends_it(cx: &mut TestAppContext) {
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
    let pinned = Rc::new(RefCell::new(None));
    step(cx, &|window, cx| {
        let ix = running_row(&screen, cx);
        let pod = identity_at(&screen, ix, cx);
        window.within(row_id(&pod)).click("name", cx);
        *pinned.borrow_mut() = Some(pod);
    });
    let pod = pinned.borrow().clone().unwrap();
    step(cx, &|window, cx| window.click("detail-tab-shell", cx));
    step(cx, &|window, cx| window.click("pod-shell-start", cx));
    step(cx, &|window, cx| {
        assert!(window.find("detail-shell-running").visible());
        // The list takes the keyboard back; the arrows move the selection
        // but leave the pane on the shell's pod.
        screen.update(cx, |screen, cx| screen.focus(window, cx));
        window.press("down", cx);
    });
    cx.executor().advance_clock(KEYBOARD_PAUSE);
    cx.run_until_parked();
    step(cx, &|_, cx| {
        assert_ne!(selected(&screen, cx).as_ref(), Some(&pod));
        assert_eq!(shown(&screen, cx).as_ref(), Some(&pod));
    });
    assert!(!cx.has_pending_prompt());
    let enter = |cx: &mut TestAppContext| {
        step(cx, &|window, cx| window.press("enter", cx));
        assert!(cx.has_pending_prompt());
        let (message, _) = cx.pending_prompt().unwrap();
        assert_eq!(message, format!("End the shell in {}?", pod.name));
    };
    // Enter on another row asks; Cancel keeps the shell and its pod.
    enter(cx);
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    step(cx, &|window, cx| {
        assert_eq!(shown(&screen, cx).as_ref(), Some(&pod));
        assert!(window.find("detail-shell-running").visible());
        // Another kind keeps the pane too, and Escape asks.
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("deployments.apps"), window, cx)
        });
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx).as_ref(), Some(&pod));
        screen.update(cx, |screen, cx| screen.set_kind(kind("pods"), window, cx));
        window.press("escape", cx);
    });
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    step(cx, &|window, cx| {
        assert_eq!(shown(&screen, cx).as_ref(), Some(&pod));
        // Clicking the shell's own row doesn't ask.
        window.within(row_id(&pod)).click("name", cx);
    });
    assert!(!cx.has_pending_prompt());
    step(cx, &|window, cx| {
        screen.update(cx, |screen, cx| screen.focus(window, cx));
        window.press("down", cx);
    });
    enter(cx);
    cx.simulate_prompt_answer("End the shell");
    cx.run_until_parked();
    step(cx, &|window, cx| {
        let next = selected(&screen, cx);
        assert!(next.is_some() && next.as_ref() != Some(&pod));
        assert_eq!(shown(&screen, cx), next);
        assert!(window.try_find("detail-shell-running").is_none());
        assert_eq!(window.find("resource-detail").focused(), Some(true));
    });
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
    step(cx, &|window, cx| window.click("detail-tab-ports", cx));
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
                ResourceBatch {
                    epoch: old_epoch,
                    events: vec![ResourceEvent::reset(
                        screen.store.columns().to_vec(),
                        old_rows,
                    )],
                },
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
        assert!(healthy.starts_with("Healthy · 20 pods in "), "{healthy}");
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
        window.click("resource-group-node:talos-home-open-node", cx);
    })
    .unwrap();
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

        problems(&screen, cx);
        window.render_frame(cx);
        window.click("resource-group-failing-select", cx);
        window.click("resource-group-pending-select", cx);
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
            assert_eq!(shown(&screen, cx), Some(first.clone()));
            // The pane stays on its tab, and the list keeps the keyboard
            // until the dock takes it.
            assert_eq!(
                screen.read(cx).detail_tab(cx),
                crate::resources::Tab::Overview
            );
            first
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(*asked.borrow(), [first]);
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
        let header = window.find(("resource-sort", 1usize)).bounds().left();
        // Column 2 starts at the pinned run's edge, where its label stays
        // once scrolled; column 3 starts clear of the scroll's 120.
        let edge = window.find(("resource-sort", 2usize)).bounds().left();
        let owner = window.find(("resource-sort", 3usize)).bounds().left();
        assert!(owner - edge > px(120.), "{:?}", owner - edge);
        window.scroll(
            "resource-table-scroll",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(-120.), px(0.))),
            cx,
        );
        window.render_frame(cx);
        let moved = owner - window.find(("resource-sort", 3usize)).bounds().left();
        assert!((f32::from(moved) - 120.).abs() <= 1.5, "{moved:?}");
        // `find` fails on an id that resolves twice.
        assert!((window.find(("resource-sort", 1usize)).bounds().left() - header).abs() <= px(1.5));
        for (row, name) in rows.iter().zip(names) {
            assert!((left(window, row) - name).abs() <= px(1.5));
        }
    })
    .unwrap();
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
            // The first optional column is Owner; the Name and glyph stay fixed.
            window.within("popup-menu").click(0usize, cx);
            assert!(
                screen
                    .read(cx)
                    .hidden_columns
                    .contains(&super::layout::ColumnSource::Owner)
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
    // Refresh lists again, as its button does.
    let before = screen.read_with(cx, |screen, _| screen.usage_generation);
    cx.update_window(handle, |_, window, cx| {
        window.click("resource-more", cx);
        window.render_frame(cx);
        // Namespace, Columns, Refresh.
        window.within("popup-menu").click(2usize, cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_ne!(
        screen.read_with(cx, |screen, _| screen.usage_generation),
        before
    );
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
