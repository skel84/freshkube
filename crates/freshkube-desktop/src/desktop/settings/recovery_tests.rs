//! The setup row-recovery warnings and empty state, using invented files.
use super::SettingsSection;
use crate::desktop::testing::mount;
use crate::{DebugOptions, GpuiOptions};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, px};

const MIXED: &str = "version: 1\nclusters: [{id: retained, role: core, context: retained}, {id: skipped, role: invented-unknown-role, context: skipped}]\ndestinations: [{server: 'ftp://example.test', entry: retained}]\n";
const WARNING_TITLE: &str = "Some rows in freshkube.yaml skipped";

fn options(directory: &std::path::Path) -> GpuiOptions {
    GpuiOptions::new(Some(directory.join("missing")), None, 100)
        .with_preferences(Some(directory.join("preferences.json")))
        .with_debug(DebugOptions::default().with_page("settings-clusters"))
}

#[gpui_kit::test]
fn recovered_rows_have_their_own_banner_title_on_every_workspace_section(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("freshkube.yaml"), MIXED).unwrap();
        let (_runtime, handle, pilot) = mount(cx, options(directory.path()), width, height);
        for section in [
            SettingsSection::Clusters,
            SettingsSection::Sources,
            SettingsSection::Destinations,
        ] {
            cx.update_window(handle, |_, window, cx| {
                pilot.update(cx, |pilot, cx| pilot.show_settings(section, window, cx));
                window.render_frame(cx);
                assert_eq!(
                    window.find("settings-banner-content").label(),
                    Some(WARNING_TITLE)
                );
                let banner = window.find("settings-banner");
                let words = banner.label().unwrap();
                assert!(words.contains("used with warnings"));
                assert!(words.contains(
                    "skipped clusters[1] (skipped): role: is unknown; use core, cicd or environment"
                ));
                assert!(words.contains("destinations[0] (retained): server"));
                assert!(!words.contains("invented-unknown-role"));
                assert!(!words.contains("ftp://"));
            })
            .unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(directory.path().join("freshkube.yaml")).unwrap(),
            MIXED
        );
    }
}

#[gpui_kit::test]
fn all_skipped_clusters_have_an_empty_explanation_that_agrees_with_the_count(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    for (yaml, count, expected) in [
        (
            "version: 1\nclusters: [[]]\n",
            "0 clusters",
            "All configured clusters were skipped. Fix the skipped rows in freshkube.yaml and Reload to use them.",
        ),
        (
            "version: 1\nclusters: []\n",
            "0 clusters",
            "Nothing is listed. The one cluster counted in the status bar is implicit: the one this window opened.",
        ),
        (
            "",
            "1 cluster",
            "Nothing is listed. The one cluster counted in the status bar is implicit: the one this window opened.",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("freshkube.yaml");
        if !yaml.is_empty() {
            std::fs::write(&path, yaml).unwrap();
        }
        let (_runtime, handle, pilot) = mount(cx, options(directory.path()), 1280., 880.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("settings-empty").visible());
            assert_eq!(
                window.find("settings-empty-explanation").label(),
                Some(expected)
            );
            let scope = window.find("settings-scope");
            assert!(
                scope.label().unwrap().contains(count),
                "{yaml:?}: {scope:?}"
            );
            window.click("settings-add", cx);
            if yaml.contains("[[]]") {
                assert!(window.try_find("settings-cluster-form").is_none());
                assert_eq!(
                    window.find("settings-banner-content").label(),
                    Some(WARNING_TITLE)
                );
            } else {
                assert!(window.find("settings-cluster-form").visible());
                window.click("settings-form-cancel", cx);
            }
        })
        .unwrap();
        std::fs::write(
            &path,
            "version: 1\nclusters: [{id: repaired, role: core, context: repaired}]\n",
        )
        .unwrap();
        super::tests::poll_setup(cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("settings-cluster-repaired").visible());
            assert!(window.try_find("settings-empty-explanation").is_none());
            assert!(window.try_find("settings-banner").is_none());
            assert!(!pilot.read(cx).settings_page.read(cx).setup_skips);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn fatal_setup_refusals_keep_their_own_title(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("freshkube.yaml"), MIXED).unwrap();
    let (_runtime, handle, _pilot) = mount(cx, options(directory.path()), 1280., 880.);
    std::fs::write(directory.path().join("freshkube.yaml"), "version: [").unwrap();
    super::tests::poll_setup(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("settings-banner-content").label(),
            Some("Setup file")
        );
        let banner = window.find("settings-banner");
        let words = banner.label().unwrap();
        assert!(words.contains("isn't used"));
        assert!(!words.contains("skipped clusters"));
        assert!(!words.contains("used with warnings"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn disabled_add_explains_recovery_without_clipping_at_the_window_edge(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    for (width, height, text_size) in [(1280., 880., 13.), (760., 560., 13.), (760., 560., 20.)] {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("freshkube.yaml"), MIXED).unwrap();
        let (_runtime, handle, _pilot) = mount(cx, options(directory.path()), width, height);
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(text_size, cx);
            window.render_frame(cx);
            window.hover("settings-add", cx);
        })
        .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(600));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let text = window.find("settings-action-tooltip");
            assert_eq!(text.label(), Some(crate::setup_file::SKIPPED_SAVE));
            let bounds = text.bounds();
            let viewport =
                gpui_kit::Bounds::new(gpui_kit::point(px(0.), px(0.)), window.viewport_size());
            assert!(
                bounds.is_contained_within(&viewport),
                "{bounds:?} outside {viewport:?}"
            );
            assert!(
                bounds.size.height > crate::ui::dp_px(26., window),
                "tooltip must wrap: {bounds:?}"
            );
            window.click("settings-add", cx);
            assert!(window.try_find("settings-cluster-form").is_none());
        })
        .unwrap();
    }
}
