use crate::test_support::until;
use std::{cell::Cell, rc::Rc, time::Duration};

use gpui_kit::{AppContext, Entity, TestAppContext};
use tokio::{runtime::Runtime, sync::oneshot};

use super::Loader;
use crate::OnShow;

const DEADLINE: Duration = Duration::from_secs(5);

/// A view that owns one loader, as a screen does.
struct Screen {
    loader: Loader<u32, &'static str>,
}

fn mount(cx: &mut TestAppContext) -> (Runtime, Entity<Screen>) {
    // Tokio answers from its own threads.
    cx.executor().allow_parking();
    let screen = cx.new(|_| Screen {
        loader: Loader::default(),
    });
    (Runtime::new().unwrap(), screen)
}

fn load(
    screen: &Entity<Screen>,
    runtime: &Runtime,
    identity: &'static str,
    deadline: Duration,
    answer: oneshot::Receiver<Result<u32, String>>,
    delivered: Rc<Cell<usize>>,
    cx: &mut TestAppContext,
) {
    screen.update(cx, |screen, cx| {
        screen.loader.load(
            identity,
            runtime.handle(),
            deadline,
            "the number",
            async move { answer.await.unwrap_or_else(|_| Err("closed".into())) },
            move |screen: &mut Screen| {
                delivered.set(delivered.get() + 1);
                &mut screen.loader
            },
            cx,
        );
    });
}

#[gpui_kit::test]
fn an_answer_lands_for_the_latest_request(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let (send, answer) = oneshot::channel();
    let delivered = Rc::new(Cell::new(0));
    load(
        &screen,
        &runtime,
        "a",
        DEADLINE,
        answer,
        delivered.clone(),
        cx,
    );
    let started = screen.read_with(cx, |screen, _| {
        assert!(screen.loader.is_loading());
        screen.loader.revision()
    });
    send.send(Ok(7)).unwrap();
    until(cx, |_| delivered.get() == 1);
    screen.read_with(cx, |screen, _| {
        assert_eq!(screen.loader.data(), Some(&7));
        assert!(!screen.loader.is_loading());
        assert!(screen.loader.last_successful().is_some());
        assert!(screen.loader.revision() > started);
    });
}

#[gpui_kit::test]
fn a_late_answer_for_an_old_identity_is_dropped(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let (old, old_answer) = oneshot::channel();
    let (new, new_answer) = oneshot::channel();
    let delivered = Rc::new(Cell::new(0));
    load(
        &screen,
        &runtime,
        "a",
        DEADLINE,
        old_answer,
        delivered.clone(),
        cx,
    );
    load(
        &screen,
        &runtime,
        "b",
        DEADLINE,
        new_answer,
        delivered.clone(),
        cx,
    );
    new.send(Ok(2)).unwrap();
    until(cx, |_| delivered.get() == 1);
    // The replaced read's job was cancelled, dropping its work: its answer
    // has nowhere to go.
    until(cx, |_| old.is_closed());
    assert!(old.send(Ok(1)).is_err());
    cx.run_until_parked();
    assert_eq!(delivered.get(), 1);
    screen.read_with(cx, |screen, _| assert_eq!(screen.loader.data(), Some(&2)));

    // An answer that does arrive for an identity no longer asked for is
    // refused, and the newer one's data stays.
    screen.update(cx, |screen, _| {
        let stale = screen.loader.begin("a");
        screen.loader.resolve("b", Ok(3));
        let revision = screen.loader.revision();
        assert!(!screen.loader.apply(&stale, Ok(1)));
        assert_eq!(screen.loader.data(), Some(&3));
        assert_eq!(screen.loader.revision(), revision);
    });
}

#[gpui_kit::test]
fn reset_clears_data_and_drops_the_read_in_flight(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let (send, answer) = oneshot::channel();
    let delivered = Rc::new(Cell::new(0));
    let before = screen.update(cx, |screen, _| {
        screen.loader.resolve("a", Ok(1));
        screen.loader.revision()
    });
    load(
        &screen,
        &runtime,
        "a",
        DEADLINE,
        answer,
        delivered.clone(),
        cx,
    );
    screen.update(cx, |screen, _| screen.loader.reset());
    // Reset cancelled the job, dropping its work.
    until(cx, |_| send.is_closed());
    assert!(send.send(Ok(2)).is_err());
    cx.run_until_parked();
    assert_eq!(delivered.get(), 0);
    screen.read_with(cx, |screen, _| {
        assert!(screen.loader.data().is_none());
        assert!(screen.loader.error().is_none());
        assert!(!screen.loader.is_loading());
        assert!(screen.loader.last_successful().is_none());
        // A view that derives by revision derives again.
        assert!(screen.loader.revision() > before);
    });
}

