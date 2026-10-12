use std::time::Duration;

use gpui_kit::{AppContext, Entity, TestAppContext};
use tokio::{runtime::Runtime, sync::mpsc};

use super::{Pace, Ticket, Watch};
use crate::OnShow;

const PACE: Pace = Pace {
    capacity: 16,
    most: 16,
    every: Duration::ZERO,
};

/// A view that owns one watch and keeps what it sent, as a list keeps its
/// rows.
struct Screen {
    watch: Watch<u32, &'static str>,
    seen: Vec<u32>,
    batches: usize,
    /// Stops the watch from its own delivery on this item, as a tab's pod
    /// watch ends when its pod goes.
    stop_on: Option<u32>,
}

fn mount(cx: &mut TestAppContext) -> (Runtime, Entity<Screen>) {
    // Tokio sends from its own threads.
    cx.executor().allow_parking();
    let screen = cx.new(|_| Screen {
        watch: Watch::default(),
        seen: Vec::new(),
        batches: 0,
        stop_on: None,
    });
    (Runtime::new().unwrap(), screen)
}

/// Starts the watch for `identity` on a stream that sends what the returned
/// sender is given, and closes when the stream's work is dropped.
fn start(
    screen: &Entity<Screen>,
    runtime: &Runtime,
    identity: &'static str,
    cx: &mut TestAppContext,
) -> mpsc::Sender<u32> {
    let (feed, mut items) = mpsc::channel::<u32>(16);
    screen.update(cx, |screen, cx| {
        screen.watch.start(
            identity,
            runtime.handle(),
            PACE,
            move |sender| async move {
                while let Some(item) = items.recv().await {
                    if sender.send(item).await.is_err() {
                        return;
                    }
                }
            },
            |screen: &mut Screen| &mut screen.watch,
            |screen: &mut Screen, batch, _| {
                screen.batches += 1;
                for item in batch {
                    screen.seen.push(item);
                    if screen.stop_on == Some(item) {
                        screen.watch.stop();
                    }
                }
            },
            cx,
        );
    });
    feed
}

