use super::*;
use freshkube_probe::probe;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, Context, IntoElement, Render, TestAppContext, Window, div, px, size,
};

#[gpui_kit::test]
fn a_choice_overrides_the_system_and_system_follows_it_again(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert_eq!(choice(cx), Choice::System);
        assert_eq!(system(cx), None);
        system::reported(true, cx);
        assert!(cx.reduce_motion());
        choose(Choice::Full, cx);
        assert!(!cx.reduce_motion());
        // A change the OS reports while overridden waits for System.
        system::reported(true, cx);
        assert!(!cx.reduce_motion());
        assert_eq!(system(cx), Some(true));
        choose(Choice::System, cx);
        assert!(cx.reduce_motion());
        system::reported(false, cx);
        assert!(!cx.reduce_motion());
        choose(Choice::Reduced, cx);
        assert!(cx.reduce_motion());
    });
}

#[gpui_kit::test]
fn without_a_reading_motion_stays_on(cx: &mut TestAppContext) {
    cx.update(|cx| {
        choose(Choice::System, cx);
        assert!(!cx.reduce_motion());
    });
}

/// A view that counts its renders and, while motion is on, asks for the
/// next frame as a moving view does.
struct Moving;

impl Render for Moving {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        probe::hit("motion.moving");
        next_frame(window, cx);
        div().size(px(10.))
    }
}

fn open(cx: &mut TestAppContext) -> AnyWindowHandle {
    let window = cx.open_window(size(px(100.), px(100.)), |_, _| Moving);
    cx.run_until_parked();
    window.into()
}

/// Draws the window if something left it dirty, and delivers the next
/// frame; returns how many frame requests that answered.
fn frame(handle: AnyWindowHandle, cx: &mut TestAppContext) -> usize {
    cx.update_window(handle, |_, _, _| ()).unwrap();
    cx.update_window(handle, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap()
}

#[gpui_kit::test]
fn a_moving_view_asks_frames_with_motion_and_none_without(cx: &mut TestAppContext) {
    let handle = open(cx);
    cx.update_window(handle, |_, window, _| window.refresh())
        .unwrap();
    assert!(frame(handle, cx) > 0);
    assert!(frame(handle, cx) > 0, "and goes on asking");
    cx.update(|cx| choose(Choice::Reduced, cx));
    // The frame asked before the change ran out; none follows it.
    frame(handle, cx);
    assert_eq!(frame(handle, cx), 0);
}

#[gpui_kit::test]
fn an_answer_that_changes_nothing_redraws_nothing(cx: &mut TestAppContext) {
    let handle = open(cx);
    cx.update(|cx| choose(Choice::Reduced, cx));
    frame(handle, cx);
    let drawn = probe::count("motion.moving");
    // macOS announces every accessibility display change alike.
    cx.update(|cx| system::reported(false, cx));
    frame(handle, cx);
    // The OS's answer changed, though reduced motion didn't: the strip
    // that names it draws again.
    assert_eq!(probe::count("motion.moving"), drawn + 1);
    cx.update(|cx| {
        system::reported(false, cx);
        choose(Choice::Reduced, cx);
    });
    frame(handle, cx);
    assert_eq!(probe::count("motion.moving"), drawn + 1);
}

#[gpui_kit::test]
fn loops_run_on_the_executor_clock(cx: &mut TestAppContext) {
    let at = cx.update(|cx| phase(PULSE, cx));
    assert_eq!(at, 0.);
    cx.background_executor.advance_clock(PULSE / 4);
    let quarter = cx.update(|cx| phase(PULSE, cx));
    assert!((quarter - 0.25).abs() < 1e-3, "{quarter}");
    cx.background_executor.advance_clock(PULSE);
    let again = cx.update(|cx| phase(PULSE, cx));
    assert!((again - 0.25).abs() < 1e-3, "a loop comes round: {again}");
    // Half way through the pulse the bars are dimmed the most.
    cx.background_executor.advance_clock(PULSE / 4);
    let dim = cx.update(pulse_dim);
    assert!((dim - PULSE_DIM).abs() < 1e-3, "{dim}");
}

#[test]
fn a_flash_fades_from_full_to_clear() {
    assert_eq!(fade_left(Duration::ZERO), 1.);
    assert!((fade_left(FADE / 2) - 0.75).abs() < 1e-6);
    assert_eq!(fade_left(FADE), 0.);
    assert_eq!(fade_left(FADE * 2), 0.);
}