#[gpui_kit::test]
fn a_changed_identity_clears_the_data(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let (send, answer) = oneshot::channel();
    let delivered = Rc::new(Cell::new(0));
    screen.update(cx, |screen, _| screen.loader.resolve("a", Ok(1)));
    load(
        &screen,
        &runtime,
        "b",
        DEADLINE,
        answer,
        delivered.clone(),
        cx,
    );
    // Unlike a refresh for the same identity, nothing of "a" shows for "b".
    screen.read_with(cx, |screen, _| {
        assert!(screen.loader.data().is_none());
        assert!(screen.loader.is_loading());
        assert!(screen.loader.last_successful().is_none());
    });
    send.send(Ok(2)).unwrap();
    until(cx, |_| delivered.get() == 1);
    screen.read_with(cx, |screen, _| assert_eq!(screen.loader.data(), Some(&2)));
}

#[gpui_kit::test]
fn a_refresh_failure_keeps_the_data_marked_stale(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let (send, answer) = oneshot::channel();
    let delivered = Rc::new(Cell::new(0));
    screen.update(cx, |screen, _| screen.loader.resolve("a", Ok(1)));
    load(
        &screen,
        &runtime,
        "a",
        DEADLINE,
        answer,
        delivered.clone(),
        cx,
    );
    screen.read_with(cx, |screen, _| {
        assert!(screen.loader.is_loading());
        assert_eq!(screen.loader.data(), Some(&1), "kept while it refreshes");
    });
    send.send(Err("offline".into())).unwrap();
    until(cx, |_| delivered.get() == 1);
    screen.read_with(cx, |screen, _| {
        assert_eq!(screen.loader.data(), Some(&1));
        assert_eq!(screen.loader.error(), Some("offline"));
        assert!(screen.loader.is_stale());
        assert!(screen.loader.last_failure().is_some());
    });
}

#[gpui_kit::test]
fn a_read_past_its_deadline_fails_in_its_own_words(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let (_send, answer) = oneshot::channel();
    let delivered = Rc::new(Cell::new(0));
    let deadline = Duration::from_millis(20);
    load(
        &screen,
        &runtime,
        "a",
        deadline,
        answer,
        delivered.clone(),
        cx,
    );
    until(cx, |_| delivered.get() == 1);
    screen.read_with(cx, |screen, _| {
        assert!(screen.loader.data().is_none());
        assert_eq!(screen.loader.error(), Some("Loading the number timed out"));
    });
}

#[gpui_kit::test]
fn hiding_drops_the_read_and_keeps_the_data(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let (send, answer) = oneshot::channel();
    let delivered = Rc::new(Cell::new(0));
    screen.update(cx, |screen, _| screen.loader.resolve("a", Ok(1)));
    load(
        &screen,
        &runtime,
        "a",
        DEADLINE,
        answer,
        delivered.clone(),
        cx,
    );
    let policy = OnShow::ReadIfOlder(Duration::from_secs(30));
    screen.update(cx, |screen, cx| {
        assert!(!screen.loader.set_visible(false, policy, cx));
        assert!(!screen.loader.is_visible());
    });
    // Hiding cancelled the job, dropping its work.
    until(cx, |_| send.is_closed());
    assert!(send.send(Ok(2)).is_err());
    cx.run_until_parked();
    assert_eq!(delivered.get(), 0);
    screen.read_with(cx, |screen, _| {
        assert_eq!(screen.loader.data(), Some(&1), "kept as last known");
        assert!(!screen.loader.is_loading());
    });
    // The read it cut short is read again, however recent the data.
    screen.update(cx, |screen, cx| {
        assert!(screen.loader.set_visible(true, policy, cx))
    });
}

