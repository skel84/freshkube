use super::*;
use crate::panel::cursor;

/// Past what fits, the readout names the highest values and the picked
/// series, and stays inside the plot.
#[gpui_kit::test]
fn a_crowded_readout_ranks_the_highest_values_and_fits_the_plot(cx: &mut TestAppContext) {
    let (handle, panels, result) = crowded(cx);
    cx.update(|cx| {
        panels[0].update(cx, |panel, cx| {
            panel.set_result(result, window_range(), cx);
            panel.toggle_picked(2, cx);
        })
    });
    frame(cx, handle);
    let (plot, readout): (gpui_kit::SharedString, gpui_kit::SharedString) = (
        "monitoring-panel-0-plot".into(),
        "monitoring-panel-0-readout".into(),
    );
    cx.update_window(handle, |_, window, cx| {
        let bounds = window.find(plot.clone()).bounds();
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
    let cursor = cx.read(|cx| panels[0].read(cx).cursor.clone()).unwrap();
    let series: Vec<usize> = cursor.rows.iter().map(|row| row.series).collect();
    // It ranks and counts only the thirty series drawn.
    let fit = series.len();
    assert!((3..=cursor::READOUT_ROWS).contains(&fit), "{series:?}");
    assert_eq!(cursor.more, 30 - fit);
    let mut expected: Vec<usize> = (0..fit - 1).map(|n| 29 - n).collect();
    expected.push(2);
    assert_eq!(series, expected);
    cx.update_window(handle, |_, window, _| {
        let (plot, readout) = (window.find(plot).bounds(), window.find(readout).bounds());
        assert!(readout.bottom() <= plot.bottom(), "{readout:?} in {plot:?}");
    })
    .unwrap();
}

/// In a narrow chart, a readout flipped left of the crosshair stays off the
/// value axis, keeps its time and values whole, and its "top N of M" never
/// spills out of its box.
#[gpui_kit::test]
fn a_narrow_readout_keeps_off_the_axis_and_inside_its_box(cx: &mut TestAppContext) {
    let (readout, plot_left) = narrow_readout(cx, 260.);
    // Layout rounds to the test window's half-pixel grid.
    assert!(
        readout.left() >= plot_left - px(0.5),
        "{readout:?} left of {plot_left:?}"
    );
}

/// Where the plot has no room for the time and values, the readout crosses
/// onto the axis rather than cut them off.
#[gpui_kit::test]
fn a_readout_too_wide_for_the_plot_keeps_its_time_and_values_whole(cx: &mut TestAppContext) {
    let (readout, plot_left) = narrow_readout(cx, 160.);
    assert!(readout.left() < plot_left, "{readout:?} fits after all");
}

/// Hovers three quarters across a crowded chart `width` wide and checks what
/// holds at any width: the readout is at least its time and values wide,
/// draws them whole and keeps its count inside. Returns the readout's bounds
/// and the plot's left edge.
fn narrow_readout(
    cx: &mut TestAppContext,
    width: f32,
) -> (gpui_kit::Bounds<gpui_kit::Pixels>, gpui_kit::Pixels) {
    let (handle, panels, result) = crowded_at(cx, width);
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.set_result(result, window_range(), cx)));
    frame(cx, handle);
    let geometry = cx.read(|cx| panels[0].read(cx).geometry.get());
    let plot_left = geometry.origin.x + geometry.left;
    cx.update_window(handle, |_, window, cx| {
        let position = point(
            plot_left + geometry.width * 0.75,
            geometry.origin.y + px(60.),
        );
        window.dispatch_event(
            gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Default::default(),
            }),
            cx,
        );
        window.draw(cx).clear(cx);
    })
    .unwrap();
    let cursor = cx.read(|cx| panels[0].read(cx).cursor.clone()).unwrap();
    assert!(cursor.flip && cursor.more > 0, "{cursor:?}");
    cx.update_window(handle, |_, window, _| {
        let readout = window.find("monitoring-panel-0-readout").bounds();
        assert!(readout.size.width >= cursor.least - px(0.5), "{readout:?}");
        // The time and every value are drawn whole, inside the box.
        let mut whole = vec!["monitoring-panel-0-readout-time".to_owned()];
        whole.extend(
            (0..cursor.rows.len()).map(|n| format!("monitoring-panel-0-readout-value-{n}")),
        );
        for id in whole {
            let text = window
                .find(gpui_kit::SharedString::from(id.clone()))
                .bounds();
            assert!(
                text.left() >= readout.left() && text.right() <= readout.right() + px(0.5),
                "{id}: {text:?} out of {readout:?}"
            );
        }
        // The count gives way to the time: it fits, or shrinks to nothing.
        let more = window.find("monitoring-panel-0-readout-more").bounds();
        assert!(
            more.size.width == px(0.) || more.right() <= readout.right(),
            "{more:?} out of {readout:?}"
        );
        (readout, plot_left)
    })
    .unwrap()
}

/// In a narrow chart, long names truncate so the readout stays inside the
/// plot on either side of the crosshair: just right of the middle it sits
/// to the left, just left of it to the right.
#[gpui_kit::test]
fn a_narrow_readout_stays_inside_the_plot_on_either_side(cx: &mut TestAppContext) {
    let (handle, panel) = narrow_chart(cx, 360.);
    let (plot, readout): (gpui_kit::SharedString, gpui_kit::SharedString) = (
        "monitoring-panel-0-plot".into(),
        "monitoring-panel-0-readout".into(),
    );
    for fraction in [0.62, 0.38, 0.9, 0.1] {
        let (plot, readout) = (plot.clone(), readout.clone());
        cx.update_window(handle, |_, window, cx| {
            let bounds = window.find(plot.clone()).bounds();
            window.dispatch_event(
                gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                    position: gpui_kit::point(
                        bounds.left() + bounds.size.width * fraction,
                        bounds.center().y,
                    ),
                    pressed_button: None,
                    modifiers: Default::default(),
                }),
                cx,
            );
            window.draw(cx).clear(cx);
            let (plot, readout) = (window.find(plot).bounds(), window.find(readout).bounds());
            assert!(
                readout.left() >= plot.left() && readout.right() <= plot.right(),
                "at {fraction}: {readout:?} in {plot:?}"
            );
        })
        .unwrap();
        assert!(cx.read(|cx| panel.read(cx).cursor.is_some()));
    }
}
