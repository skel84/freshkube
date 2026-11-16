//! The Build history group of the Clusters Inspector: what the entry reads,
//! and the state of the last Check sources, in each case.
use super::*;
use crate::desktop::settings::SettingsSection;
use crate::desktop::settings::history;
use crate::desktop::tests::fixture;
use freshkube_core::workspace::Loaded;
use freshkube_delivery::entry::Reach;

const SETUP: &str = r#"version: 1
clusters:
- {id: cluster-a, role: core, context: acme-a}
- id: cluster-b
  role: cicd
  context: acme-b
  results: {forward: {namespace: tekton-pipelines, service: results-api, port: 8080}}
sources: {builds: cluster-b}
"#;

#[gpui_kit::test]
fn the_group_says_what_an_entry_reads_and_what_the_last_check_found(cx: &mut TestAppContext) {
    let cases = [
        (Reach::Readable, "Found"),
        (Reach::not_served("x"), "Not installed"),
        (Reach::refused("x"), "Not allowed to read"),
        (Reach::unreachable("x"), "Couldn’t reach"),
    ];
    for (reach, word) in cases {
        let (_runtime, handle, view) = fixture(cx, 1280., 880.);
        cx.update_window(handle, |_, window, cx| {
            let workspace = setup::parse(SETUP.as_bytes()).unwrap().workspace;
            view.update(cx, |pilot, cx| {
                pilot.settings_page.update(cx, |page, cx| {
                    page.set_workspace(&Loaded::Workspace(workspace), true, cx)
                });
                pilot.show_settings(SettingsSection::Clusters, window, cx);
            });
            window.render_frame(cx);
            window.click("settings-cluster-cluster-b", cx);
        })
        .unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // Before any check, the group says so and names no state.
            for id in [
                "settings-history",
                "settings-history-results-row",
                "settings-history-chains-row",
                "settings-history-state-row",
            ] {
                // The Inspector scrolls: the group is below Metrics.
                assert!(window.try_find(id).is_some(), "{id}");
            }
            let sources = view.read(cx).settings_page.read(cx).sources.clone();
            assert!(sources.read(cx).history_for("cluster-b").is_none());
            sources.update(cx, |list, cx| {
                list.set_example_answer(
                    "cluster-b",
                    crate::desktop::settings::sources::Read::SupplyChain,
                    reach.clone(),
                );
                list.set_example_answer(
                    "cluster-b",
                    crate::desktop::settings::sources::Read::Results,
                    reach.clone(),
                );
                list.run_check(cx);
            });
            window.render_frame(cx);
            let found = sources.read(cx).history_for("cluster-b").cloned().unwrap();
            assert_eq!(history::state(&found.chains), word);
            assert!(window.try_find("settings-history-state-row").is_some());
            // Only the entry Builds is read from has a state.
            assert!(sources.read(cx).history_for("cluster-a").is_none());
        })
        .unwrap();
    }
}
