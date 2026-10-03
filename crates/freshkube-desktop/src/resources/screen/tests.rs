use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, px, size};
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
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        cx.set_reduce_motion(true);
    });
    let runtime = Runtime::new().unwrap();
    let source = context.map(source);
    let mut screen = None;
    let handle = cx.open_window(size(px(width), px(800.)), |window, cx| {
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
fn header_controls_share_one_row_and_fit_a_narrow_page(cx: &mut TestAppContext) {
    // The page alone, in windows as wide as the app's: breakpoints leave
    // out the sidebar, so 760, the narrowest window, leaves the page 480.
    for (width, beside_title) in [(1280., true), (760., false)] {
        let (_runtime, _screen, handle) = mount_sized(cx, Some("homelab"), width);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let title = window.find("resource-title").bounds();
            let namespace = window.find("resource-namespace").bounds();
            let filter = window.find("resource-filter").bounds();
            let refresh = window.find("resource-refresh").bounds();
            for control in [namespace, filter, refresh] {
                assert_eq!(control.center().y, filter.center().y, "{width}");
                assert!(control.left() >= px(0.), "{width}: {control:?}");
                assert!(control.right() <= px(width), "{width}: {control:?}");
            }
            assert!(namespace.right() <= filter.left());
            assert!(filter.right() <= refresh.left());
            assert!(filter.size.width >= px(120.), "{width}: {filter:?}");
            if beside_title {
                assert!(namespace.left() > title.right(), "{namespace:?}");
            } else {
                assert!(namespace.top() > title.bottom(), "{namespace:?}");
            }
            assert!(window.find("resource-list").bounds().top() > filter.bottom());
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
        window.click(row_id(&third), cx);
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
        window.click(row_id(&first), cx);
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
        window.click(row_id(&first), cx);

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
            window.click(row_id(&third), cx);
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

#[gpui_kit::test]
fn arrow_keys_show_the_next_row_once_the_keyboard_pauses(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let first = identity_at(&screen, 0, cx);
        window.click(row_id(&first), cx);
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
            window.click(row_id(&first), cx);
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
            window.click(row_id(&identity_at(&screen, 0, cx)), cx);
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
        // on the list. A pod's tabs wrap round to Ports, Shell, then Logs.
        window.press("secondary-}", cx);
        assert_eq!(window.find("detail-tab-events").selected(), Some(true));
        window.press("secondary-{", cx);
        window.press("secondary-{", cx);
        window.press("secondary-{", cx);
        window.press("secondary-{", cx);
        window.press("secondary-{", cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-tab-logs").selected(), Some(true));
        assert_eq!(focused(window, "resource-body"), Some(true));
        // Enter on the open row hands the keyboard to the lines.
        window.press("enter", cx);
        window.render_frame(cx);
        assert_eq!(focused(window, "logs-viewport"), Some(true));
        // With no line selected, Escape goes on to the pane, which steps
        // back to the list.
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
        window.click(row_id(&pod), cx);
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
        window.click(row_id(&pod), cx);
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
        window.click(row_id(&identity_at(&screen, ix, cx)), cx);
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
        assert_eq!(
            screen.read(cx).detail.read(cx).tab(),
            crate::resources::Tab::Logs
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
        assert!(window.find("resource-collapsed").visible());
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
        window.click(row_id(&first), cx);
        window.press("x", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).marked.contains(&first));
        assert!(window.find("resource-marks").visible());
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
fn l_opens_the_selected_pod_on_its_logs(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let first = identity_at(&screen, 0, cx);
        window.click(row_id(&first), cx);
        window.render_frame(cx);
        window.press("l", cx);
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), Some(first));
        assert_eq!(screen.read(cx).detail_tab(cx), crate::resources::Tab::Logs);
    })
    .unwrap();
}

#[gpui_kit::test]
fn density_switches_between_comfortable_and_compact_rows(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let first = row_id(&identity_at(&screen, 0, cx));
        let comfortable = window.find(first.clone()).bounds().size.height;
        window.click("resource-density", cx);
        window.render_frame(cx);
        let compact = window.find(first.clone()).bounds().size.height;
        assert!(
            (compact / comfortable - 26. / 34.).abs() < 0.01,
            "{compact:?}"
        );
        window.click("resource-density", cx);
        window.render_frame(cx);
        assert_eq!(window.find(first).bounds().size.height, comfortable);
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
