//! The Supply chain and Results rows and their edits, and what the Clusters
//! Inspector's Build history group says of the same check.
use super::*;
use crate::desktop::settings::history;
use crate::desktop::settings::tests::launch;
use freshkube_core::setup;

const SUPPLY: Read = Read::SupplyChain;
const RESULTS: Read = Read::Results;

/// A Builds entry that forwards to a Results Service, with a token and an
/// unknown key that no edit here may lose.
const SETUP: &str = r#"version: 1
clusters:
- {id: cluster-a, role: core, context: acme-a}
- id: cluster-b
  role: cicd
  context: acme-b
  results: {forward: {namespace: tekton-pipelines, service: results-api, port: 8080}, token: keys.json, future_results: keep}
sources: {builds: cluster-b}
future_setup: {keep: [1, 2]}
"#;

fn forwarding() -> Workspace {
    setup::parse(SETUP.as_bytes()).unwrap().workspace
}

fn checked(view: &View, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    window.click("settings-sources-check", cx);
    window.render_frame(cx);
    let _ = view;
}

#[gpui_kit::test]
fn the_rows_say_what_is_chosen_and_who_chose_it(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        set_workspace(&view, forwarding(), true, cx);
        open(window, cx, &view);
        assert_eq!(state(&view, SUPPLY, cx), "No key");
        assert_eq!(state(&view, RESULTS, cx), "Port forward");
        let supply = tooltip(&view, SUPPLY, cx);
        assert!(
            supply.contains("read from Tekton Chains on cluster-b")
                && supply.contains("verifying key is not set"),
            "{supply}"
        );
        assert!(supply.contains("Not checked"), "{supply}");
        let results = tooltip(&view, RESULTS, cx);
        assert!(
            results.contains("reached through a port forward to tekton-pipelines/results-api:8080"),
            "{results}"
        );
        for read in [SUPPLY, RESULTS] {
            window.click(format!("settings-source-{}", read.key()), cx);
            window.render_frame(cx);
            assert!(window.find("settings-source-choice").visible());
            assert!(window.find("settings-source-state").visible());
        }
        // The Inspector says where the setting lives, after the plain words.
        let hint = list(&view, cx)
            .read(cx)
            .rows
            .iter()
            .find(|row| row.read == SUPPLY)
            .and_then(|row| row.setup_hint.clone());
        assert_eq!(hint.as_deref(), Some("In freshkube.yaml as supply_chain"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn each_state_of_chains_and_results_is_told_apart(cx: &mut TestAppContext) {
    let cases = [
        (Reach::Readable, "Found", "Found Tekton Chains on cluster-b"),
        (
            Reach::not_served("x"),
            "Not installed",
            "isn’t installed on cluster-b",
        ),
        (
            Reach::refused("x"),
            "Not allowed to read",
            "doesn’t allow this account to read",
        ),
        (
            Reach::unreachable("x"),
            "Couldn’t reach",
            "Couldn’t reach cluster-b to look for",
        ),
    ];
    for (reach, word, sentence) in cases {
        let (_runtime, handle, view) = fixture(cx, 1280., 880.);
        cx.update_window(handle, |_, window, cx| {
            set_workspace(&view, forwarding(), true, cx);
            open(window, cx, &view);
            list(&view, cx).update(cx, |list, _| {
                list.set_example_answer("cluster-b", SUPPLY, reach.clone());
                list.set_example_answer("cluster-b", RESULTS, reach.clone());
            });
            checked(&view, window, cx);
            assert_eq!(state(&view, RESULTS, cx), word);
            let chains = tooltip(&view, SUPPLY, cx);
            let results = tooltip(&view, RESULTS, cx);
            assert!(
                chains.contains(sentence) || results.contains(sentence),
                "{chains} / {results}"
            );
            assert!(chains.contains("Tekton Chains") && results.contains("results-api"));
            // Optional tools: not installed is a note, never a failure.
            if matches!(reach, Reach::NotServed(_)) {
                assert!(chains.contains("rest of Freshkube works without it"));
            }
            // The Clusters Inspector says the same for the entry.
            let found = list(&view, cx).read(cx).history_for("cluster-b").cloned();
            assert_eq!(
                found.as_ref().map(|found| history::state(&found.chains)),
                Some(word)
            );
            assert!(list(&view, cx).read(cx).history_for("cluster-a").is_none());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_results_address_and_no_results_are_not_read_by_a_check(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let text = SETUP.replace(
            "{forward: {namespace: tekton-pipelines, service: results-api, port: 8080}, token: keys.json, future_results: keep}",
            "{address: 'https://results.acme.example'}",
        );
        let address = setup::parse(text.as_bytes()).unwrap().workspace;
        set_workspace(&view, address, true, cx);
        open(window, cx, &view);
        checked(&view, window, cx);
        assert_eq!(state(&view, RESULTS, cx), "Address");
        assert!(tooltip(&view, RESULTS, cx).contains("only when builds are read"));
        let found = list(&view, cx).read(cx).history_for("cluster-b").cloned();
        assert_eq!(found.map(|found| found.results), Some(None));

        let off = setup::parse(
            SETUP
                .replace(
                    "results: {forward: {namespace: tekton-pipelines, service: results-api, port: 8080}, token: keys.json, future_results: keep}",
                    "context: acme-b",
                )
                .replace("context: acme-b\n  context: acme-b", "context: acme-b")
                .as_bytes(),
        )
        .unwrap()
        .workspace;
        set_workspace(&view, off, true, cx);
        checked(&view, window, cx);
        assert_eq!(state(&view, RESULTS, cx), "Off");
        assert!(tooltip(&view, RESULTS, cx).contains("rest of Freshkube works without it"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn with_no_builds_entry_both_rows_say_so(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let none = setup::parse(
            b"version: 1\nclusters:\n- {id: cluster-a, role: core, context: acme-a}\n",
        )
        .unwrap()
        .workspace;
        set_workspace(&view, none, true, cx);
        open(window, cx, &view);
        // A core entry holds Builds when no CI/CD entry does.
        assert!(tooltip(&view, SUPPLY, cx).contains("cluster-a"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_inspectors_fit_beside_and_stacked(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        for read in [SUPPLY, RESULTS] {
            let (_runtime, handle, view) = fixture(cx, width, height);
            cx.update_window(handle, |_, window, cx| {
                set_workspace(&view, forwarding(), true, cx);
                open(window, cx, &view);
                window.click(format!("settings-source-{}", read.key()), cx);
                settle_sources(window, cx);
                if width > 1000. {
                    layout_check::assert_inspector(
                        window,
                        cx,
                        "settings-sources-split",
                        "settings-sources-table",
                        "settings-source-inspector",
                        "settings-source-inspector-title",
                    );
                    return;
                }
                // Stacked, the table shows three rows, so a row past them
                // is below its window until it is scrolled to, which a
                // headless frame doesn't do: assert_inspector's selected-row
                // check can't hold here. The rest of its geometry does.
                let table = window.find("settings-sources-table").bounds();
                let split = window.find("settings-sources-split").bounds();
                let pane = window.find("settings-source-inspector").bounds();
                let title = window.find("settings-source-inspector-title").bounds();
                let near = |a: gpui_kit::Pixels, b: gpui_kit::Pixels| {
                    (f32::from(a) - f32::from(b)).abs() <= 1.
                };
                assert!(near(pane.top(), table.bottom()), "{pane:?} / {table:?}");
                assert!(
                    near(pane.size.width, split.size.width),
                    "{pane:?} / {split:?}"
                );
                assert!(title.top() >= pane.top() && title.bottom() <= pane.bottom());
                assert!(window.find("settings-source-details").visible());
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
async fn edits_save_through_the_setup_file_and_keep_everything_else(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), SETUP);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    let key = std::env::temp_dir().join("acme").join("cosign.pub");
    let chosen = key.clone();
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-source-supply-chain", cx);
        window.render_frame(cx);
        assert!(window.find("settings-source-key-browse").visible());
        list(&view, cx).update(cx, |list, cx| list.set_key_file(Some(chosen), cx));
    })
    .unwrap();
    super::super::super::edit_tests::saved(cx, handle, &view).await;
    cx.update_window(handle, |_, _, cx| {
        list(&view, cx).update(cx, |list, cx| list.set_transparency_log(true, cx));
    })
    .unwrap();
    super::super::super::edit_tests::saved(cx, handle, &view).await;
    let file = std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap();
    let parsed = setup::parse(file.as_bytes()).unwrap();
    let chain = parsed.supply_chain.clone().unwrap();
    assert_eq!(chain.public_key_file.as_deref(), Some(key.as_path()));
    assert!(chain.transparency_log);
    assert!(
        file.contains("future_setup") && file.contains("future_results"),
        "{file}"
    );
    // No key's content is ever in the file, only where it is.
    assert!(!file.contains("PRIVATE"));

    cx.update_window(handle, |_, _, cx| {
        list(&view, cx).update(cx, |list, cx| list.set_key_file(None, cx));
    })
    .unwrap();
    super::super::super::edit_tests::saved(cx, handle, &view).await;
    cx.update_window(handle, |_, _, cx| {
        list(&view, cx).update(cx, |list, cx| list.set_transparency_log(false, cx));
    })
    .unwrap();
    super::super::super::edit_tests::saved(cx, handle, &view).await;
    // Both undone, the block goes: the file says nothing a file without it wouldn't.
    let file = std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap();
    assert!(
        setup::parse(file.as_bytes())
            .unwrap()
            .supply_chain
            .is_none(),
        "{file}"
    );
}

#[gpui_kit::test]
async fn results_is_typed_saved_and_turned_off_on_the_builds_entry(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), SETUP);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-source-results", cx);
        window.render_frame(cx);
        assert!(window.find("settings-source-results-input").visible());
        // Text that is neither shape is refused where it was typed.
        window.click("settings-source-results-input", cx);
        window.press("secondary-a", cx);
        window.input("results", cx);
        window.render_frame(cx);
        window.click("settings-source-results-save", cx);
        window.render_frame(cx);
        assert!(list(&view, cx).read(cx).results_error.is_some());
        window.press("secondary-a", cx);
        window.input("https://results.acme.example", cx);
        window.render_frame(cx);
        window.click("settings-source-results-save", cx);
    })
    .unwrap();
    super::super::super::edit_tests::saved(cx, handle, &view).await;
    let builds = |directory: &std::path::Path| {
        super::super::super::edit_tests::read_workspace(directory)
            .clusters
            .into_iter()
            .find(|entry| entry.id == "cluster-b")
            .unwrap()
    };
    let entry = builds(guard.path());
    let results = entry.results.as_deref().unwrap();
    assert_eq!(
        results.address.as_deref(),
        Some("https://results.acme.example")
    );
    assert!(results.forward.is_none());
    // The token it names is kept, and so is the key this build doesn't know.
    assert!(results.token.is_some());
    let file = std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap();
    assert!(
        file.contains("future_results") && file.contains("future_setup"),
        "{file}"
    );

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        list(&view, cx).update(cx, |list, cx| list.turn_results_off(cx));
    })
    .unwrap();
    super::super::super::edit_tests::saved(cx, handle, &view).await;
    assert!(builds(guard.path()).results.is_none());
}

#[gpui_kit::test]
fn example_data_refuses_every_edit(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        set_workspace(&view, forwarding(), true, cx);
        open(window, cx, &view);
        list(&view, cx).update(cx, |list, cx| {
            list.set_key_file(Some(std::env::temp_dir().join("acme").join("a.pub")), cx);
            list.set_transparency_log(true, cx);
        });
        window.render_frame(cx);
        assert_eq!(state(&view, SUPPLY, cx), "No key");
        assert!(list(&view, cx).read(cx).supply_chain.is_none());
    })
    .unwrap();
}
