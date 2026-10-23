use super::*;

#[gpui_kit::test]
fn a_remembered_width_is_kept_above_the_least(cx: &mut TestAppContext) {
    let start = |width: f32, cx: &mut TestAppContext| {
        cx.update(|cx| {
            set_store(Rc::new(MemorySizes::with(width_key(PAGE), width)), cx);
            InspectorSplit::new(PAGE, cx).width()
        })
    };
    assert_eq!(start(600., cx), 600.);
    assert_eq!(start(100., cx), MIN_WIDTH);
    assert_eq!(start(f32::NAN, cx), WIDTH);
    assert_eq!(start(f32::INFINITY, cx), WIDTH);
    cx.update(|cx| {
        set_store(Rc::new(MemorySizes::default()), cx);
        assert_eq!(InspectorSplit::new(PAGE, cx).width(), WIDTH);
    });
}

#[gpui_kit::test]
fn the_inspector_opens_at_the_remembered_width(cx: &mut TestAppContext) {
    let (handle, _, _) = open(cx, Some(600.), true, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let inspector = window.find("inspector").bounds();
        close("width", inspector.size.width, dp_px(600., window));
    })
    .unwrap();
}

/// `resize_panel` takes the path a drag's end takes; the width is saved
/// once, in dp, so it scales with the text size.
#[gpui_kit::test]
fn a_resize_reports_the_width_once_in_dp(cx: &mut TestAppContext) {
    let (handle, page, sizes) = open(cx, None, true, true);
    cx.update_window(handle, |_, _, cx| crate::text_size::set(20., cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let state = page.read(cx).split.beside_state().clone();
        state.update(cx, |state, cx| state.resize_panel(1, px(650.), window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(sizes.saves(), 1);
    let saved = sizes.size(width_key(PAGE)).unwrap();
    near("saved", saved, 650. * BASE_TEXT / 20.);
    cx.update(|cx| near("kept", page.read(cx).split.width(), saved));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        close(
            "drawn",
            window.find("inspector").bounds().size.width,
            px(650.),
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn dragging_the_hairline_widens_the_inspector(cx: &mut TestAppContext) {
    let (handle, page, sizes) = open(cx, None, true, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let inspector = window.find("inspector").bounds();
        let y = inspector.center().y;
        window.drag(
            point(inspector.left(), y),
            point(inspector.left() - px(100.), y),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(sizes.saves(), 1, "a drag writes once");
    near("saved", sizes.size(width_key(PAGE)).unwrap(), WIDTH + 100.);
    cx.update(|cx| near("kept", page.read(cx).split.width(), WIDTH + 100.));
}

/// A table that keeps its width leaves the inspector the rest, however
/// wide the user left it before, so it never pushes the table aside.
#[gpui_kit::test]
fn a_kept_table_caps_a_remembered_width(cx: &mut TestAppContext) {
    let (handle, page, sizes) = open(cx, Some(1000.), true, true);
    cx.update(|cx| page.update(cx, |page, _| page.keep = Some((800., 1200.))));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let table = window.find("table").bounds();
        let inspector = window.find("inspector").bounds();
        close("inspector", inspector.size.width, dp_px(400., window));
        close("table", table.size.width, dp_px(800., window));
        close("meeting", inspector.left(), table.right());
    })
    .unwrap();
    cx.run_until_parked();
    // Capped to the room, the width the user gave stays saved.
    cx.update(|cx| assert_eq!(page.read(cx).split.width(), 1000.));
    assert_eq!(sizes.saves(), 0);
    assert_eq!(sizes.size(width_key(PAGE)), Some(1000.));
}

/// A press on the hairline that moves no width, as a click that jitters
/// past the drag threshold does, saves nothing: Kit reports it as a resize,
/// and the width it shows is the clamp's, not the user's.
#[gpui_kit::test]
fn a_drag_that_moves_nothing_keeps_a_clamped_width(cx: &mut TestAppContext) {
    let (handle, page, sizes) = open(cx, Some(1000.), true, true);
    cx.update(|cx| page.update(cx, |page, _| page.keep = Some((800., 1200.))));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let inspector = window.find("inspector").bounds();
        let y = inspector.center().y;
        // Into the kept table, which can't give the inspector more room.
        window.drag(
            point(inspector.left(), y),
            point(inspector.left() - px(3.), y),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(sizes.saves(), 0, "nothing moved, nothing saved");
    assert_eq!(sizes.size(width_key(PAGE)), Some(1000.));
    cx.update(|cx| assert_eq!(page.read(cx).split.width(), 1000.));
}

/// A drag stops where the kept table starts, so the width saved fits the
/// room beside it.
#[gpui_kit::test]
fn a_drag_stops_at_the_kept_table(cx: &mut TestAppContext) {
    let (handle, page, sizes) = open(cx, None, true, true);
    cx.update(|cx| page.update(cx, |page, _| page.keep = Some((700., 1200.))));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let inspector = window.find("inspector").bounds();
        let y = inspector.center().y;
        window.drag(
            point(inspector.left(), y),
            point(inspector.left() - px(300.), y),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(sizes.saves(), 1);
    near("saved", sizes.size(width_key(PAGE)).unwrap(), 500.);
}

/// A table whose height follows its data gives it while rendering: each
/// new height lays the split out again, within the lead's least and most,
/// until the user drags it.
#[gpui_kit::test]
fn a_given_start_applies_until_the_user_drags(cx: &mut TestAppContext) {
    let heights = Stacked {
        lead: Pane::new(300.).least(300.).most(700.),
        trail: Pane::new(380.).least(220.),
    };
    let (handle, page, _) = open_with(cx, None, false, true, heights, 1000.);
    let table_after = |lead: f32, cx: &mut TestAppContext| {
        page.update(cx, |page, cx| {
            page.lead = Some(lead);
            cx.notify();
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let table = window.find("table").bounds();
            let inspector = window.find("inspector").bounds();
            close("under it", inspector.top(), table.bottom());
            close("to the bottom", inspector.bottom(), px(1000.));
            table.size.height
        })
        .unwrap()
    };
    close("given", table_after(400., cx), px(400.));
    close("a new height", table_after(520., cx), px(520.));
    close("its least", table_after(86., cx), px(300.));
    close("its most", table_after(900., cx), px(700.));

    cx.update_window(handle, |_, window, cx| {
        let state = page.read(cx).split.stacked_state().clone();
        state.update(cx, |state, cx| state.resize_panel(0, px(450.), window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    close("dragged", table_after(900., cx), px(450.));
    close("still the user's", table_after(400., cx), px(450.));
}
