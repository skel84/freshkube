use super::*;

#[gpui_kit::test]
fn a_source_that_is_not_a_stream_names_its_panel_and_follow_in_its_own_words(
    cx: &mut TestAppContext,
) {
    let (_runtime, panel, handle) = mount(cx);
    let label = |cx: &mut TestAppContext| {
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.find("logs-panel").label().map(str::to_owned)
        })
        .unwrap()
    };
    assert_eq!(label(cx).as_deref(), Some("Live logs panel"));
    panel.update(cx, |view, cx| {
        view.source_mut().words = Some((
            "Pause to review. Refresh adds newer messages.".into(),
            "Saved logs panel".into(),
        ));
        cx.notify();
    });
    assert_eq!(label(cx).as_deref(), Some("Saved logs panel"));
    panel.read_with(cx, |view, _| {
        assert_eq!(
            TestLogs::follow_tooltip(view),
            "Pause to review. Refresh adds newer messages."
        )
    });
}
