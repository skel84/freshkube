use std::{cell::Cell, rc::Rc, time::Duration};

use gpui_kit::{AppContext, Entity, TestAppContext};
use tokio::{runtime::Runtime, sync::oneshot};

use super::Loader;

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

/// Runs GPUI's executor until `done`, while Tokio works on its own threads,
/// or fails after a few seconds.
fn until(cx: &mut TestAppContext, done: impl Fn() -> bool) {
    for _ in 0..500 {
        cx.run_until_parked();
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the condition never held");
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
    until(cx, || delivered.get() == 1);
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
    until(cx, || delivered.get() == 1);
    // The replaced read's job was cancelled, dropping its work: its answer
    // has nowhere to go.
    until(cx, || old.is_closed());
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
    until(cx, || send.is_closed());
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
    until(cx, || delivered.get() == 1);
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
    until(cx, || delivered.get() == 1);
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
    until(cx, || delivered.get() == 1);
    screen.read_with(cx, |screen, _| {
        assert!(screen.loader.data().is_none());
        assert_eq!(screen.loader.error(), Some("Loading the number timed out"));
    });
}
