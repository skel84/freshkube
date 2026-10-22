//! Frozen rows: a tab's last list shown muted while the list is read again,
//! flashing what changed once it lands, and kept, marked stale, when it
//! fails.

use super::*;
use crate::frozen::FrozenRows;
use freshkube_core::cluster_source::Hold;

/// The page's pods, frozen, and the page switched to example data that
/// never answers its lists, so the next read stays on what froze.
fn frozen_and_held(
    cx: &mut TestAppContext,
) -> (
    Runtime,
    Entity<ResourcesScreen>,
    AnyWindowHandle,
    FrozenRows,
) {
    let (runtime, screen, handle) = mount(cx, Some("demo"));
    let frozen = cx.update(|cx| screen.read(cx).freeze("pods", None).expect("pods froze"));
    cx.update(|cx| {
        screen.update(cx, |screen, _| {
            screen.source.as_mut().unwrap().access = ClusterAccess::Example(Hold {
                lists: true,
                ..Hold::NONE
            })
        })
    });
    (runtime, screen, handle, frozen)
}

fn restart(cx: &mut TestAppContext, screen: &Entity<ResourcesScreen>, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| screen.restart(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn frozen_rows_show_muted_until_the_list_answers_and_then_flash_what_changed(
    cx: &mut TestAppContext,
) {
    let (_runtime, screen, handle, frozen) = frozen_and_held(cx);
    cx.update(|cx| cx.set_reduce_motion(false));
    let kept = frozen.kept();
    assert!(kept > 1);
    let (columns, mut rows) = cx.update(|cx| {
        let store = &screen.read(cx).store;
        (
            store.columns().to_vec(),
            store
                .entries()
                .iter()
                .map(|entry| entry.row().clone())
                .collect::<Vec<_>>(),
        )
    });
    cx.update(|cx| screen.update(cx, |screen, cx| screen.hold_frozen(frozen, cx)));
    restart(cx, &screen, handle);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let screen = screen.read(cx);
        assert!(screen.rows_are_frozen());
        assert_eq!(screen.store.len(), kept);
        assert_eq!(*screen.store.read_state(), ReadState::Loading);
        assert!(window.find("resource-frozen").visible());
        assert!(window.try_find("resource-stale").is_none());
        // Rows, not loading rows.
        assert!(window.find("resource-list").visible());
    })
    .unwrap();

    // The banner's words are derived when the rows freeze and as the age
    // clock ticks, not in render.
    let words = |cx: &mut TestAppContext| {
        cx.read(|cx| screen.read(cx).frozen.as_ref().unwrap().text.to_string())
    };
    assert!(
        words(cx).contains("last known moments ago"),
        "{}",
        words(cx)
    );
    cx.update(|cx| {
        screen.update(cx, |screen, cx| {
            screen.now += 300;
            screen.tick_frozen(cx);
        })
    });
    assert!(words(cx).contains("last known 5 min ago"), "{}", words(cx));

    // The list answers: one object changed meanwhile.
    rows[0].resource_version = "2".into();
    let changed = rows[0].identity.clone();
    cx.update(|cx| {
        deliver(
            &screen,
            vec![
                ResourceEvent::reset(columns, rows),
                ResourceEvent::Read(ReadState::Loaded),
            ],
            cx,
        )
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!screen.read(cx).rows_are_frozen());
        assert!(window.try_find("resource-frozen").is_none());
        assert_eq!(screen.read(cx).flashing(cx), vec![changed]);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_failed_list_keeps_frozen_rows_marked_stale_with_retry(cx: &mut TestAppContext) {
    let (_runtime, screen, handle, frozen) = frozen_and_held(cx);
    let kept = frozen.kept();
    cx.update(|cx| screen.update(cx, |screen, cx| screen.hold_frozen(frozen, cx)));
    restart(cx, &screen, handle);
    cx.update(|cx| {
        deliver(
            &screen,
            vec![ResourceEvent::Read(ReadState::Failed(
                "Unreachable · connection reset".into(),
            ))],
            cx,
        )
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = screen.read(cx);
        // Never an empty list: the rows stay, marked stale.
        assert_eq!(page.store.len(), kept);
        assert!(matches!(page.store.read_state(), ReadState::Stale(_)));
        assert!(page.rows_are_frozen());
        assert!(window.find("resource-stale").visible());
        assert!(window.find("resource-list").visible());
        // One banner says it, not two.
        assert!(window.try_find("resource-frozen").is_none());
    })
    .unwrap();

    // Retry reads again, and the rows stay up until the list answers.
    cx.update_window(handle, |_, window, cx| {
        window.click("resource-stale-retry", cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let page = screen.read(cx);
        assert!(page.rows_are_frozen());
        assert_eq!(page.store.len(), kept);
    });
}

#[gpui_kit::test]
fn freezing_asks_for_the_kind_and_namespace_the_page_lists(cx: &mut TestAppContext) {
    let (_runtime, screen, _handle) = mount(cx, Some("demo"));
    cx.update(|cx| {
        let page = screen.read(cx);
        assert!(page.freeze("pods", None).is_some());
        assert!(page.freeze("deployments.apps", None).is_none());
        assert!(page.freeze("pods", Some("payments")).is_none());
    });
}

#[gpui_kit::test]
fn frozen_rows_for_another_read_are_not_shown(cx: &mut TestAppContext) {
    let (_runtime, screen, handle, frozen) = frozen_and_held(cx);
    cx.update(|cx| screen.update(cx, |screen, cx| screen.hold_frozen(frozen, cx)));
    // The page moves to another kind first: the held rows are not its own.
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("deployments.apps"), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let page = screen.read(cx);
        assert!(!page.rows_are_frozen());
        assert_eq!(page.store.len(), 0);
    });
}

#[gpui_kit::test]
fn a_freeze_not_taken_soon_is_forgotten(cx: &mut TestAppContext) {
    let (_runtime, screen, handle, frozen) = frozen_and_held(cx);
    cx.update(|cx| screen.update(cx, |screen, cx| screen.hold_frozen(frozen, cx)));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(5));
    restart(cx, &screen, handle);
    cx.update(|cx| assert!(!screen.read(cx).rows_are_frozen()));
}
