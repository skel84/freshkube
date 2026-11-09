//! The wake watcher on the executor's clock (#968). Real sleep can't be
//! tested: the clocks are stepped by hand, the wall clock by a cell.
use super::{RECONNECTING_LIMIT, WAKE_DEBOUNCE};
use crate::desktop::Pilot;
use crate::desktop::testing::fixture;
use freshkube_core::wake::TICK;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use std::time::Duration;
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
    cx.update_window(handle, |_, _, cx| {
        view.read(cx).chrome.reconnecting.is_some()
    })
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
    // Mark the wake as the shell does, and look before anything answers.
    wall.set(wall.get() + Duration::from_secs(600));
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.chrome.reconnecting = Some(Duration::ZERO);
            cx.notify();
        });
        window.render_frame(cx);
        let label = window.find("overview-status").label().unwrap().to_string();
        assert!(label.contains("Reconnecting to"), "{label}");
        view.update(cx, |pilot, cx| pilot.reconnected(cx));
        window.render_frame(cx);
        let label = window.find("overview-status").label().unwrap().to_string();
        assert!(!label.contains("Reconnecting"), "{label}");
    })
    .unwrap();
}

/// The generation of the summary session's reads, which each relist raises.
fn generation(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    view: &gpui_kit::Entity<Pilot>,
) -> u64 {
    cx.update_window(handle, |_, _, cx| {
        view.read(cx)
            .switch
            .registry
            .active()
            .summary_session
            .as_ref()
            .expect("the fixture's summary session")
            .core
            .generation()
    })
    .unwrap()
}

#[gpui_kit::test]
fn each_sleep_relists_the_summary_once(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let wall = watch(cx, handle, &view);
    tick(cx, &wall, Duration::ZERO);
    let before = generation(cx, handle, &view);
    // A ten-minute sleep inside one tick relists once.
    tick(cx, &wall, Duration::from_secs(600));
    assert_eq!(generation(cx, handle, &view), before + 1);
    // The ordinary ticks after it relist nothing.
    for _ in 0..5 {
        tick(cx, &wall, Duration::ZERO);
    }
    assert_eq!(generation(cx, handle, &view), before + 1);
    // A second sleep, after the debounce, is a second relist.
    for _ in 0..(WAKE_DEBOUNCE.as_secs() / TICK.as_secs()) {
        tick(cx, &wall, Duration::ZERO);
    }
    tick(cx, &wall, Duration::from_secs(90));
    assert_eq!(generation(cx, handle, &view), before + 2);
}

#[gpui_kit::test]
fn a_second_gap_soon_after_the_first_is_the_same_wake(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let wall = watch(cx, handle, &view);
    tick(cx, &wall, Duration::ZERO);
    let before = generation(cx, handle, &view);
    // A sleep, then a clock step five seconds on (an NTP correction).
    tick(cx, &wall, Duration::from_secs(600));
    tick(cx, &wall, Duration::from_secs(40));
    assert_eq!(generation(cx, handle, &view), before + 1);
}

#[gpui_kit::test]
fn a_wake_with_nothing_read_yet_says_nothing_and_reads_nothing(cx: &mut TestAppContext) {
    use crate::{DebugOptions, GpuiOptions};
    let mut debug = DebugOptions::default();
    debug.hold.talos = true;
    let (_runtime, handle, view) =
        crate::desktop::testing::mount(cx, GpuiOptions::fixture().with_debug(debug), 1280., 820.);
    let wall = watch(cx, handle, &view);
    tick(cx, &wall, Duration::ZERO);
    tick(cx, &wall, Duration::from_secs(600));
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
