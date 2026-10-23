use super::*;

fn data_table(workbench: &Entity<Workbench>, cx: &gpui_kit::App) -> Entity<DataTableStory> {
    workbench
        .read(cx)
        .view()
        .clone()
        .downcast::<DataTableStory>()
        .unwrap()
}

fn sorted(mut names: Vec<gpui_kit::SharedString>) -> Vec<gpui_kit::SharedString> {
    names.sort();
    names
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
