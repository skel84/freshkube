use super::*;

#[gpui_kit::test]
fn the_dock_story_adds_closes_resizes_and_fits_its_tabs(cx: &mut TestAppContext) {
    let (handle, workbench) = open(cx, super::stories::find("dock").unwrap());
    let story = cx.update(|cx| story::<DockStory>(&workbench, cx));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = window.find("dock-page").bounds();
        let dock = window.find("dock").bounds();
        // The dock spans the story under its page, at the default height.
        assert!((dock.bottom() - page.bottom()).abs() <= px(1.), "{dock:?}");
        assert!((dock.size.width - page.size.width).abs() <= px(1.));
        assert!((dock.size.height - px(freshkube_ui::dock::DEFAULT_HEIGHT)).abs() <= px(1.));
        window.click("dock-add", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).tabs().len(), 3);
        assert_eq!(window.find("dock-tab-2").selected(), Some(true));
        window.click("dock-tab-0-close", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).tabs().len(), 2);

        // Below the least height the dock minimizes to its bar.
        story.update(cx, |story, cx| story.resize(60., cx));
        window.render_frame(cx);
        assert_eq!(story.read(cx).state(), dock_story::State::Minimized);
        assert!(window.try_find("dock-body").is_none());
        story.update(cx, |story, cx| story.resize(220., cx));
        window.render_frame(cx);
        let dock = window.find("dock").bounds();
        assert!((dock.size.height - px(220.)).abs() <= px(1.), "{dock:?}");

        // Fit to window takes the page's room.
        window.click("dock-state-fitted", cx);
        window.render_frame(cx);
        let page = window.find("dock-page").bounds();
        let dock = window.find("dock").bounds();
        assert!(
            (dock.size.height - page.size.height).abs() <= px(1.),
            "{dock:?} {page:?}"
        );
    })
    .unwrap();
}
