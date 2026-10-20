use super::*;
use crate::panel::cursor;

#[gpui_kit::test]
fn the_cursor_reads_out_values_and_tells_the_page_without_rebuilding_paths(
    cx: &mut TestAppContext,
) {
    let (dashboard, specs) = cluster();
    let cpu = index_of(&specs, "CPU usage by node");
    let memory = index_of(&specs, "Memory usage by node");
    let (handle, panels) = mount(cx, specs);
    answer(cx, &dashboard, &panels);
    cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    let _subscription = cx.update(|cx| {
        let events = events.clone();
        cx.subscribe(&panels[cpu], move |_, event: &PanelEvent, _| {
            events.borrow_mut().push(event.clone())
        })
    });
    let plot: gpui_kit::SharedString = format!("monitoring-panel-{cpu}-plot").into();
    let title: gpui_kit::SharedString = format!("monitoring-panel-{cpu}-title").into();
    // Real moves and real frames: `hover` draws with the view caches off.
    let move_to = |cx: &mut TestAppContext, id: gpui_kit::SharedString, dx: f32| {
        cx.update_window(handle, |_, window, cx| {
            let bounds = window.find(id).bounds();
            window.dispatch_event(
                gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                    position: bounds.center() + gpui_kit::point(px(dx), px(0.)),
                    pressed_button: None,
                    modifiers: Default::default(),
                }),
                cx,
            );
            window.draw(cx).clear(cx);
        })
        .unwrap();
    };
    move_to(cx, title.clone(), 0.);
    let counts = || {
        [
            "monitoring-panel",
            "monitoring-plot",
            "monitoring-plot-paint",
            "monitoring-legend",
            "monitoring-path",
        ]
        .map(probe::count)
    };
    let before = counts();
    let readout: gpui_kit::SharedString = format!("monitoring-panel-{cpu}-readout").into();
    let readout_right = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, _| {
            window.find(readout.clone()).bounds().right()
        })
        .unwrap()
    };
    move_to(cx, plot.clone(), 60.);
    let near = readout_right(cx);
    move_to(cx, plot.clone(), 100.);
    let far = readout_right(cx);
    move_to(cx, plot.clone(), 40.);
    // Only the cursor's own view draws: the panel, its plot and its legend
    // stay cached, and no shape is built.
    assert_eq!(counts(), before, "a pointer move drew the panel again");
    assert!(far > near, "the readout stayed at {near:?}");
    let cursor = cx.read(|cx| panels[cpu].read(cx).cursor.clone()).unwrap();
    assert!(!cursor.rows.is_empty());
    assert!(!cursor.time.is_empty());
    assert!(shown(cx, handle, &readout));
    let time = match events.borrow().last() {
        Some(PanelEvent::Cursor(Some(time))) => *time,
        other => panic!("{other:?}"),
    };

    // The page passes it on: the other chart's crosshair falls on the same
    // sample, and that chart shows no readout.
    let shared = cx
        .read(|cx| panels[memory].read(cx).crosshair(time))
        .unwrap();
    let chart = cx.read(|cx| panels[memory].read(cx).chart()).unwrap();
    assert_eq!(shared.x, chart.xs[cursor.index]);
    assert!(!shown(
        cx,
        handle,
        &format!("monitoring-panel-{memory}-readout")
    ));

    // Leaving the plot clears it and says so.
    cx.update_window(handle, |_, window, cx| window.hover(title.clone(), cx))
        .unwrap();
    assert!(cx.read(|cx| panels[cpu].read(cx).cursor.is_none()));
    assert!(!shown(cx, handle, &readout));
    assert!(matches!(
        events.borrow().last(),
        Some(PanelEvent::Cursor(None))
    ));
}

#[test]
fn the_nearest_sample_wins() {
    let xs = [0., 0.25, 0.5, 1.];
    assert_eq!(cursor::nearest_x(&xs, 0.3), Some(1));
    assert_eq!(cursor::nearest_x(&xs, 0.8), Some(3));
    assert_eq!(cursor::nearest_x(&[], 0.5), None);
    let times = [10., 20., 30.];
    assert_eq!(cursor::nearest_time(&times, 24.), Some(1));
    assert_eq!(cursor::nearest_time(&times, 26.), Some(2));
    assert_eq!(cursor::nearest_time(&times, 31.), None);
}

/// A series picked in the legend while the cursor shows is marked in the
/// readout at once, and the panel never draws for the pointer.
#[gpui_kit::test]
fn a_pick_under_the_cursor_shows_in_the_readout(cx: &mut TestAppContext) {
    let (handle, panels, result) = crowded(cx);
    let again = result.clone();
    cx.update(|cx| {
        panels[0].update(cx, |panel, cx| {
            panel.set_result(result, window_range(), cx);
        })
    });
    frame(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        let bounds = window.find("monitoring-panel-0-plot").bounds();
        window.dispatch_event(
            gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                position: bounds.center(),
                pressed_button: None,
                modifiers: Default::default(),
            }),
            cx,
        );
        window.draw(cx).clear(cx);
    })
    .unwrap();
    let overlay = cx.read(|cx| panels[0].read(cx).cursor_overlay()).unwrap();
    let (rows, focused) = cx.read(|cx| overlay.read(cx).named()).unwrap();
    assert_eq!(focused, None);
    let pick = rows[1];
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.toggle_picked(pick, cx)));
    assert_eq!(
        cx.read(|cx| overlay.read(cx).named()),
        Some((rows.clone(), Some(pick)))
    );
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.toggle_picked(pick, cx)));
    assert_eq!(cx.read(|cx| overlay.read(cx).named()), Some((rows, None)));

    // A new answer takes the cursor away, as before.
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.set_result(again, window_range(), cx)));
    assert_eq!(cx.read(|cx| overlay.read(cx).named()), None);
}

/// The readout takes the theme's inks as it draws: switching the theme while
/// the cursor shows recolours its swatches before the pointer moves (#350).
#[gpui_kit::test]
fn a_theme_change_recolours_the_readout_under_a_still_pointer(cx: &mut TestAppContext) {
    // Its first two series take the slots, whose inks differ by theme.
    let (dashboard, specs) = cluster();
    let cpu = index_of(&specs, "CPU usage by node");
    let (handle, panels) = mount(cx, specs);
    answer(cx, &dashboard, &panels);
    frame(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        let bounds = window.find(format!("monitoring-panel-{cpu}-plot")).bounds();
        window.dispatch_event(
            gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                position: bounds.center(),
                pressed_button: None,
                modifiers: Default::default(),
            }),
            cx,
        );
        window.draw(cx).clear(cx);
    })
    .unwrap();
    let overlay = cx.read(|cx| panels[cpu].read(cx).cursor_overlay()).unwrap();
    let (rows, _) = cx.read(|cx| overlay.read(cx).named()).unwrap();
    let chart = cx.read(|cx| panels[cpu].read(cx).chart()).unwrap();
    let expected = |p: &crate::palette::Palette| -> Vec<_> {
        (rows.iter())
            .map(|series| chart.series[*series].ink.color(p, false))
            .collect()
    };
    let dark = cx.read(|cx| overlay.read(cx).swatches(cx)).unwrap();
    assert_eq!(dark, expected(&crate::palette::dark()));

    cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
    let light = cx.read(|cx| overlay.read(cx).swatches(cx)).unwrap();
    assert_eq!(light, expected(&crate::palette::light()));
    assert_ne!(light, dark);
}
