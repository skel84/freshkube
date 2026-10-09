//! Changing the workspace through the page: the form, the actions on the
//! selection, and the save they end in.
use super::tests::launch;
use super::{Notice, Origin};
use crate::desktop::Page;
use freshkube_core::workspace::{self, Loaded};
use gpui_kit::{
    AppContext, TestAppContext,
    test::{TestAppContextExt, TestWindowExt},
};
use std::{path::Path, time::Duration};

const TWO: &str = r#"{"version":1,"clusters":[
    {"id":"mgmt","role":"core","context":"acme-mgmt"},
    {"id":"prod","role":"environment","context":"acme-prod"}]}"#;

fn ids(directory: &Path) -> Vec<String> {
    match workspace::load(&directory.join("workspace.json")) {
        Loaded::Workspace(workspace) => workspace.clusters.iter().map(|e| e.id.clone()).collect(),
        other => panic!("{other:?}"),
    }
}

async fn open_settings(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    view: &gpui_kit::Entity<crate::desktop::Pilot>,
) {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.navigate_from_keyboard(Page::Settings, window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
}

/// Waits for the save in flight to answer.
async fn saved(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    view: &gpui_kit::Entity<crate::desktop::Pilot>,
) {
    let view = view.clone();
    cx.wait_for(handle, Duration::from_secs(5), move |_, cx| {
        // A click starts the save at once; it is over when it has answered.
        view.read(cx).settings_page.read(cx).saving.is_none()
    })
    .await;
}

#[gpui_kit::test]
async fn adding_a_cluster_creates_the_file_and_lists_it(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-add", cx);
        window.render_frame(cx);
        window.click("settings-form-context", cx);
        window.input("Acme Staging", cx);
        window.click("settings-form-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    assert_eq!(ids(guard.path()), vec!["acme-staging"]);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-cluster-acme-staging").visible());
        assert!(window.find("settings-saved").visible());
        // The new row is the selected one.
        let page = view.read(cx).settings_page.read(cx);
        assert_eq!(page.selected.as_deref(), Some("acme-staging"));
        let page = view.read(cx).settings_page.read(cx);
        assert_eq!(page.origin(), &Origin::File);
        assert!(matches!(page.notice, Some(Notice::Saved(_))));
    })
    .unwrap();
}

fn write(directory: &Path, text: &str) {
    std::fs::write(directory.join("workspace.json"), text).unwrap();
}

/// Adds a cluster through the form, as a person would.
fn add(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, context: &str, talosconfig: &str) {
    window.click("settings-add", cx);
    window.render_frame(cx);
    window.click("settings-form-context", cx);
    window.input(context, cx);
    if !talosconfig.is_empty() {
        window.click("settings-form-talosconfig", cx);
        window.input(talosconfig, cx);
    }
    window.click("settings-form-save", cx);
}

#[gpui_kit::test]
async fn editing_a_cluster_changes_its_role_and_keeps_its_place(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-cluster-prod", cx);
        window.press("e", cx);
        window.render_frame(cx);
        // The id is shown, not a field: there is nothing to type into, and
        // the id saved below is the one the cluster had. (Clicking the text
        // does not move the keyboard off the context field, so typing after
        // it would edit the context; this test does not type.)
        assert!(window.find("settings-form-id").visible());
        assert!(window.try_find("settings-form-id-input").is_none());
        window.click(("settings-role", 1usize), cx);
        window.click("settings-form-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let Loaded::Workspace(file) = workspace::load(&guard.path().join("workspace.json")) else {
        panic!("not saved");
    };
    let roles: Vec<_> = file
        .clusters
        .iter()
        .map(|e| (e.id.as_str(), e.role))
        .collect();
    assert_eq!(
        roles,
        vec![
            ("mgmt", workspace::Role::Core),
            ("prod", workspace::Role::Cicd)
        ]
    );
    let contexts: Vec<_> = file.clusters.iter().map(|e| e.context.as_str()).collect();
    assert_eq!(contexts, vec!["acme-mgmt", "acme-prod"]);
}

