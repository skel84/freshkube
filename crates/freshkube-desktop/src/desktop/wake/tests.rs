//! The wake watcher on the executor's clock (#968). Real sleep can't be
//! tested: the clocks are stepped by hand, the wall clock by a cell.
use super::*;
use crate::desktop::testing::fixture;
use freshkube_core::wake::TICK;
use gpui_kit::test::TestWindowExt;
use gpui_kit::TestAppContext;
use std::{cell::Cell, rc::Rc};

fn watch(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    view: &gpui_kit::Entity<Pilot>,
) -> Rc<Cell<Duration>> {
    let wall = Rc::new(Cell::new(Duration::from_secs(1_000)));
    let clock = wall.clone();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.watch_wake(move || clock.get(), window, cx)
        });
    })
    .unwrap();
    wall
}

/// One tick of both clocks, the wall clock also moving by `extra`.
fn tick(cx: &mut TestAppContext, wall: &Cell<Duration>, extra: Duration) {
    wall.set(wall.get() + TICK + extra);
    cx.executor().advance_clock(TICK);
    cx.run_until_parked();
}

fn reconnecting(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    view: &gpui_kit::Entity<Pilot>,
) -> bool {
    cx.update_window(handle, |_, _, cx| view.read(cx).chrome.reconnecting.is_some())
        .unwrap()
}

#[gpui_kit::test]
fn clocks_that_keep_step_never_say_reconnecting(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let wall = watch(cx, handle, &view);
    for _ in 0..20 {
        tick(cx, &wall, Duration::ZERO);
        assert!(!reconnecting(cx, handle, &view));
    }
}

#[gpui_kit::test]
fn a_wake_says_reconnecting_in_the_status_bar_until_an_answer_lands(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let wall = watch(cx, handle, &view);
    tick(cx, &wall, Duration::ZERO);
    // The summary's answer clears the state, so hold it by marking the wake
    // as the watcher does and looking before anything answers.
    wall.set(wall.get() + Duration::from_secs(600));
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.chrome.reconnecting = Some(Duration::ZERO);
            cx.notify();
        });
        window.render_frame(cx);
        let label = window.find("overview-status").label().unwrap();
        assert!(label.contains("Reconnecting to"), "{label}");
        view.update(cx, |pilot, cx| pilot.reconnected(cx));
        window.render_frame(cx);
        let label = window.find("overview-status").label().unwrap();
        assert!(!label.contains("Reconnecting"), "{label}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_watcher_runs_the_wake_once_per_sleep(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let wall = watch(cx, handle, &view);
    tick(cx, &wall, Duration::ZERO);
    cx.update_window(handle, |_, _, cx| {
        view.update(cx, |pilot, _| pilot.chrome.reconnecting = None)
    })
    .unwrap();
    // A ten-minute sleep inside one tick: the wake runs, and what it starts
    // is a refresh the person could have asked for.
    tick(cx, &wall, Duration::from_secs(600));
    // The ordinary tick after it is quiet: nothing marks it again.
    cx.update_window(handle, |_, _, cx| {
        view.update(cx, |pilot, _| pilot.chrome.reconnecting = None)
    })
    .unwrap();
    tick(cx, &wall, Duration::ZERO);
    assert!(!reconnecting(cx, handle, &view));
}

#[gpui_kit::test]
fn the_words_do_not_outlive_the_limit(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, _, cx| {
        view.update(cx, |pilot, cx| {
            pilot.chrome.reconnecting = Some(Duration::ZERO);
            for _ in 0..RECONNECTING_LIMIT.as_secs() - 1 {
                pilot.tick_reconnecting(cx);
            }
            assert!(pilot.chrome.reconnecting.is_some());
            pilot.tick_reconnecting(cx);
            assert!(pilot.chrome.reconnecting.is_none());
        })
    })
    .unwrap();
}
