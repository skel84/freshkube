use super::*;

#[gpui_kit::test]
fn threshold_edits_validate_and_remain_local(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.open_app(example::id(example::WORKER), Report::Memory, cx)
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-threshold", cx);
        window.render_frame(cx);
        window.click("obs-threshold-input", cx);
        window.press("secondary-a", cx);
        window.input("-1", cx);
        window.click("ok", cx);
        window.render_frame(cx);
        assert!(page.read(cx).thresholds.is_empty());
        assert!(window.find("obs-threshold-input").visible());
        window.click("obs-threshold-input", cx);
        window.press("secondary-a", cx);
        window.input("90", cx);
        window.click("ok", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx)
                .thresholds
                .get(&(example::WORKER.into(), Report::Memory))
                .map(String::as_str),
            Some("90")
        );
    })
    .unwrap();
}

/// The threshold dialog reads the report tab on screen, not the report the
/// application opened on (#408).
#[gpui_kit::test]
fn the_threshold_dialog_reads_the_selected_report(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.open_app(example::id(example::WORKER), Report::Net, cx)
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-report-cpu", cx);
        window.render_frame(cx);
        let dialog = page.read(cx).threshold_dialog().unwrap();
        assert_eq!(dialog.title.as_ref(), "CPU threshold");
        assert_eq!(dialog.value, "85");
        window.click("obs-threshold", cx);
        window.render_frame(cx);
        assert!(window.find("obs-threshold-input").visible());
        assert_eq!(page.read(cx).threshold.read(cx).value().as_ref(), "85");
    })
    .unwrap();
}

/// A saved threshold lands under the report on screen (#408).
#[gpui_kit::test]
fn a_saved_threshold_lands_under_the_selected_report(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.open_app(example::id(example::WORKER), Report::Net, cx)
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-report-cpu", cx);
        window.render_frame(cx);
        window.click("obs-threshold", cx);
        window.render_frame(cx);
        window.click("obs-threshold-input", cx);
        window.press("secondary-a", cx);
        window.input("70", cx);
        window.click("ok", cx);
        window.render_frame(cx);
        let thresholds = &page.read(cx).thresholds;
        assert_eq!(
            thresholds
                .get(&(example::WORKER.into(), Report::Cpu))
                .map(String::as_str),
            Some("70")
        );
        assert!(!thresholds.contains_key(&(example::WORKER.into(), Report::Net)));
        // Back on CPU, the dialog opens on the saved value.
        assert_eq!(page.read(cx).threshold_dialog().unwrap().value, "70");
    })
    .unwrap();
}
