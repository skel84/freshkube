use super::*;

/// The squares story draws a row per state, and Many draws a pod whose
/// squares the cell cuts at eight and counts the rest.
#[gpui_kit::test]
fn the_squares_story_shows_each_state_and_a_crowded_pod(cx: &mut TestAppContext) {
    use super::stories::squares::SquaresStory;
    let ix = super::stories::find("squares").unwrap();
    let (handle, workbench) = open(cx, ix);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let story = workbench
            .read(cx)
            .view()
            .clone()
            .downcast::<SquaresStory>()
            .unwrap();
        for pod in ["app-a", "app-e", "app-h", "shop-api"] {
            assert!(window.find(format!("squares-{pod}")).visible(), "{pod}");
        }
        window.click("squares-many", cx);
        window.render_frame(cx);
        assert!(story.read(cx).many());
        let crowded = window.find("squares-shop-mesh").bounds();
        assert!(window.try_find("squares-app-a").is_none());
        assert!(crowded.size.width > px(0.));
        window.click("squares-states", cx);
        window.render_frame(cx);
        assert!(!story.read(cx).many());
        assert!(window.find("squares-app-a").visible());
    })
    .unwrap();
}
