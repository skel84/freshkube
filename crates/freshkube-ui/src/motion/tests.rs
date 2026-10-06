use super::*;
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

/// A pulse, a shimmer and a fade, each in its own element.
struct Animated;

impl Render for Animated {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .child(pulse("pulse", div().size(px(10.))))
            .child(shimmer("shimmer", div().size(px(10.)), |el, phase| {
                el.left(px(phase * 10.))
            }))
            .child(fade_out("fade", div().size(px(10.))))
    }
}

fn open(cx: &mut TestAppContext) -> AnyWindowHandle {
    let window = cx.open_window(size(px(100.), px(100.)), |_, _| Animated);
    cx.run_until_parked();
    window.into()
}

#[gpui_kit::test]
fn the_animations_ask_frames_with_motion_and_none_without(cx: &mut TestAppContext) {
    let handle = open(cx);
    // Each update's end draws the window it left dirty.
    cx.update_window(handle, |_, window, _| window.refresh())
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        assert!(window.simulate_next_frame(cx) > 0);
    })
    .unwrap();
    cx.update(|cx| choose(Choice::Reduced, cx));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        // The frame asked before the change ran out; none follows it.
        window.simulate_next_frame(cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(window.simulate_next_frame(cx), 0);
    })
    .unwrap();
}
