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
    /// What had landed each time a stream ended by itself.
    ends: Vec<Vec<u32>>,
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

    /// As `begin`, noting in `ends` what had landed when the stream ends by
    /// itself.
    fn begin_then(&mut self, identity: &'static str, cx: &mut Context<Self>) -> mpsc::Sender<u32> {
        let (feed, mut items) = mpsc::channel::<u32>(16);
        let runtime = self.runtime.clone();
        self.watch.start_then(
            identity,
            &runtime,
            self.pace,
            move |sender| async move {
                while let Some(item) = items.recv().await {
                    if sender.send(item).await.is_err() {
                        return;
                    }
                }
            },
            |screen: &mut Screen| &mut screen.watch,
            Self::apply,
            |screen: &mut Screen, _| {
                assert!(!screen.watch.is_running(), "ended before its end runs");
                screen.ends.push(screen.seen.clone());
            },
            cx,
        );
        feed
    }

    /// Opens the watch for `identity` with no work of its own.
    fn open(&mut self, identity: &'static str, cx: &mut Context<Self>) {
        self.watch.open(
            identity,
            self.pace,
            |screen: &mut Screen| &mut screen.watch,
            Self::apply,
            cx,
        );
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
        ends: Vec::new(),
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

#[gpui_kit::test]
fn a_sender_beside_the_work_feeds_the_same_delivery(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let feed = start(&screen, &runtime, "a", cx);
    let beside = screen.read_with(cx, |screen, _| screen.watch.sender().expect("running"));
    runtime.block_on(feed.send(1)).unwrap();
    until(cx, |cx| seen(&screen, cx) == [1]);
    beside.try_send(2).unwrap();
    until(cx, |cx| seen(&screen, cx) == [1, 2]);

    // Stopping closes it with the stream, and the watch has none to give.
    screen.update(cx, |screen, _| {
        screen.watch.stop();
        assert!(screen.watch.sender().is_none());
    });
    until(cx, |_| beside.is_closed());
    assert!(beside.try_send(3).is_err());
    assert_eq!(seen(&screen, cx), [1, 2]);
}

#[gpui_kit::test]
fn a_sender_taken_before_starting_again_never_lands(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let _old = start(&screen, &runtime, "a", cx);
    let old = screen.read_with(cx, |screen, _| screen.watch.sender().expect("running"));
    let _new = start(&screen, &runtime, "b", cx);
    until(cx, |_| old.is_closed());
    assert!(old.try_send(1).is_err());
    let new = screen.read_with(cx, |screen, _| screen.watch.sender().expect("running"));
    new.try_send(2).unwrap();
    until(cx, |cx| seen(&screen, cx) == [2]);
}

#[gpui_kit::test]
fn a_sender_used_right_after_a_restart_for_the_same_identity_never_lands(
    cx: &mut TestAppContext,
) {
    let (runtime, screen) = mount(cx);
    let _old = start(&screen, &runtime, "a", cx);
    let old = screen.read_with(cx, |screen, _| screen.watch.sender().expect("running"));
    let _new = start(&screen, &runtime, "a", cx);
    // The old channel still takes it: dropping the old delivery keeps it
    // out, and so would the generation, since the identity is the same.
    old.try_send(1).unwrap();
    let new = screen.read_with(cx, |screen, _| screen.watch.sender().expect("running"));
    new.try_send(2).unwrap();
    until(cx, |cx| seen(&screen, cx) == [2]);
    cx.run_until_parked();
    assert_eq!(seen(&screen, cx), [2]);
}

#[gpui_kit::test]
fn a_taken_sender_keeps_the_stream_open_and_the_watch_does_not(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let feed = start(&screen, &runtime, "a", cx);
    let beside = screen.read_with(cx, |screen, _| screen.watch.sender().expect("running"));
    // The work ends, and the sender taken beside it still feeds.
    drop(feed);
    beside.try_send(1).unwrap();
    until(cx, |cx| seen(&screen, cx) == [1]);
    assert!(screen.read_with(cx, |screen, _| screen.watch.is_running()));
    // Once it goes too, the stream ends: the watch held it only weakly.
    drop(beside);
    until(cx, |cx| {
        screen.read_with(cx, |screen, _| !screen.watch.is_running())
    });
    assert!(screen.read_with(cx, |screen, _| screen.watch.sender().is_none()));
}

#[gpui_kit::test]
fn each_start_and_stop_moves_the_generation(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let first = screen.read_with(cx, |screen, _| screen.watch.generation());
    let _feed = start(&screen, &runtime, "a", cx);
    let started = screen.read_with(cx, |screen, _| screen.watch.generation());
    assert!(started > first);
    screen.update(cx, |screen, _| screen.watch.stop());
    assert!(screen.read_with(cx, |screen, _| screen.watch.generation()) > started);
}

#[gpui_kit::test]
fn an_opened_watch_is_fed_only_through_its_senders(cx: &mut TestAppContext) {
    let pace = Pace {
        every: EVERY,
        ..PACE
    };
    let (_runtime, screen) = mount_paced(pace, cx);
    screen.update(cx, |screen, cx| screen.open("a", cx));
    let feed = screen.read_with(cx, |screen, _| {
        assert!(screen.watch.is_running());
        assert_eq!(screen.watch.identity(), Some(&"a"));
        screen.watch.sender().expect("open")
    });
    // Sent on the GPUI thread, as an example writer does, and paced as a
    // started stream's items are.
    feed.try_send(1).unwrap();
    cx.run_until_parked();
    assert_eq!(seen(&screen, cx), [1]);
    feed.try_send(2).unwrap();
    feed.try_send(3).unwrap();
    cx.run_until_parked();
    assert_eq!(seen(&screen, cx), [1], "not yet");
    cx.executor().advance_clock(EVERY);
    cx.run_until_parked();
    screen.read_with(cx, |screen, _| {
        assert_eq!(screen.seen, [1, 2, 3]);
        assert_eq!(screen.sizes, [1, 2]);
    });

    // No sender taken keeps it open: dropping this one ends nothing.
    drop(feed);
    cx.run_until_parked();
    assert!(screen.read_with(cx, |screen, _| screen.watch.is_running()));
}

#[gpui_kit::test]
fn an_opened_watch_closes_when_it_stops_or_starts_again(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    screen.update(cx, |screen, cx| screen.open("a", cx));
    let first = screen.read_with(cx, |screen, _| screen.watch.sender().expect("open"));
    screen.update(cx, |screen, cx| screen.open("b", cx));
    // A dropped delivery lets its channel go once the executor runs.
    cx.run_until_parked();
    assert!(first.is_closed(), "opening again dropped the first");
    assert!(first.try_send(1).is_err());

    let second = screen.read_with(cx, |screen, _| screen.watch.sender().expect("open"));
    second.try_send(2).unwrap();
    cx.run_until_parked();
    assert_eq!(seen(&screen, cx), [2]);
    screen.update(cx, |screen, _| {
        screen.watch.stop();
        assert!(!screen.watch.is_running());
        assert!(screen.watch.sender().is_none());
    });
    cx.run_until_parked();
    assert!(second.is_closed());

    // A start replaces an opened stream too.
    screen.update(cx, |screen, cx| screen.open("a", cx));
    let opened = screen.read_with(cx, |screen, _| screen.watch.sender().expect("open"));
    let _feed = start(&screen, &runtime, "a", cx);
    cx.run_until_parked();
    assert!(opened.is_closed());
}

#[gpui_kit::test]
fn a_stream_that_ends_by_itself_runs_its_end_after_its_last_batch(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let feed = screen.update(cx, |screen, cx| screen.begin_then("a", cx));
    runtime.block_on(feed.send(1)).unwrap();
    runtime.block_on(feed.send(2)).unwrap();
    drop(feed);
    until(cx, |cx| {
        screen.read_with(cx, |screen, _| !screen.watch.is_running())
    });
    screen.read_with(cx, |screen, _| {
        assert_eq!(screen.seen, [1, 2]);
        assert_eq!(screen.ends, [vec![1, 2]], "once, after the last batch");
    });
}

#[gpui_kit::test]
fn stopping_or_starting_again_runs_no_end(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let feed = screen.update(cx, |screen, cx| screen.begin_then("a", cx));
    screen.update(cx, |screen, _| screen.watch.stop());
    until(cx, |_| feed.is_closed());

    let feed = screen.update(cx, |screen, cx| screen.begin_then("a", cx));
    let _new = start(&screen, &runtime, "b", cx);
    until(cx, |_| feed.is_closed());
    cx.run_until_parked();
    assert!(screen.read_with(cx, |screen, _| screen.ends.is_empty()));
}
