use super::*;

#[gpui_kit::test]
fn a_rows_level_follows_its_severity(cx: &mut TestAppContext) {
    use freshkube_core::types::LogLevel;
    use freshkube_ui::palette::palette;

    let (_runtime, _panel, _handle) = mount(cx);
    cx.update(|cx| {
        let p = palette(cx);
        let style = |level| super::super::view::level_style(&level, &p);
        // An error reads as more severe than a warning, not as information.
        assert_eq!(style(LogLevel::Error), ("ERROR", p.crit_ink, p.crit));
        assert_eq!(style(LogLevel::Warning), ("WARN", p.warn_ink, p.warn));
        assert_eq!(style(LogLevel::Info).1, p.muted);
    });
}

#[gpui_kit::test]
fn the_levels_menu_hides_a_level_and_says_how_many_show(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        assert_eq!(
            window.find("logs-levels").label(),
            Some("Levels: all shown")
        );
        // The tooltip counts each level, derived when the counts changed.
        assert_eq!(
            panel.read(cx).levels_tip.1.as_ref(),
            "Lines by level: Error 2 · Warn 0 · Info 118 · Debug 0 · Unknown 0"
        );
        window.click("logs-levels", cx);
    })
    .unwrap();
    cx.run_until_parked();
    // The first item, Error, from the keyboard.
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = panel.read(cx);
        let levels = &view.review.logs.buffer().filters().levels;
        assert!(!levels.accepts(&freshkube_core::types::LogLevel::Error));
        assert!(levels.accepts(&freshkube_core::types::LogLevel::Info));
        assert_eq!(
            window.find("logs-levels").label(),
            Some("Levels: 4 of 5 shown")
        );
    })
    .unwrap();
}
