use super::*;

/// Scrolled sideways, a group's label and the pinned columns stay at
/// the table's left edge while the other cells pass under them.
#[gpui_kit::test]
fn group_labels_and_pinned_columns_stay_in_view_when_scrolled(cx: &mut TestAppContext) {
    // The two pinned columns' 400 take less than two thirds of the 900.
    let (handle, _) = open(cx, Wide::new(2, 4, true), 900.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(inset(window, "Column 0 1").abs() <= 1.5);
        assert!(window.try_find("wide-pinned-header").is_none());
        scroll_right(window, 100., cx);
        assert!(
            (inset(window, "Column 4 1") - 700.).abs() <= 1.5,
            "no scroll"
        );
        assert!(inset(window, "wide-group").abs() <= 1.5);
        // Inside the row's one-pixel border, as before the scroll.
        assert!(pinned_inset(window, Some(1), "Column 0 1").abs() <= 1.5);
        assert!((pinned_inset(window, Some(1), "Column 1 1") - 200.).abs() <= 1.5);
        assert!(pinned_inset(window, None, ("wide-sort", 0usize)).abs() <= 1.5);
        assert!((pinned_inset(window, None, ("wide-sort", 1usize)) - 200.).abs() <= 1.5);
        // Each cell is drawn once, so its id finds one element.
        assert!(inset(window, "Column 0 1").abs() <= 1.5);
        assert!(inset(window, ("wide-sort", 0usize)).abs() <= 1.5);
        // Back at the left edge, nothing pins.
        scroll_right(window, -100., cx);
        assert!(window.try_find(("wide-pinned", 1usize)).is_none());
        assert!(window.try_find("wide-pinned-header").is_none());
        assert!(inset(window, "Column 0 1").abs() <= 1.5);
    })
    .unwrap();
}

/// Pinned columns wider than two thirds of the table's visible width
/// scroll with the rest, built as if unpinned, so they never cover it;
/// the group's label still stays.
#[gpui_kit::test]
fn pinned_columns_too_wide_for_the_table_scroll(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, Wide::new(2, 4, true), 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        scroll_right(window, 500., cx);
        assert!(inset(window, "wide-group").abs() <= 1.5);
        assert!(window.try_find(("wide-pinned", 1usize)).is_none());
        assert!(window.try_find("wide-pinned-header").is_none());
        assert!((inset(window, "Column 0 1") + 499.).abs() <= 1.5);
        assert!((inset(window, ("wide-sort", 0usize)) + 500.).abs() <= 1.5);
    })
    .unwrap();
}

/// Over the pinned run, only the row and the pinned cells take the
/// mouse: the cells passing under it are clipped there, so they neither
/// hover nor take a click, in the rows and in the header. Right of the
/// run they take both as before.
#[gpui_kit::test]
fn cells_passing_under_the_pinned_run_dont_take_the_mouse(cx: &mut TestAppContext) {
    let (handle, wide) = open(cx, Wide::new(2, 4, false).interactive(), 900.);
    // Scrolled 100, Column 2 runs from 300 to 500, its first 100 under
    // the pinned Column 1, which runs from 200 to 400.
    let under_the_run = point(px(150.), px(5.));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        scroll_right(window, 100., cx);
        assert!(window.find("wide-pinned-header").visible());
        window
            .within(("wide-pinned", 1usize))
            .click_at("Column 1 1", under_the_run, cx);
        window
            .within("wide-pinned-header")
            .click_at(("wide-sort", 1usize), under_the_run, cx);
    })
    .unwrap();
    cx.read(|cx| {
        let wide = wide.read(cx);
        assert_eq!(wide.selected, Some(1));
        assert_eq!(wide.cell_clicks, ["Column 1 1"]);
        assert!(!wide.hovered.iter().any(|label| label == "Column 2 1"));
        assert_eq!(wide.sorts, ["Column 1"]);
    });
    cx.update_window(handle, |_, window, cx| {
        window.click_at("Column 2 1", point(px(150.), px(5.)), cx);
        // The header's cell has moved beside the run, so its visible
        // part starts at its own left.
        window.click_at(("wide-sort", 2usize), point(px(50.), px(5.)), cx);
    })
    .unwrap();
    cx.read(|cx| {
        let wide = wide.read(cx);
        assert_eq!(wide.cell_clicks, ["Column 1 1", "Column 2 1"]);
        assert!(wide.hovered.iter().any(|label| label == "Column 2 1"));
        assert_eq!(wide.sorts, ["Column 1", "Column 2"]);
    });
}

/// A column passing under the pinned run keeps its label beside the run
/// while any of it shows, clipped to its own place, though its rows'
/// cells pass under as before. A short label would otherwise vanish
/// while a long cell's tail still shows, leaving the column unnamed.
#[gpui_kit::test]
fn a_passing_column_keeps_its_label_beside_the_run(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, Wide::new(2, 4, false).interactive(), 900.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Column 2 runs from 300 to 500, its first 100 under the run.
        scroll_right(window, 100., cx);
        let label = inset(window, ("wide-sort", 2usize));
        assert!(
            (label - 400.).abs() <= 1.5,
            "label at {label}, under the run"
        );
        assert!((inset(window, "Column 2 1") - 300.).abs() <= 1.5);
        // A column right of the run stays where it is.
        assert!((inset(window, ("wide-sort", 3usize)) - 500.).abs() <= 1.5);
        // Back at the left edge, the label returns to its column.
        scroll_right(window, -100., cx);
        assert!((inset(window, ("wide-sort", 2usize)) - 400.).abs() <= 1.5);
        scroll_right(window, 50., cx);
        let label = inset(window, ("wide-sort", 2usize));
        assert!((label - 400.).abs() <= 1.5, "label at {label}");
    })
    .unwrap();
}

