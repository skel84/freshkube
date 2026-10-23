use super::*;

#[gpui_kit::test]
fn the_form_story_shows_one_state_at_a_time(cx: &mut TestAppContext) {
    let index = super::stories::find("form").unwrap();
    let (handle, _) = open(cx, index);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let hint = window.find("form-context-hint").bounds();
        assert!(window.try_find("form-context-error").is_none());
        assert!(window.try_find("form-connection-dirty").is_none());
        assert!(window.try_find("form-save").is_none());

        window.click("form-error", cx);
        window.render_frame(cx);
        assert!(window.try_find("form-context-hint").is_none());
        let error = window.find("form-context-error");
        assert_eq!(error.role(), Some(gpui_kit::Role::Alert));
        assert_eq!(error.bounds().origin, hint.origin);
        let input = window
            .find("form-context-input")
            .bounds()
            .scale(window.scale_factor());
        assert!(window.painted_quads().iter().any(|quad| {
            quad.border_color == freshkube_ui::palette::palette(cx).crit
                && quad.bounds == input
                && quad.border_widths.top.0 > 0.
        }));

        window.click("form-secret", cx);
        window.render_frame(cx);
        assert!(window.find("form-credential-kept").visible());
        assert!(window.find("form-credential-replace").visible());
        assert!(window.find("form-credential-forget").visible());
        assert!(window.try_find("form-context").is_none());
        assert!(window.try_find("form-credential-input").is_none());
        window.click("form-credential-replace", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("form-credential-input").role(),
            Some(gpui_kit::Role::PasswordInput)
        );
        assert!(window.find("form-credential-input").value().is_none());

        window.click("form-plain", cx);
        window.click("form-secret", cx);
        window.render_frame(cx);
        window.click("form-credential-forget", cx);
        window.render_frame(cx);
        assert!(window.try_find("form-credential-kept").is_none());
        assert!(window.find("form-credential-input").visible());

        window.click("form-plain", cx);
        window.render_frame(cx);
        assert!(window.find("form-context-hint").visible());
        assert!(window.try_find("form-credential-input").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_form_story_stacks_narrow_labels_and_orders_its_actions(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, super::stories::find("form").unwrap());
    cx.update_window(handle, |_, window, cx| {
        for state in ["dirty", "narrow"] {
            window.click(format!("form-{state}"), cx);
            window.render_frame(cx);
            assert!(window.find("form-connection-dirty").visible());
            let label = window.find("form-context-label").bounds();
            let input = window.find("form-context-input").bounds();
            if state == "narrow" {
                assert!(
                    window.find("form-connection").bounds().size.width
                        < px(freshkube_ui::form::NARROW)
                );
                assert_eq!(label.left(), input.left());
                assert_eq!(input.top() - label.bottom(), px(4.));
            } else {
                assert_eq!(label.size.width, px(freshkube_ui::form::LABEL_WIDTH));
                assert_eq!(input.left() - label.right(), px(12.));
                assert_eq!(label.center().y, input.center().y);
            }
            let save = window.find("form-save").bounds();
            let revert = window.find("form-revert").bounds();
            use freshkube_ui::platform::{Platform, Side};
            for platform in Platform::ALL {
                // The component's layout tests render all three orders;
                // the story uses this build's platform answer.
                let expected = match platform {
                    Platform::Windows => Side::Leading,
                    Platform::MacOs | Platform::Linux => Side::Trailing,
                };
                assert_eq!(platform.default_button(), expected);
                if platform == Platform::current() {
                    assert_eq!(save.left() < revert.left(), expected == Side::Leading);
                }
            }
            assert_eq!(save.size.height, px(24.));
            assert_eq!(revert.size.height, px(24.));
            window.click(
                if state == "dirty" {
                    "form-save"
                } else {
                    "form-revert"
                },
                cx,
            );
            window.render_frame(cx);
            assert!(window.try_find("form-connection-dirty").is_none());
            // Clicking the clean group's Save leaves it clean.
            window.click("form-save", cx);
            window.render_frame(cx);
            assert!(window.try_find("form-connection-dirty").is_none());
        }
    })
    .unwrap();
}
