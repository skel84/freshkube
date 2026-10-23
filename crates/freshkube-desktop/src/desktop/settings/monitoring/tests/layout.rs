use super::*;

#[gpui_kit::test]
fn monitoring_has_the_padded_frame_at_each_window_and_text_size(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, pilot) = fixture(cx, width, height);
        let page = open(cx, handle, &pilot);
        for size in [13., 20.] {
            cx.update_window(handle, |_, window, cx| {
                crate::text_size::set(size, cx);
                window.render_frame(cx);
                layout_check::assert_page_frame(
                    window,
                    cx,
                    &layout_check::PageFrame {
                        page: "settings-page",
                        title: "settings-monitoring-title",
                        title_text: "Monitoring",
                        content: "settings-monitoring-form",
                    },
                );
                let group = window.find("monitoring-metrics").bounds();
                assert!(group.size.width <= ui::dp_px(form::MAX_WIDTH, window) + px(1.));
                for id in ["metrics-automatic", "metrics-central", "metrics-per-entry"] {
                    let control = window.find(id).bounds();
                    assert!(
                        (control.size.height - ui::dp_px(ui::CONTROL_HEIGHT, window)).abs()
                            < px(1.)
                    );
                    assert!(
                        control.right() <= group.right() + px(1.),
                        "{width} x {height}, text {size}: {id} exceeds the group"
                    );
                }
                let long = "Failed · The store answers: Unreachable · couldn’t connect to the server at https://metrics.example.test while checking the selected entry. The connection attempt ended before the store answered. ".repeat(2);
                for outcome in [None, Some((ui::Tone::Crit, long))] {
                    let has_result = outcome.is_some();
                    page.update(cx, |page, _| {
                        let draft = page.monitoring.as_mut().unwrap();
                        draft.mode = setup::Mode::Automatic;
                        draft.outcome = outcome;
                    });
                    layout_check::assert_page_frame(window, cx, &layout_check::PageFrame {
                        page: "settings-page", title: "settings-monitoring-title", title_text: "Monitoring", content: "settings-monitoring-form",
                    });
                    let group = window.find("monitoring-metrics").bounds();
                    let line = window.find("metrics-test-result").bounds();
                    let text = window.find("metrics-test-text").bounds();
                    assert!(line.right() <= group.right() + px(1.));
                    assert!(text.right() <= line.right() + px(1.));
                    assert!(text.left() > window.find("metrics-test").bounds().right());
                    if has_result {
                        assert!(text.size.height > ui::dp_px(24., window), "the result wraps");
                        let glyph = window.find("metrics-test-glyph").bounds();
                        assert!(glyph.right() < text.left());
                        assert!((glyph.top() - text.top()).abs() <= ui::dp_px(4., window));
                    } else if width == 760. && size == 20. {
                        assert!(text.size.height > ui::dp_px(18., window), "the narrow hint wraps");
                    }
                }
                page.update(cx, |page, _| {
                    page.banner = Some("An invented setup warning.".into());
                    page.warning = Some("An invented unknown key.".into());
                    page.monitoring.as_mut().unwrap().error = Some("An invented save refusal.".into());
                });
                window.render_frame(cx);
                for id in ["monitoring-setup-banner", "monitoring-unknown-banner", "monitoring-save-banner"] {
                    let banner = window.find(id).bounds();
                    assert!(banner.right() <= group.right() + px(1.));
                    assert!((banner.left() - group.left()).abs() < px(1.));
                }
            })
            .unwrap();
        }
    }
}