#[gpui_kit::test]
async fn removing_asks_first_and_takes_the_entry_out_of_the_file_only(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-cluster-prod", cx);
        window.press("backspace", cx);
        window.render_frame(cx);
        assert!(window.find("settings-remove-dialog").visible());
        window.click("settings-remove-cancel", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings-remove-dialog").is_none());
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(ids(guard.path()), vec!["mgmt", "prod"]);
    cx.update_window(handle, |_, window, cx| {
        window.press("backspace", cx);
        window.render_frame(cx);
        window.click("settings-remove-confirm", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    assert_eq!(ids(guard.path()), vec!["mgmt"]);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-cluster-prod").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn moving_a_cluster_saves_the_new_order(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-cluster-prod", cx);
        window.press("secondary-alt-up", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    assert_eq!(ids(guard.path()), vec!["prod", "mgmt"]);
    // The first cannot go further up: nothing is written.
    cx.update_window(handle, |_, window, cx| {
        view.read(cx)
            .settings_page
            .clone()
            .update(cx, |page, _| page.notice = None);
        window.press("secondary-alt-up", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        assert!(view.read(cx).settings_page.read(cx).notice.is_none());
    })
    .unwrap();
    assert_eq!(ids(guard.path()), vec!["prod", "mgmt"]);
}

#[gpui_kit::test]
async fn the_first_save_sets_a_refused_file_aside_and_keeps_it(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), "{\"version\":9}");
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| add(window, cx, "acme-ci", ""))
        .unwrap();
    saved(cx, handle, &view).await;
    assert_eq!(ids(guard.path()), vec!["acme-ci"]);
    let backup = guard.path().join("workspace.json.bak");
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), "{\"version\":9}");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-banner").is_none());
        let page = view.read(cx).settings_page.read(cx);
        let Some(Notice::Saved(text)) = &page.notice else {
            panic!("{:?}", page.notice);
        };
        assert!(text.contains("workspace.json.bak"), "{text}");
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_save_that_cannot_set_the_file_aside_changes_nothing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    // A directory where the file belongs: refused, and it cannot be linked.
    std::fs::create_dir(guard.path().join("workspace.json")).unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| add(window, cx, "acme-ci", ""))
        .unwrap();
    saved(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        assert!(matches!(page.origin(), Origin::Refused(_)));
        assert!(window.try_find("settings-cluster-acme-ci").is_none());
        // The form stays open with what was typed, and says why.
        let error = window.find("settings-form-error");
        assert!(error.visible());
        let text = error.label().unwrap_or_default().to_owned();
        assert!(
            text.starts_with("Couldn’t set the unused workspace.json aside:"),
            "{text}"
        );
        assert!(window.find("settings-cluster-form").visible());
    })
    .unwrap();
    assert!(guard.path().join("workspace.json").is_dir());
    assert_eq!(std::fs::read_dir(guard.path()).unwrap().count(), 1);
}

#[gpui_kit::test]
async fn unknown_keys_are_named_and_kept_by_a_save(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(
        guard.path(),
        r#"{"version":1,"clusters":[
            {"id":"mgmt","role":"core","context":"acme-mgmt","talosconfg":"/typo"}],
            "destinations":[{"server":"https://prod.example.test","entry":"mgmt"}]}"#,
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let banner = window.find("settings-unknown-keys");
        assert!(banner.visible());
        let page = view.read(cx).settings_page.read(cx);
        let warning = page.warning.clone().unwrap();
        assert!(warning.contains("mgmt.talosconfg"), "{warning}");
        assert!(!warning.contains("destinations"), "{warning}");
        add(window, cx, "acme-ci", "");
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(guard.path().join("workspace.json")).unwrap())
            .unwrap();
    assert_eq!(written["clusters"][0]["talosconfg"], "/typo");
    assert_eq!(
        written["destinations"][0]["server"],
        "https://prod.example.test"
    );
    assert_eq!(written["clusters"][1]["id"], "acme-ci");
}

