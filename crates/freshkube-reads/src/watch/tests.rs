use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use gpui_kit::{AppContext, Context, Entity, TestAppContext};
use tokio::{
    runtime::{Handle, Runtime},
    sync::mpsc,
};

use super::{Pace, Ticket, Watch};

const PACE: Pace = Pace {
    capacity: 16,
    most: 16,
    every: Duration::ZERO,
};

/// The least time between batches, where a test paces them.
const EVERY: Duration = Duration::from_millis(100);

/// A view that owns one watch and keeps what it sent, as a list keeps its
/// rows.
struct Screen {
    runtime: Handle,
    pace: Pace,
    watch: Watch<u32, &'static str>,
    seen: Vec<u32>,
    /// Each batch's length, in order.
    sizes: Vec<usize>,
    /// Stops the watch from its own delivery on this item, as a tab's pod
    /// watch ends when its pod goes.
    stop_on: Option<u32>,
    /// Starts the watch again for "b" from its own delivery on this item,
    /// keeping the new stream's feed in `restarted`.
    restart_on: Option<u32>,
    restarted: Option<mpsc::Sender<u32>>,
    /// How many items the current stream has passed on to the watch.
    forwarded: Arc<AtomicUsize>,
}

impl Screen {
    /// Starts the watch for `identity` on a stream that sends what the
    /// returned sender is given, and closes when the stream's work is
    /// dropped.
    fn begin(&mut self, identity: &'static str, cx: &mut Context<Self>) -> mpsc::Sender<u32> {
        let (feed, mut items) = mpsc::channel::<u32>(16);
        let forwarded = Arc::new(AtomicUsize::new(0));
        self.forwarded = forwarded.clone();
        let runtime = self.runtime.clone();
        self.watch.start(
            identity,
            &runtime,
            self.pace,
            move |sender| async move {
                while let Some(item) = items.recv().await {
                    if sender.send(item).await.is_err() {
                        return;
                    }
                    forwarded.fetch_add(1, Ordering::SeqCst);
                }
            },
            |screen: &mut Screen| &mut screen.watch,
            Self::apply,
            cx,
        );
        feed
    }

    fn apply(&mut self, batch: Vec<u32>, cx: &mut Context<Self>) {
        self.sizes.push(batch.len());
        for item in batch {
            self.seen.push(item);
            if self.stop_on == Some(item) {
                self.watch.stop();
            }
            if self.restart_on == Some(item) {
                self.restarted = Some(self.begin("b", cx));
            }
        }
    }
}

fn mount(cx: &mut TestAppContext) -> (Runtime, Entity<Screen>) {
    mount_paced(PACE, cx)
}

fn mount_paced(pace: Pace, cx: &mut TestAppContext) -> (Runtime, Entity<Screen>) {
    // Tokio sends from its own threads.
    cx.executor().allow_parking();
    let runtime = Runtime::new().unwrap();
    let handle = runtime.handle().clone();
    let screen = cx.new(|_| Screen {
        runtime: handle,
        pace,
        watch: Watch::default(),
        seen: Vec::new(),
        sizes: Vec::new(),
        stop_on: None,
        restart_on: None,
        restarted: None,
        forwarded: Arc::default(),
    });
    (runtime, screen)
}

fn start(
    screen: &Entity<Screen>,
    _runtime: &Runtime,
    identity: &'static str,
    cx: &mut TestAppContext,
) -> mpsc::Sender<u32> {
    screen.update(cx, |screen, cx| screen.begin(identity, cx))
}

/// Sends `items` and runs GPUI until the stream has passed them all on to
/// the watch's channel; a paced delivery holds them there until its time.
fn queue(
    screen: &Entity<Screen>,
    feed: &mpsc::Sender<u32>,
    runtime: &Runtime,
    items: &[u32],
    cx: &mut TestAppContext,
) {
    let forwarded = screen.read_with(cx, |screen, _| screen.forwarded.clone());
    let target = forwarded.load(Ordering::SeqCst) + items.len();
    for item in items {
        runtime.block_on(feed.send(*item)).unwrap();
    }
    const TURNS: usize = 100_000;
    for _ in 0..TURNS {
        cx.run_until_parked();
        if forwarded.load(Ordering::SeqCst) >= target {
            return;
        }
        std::thread::yield_now();
    }
    panic!(
        "the stream passed on {} of {target} items in {TURNS} turns",
        forwarded.load(Ordering::SeqCst)
    );
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
        assert_eq!(screen.watch.revision(), started + screen.sizes.len() as u64);
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
    screen.update(cx, |screen, _| {
        assert!(!screen.watch.set_visible(false));
        assert!(!screen.watch.is_visible());
    });
    until(cx, |_| feed.is_closed());
    screen.read_with(cx, |screen, _| {
        assert!(!screen.watch.is_running());
        assert_eq!(screen.seen, [1]);
    });
}

