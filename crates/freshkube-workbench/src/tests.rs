use super::stories::data_table::{DataTableStory, Sort, State};
use super::{STORIES, Workbench, window_size};
use freshkube_ui::table::SortOrder;
use freshkube_ui::text_size;
use gpui_kit::component::{ActiveTheme, Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, px, size};

/// The workbench on `story` in a 1280 × 880 window, light, at text 13.
fn open(cx: &mut TestAppContext, story: usize) -> (AnyWindowHandle, Entity<Workbench>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        freshkube_ui::theme::install(cx);
        text_size::install(None, cx);
        Theme::change(ThemeMode::Light, None, cx);
        // Dialogs animate on the real clock, which the test clock can't
        // advance; reduced motion opens and closes them at once.
        cx.set_reduce_motion(true);
    });
    let mut view = None;
    let window = cx.open_window(size(px(1280.), px(880.)), |window, cx| {
        let workbench = cx.new(|cx| Workbench::new(story, window, cx));
        view = Some(workbench.clone());
        Root::new(workbench, window, cx)
    });
    cx.run_until_parked();
    (window.into(), view.unwrap())
}

fn data_table(workbench: &Entity<Workbench>, cx: &gpui_kit::App) -> Entity<DataTableStory> {
    workbench
        .read(cx)
        .view()
        .clone()
        .downcast::<DataTableStory>()
        .unwrap()
}

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
fn the_data_table_story_shows_each_state_and_returns_to_its_rows(cx: &mut TestAppContext) {
    let (handle, workbench) = open(cx, 0);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let story = data_table(&workbench, cx);
        assert!(window.find(("data-table-group", 0usize)).visible());
        for (id, state) in [
            ("loading", State::Loading),
            ("empty", State::Empty),
            ("failed", State::Failed),
        ] {
            window.click(format!("data-table-state-{id}"), cx);
            window.render_frame(cx);
            assert_eq!(story.read(cx).state(), state);
            assert!(window.find(format!("data-table-{id}")).visible());
            assert!(window.try_find(("data-table-group", 0usize)).is_none());
        }
        window.click("data-table-retry", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).state(), State::Rows);
        assert!(window.find(("data-table-group", 0usize)).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_data_table_story_sorts_and_holds_two_thousand_rows(cx: &mut TestAppContext) {
    let (handle, workbench) = open(cx, 0);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let story = data_table(&workbench, cx);
        assert_eq!(story.read(cx).rows(), 24);
        assert_eq!(
            story.read(cx).sorted_by(),
            (Sort::Name, SortOrder::Ascending)
        );
        let ascending = story.read(cx).names();
        // Column 1 is Name; a second press reverses it.
        window.click(("pods-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(
            story.read(cx).sorted_by(),
            (Sort::Name, SortOrder::Descending)
        );
        let descending = story.read(cx).names();
        assert_ne!(ascending, descending);
        assert_eq!(
            sorted(ascending),
            sorted(descending),
            "the same pods, in another order"
        );
        window.click("data-table-count-2000", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).rows(), 2_000);
        assert_eq!(story.read(cx).names().len(), 2_000);
        assert!(window.find(("data-table-group", 0usize)).visible());
    })
    .unwrap();
}

fn sorted(mut names: Vec<gpui_kit::SharedString>) -> Vec<gpui_kit::SharedString> {
    names.sort();
    names
}

#[test]
fn window_sizes_read_as_the_app_reads_them() {
    assert_eq!(window_size("1280x880"), Some(size(px(1280.), px(880.))));
    assert_eq!(window_size("700x880"), None);
    assert_eq!(window_size("wide"), None);
}