#[gpui_kit::test]
async fn a_form_the_file_would_refuse_says_why_and_saves_nothing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        add(window, cx, "acme-ci", "relative/talosconfig");
        window.render_frame(cx);
        assert!(window.find("settings-form-error").visible());
        // The message names the field that is wrong, not an id.
        let text = window
            .find("settings-form-error")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(
            text.starts_with("Talosconfig must be an absolute path"),
            "{text}"
        );
        window.click("settings-form-cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!guard.path().join("workspace.json").exists());
    cx.update_window(handle, |_, _, cx| {
        assert!(view.read(cx).settings_page.read(cx).notice.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn example_data_never_writes_and_its_actions_are_off(cx: &mut TestAppContext) {
    use crate::GpuiOptions;
    use crate::desktop::tests::mount;
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let options =
        GpuiOptions::fixture().with_preferences(Some(guard.path().join("preferences.json")));
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-cluster-prod-fra", cx);
        for key in ["a", "e", "backspace", "secondary-alt-up"] {
            window.press(key, cx);
        }
        window.click("settings-add", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings-cluster-form").is_none());
        assert!(window.try_find("settings-remove-dialog").is_none());
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read_dir(guard.path()).unwrap().count(), 0);
    cx.update_window(handle, |_, _, cx| {
        let page = view.read(cx).settings_page.read(cx);
        assert!(page.notice.is_none() && page.saving.is_none());
        assert_eq!(page.origin(), &Origin::Example);
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_hand_edit_made_while_the_app_runs_is_kept_and_reload_shows_it(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    // Someone edits the file in an editor.
    let edited = TWO.replace("acme-prod", "acme-prod-edited");
    write(guard.path(), &edited);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-cluster-prod", cx);
        window.press("secondary-alt-up", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        let Some(Notice::Failed(why)) = &page.notice else {
            panic!("{:?}", page.notice);
        };
        assert!(why.contains("changed on disk"), "{why}");
        assert!(window.find("settings-save-failed").visible());
    })
    .unwrap();
    // Nothing was written or moved.
    assert_eq!(
        std::fs::read_to_string(guard.path().join("workspace.json")).unwrap(),
        edited
    );
    assert_eq!(std::fs::read_dir(guard.path()).unwrap().count(), 1);

    // Reload shows the file as it is, and a change then goes through.
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-reload", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings-save-failed").is_none());
        window.click("settings-cluster-prod", cx);
        window.press("secondary-alt-up", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    assert_eq!(ids(guard.path()), vec!["prod", "mgmt"]);
    let kept = std::fs::read_to_string(guard.path().join("workspace.json")).unwrap();
    assert!(kept.contains("acme-prod-edited"), "{kept}");
}

#[gpui_kit::test]
async fn a_file_made_after_launch_is_not_replaced_by_the_first_save(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    // Another window, or an editor, makes the file after this one launched.
    write(guard.path(), TWO);
    cx.update_window(handle, |_, window, cx| add(window, cx, "acme-ci", ""))
        .unwrap();
    saved(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let text = window
            .find("settings-form-error")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(text.contains("changed on disk"), "{text}");
    })
    .unwrap();
    assert_eq!(ids(guard.path()), vec!["mgmt", "prod"]);
}

#[gpui_kit::test]
async fn the_form_stays_open_with_what_was_typed_when_the_save_fails(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-add", cx);
        window.render_frame(cx);
        window.click("settings-form-context", cx);
        window.input("acme-ci", cx);
    })
    .unwrap();
    write(guard.path(), TWO);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-form-save", cx)
    })
    .unwrap();
    saved(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-form-error").visible());
        // Reading the file again behind the open form, as the Reload button
        // would, makes the same Save go through. Nothing is typed again, so
        // the cluster that is written is the one that was typed before.
        let page = view.read(cx).settings_page.clone();
        page.update(cx, |page, cx| page.reload(cx));
        window.click("settings-form-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    assert_eq!(ids(guard.path()), vec!["mgmt", "prod", "acme-ci"]);
}

#[gpui_kit::test]
async fn a_second_change_while_a_save_runs_is_refused(cx: &mut TestAppContext) {
    use freshkube_core::workspace::Role;
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        add(window, cx, "first", "");
        let page = view.read(cx).settings_page.clone();
        let second = page.update(cx, |page, cx| {
            page.upsert(None, Role::Cicd, "second", "", "", cx)
        });
        assert!(second.is_err(), "a save is running");
    })
    .unwrap();
    saved(cx, handle, &view).await;
    assert_eq!(ids(guard.path()), vec!["first"]);
}

