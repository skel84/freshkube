use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, px, size};
use tokio::runtime::Runtime;

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use std::cell::RefCell;
use std::rc::Rc;

use freshkube_core::resources::ResourceKind;

use super::{KEYBOARD_PAUSE, KubeAccess, KubeSource, NotServed, ResourcesScreen, row_id};
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
    // The page alone: 1050 is a wide window less the sidebar, 480 the
    // narrowest window less the sidebar.
    for (width, beside_title) in [(1050., true), (480., false)] {
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
        // Every namespace: a Namespace column after the name. The wide
        // columns (IP, Node) are left out.
        let labels: Vec<String> = (0..6usize)
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
            ["Name", "Namespace", "Ready", "Status", "Restarts", "Age"]
        );
        assert!(window.try_find(("resource-sort", 6usize)).is_none());

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
        window.click(("resource-sort", 0usize), cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("resource-sort", 0usize)).label(),
            Some("Name, sorted ascending")
        );
        assert_eq!(selected(&screen, cx), Some(fourth.clone()));
        assert_eq!(window.find(row_id(&fourth)).selected(), Some(true));
        assert_ne!(identity_at(&screen, 3, cx), fourth);
        window.click(("resource-sort", 0usize), cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("resource-sort", 0usize)).label(),
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
        // One namespace needs no Namespace column.
        assert_eq!(
            window.find(("resource-sort", 1usize)).label(),
            Some("Ready")
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
        assert_eq!(
            window.find(("resource-sort", 1usize)).label(),
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
            let list = window.find("resource-list").bounds();
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

        // The same connection again (new credentials) keeps it open.
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
    step(cx, &|window, cx| window.press("/", cx));
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
    // A choice applies, and the arrows move through the list again.
    step(cx, &|window, cx| window.click("resource-namespace", cx));
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
