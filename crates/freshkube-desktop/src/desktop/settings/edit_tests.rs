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

/// Waits for the save in flight to land on the page.
async fn saved(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    view: &gpui_kit::Entity<crate::desktop::Pilot>,
) {
    let view = view.clone();
    cx.wait_for(handle, Duration::from_secs(5), move |_, cx| {
        let page = view.read(cx).settings_page.read(cx);
        page.notice.is_some()
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
        assert_eq!(window.find("settings-form-id").label(), None);
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
        window.press("alt-up", cx);
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
        window.press("alt-up", cx);
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
        assert!(matches!(page.notice, Some(Notice::Failed(_))));
        assert!(window.find("settings-save-failed").visible());
        assert!(window.try_find("settings-cluster-acme-ci").is_none());
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
        for key in ["a", "e", "backspace", "alt-up"] {
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