#[gpui_kit::test]
async fn an_empty_context_names_the_field_and_a_too_large_file_says_so(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    // Compact this stays inside what a launch reads; written with
    // indentation it would not.
    let many: Vec<_> = (0..12_000).map(|n| serde_json::json!({"a": n})).collect();
    let big = serde_json::json!({"version": 1, "sources": many}).to_string();
    assert!(big.len() < workspace::MAX_BYTES as usize);
    write(guard.path(), &big);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-add", cx);
        window.render_frame(cx);
        window.click("settings-form-save", cx);
        window.render_frame(cx);
        let empty = window
            .find("settings-form-error")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(empty.contains("context"), "{empty}");
        window.click("settings-form-context", cx);
        window.input("acme-ci", cx);
        window.click("settings-form-save", cx);
        window.render_frame(cx);
        let large = window
            .find("settings-form-error")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(large.contains("256 KiB"), "{large}");
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(guard.path().join("workspace.json")).unwrap(),
        big
    );
}

#[gpui_kit::test]
async fn many_unknown_keys_are_capped_in_the_banner(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(
        guard.path(),
        r#"{"version":1,"a1":1,"a2":1,"a3":1,"a4":1,"a5":1,"a6":1,"a7":1,"clusters":[]}"#,
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, _, cx| {
        let warning = view
            .read(cx)
            .settings_page
            .read(cx)
            .warning
            .clone()
            .unwrap();
        assert!(warning.contains("a5"), "{warning}");
        assert!(!warning.contains("a6"), "{warning}");
        assert!(warning.contains("and 2 more"), "{warning}");
    })
    .unwrap();
}

#[gpui_kit::test]
async fn the_r_key_reloads_the_file_behind_the_page(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    write(
        guard.path(),
        &TWO.replace("\"id\":\"prod\"", "\"id\":\"prod-new\""),
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-cluster-prod-new").is_none());
        // A click on a row puts the keyboard on the list, where R is bound.
        window.click("settings-cluster-mgmt", cx);
        window.press("r", cx);
        window.render_frame(cx);
        assert!(window.find("settings-cluster-prod-new").visible());
        assert!(window.try_find("settings-cluster-prod").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_talos_context_is_saved_with_its_talosconfig_and_refused_without_one(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let talosconfig = guard.path().join("acme.talosconfig");
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-add", cx);
        window.render_frame(cx);
        window.click("settings-form-context", cx);
        window.input("acme-ci", cx);
        window.click("settings-form-talos-context", cx);
        window.input("acme-talos", cx);
        window.click("settings-form-save", cx);
        window.render_frame(cx);
        let text = window
            .find("settings-form-error")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert_eq!(text, "A Talos context needs a talosconfig");
        window.click("settings-form-talosconfig", cx);
        window.input(&talosconfig.display().to_string(), cx);
        window.click("settings-form-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let Loaded::Workspace(file) = workspace::load(&guard.path().join("workspace.json")) else {
        panic!("not saved");
    };
    assert_eq!(
        file.clusters[0].talos_context.as_deref(),
        Some("acme-talos")
    );
    assert_eq!(
        file.clusters[0].talosconfig.as_deref(),
        Some(talosconfig.as_path())
    );
    // The header offers the new cluster at once.
    cx.update_window(handle, |_, _, cx| {
        assert_eq!(view.read(cx).switcher.len(), 1);
    })
    .unwrap();
}
