use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    let (runtime, screen, _handle) = mount(cx, Some("homelab"));
    // The watch's stream sends from Tokio's threads.
    cx.executor().allow_parking();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    // How many batches the stream has passed on to the watch.
    let forwarded = Arc::new(AtomicUsize::new(0));
    let passed = forwarded.clone();
    screen.update(cx, |screen, cx| {
        let epoch = screen.store.epoch();
        screen.start_watch(
            epoch,
            move |watch| async move {
                while let Some(events) = receiver.recv().await {
                    if watch.send(events).await.is_err() {
                        return;
                    }
                    passed.fetch_add(1, Ordering::SeqCst);
                }
            },
            cx,
        );
    });
    let rows = |cx: &mut TestAppContext| screen.read_with(cx, |screen, _| screen.store.len());
    let upsert = |n| vec![ResourceEvent::Upsert(example::inserted_pod(n))];
    // Sends a batch, then runs GPUI until the stream, on Tokio, has passed
    // it on to the watch.
    let send = |events: Vec<ResourceEvent>, cx: &mut TestAppContext| {
        let target = forwarded.load(Ordering::SeqCst) + 1;
        runtime.block_on(sender.send(events)).unwrap();
        for _ in 0..100_000 {
            cx.run_until_parked();
            if forwarded.load(Ordering::SeqCst) >= target {
                return;
            }
            std::thread::yield_now();
        }
        panic!("the stream never passed the batch on");
    };
    let listed = rows(cx);

    // After a quiet spell a change shows at once.
    send(upsert(1), cx);
    cx.run_until_parked();
    assert_eq!(rows(cx), listed + 1);

    // More within the window wait for it, then apply together.
    send(upsert(2), cx);
    cx.run_until_parked();
    cx.executor().advance_clock(WATCH_COALESCE / 2);
    send(upsert(3), cx);
    cx.run_until_parked();
    assert_eq!(rows(cx), listed + 1);
    cx.executor().advance_clock(WATCH_COALESCE / 2);
    cx.run_until_parked();
    assert_eq!(rows(cx), listed + 3);

    cx.executor().advance_clock(WATCH_COALESCE * 2);
    send(upsert(4), cx);
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
                example::read("homelab", "pods", None, super::super::live::now()).unwrap();
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
fn old_metrics_cannot_change_the_new_access_or_a_disconnected_page(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            let generation = screen.usage_generation;
            screen.set_source(Some(source("prod-fra")), window, cx);
            assert!(!screen.apply_usage(generation, Err("late failure".into()), cx));
            assert_eq!(screen.usage_state, super::super::UsageState::Known);
            let generation = screen.usage_generation;
            assert!(screen.apply_usage(generation, Err("current failure".into()), cx));
            assert_eq!(screen.usage_state, super::super::UsageState::Stale);
            screen.set_source(None, window, cx);
            assert!(screen.usage.is_none());
            assert!(!screen.apply_usage(generation, Ok(Vec::new()), cx));
            assert_eq!(screen.usage_state, super::super::UsageState::Unknown);
        });
    })
    .unwrap();
}
