use super::*;

#[gpui_kit::test]
fn every_story_draws_in_both_themes_at_text_13_and_20(cx: &mut TestAppContext) {
    for (ix, story) in STORIES.iter().enumerate() {
        let (handle, _) = open(cx, ix);
        for dark in [false, true] {
            for text in [13., 20.] {
                cx.update_window(handle, |_, window, cx| {
                    let mode = if dark {
                        ThemeMode::Dark
                    } else {
                        ThemeMode::Light
                    };
                    Theme::change(mode, Some(window), cx);
                    text_size::set(text, cx);
                    window.render_frame(cx);
                    assert_eq!(cx.theme().mode.is_dark(), dark);
                    assert_eq!(window.rem_size(), px(text));
                    let page = window.find(format!("{}-page", story.slug));
                    assert!(page.visible(), "{} at {text}, dark {dark}", story.slug);
                    assert!(window.find(format!("story-{}", story.slug)).visible());
                })
                .unwrap();
            }
        }
    }
}

#[gpui_kit::test]
fn the_strip_changes_theme_text_size_and_width(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, 0);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("workbench-theme-dark", cx);
        assert!(cx.theme().mode.is_dark());
        window.click("workbench-theme-light", cx);
        assert!(!cx.theme().mode.is_dark());

        window.click("workbench-text-20", cx);
        window.render_frame(cx);
        assert_eq!(text_size::current(cx), 20.);
        assert_eq!(window.rem_size(), px(20.));
        window.click("workbench-text-13", cx);
        assert_eq!(text_size::current(cx), 13.);

        window.click("workbench-width-760", cx);
        assert_eq!(window.bounds().size.width, px(760.));
        window.click("workbench-width-1280", cx);
        assert_eq!(window.bounds().size.width, px(1280.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_list_switches_stories(cx: &mut TestAppContext) {
    let (handle, workbench) = open(cx, 0);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("story-data-table").selected(), Some(true));
        window.click("story-graph", cx);
        window.render_frame(cx);
        assert_eq!(STORIES[workbench.read(cx).story()].slug, "graph");
        assert_eq!(window.find("story-graph").selected(), Some(true));
        assert!(window.find("graph-page").visible());
        assert!(window.try_find("data-table-page").is_none());
    })
    .unwrap();
}

/// The story list takes the keyboard as the app's column does.
#[gpui_kit::test]
fn the_story_list_moves_with_the_keyboard(cx: &mut TestAppContext) {
    let (handle, workbench) = open(cx, 0);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let focus = workbench.read(cx).list.focus_handle().clone();
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert_eq!(workbench.read(cx).story(), 1);
        window.press("end", cx);
        window.press("enter", cx);
        assert_eq!(workbench.read(cx).story(), STORIES.len() - 1);
    })
    .unwrap();
}