/// Runs GPUI's executor until `done`, while Tokio works on its own threads,
/// or fails after a few seconds.
fn until(cx: &mut TestAppContext, done: impl Fn(&mut TestAppContext) -> bool) {
    for _ in 0..500 {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the condition never held");
}

fn seen(screen: &Entity<Screen>, cx: &mut TestAppContext) -> Vec<u32> {
    screen.read_with(cx, |screen, _| screen.seen.clone())
}

#[gpui_kit::test]
fn batches_land_for_the_latest_start(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let feed = start(&screen, &runtime, "a", cx);
    let started = screen.read_with(cx, |screen, _| {
        assert!(screen.watch.is_running());
        assert_eq!(screen.watch.identity(), Some(&"a"));
        screen.watch.revision()
    });
    runtime.block_on(feed.send(1)).unwrap();
    until(cx, |cx| seen(&screen, cx) == [1]);
    runtime.block_on(feed.send(2)).unwrap();
    until(cx, |cx| seen(&screen, cx) == [1, 2]);
    screen.read_with(cx, |screen, _| {
        assert!(screen.watch.is_running());
        // One revision a batch.
        assert_eq!(screen.watch.revision(), started + screen.batches as u64);
    });
}

#[gpui_kit::test]
fn a_late_batch_for_an_old_identity_is_dropped(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let old = start(&screen, &runtime, "a", cx);
    let new = start(&screen, &runtime, "b", cx);
    // Starting again dropped the old stream's work: what it would send has
    // nowhere to go.
    until(cx, |_| old.is_closed());
    assert!(runtime.block_on(old.send(1)).is_err());
    runtime.block_on(new.send(2)).unwrap();
    until(cx, |cx| seen(&screen, cx) == [2]);

    // A batch that does arrive for a start no longer current is refused,
    // and the revision stays.
    screen.update(cx, |screen, cx| {
        let stale = Ticket {
            identity: "a",
            generation: screen.watch.generation,
        };
        let revision = screen.watch.revision();
        let delivered = Watch::deliver(
            screen,
            &stale,
            vec![3],
            &|screen: &mut Screen| &mut screen.watch,
            &|screen: &mut Screen, batch, _| screen.seen.extend(batch),
            cx,
        );
        assert!(!delivered);
        assert_eq!(screen.seen, [2]);
        assert_eq!(screen.watch.revision(), revision);
    });
}

#[gpui_kit::test]
fn stop_cancels_the_stream(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let feed = start(&screen, &runtime, "a", cx);
    runtime.block_on(feed.send(1)).unwrap();
    until(cx, |cx| seen(&screen, cx) == [1]);
    screen.update(cx, |screen, _| screen.watch.stop());
    until(cx, |_| feed.is_closed());
    screen.read_with(cx, |screen, _| {
        assert!(!screen.watch.is_running());
        assert_eq!(screen.watch.identity(), Some(&"a"));
        assert_eq!(screen.seen, [1], "what it sent stays");
    });
}

#[gpui_kit::test]
fn a_delivery_may_stop_its_own_watch(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    screen.update(cx, |screen, _| screen.stop_on = Some(1));
    let feed = start(&screen, &runtime, "a", cx);
    runtime.block_on(feed.send(1)).unwrap();
    until(cx, |_| feed.is_closed());
    assert!(runtime.block_on(feed.send(2)).is_err());
    cx.run_until_parked();
    screen.read_with(cx, |screen, _| {
        assert!(!screen.watch.is_running());
        assert_eq!(screen.seen, [1]);
    });
}

#[gpui_kit::test]
fn a_stream_that_ends_stops_its_watch(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let feed = start(&screen, &runtime, "a", cx);
    runtime.block_on(feed.send(1)).unwrap();
    drop(feed);
    until(cx, |cx| {
        screen.read_with(cx, |screen, _| !screen.watch.is_running())
    });
    assert_eq!(seen(&screen, cx), [1]);
}

#[gpui_kit::test]
fn hiding_stops_the_stream_and_keeps_what_it_sent(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let feed = start(&screen, &runtime, "a", cx);
    runtime.block_on(feed.send(1)).unwrap();
    until(cx, |cx| seen(&screen, cx) == [1]);
    let policy = OnShow::ReadIfOlder(Duration::from_secs(30));
    screen.update(cx, |screen, cx| {
        assert!(!screen.watch.set_visible(false, policy, cx));
        assert!(!screen.watch.is_visible());
    });
    until(cx, |_| feed.is_closed());
    screen.read_with(cx, |screen, _| {
        assert!(!screen.watch.is_running());
        assert_eq!(screen.seen, [1]);
    });
}

#[gpui_kit::test]
fn showing_starts_again_past_the_threshold_and_not_before(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let _feed = start(&screen, &runtime, "a", cx);
    let policy = OnShow::ReadIfOlder(Duration::from_secs(30));
    screen.update(cx, |screen, cx| screen.watch.set_visible(false, policy, cx));
    cx.executor().advance_clock(Duration::from_secs(29));
    screen.update(cx, |screen, cx| {
        assert!(!screen.watch.set_visible(true, policy, cx), "not yet");
        screen.watch.set_visible(false, policy, cx);
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    screen.update(cx, |screen, cx| {
        assert!(
            screen.watch.set_visible(true, policy, cx),
            "30 s since it hid"
        );
    });

    // Started again, it hides and shows anew; one that was stopped before
    // it hid isn't started again.
    let _feed = start(&screen, &runtime, "a", cx);
    screen.update(cx, |screen, cx| {
        screen.watch.stop();
        screen.watch.set_visible(false, policy, cx);
    });
    cx.executor().advance_clock(Duration::from_secs(60));
    screen.update(cx, |screen, cx| {
        assert!(!screen.watch.set_visible(true, policy, cx))
    });
}

#[gpui_kit::test]
fn never_starts_again_on_showing(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let _feed = start(&screen, &runtime, "a", cx);
    screen.update(cx, |screen, cx| {
        screen.watch.set_visible(false, OnShow::Never, cx)
    });
    cx.executor().advance_clock(Duration::from_secs(3_600));
    screen.update(cx, |screen, cx| {
        assert!(!screen.watch.set_visible(true, OnShow::Never, cx));
        assert!(!screen.watch.is_running());
    });
}
