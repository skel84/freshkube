use super::*;

#[gpui_kit::test]
fn a_hovered_flame_frame_keeps_its_colour_in_light(cx: &mut TestAppContext) {
    use gpui_kit::{InputEvent, MouseMoveEvent, SharedString};
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        Theme::change(ThemeMode::Light, None, cx);
        page.update(cx, |page, cx| page.open(Destination::Profiling, cx));
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let colour = |index: usize| crate::palette::flame_color(page.read(cx).frames[index].delta);
        let (index, other) = (
            page.read(cx).visible_frames[0],
            page.read(cx).visible_frames[1],
        );
        let (color, other_color) = (colour(index), colour(other));
        let id = SharedString::from(format!("obs-flame-{index}"));
        let fill = |window: &mut Window, index: usize| {
            let bounds = window
                .find(SharedString::from(format!("obs-flame-{index}")))
                .bounds()
                .scale(window.scale_factor());
            window
                .painted_quads()
                .into_iter()
                .filter(|quad| (quad.bounds.size.width.0 - bounds.size.width.0).abs() < 1.)
                .find(|quad| bounds.contains(&quad.bounds.center()))
                .and_then(|quad| quad.background.as_solid())
                .expect("the frame paints its fill")
        };
        assert_eq!(fill(window, index), color);
        // Still a button for Tab, Enter and Space, as the Kit button was.
        assert_eq!(window.find(id.clone()).role(), Some(gpui_kit::Role::Button));
        let position = window.find(id.clone()).bounds().center();
        window.dispatch_event(
            MouseMoveEvent {
                position,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        // Ghost's hover filled it pale under the white label (#404); it now
        // lightens the frame's own colour a little.
        let hovered = fill(window, index);
        assert_eq!((hovered.h, hovered.s), (color.h, color.s));
        assert!(
            hovered.l > color.l && hovered.l < color.l + 0.1,
            "{hovered:?} should be {color:?} a little lighter"
        );
        assert_eq!(
            fill(window, other),
            other_color,
            "the other frames keep theirs"
        );
    })
    .unwrap();
}