#[gpui_kit::test]
fn showing_starts_again_whatever_the_age(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let _feed = start(&screen, &runtime, "a", cx);
    // Shown again at once: a stopped stream holds nothing fresh.
    screen.update(cx, |screen, _| {
        screen.watch.set_visible(false);
        assert!(screen.watch.set_visible(true));
        assert!(!screen.watch.set_visible(true), "already shown");
    });

    // One the owner didn't start again isn't asked for twice, and one
    // stopped before it hid isn't started again.
    screen.update(cx, |screen, _| {
        screen.watch.set_visible(false);
        assert!(!screen.watch.set_visible(true));
    });
    let _feed = start(&screen, &runtime, "a", cx);
    screen.update(cx, |screen, _| {
        screen.watch.stop();
        screen.watch.set_visible(false);
        assert!(!screen.watch.set_visible(true));
    });
}

#[gpui_kit::test]
fn batches_wait_for_their_pace(cx: &mut TestAppContext) {
    let pace = Pace {
        every: EVERY,
        ..PACE
    };
    let (runtime, screen) = mount_paced(pace, cx);
    let feed = start(&screen, &runtime, "a", cx);
    // The first batch doesn't wait.
    runtime.block_on(feed.send(1)).unwrap();
    until(cx, |cx| seen(&screen, cx) == [1]);

    // What arrives sooner waits, then lands as one batch.
    queue(&screen, &feed, &runtime, &[2, 3, 4], cx);
    cx.run_until_parked();
    assert_eq!(seen(&screen, cx), [1]);
    cx.executor().advance_clock(EVERY);
    cx.run_until_parked();
    screen.read_with(cx, |screen, _| {
        assert_eq!(screen.seen, [1, 2, 3, 4]);
        assert_eq!(screen.sizes, [1, 3]);
    });

    queue(&screen, &feed, &runtime, &[5], cx);
    cx.executor()
        .advance_clock(EVERY - Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(seen(&screen, cx), [1, 2, 3, 4], "not yet");
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(seen(&screen, cx), [1, 2, 3, 4, 5]);
}

#[gpui_kit::test]
fn a_batch_carries_at_most_its_cap(cx: &mut TestAppContext) {
    let pace = Pace {
        most: 2,
        every: EVERY,
        ..PACE
    };
    let (runtime, screen) = mount_paced(pace, cx);
    let feed = start(&screen, &runtime, "a", cx);
    runtime.block_on(feed.send(1)).unwrap();
    until(cx, |cx| seen(&screen, cx) == [1]);
    queue(&screen, &feed, &runtime, &[2, 3, 4, 5, 6], cx);
    for _ in 0..3 {
        cx.executor().advance_clock(EVERY);
        cx.run_until_parked();
    }
    screen.read_with(cx, |screen, _| {
        assert_eq!(screen.seen, [1, 2, 3, 4, 5, 6]);
        assert_eq!(screen.sizes, [1, 2, 2, 1]);
    });
}

#[gpui_kit::test]
fn a_delivery_may_start_its_own_watch_again(cx: &mut TestAppContext) {
    let pace = Pace {
        most: 1,
        every: EVERY,
        ..PACE
    };
    let (runtime, screen) = mount_paced(pace, cx);
    screen.update(cx, |screen, _| screen.restart_on = Some(2));
    let old = start(&screen, &runtime, "a", cx);
    runtime.block_on(old.send(1)).unwrap();
    until(cx, |cx| seen(&screen, cx) == [1]);
    // 3 is queued behind 2, whose delivery starts the watch again.
    queue(&screen, &old, &runtime, &[2, 3], cx);
    cx.executor().advance_clock(EVERY);
    cx.run_until_parked();
    until(cx, |_| old.is_closed());
    let new = screen.update(cx, |screen, _| {
        assert_eq!(screen.watch.identity(), Some(&"b"));
        assert!(screen.watch.is_running());
        screen.restarted.take().expect("started again")
    });
    runtime.block_on(new.send(10)).unwrap();
    until(cx, |cx| seen(&screen, cx).ends_with(&[10]));
    for _ in 0..3 {
        cx.executor().advance_clock(EVERY);
        cx.run_until_parked();
    }
    assert_eq!(
        seen(&screen, cx),
        [1, 2, 10],
        "the old stream's 3 never lands"
    );
}
