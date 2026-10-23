use super::*;

#[gpui_kit::test]
fn the_drawer_story_opens_swaps_resizes_and_closes(cx: &mut TestAppContext) {
    use freshkube_ui::drawer::{MIN_WIDTH, WIDTH};
    let (handle, workbench) = open(cx, super::stories::find("drawer").unwrap());
    let story = cx.update(|cx| story::<DrawerStory>(&workbench, cx));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("drawer-frame").is_none());
        window.click("drawer-row-2", cx);
        window.render_frame(cx);
        window.render_frame(cx);
        let body = window.find("drawer-body").bounds();
        let frame = window.find("drawer-frame").bounds();
        assert!((frame.right() - body.right()).abs() <= px(1.), "{frame:?}");
        assert!((frame.top() - body.top()).abs() <= px(1.), "{frame:?}");
        assert!((frame.size.width - px(WIDTH)).abs() <= px(1.), "{frame:?}");
        assert_eq!(
            window.find("drawer-object").label(),
            Some("worker-5c8d7-hq4tn")
        );

        // Another row swaps what it shows; the edge resizes within bounds.
        window.click_at("drawer-row-0", point(px(40.), px(8.)), cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).selected(), Some(0));
        assert!(story.read(cx).is_open());
        let y = frame.center().y;
        window.drag(
            point(frame.left() + px(2.), y),
            point(frame.left() + px(200.), y),
            cx,
        );
        window.render_frame(cx);
        // The edge follows the pointer.
        let width = story.read(cx).width();
        assert!((width - (WIDTH - 200.)).abs() <= 1., "{width}");
        story.update(cx, |story, cx| story.resize(10., window, cx));
        assert_eq!(story.read(cx).width(), MIN_WIDTH);

        // A click beside the rows closes it.
        let list = window.find("drawer-list").bounds();
        window.click_at(
            "drawer-list",
            point(px(40.), list.size.height - px(10.)),
            cx,
        );
        window.render_frame(cx);
        assert!(!story.read(cx).is_open());
        assert!(window.try_find("drawer-frame").is_none());
    })
    .unwrap();
}