/// The first sideways scroll from the left edge pins on the frame it
/// draws, inside a cached view too: the wheel notifies the table's view,
/// which marks the cached views around it dirty. The frames here are
/// the ones the app draws, through the caches, never `render_frame`.
#[gpui_kit::test]
fn the_first_scroll_pins_through_a_cached_view(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        cx.set_reduce_motion(true);
    });
    let mut wide = None;
    let handle: AnyWindowHandle = cx
        .open_window(size(px(900.), px(400.)), |window, cx| {
            let table = cx.new(|_| Wide::new(2, 4, true).interactive());
            wide = Some(table.clone());
            let page = cx.new(|_| CachedTable(table));
            Root::new(page, window, cx)
        })
        .into();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(window.try_find(("wide-pinned", 1usize)).is_none());
        let position = window.find("wide-table-scroll").bounds().center();
        window.dispatch_event(
            MouseMoveEvent {
                position,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(-100.), px(0.))),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    // The update's end drew the window the wheel left dirty.
    cx.update_window(handle, |_, window, cx| {
        assert!(
            (inset(window, "Column 4 1") - 700.).abs() <= 1.5,
            "no scroll"
        );
        assert!(inset(window, "wide-group").abs() <= 1.5);
        assert!(pinned_inset(window, Some(1), "Column 0 1").abs() <= 1.5);
        assert!(pinned_inset(window, None, ("wide-sort", 0usize)).abs() <= 1.5);
        // Nothing waits for a next frame, and the next one changes nothing.
        assert_eq!(window.simulate_next_frame(cx), 0);
    })
    .unwrap();
    cx.run_until_parked();
    // The cells passing under the run were clipped on that same frame:
    // a press over Column 1, where Column 2 passes under it, reaches
    // only the pinned cell.
    cx.update_window(handle, |_, window, cx| {
        assert!(pinned_inset(window, Some(1), "Column 0 1").abs() <= 1.5);
        let viewport = window.find("wide-table-scroll").bounds();
        let row = window.find("wide-row-1").bounds();
        press(
            window,
            point(viewport.left() + px(350.), row.center().y),
            cx,
        );
    })
    .unwrap();
    let wide = wide.unwrap();
    cx.read(|cx| assert_eq!(wide.read(cx).cell_clicks, ["Column 1 1"]));
}

/// A table without pinned columns still keeps its group labels in view,
/// and draws its rows as it always has.
#[gpui_kit::test]
fn without_pinned_columns_only_group_labels_stay(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, Wide::new(0, 4, true), 600.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        scroll_right(window, 500., cx);
        assert!(inset(window, "wide-group").abs() <= 1.5);
        assert!((inset(window, "Column 0 1") + 500.).abs() <= 1.5);
        assert!(window.try_find(("wide-pinned", 1usize)).is_none());
        assert!(window.try_find("wide-pinned-header").is_none());
    })
    .unwrap();
}

/// A click on a pinned cell selects its row, and stepping and revealing
/// the selection keep the sideways scroll and the pinned cells.
#[gpui_kit::test]
fn pinned_cells_click_and_step_like_their_rows(cx: &mut TestAppContext) {
    let (handle, wide) = open(cx, Wide::new(2, 30, false), 900.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        scroll_right(window, 100., cx);
        // The wheel's sideways delta also moves the list down a little,
        // as GPUI scrolls a list without a sideways scroll of its own.
        window
            .within(("wide-pinned", 8usize))
            .click("Column 0 8", cx);
        assert_eq!(wide.read(cx).selected, Some(8));
        for _ in 0..14 {
            let next = step(wide.read(cx), 1, cx);
            wide.update(cx, |wide, cx| {
                wide.selected = next;
                reveal(wide, ScrollStrategy::Top);
                cx.notify();
            });
        }
        window.render_frame(cx);
        assert_eq!(wide.read(cx).selected, Some(22));
        assert!(window.find("wide-row-22").visible());
        assert!(pinned_inset(window, Some(22), "Column 0 22").abs() <= 1.5);
        assert!((inset(window, "Column 4 22") - 700.).abs() <= 1.5);
    })
    .unwrap();
}

/// A window grown wide enough for the whole table clamps the scroll to
/// its left edge without a scroll event; the pinned cells stay in their
/// place, and the table draws again unpinned without any input.
#[gpui_kit::test]
fn a_resize_that_ends_the_scroll_unpins_at_once(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, Wide::new(2, 4, true), 900.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        scroll_right(window, 100., cx);
        assert!(window.find("wide-pinned-header").visible());
    })
    .unwrap();
    cx.simulate_window_resize(handle, size(px(1400.), px(400.)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(inset(window, ("wide-sort", 0usize)).abs() <= 1.5);
        let mut frames = 0;
        while window.simulate_next_frame(cx) > 0 {
            frames += 1;
            assert!(frames < 4, "the table didn't settle");
            window.render_frame(cx);
        }
        assert_eq!(frames, 1);
        assert!(window.try_find("wide-pinned-header").is_none());
        assert!(window.try_find(("wide-pinned", 1usize)).is_none());
        assert!((inset(window, "Column 4 1") - 800.).abs() <= 1.5);
        assert!(inset(window, "wide-group").abs() <= 1.5);
    })
    .unwrap();
}
