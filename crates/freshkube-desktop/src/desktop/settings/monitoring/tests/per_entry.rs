use super::*;

#[gpui_kit::test]
fn per_entry_links_to_clusters_and_selects_the_first_entry(cx: &mut TestAppContext) {
    for entries in [true, false] {
        let (_runtime, handle, pilot) = fixture(cx, 1280., 1600.);
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| {
                pilot.show_settings(SettingsSection::Clusters, window, cx);
                if !entries {
                    pilot.settings_page.update(cx, |page, cx| {
                        page.set_workspace(&freshkube_core::workspace::Loaded::Missing, false, cx)
                    });
                }
            });
            window.render_frame(cx);
            if entries {
                window.click("settings-cluster-prod-fra", cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
        let page = open(cx, handle, &pilot);
        cx.update_window(handle, |_, window, cx| {
            window.click("metrics-per-entry", cx);
            window.render_frame(cx);
            assert!(window.find("metrics-per-entry-row-hint").visible());
            assert_eq!(
                window.find("metrics-per-entry-explanation").label(),
                Some("Each entry names its own source in freshkube.yaml.")
            );
            assert_eq!(
                window.find("metrics-per-entry-clusters").label(),
                Some("Edit it in Clusters")
            );
            window.click("metrics-per-entry-clusters", cx);
        })
        .unwrap();
        cx.run_until_parked();
        // Choosing Per entry changed the draft, so leaving asks first.
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("settings-unsaved-discard", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                pilot.read(cx).navigation.page,
                crate::desktop::Page::Settings
            );
            assert_eq!(page.read(cx).section(), SettingsSection::Clusters);
            assert!(window.find("nav-settings-clusters").visible());
            if entries {
                assert_eq!(page.read(cx).selected.as_deref(), Some("core-fra"));
                assert_eq!(
                    window.find("settings-detail-title").label(),
                    Some("core-fra")
                );
                assert!(window.find("settings-metrics").visible());
            } else {
                assert!(page.read(cx).selected.is_none());
                assert!(window.find("settings-empty").visible());
                assert!(window.try_find("settings-metrics").is_none());
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn per_entry_explanation_wraps_at_text_twenty_in_a_small_window(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, _, cx| crate::text_size::set(20., cx))
        .unwrap();
    let page = open(cx, handle, &pilot);
    cx.update_window(handle, |_, window, cx| {
        window.click("metrics-per-entry", cx);
        window.render_frame(cx);
        let hint = window.find("metrics-per-entry-row-hint").bounds();
        let explanation = window.find("metrics-per-entry-explanation").bounds();
        let link = window.find("metrics-per-entry-clusters");
        assert!(
            explanation.size.height > ui::dp_px(24., window),
            "the explanation must wrap: {explanation:?}"
        );
        assert!(explanation.left() >= hint.left() - px(1.));
        assert!(explanation.right() <= hint.right() + px(1.));
        assert!(link.bounds().right() <= hint.right() + px(1.));
        assert!(link.bounds().top() >= explanation.bottom() - px(1.));
        assert!(link.visible());
        window.click("metrics-per-entry-clusters", cx);
    })
    .unwrap();
    cx.run_until_parked();
    // Choosing Per entry changed the draft, so leaving asks first.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-unsaved-discard", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(page.read(cx).section(), SettingsSection::Clusters);
        assert_eq!(page.read(cx).selected.as_deref(), Some("core-fra"));
    })
    .unwrap();
}
