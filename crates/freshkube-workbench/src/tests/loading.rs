use super::*;

#[gpui_kit::test]
fn the_loading_motion_leaves_the_table_alone(cx: &mut TestAppContext) {
    let (handle, workbench) = open(cx, super::stories::find("loading").unwrap());
    let story = cx.read(|cx| story::<LoadingStory>(&workbench, cx));
    let motion = cx.read(|cx| story.read(cx).motion().clone());
    for (look, id) in [(Look::Pulse, "pulse"), (Look::Shimmer, "shimmer")] {
        cx.update_window(handle, |_, window, cx| {
            motion::choose(Choice::Reduced, cx);
            window.click(format!("loading-look-{id}"), cx);
            assert_eq!(motion.read(cx).look(), look);
            assert!(window.find("loading-pods-loading").visible());
            // The table's real header shows above the rows.
            assert!(window.find(("loading-pods-sort", 1usize)).visible());
        })
        .unwrap();
        // The frame asked before reduced motion ran out; none follows it.
        next_frame(handle, cx);
        assert_eq!(next_frame(handle, cx), 0, "{id} stands still");
        cx.update(|cx| motion::choose(Choice::Full, cx));
        cx.run_until_parked();
        next_frame(handle, cx);
        let table = probe::count("workbench.loading-table");
        let moved = probe::count("table.loading-motion");
        for _ in 0..4 {
            assert!(next_frame(handle, cx) > 0, "{id} moves");
        }
        assert_eq!(probe::count("table.loading-motion"), moved + 4);
        assert_eq!(
            probe::count("workbench.loading-table"),
            table,
            "{id}'s frames leave the table alone"
        );
    }
}

#[gpui_kit::test]
fn the_story_answers_with_the_loading_rows_gone(cx: &mut TestAppContext) {
    let (handle, workbench) = open(cx, super::stories::find("loading").unwrap());
    let story = cx.read(|cx| story::<LoadingStory>(&workbench, cx));
    let table = cx.read(|cx| story.read(cx).table().clone());
    cx.update(|cx| table.update(cx, |table, cx| table.answer(cx)));
    cx.run_until_parked();
    // Under reduced motion nothing asks frames, so the story's own Kit
    // skeleton doesn't hide what the motion does; freshkube-ui's table
    // tests count the motion's frames on their own.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("loading-pods-loading").is_none());
        // The wake the table asked when the rows first showed, then none.
        window.simulate_next_frame(cx);
        assert_eq!(window.simulate_next_frame(cx), 0);
    })
    .unwrap();
}
