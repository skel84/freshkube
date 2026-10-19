use super::*;

/// Hides the Error level from the Levels menu: 118 of the example's 120
/// lines show.
fn hide_errors(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("logs-levels", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// Opens the Download menu, checks its items' words and clicks `item`.
fn download(cx: &mut TestAppContext, handle: WindowHandle<Root>, words: [&str; 2], item: usize) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("logs-download", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let mut menu = window.within("popup-menu");
        for (ix, words) in words.into_iter().enumerate() {
            assert_eq!(menu.find(ix).label(), Some(words));
        }
        menu.click(item, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn download_saves_the_visible_lines_as_copy_copies_them(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("visible.log");
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    hide_errors(cx, handle);
    let copied = cx.update(|cx| {
        panel.update(cx, |view, cx| {
            view.select_all(cx);
            let copied = view.review.copy_text(view.copy_as()).unwrap();
            view.review.selected.clear();
            copied
        })
    });
    download(
        cx,
        handle,
        ["Visible lines (118)", "All retained lines (120)"],
        0,
    );
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|folder| {
        // The Downloads folder, else home.
        assert_eq!(folder, super::super::download::default_folder());
        Some(path.clone())
    });
    cx.run_until_parked();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert_eq!(saved, format!("{copied}\n"));
    assert!(!saved.contains("error needle"));
    let feedback = cx.update(|cx| panel.read(cx).feedback.clone()).unwrap();
    assert!(!feedback.failed);
    // One line names the file; its tooltip, the folder too.
    assert_eq!(feedback.text.as_ref(), "Saved 118 lines to visible.log");
    assert_eq!(
        feedback.whole.as_ref(),
        format!("Saved 118 lines to {}", path.display())
    );
    // Only the file is left: no temporary beside it.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[gpui_kit::test]
fn download_saves_every_retained_line_and_cancel_saves_nothing(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("all.log");
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    hide_errors(cx, handle);
    download(
        cx,
        handle,
        ["Visible lines (118)", "All retained lines (120)"],
        1,
    );
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    cx.run_until_parked();
    let saved = std::fs::read_to_string(&path).unwrap();
    // The lines the filters hide are saved too.
    assert_eq!(saved.lines().count(), 120);
    assert!(saved.starts_with("info ordinary complete line 0\n"));
    assert!(saved.contains("error needle first 東京\n"));

    cx.update(|cx| panel.update(cx, |view, _| view.feedback = None));
    download(
        cx,
        handle,
        ["Visible lines (118)", "All retained lines (120)"],
        0,
    );
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| panel.read(cx).feedback.clone()), None);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[gpui_kit::test]
fn a_download_that_cant_be_written_says_so_and_leaves_no_file(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing").join("lines.log");
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    download(
        cx,
        handle,
        ["Visible lines (120)", "All retained lines (120)"],
        0,
    );
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    cx.run_until_parked();
    let feedback = cx.update(|cx| panel.read(cx).feedback.clone()).unwrap();
    assert!(feedback.failed);
    assert!(
        feedback.text.starts_with("Couldn't save lines.log: "),
        "{}",
        feedback.text
    );
    assert!(
        feedback
            .whole
            .starts_with(&format!("Couldn't save {}: ", path.display())),
        "{}",
        feedback.whole
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("logs-summary-text").visible());
    })
    .unwrap();
}

/// While a save dialog is open, another Download does nothing: the first
/// save goes through with the lines it was asked for, and once it has
/// answered Download asks again.
#[gpui_kit::test]
fn a_second_download_waits_for_the_open_save_dialog(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("first.log");
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    download(
        cx,
        handle,
        ["Visible lines (120)", "All retained lines (120)"],
        0,
    );
    assert!(cx.did_prompt_for_new_path());
    deliver_more(cx, &panel, 0, 5);
    download(
        cx,
        handle,
        ["Visible lines (125)", "All retained lines (125)"],
        1,
    );
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    cx.run_until_parked();
    assert!(!cx.did_prompt_for_new_path());
    let saved = std::fs::read_to_string(&path).unwrap();
    assert_eq!(saved.lines().count(), 120);
    assert!(!saved.contains("later line"));

    download(
        cx,
        handle,
        ["Visible lines (125)", "All retained lines (125)"],
        1,
    );
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
}

/// The menu counts its lines each time it opens, however it last closed.
#[gpui_kit::test]
fn the_download_menu_counts_again_each_time_it_opens(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    let labels = |cx: &mut TestAppContext| {
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("logs-download", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let menu = window.within("popup-menu");
            [0, 1].map(|ix| menu.find(ix).label().unwrap_or_default().to_owned())
        })
        .unwrap()
    };
    assert_eq!(
        labels(cx),
        ["Visible lines (120)", "All retained lines (120)"]
    );
    // Closed by its button.
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("logs-download", cx);
    })
    .unwrap();
    cx.run_until_parked();
    deliver_more(cx, &panel, 0, 5);
    assert_eq!(
        labels(cx),
        ["Visible lines (125)", "All retained lines (125)"]
    );
    // Closed by Escape.
    cx.update_window(handle.into(), |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    deliver_more(cx, &panel, 5, 3);
    assert_eq!(
        labels(cx),
        ["Visible lines (128)", "All retained lines (128)"]
    );
}

/// Under 40 rem the toolbar shows its buttons' icons alone; the menus'
/// carets still fit beside them.
#[gpui_kit::test]
fn icon_only_menu_buttons_keep_room_for_their_caret(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount_sized(cx, 480., 760.);
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("logs-levels").label().is_some());
        for id in ["logs-levels", "logs-download"] {
            let bounds = window.find(id).bounds();
            assert!(
                bounds.size.width > bounds.size.height * 1.4,
                "{id}: {:?}",
                bounds.size
            );
        }
    })
    .unwrap();
}