#[gpui_kit::test]
fn showing_reads_again_past_the_threshold_and_not_before(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let (send, answer) = oneshot::channel();
    let delivered = Rc::new(Cell::new(0));
    load(
        &screen,
        &runtime,
        "a",
        DEADLINE,
        answer,
        delivered.clone(),
        cx,
    );
    send.send(Ok(7)).unwrap();
    until(cx, |_| delivered.get() == 1);
    let policy = OnShow::ReadIfOlder(Duration::from_secs(30));
    screen.update(cx, |screen, cx| {
        screen.loader.set_visible(false, policy, cx)
    });
    cx.executor().advance_clock(Duration::from_secs(29));
    screen.update(cx, |screen, cx| {
        assert!(!screen.loader.set_visible(true, policy, cx), "not yet");
        assert!(
            !screen.loader.set_visible(true, policy, cx),
            "already shown"
        );
        screen.loader.set_visible(false, policy, cx);
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    screen.update(cx, |screen, cx| {
        assert!(screen.loader.set_visible(true, policy, cx), "30 s old");
        assert_eq!(screen.loader.data(), Some(&7));
    });

    // Example data has no age, so it counts as old.
    screen.update(cx, |screen, cx| {
        screen.loader.resolve("a", Ok(8));
        screen.loader.set_visible(false, policy, cx);
        assert!(screen.loader.set_visible(true, policy, cx));
    });
}

#[gpui_kit::test]
fn never_reads_again_on_showing(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let (_send, answer) = oneshot::channel();
    let delivered = Rc::new(Cell::new(0));
    screen.update(cx, |screen, _| screen.loader.resolve("a", Ok(1)));
    load(
        &screen,
        &runtime,
        "a",
        DEADLINE,
        answer,
        delivered.clone(),
        cx,
    );
    screen.update(cx, |screen, cx| {
        screen.loader.set_visible(false, OnShow::Never, cx)
    });
    cx.executor().advance_clock(Duration::from_secs(3_600));
    screen.update(cx, |screen, cx| {
        assert!(!screen.loader.set_visible(true, OnShow::Never, cx));
        assert_eq!(screen.loader.data(), Some(&1));
        assert!(!screen.loader.is_loading());
    });
}

#[gpui_kit::test]
fn reset_keeps_whether_it_shows(cx: &mut TestAppContext) {
    let (_runtime, screen) = mount(cx);
    screen.update(cx, |screen, cx| {
        screen.loader.set_visible(false, OnShow::Never, cx);
        screen.loader.reset();
        assert!(!screen.loader.is_visible());
    });
}

#[gpui_kit::test]
fn a_failed_refresh_reads_again_on_showing(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let delivered = Rc::new(Cell::new(0));
    let (send, answer) = oneshot::channel();
    load(
        &screen,
        &runtime,
        "a",
        DEADLINE,
        answer,
        delivered.clone(),
        cx,
    );
    send.send(Ok(1)).unwrap();
    until(cx, |_| delivered.get() == 1);
    let (send, answer) = oneshot::channel();
    load(
        &screen,
        &runtime,
        "a",
        DEADLINE,
        answer,
        delivered.clone(),
        cx,
    );
    send.send(Err("offline".into())).unwrap();
    until(cx, |_| delivered.get() == 2);
    let policy = OnShow::ReadIfOlder(Duration::from_secs(30));
    screen.update(cx, |screen, cx| {
        screen.loader.set_visible(false, policy, cx)
    });
    cx.executor().advance_clock(Duration::from_secs(5));
    screen.update(cx, |screen, cx| {
        // The data is 5 s old, but the refresh failed: it isn't fresh.
        assert!(screen.loader.set_visible(true, policy, cx));
        assert_eq!(screen.loader.data(), Some(&1));
    });
}

#[gpui_kit::test]
fn a_read_after_showing_again_lands(cx: &mut TestAppContext) {
    let (runtime, screen) = mount(cx);
    let delivered = Rc::new(Cell::new(0));
    let (cut, answer) = oneshot::channel();
    load(
        &screen,
        &runtime,
        "a",
        DEADLINE,
        answer,
        delivered.clone(),
        cx,
    );
    let policy = OnShow::ReadIfOlder(Duration::from_secs(30));
    let hidden = screen.update(cx, |screen, cx| {
        screen.loader.set_visible(false, policy, cx);
        assert!(!screen.loader.is_loading());
        screen.loader.revision()
    });
    until(cx, |_| cut.is_closed());
    let reads_again = screen.update(cx, |screen, cx| screen.loader.set_visible(true, policy, cx));
    assert!(reads_again);
    let (send, answer) = oneshot::channel();
    load(
        &screen,
        &runtime,
        "a",
        DEADLINE,
        answer,
        delivered.clone(),
        cx,
    );
    screen.read_with(cx, |screen, _| assert!(screen.loader.is_loading()));
    send.send(Ok(4)).unwrap();
    until(cx, |_| delivered.get() == 1);
    screen.read_with(cx, |screen, _| {
        assert_eq!(screen.loader.data(), Some(&4));
        assert!(!screen.loader.is_loading());
        assert!(screen.loader.revision() > hidden);
    });
}

#[gpui_kit::test]
fn a_read_the_owner_lands_is_timed_and_superseded_like_a_load(cx: &mut TestAppContext) {
    let screen = cx.new(|_| Screen {
        loader: Loader::default(),
    });
    let policy = OnShow::ReadIfOlder(Duration::from_secs(30));
    screen.update(cx, |screen, cx| {
        let old = screen.loader.start("a");
        assert!(screen.loader.is_loading());
        let request = screen.loader.start("a");
        assert!(!screen.loader.land(&old, Ok(1), cx), "superseded");
        assert!(screen.loader.land(&request, Ok(2), cx));
        assert_eq!(screen.loader.data(), Some(&2));
        assert!(!screen.loader.is_loading());
        screen.loader.set_visible(false, policy, cx);
    });
    // Unlike example data resolved at once, a landed answer has an age.
    cx.executor().advance_clock(Duration::from_secs(29));
    screen.update(cx, |screen, cx| {
        assert!(!screen.loader.set_visible(true, policy, cx), "fresh");
        screen.loader.set_visible(false, policy, cx);
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    screen.update(cx, |screen, cx| {
        assert!(screen.loader.set_visible(true, policy, cx), "30 s old");
    });
}

#[gpui_kit::test]
fn a_failed_landing_keeps_the_data_stale_and_old(cx: &mut TestAppContext) {
    let screen = cx.new(|_| Screen {
        loader: Loader::default(),
    });
    let policy = OnShow::ReadIfOlder(Duration::from_secs(30));
    screen.update(cx, |screen, cx| {
        let request = screen.loader.start("a");
        screen.loader.land(&request, Ok(1), cx);
        let request = screen.loader.start("a");
        assert!(screen.loader.land(&request, Err("offline".into()), cx));
        assert_eq!(screen.loader.data(), Some(&1));
        assert!(screen.loader.is_stale());
        screen.loader.set_visible(false, policy, cx);
        assert!(screen.loader.set_visible(true, policy, cx));
    });
}

#[gpui_kit::test]
fn hiding_cuts_a_held_read_and_drops_its_task(cx: &mut TestAppContext) {
    let screen = cx.new(|_| Screen {
        loader: Loader::default(),
    });
    let policy = OnShow::ReadIfOlder(Duration::from_secs(30));
    let landed = Rc::new(Cell::new(false));
    // Held: a read with no work still waits.
    screen.update(cx, |screen, cx| {
        let request = screen.loader.start("a");
        screen.loader.land(&request, Ok(1), cx);
        let held = screen.loader.start("a");
        assert!(screen.loader.is_loading());
        let revision = screen.loader.revision();
        screen.loader.set_visible(false, policy, cx);
        assert!(!screen.loader.is_loading());
        assert!(screen.loader.revision() > revision);
        assert_eq!(screen.loader.data(), Some(&1), "kept as last known");
        // Its answer, arriving after all, never lands.
        assert!(!screen.loader.land(&held, Ok(2), cx));
        assert_eq!(screen.loader.data(), Some(&1));
        assert!(!screen.loader.is_loading());
        // A read cut short counts as old.
        assert!(screen.loader.set_visible(true, policy, cx));
    });
    // A read with a task: hiding drops the task, so its answer never lands.
    screen.update(cx, |screen, cx| {
        let request = screen.loader.start("a");
        let landed = landed.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            _ = this.update(cx, |screen, cx| {
                landed.set(screen.loader.land(&request, Ok(9), cx));
            });
        });
        screen.loader.run(None, task);
        screen.loader.set_visible(false, policy, cx);
    });
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert!(!landed.get());
    screen.read_with(cx, |screen, _| assert_eq!(screen.loader.data(), Some(&1)));
}

#[gpui_kit::test]
fn expiring_reads_again_on_showing_however_recent(cx: &mut TestAppContext) {
    let screen = cx.new(|_| Screen {
        loader: Loader::default(),
    });
    let policy = OnShow::ReadIfOlder(Duration::from_secs(30));
    screen.update(cx, |screen, cx| {
        let request = screen.loader.start("a");
        screen.loader.land(&request, Ok(1), cx);
        screen.loader.set_visible(false, policy, cx);
        screen.loader.expire();
        assert!(screen.loader.set_visible(true, policy, cx));
        assert_eq!(screen.loader.data(), Some(&1), "the data stays");
    });
}

#[gpui_kit::test]
fn retaining_a_route_invalidates_old_work_without_clearing_evidence(cx: &mut TestAppContext) {
    let (_runtime, screen) = mount(cx);
    screen.update(cx, |screen, cx| {
        screen.loader.resolve("source/old-route", Ok(7));
        let at = screen.loader.last_successful();
        let old = screen.loader.start("source/old-route");
        screen.loader.retain_stale("source/new-route");
        assert_eq!(screen.loader.data(), Some(&7));
        assert!(screen.loader.is_stale());
        assert!(!screen.loader.is_loading());
        assert_eq!(screen.loader.last_successful(), at);
        assert!(!screen.loader.land(&old, Ok(9), cx));
        let new = screen.loader.start("source/new-route");
        assert_eq!(screen.loader.data(), Some(&7));
        assert!(screen.loader.is_stale());
        assert!(screen.loader.land(&new, Ok(8), cx));
        assert_eq!(screen.loader.data(), Some(&8));
        assert!(!screen.loader.is_stale());
    });
}
