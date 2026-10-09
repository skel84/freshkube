//! The destinations list in the real app: mapping, changing and removing a
//! destination, saved to the file, and the list beside the clusters.
use super::super::edit_tests::{open_settings, saved, write};
use super::super::tests::launch;
use crate::desktop::{Page, layout_check, tests::fixture};
use freshkube_core::workspace::{self, Destination, Key, Loaded};
use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};
use std::path::Path;

const FRAME: layout_check::PageFrame = layout_check::PageFrame {
    page: "settings-page",
    title: "settings-destinations-title",
    title_text: "Argo CD destinations",
    content: "settings-destinations-table-scroll",
};

const TABLE: layout_check::Table = layout_check::Table {
    table: Some("settings-destinations-table-scroll"),
    list: "settings-destinations-list",
};

/// One cluster, so the form picks it; and a mapping by name.
const ONE: &str = r#"{"version":1,"clusters":[
    {"id":"prod","role":"environment","context":"acme-prod"}],
  "destinations":[{"name":"prod-lon","entry":"prod","note":"kept"}]}"#;

fn destinations(directory: &Path) -> Vec<Destination> {
    match workspace::load(&directory.join("workspace.json")) {
        Loaded::Workspace(workspace) => workspace.destinations,
        other => panic!("{other:?}"),
    }
}

fn server(server: &str, entry: &str) -> Destination {
    Destination::new(Key::Server(server.into()), entry)
}

#[gpui_kit::test]
fn the_example_lists_its_destinations_under_the_clusters(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            view.update(cx, |pilot, cx| {
                pilot.navigate_from_keyboard(Page::Settings, window, cx)
            });
            window.render_frame(cx);
            layout_check::assert_edge_frame(window, cx, &FRAME);
            let rows = layout_check::assert_table(window, cx, &TABLE);
            assert!(rows.header.is_some(), "{rows:#?}");
            for row in 0..3 {
                assert!(window.find(format!("settings-destination-{row}")).visible());
            }
            let below = window.find("settings-destinations-title").bounds().top();
            assert!(window.find("settings-list").bounds().bottom() <= below);
            assert!(
                window
                    .find("settings-scope")
                    .label()
                    .unwrap_or_default()
                    .contains("3 destinations mapped")
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
async fn example_destinations_are_never_changed(cx: &mut TestAppContext) {
    use crate::GpuiOptions;
    use crate::desktop::tests::mount;
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let options =
        GpuiOptions::fixture().with_preferences(Some(guard.path().join("preferences.json")));
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destination-0", cx);
        for key in ["a", "e", "enter", "backspace"] {
            window.press(key, cx);
        }
        window.click("settings-destinations-map", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings-destination-form").is_none());
        assert!(
            window
                .try_find("settings-destination-remove-dialog")
                .is_none()
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read_dir(guard.path()).unwrap().count(), 0);
}

#[gpui_kit::test]
async fn mapping_a_destination_saves_it_and_selects_it(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), ONE);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destination-0", cx);
        window.press("a", cx);
        window.render_frame(cx);
        assert!(window.find("settings-destination-form").visible());
        window.click("settings-destination-value", cx);
        window.input("https://prod.example.test", cx);
        window.click("settings-destination-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let rows = destinations(guard.path());
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1], server("https://prod.example.test", "prod"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-destination-form").is_none());
        assert!(window.find("settings-destination-1").visible());
        let list = view.read(cx).settings_page.read(cx).destinations.read(cx);
        assert_eq!(list.selected, Some(1));
        assert!(window.find("settings-saved").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn changing_a_mapping_keeps_its_place_and_unknown_keys(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), ONE);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destination-0", cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(window.find("settings-destination-form").visible());
        // From a name to a server: select what was there, type over it.
        window.click(("settings-destination-by", 0usize), cx);
        window.click("settings-destination-value", cx);
        window.press("secondary-a", cx);
        window.input("https://lon.example.test", cx);
        window.click("settings-destination-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let text = std::fs::read_to_string(guard.path().join("workspace.json")).unwrap();
    let file: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        file["destinations"],
        serde_json::json!([{"server": "https://lon.example.test", "entry": "prod", "note": "kept"}])
    );
}

#[gpui_kit::test]
async fn removing_a_mapping_asks_first(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), ONE);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destination-0", cx);
        window.press("backspace", cx);
        window.render_frame(cx);
        assert!(window.find("settings-destination-remove-dialog").visible());
        window.click("settings-destination-remove-cancel", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(destinations(guard.path()).len(), 1);
    cx.update_window(handle, |_, window, cx| {
        window.press("backspace", cx);
        window.render_frame(cx);
        window.click("settings-destination-remove-confirm", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    assert!(destinations(guard.path()).is_empty());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-destinations-empty").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn the_form_refuses_what_wouldnt_map_and_saves_nothing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), ONE);
    let before = std::fs::read(guard.path().join("workspace.json")).unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    for (by, value, why) in [
        (
            1usize,
            "prod-lon",
            "Name prod-lon is already mapped to prod",
        ),
        (
            1,
            "in-cluster",
            "Argo CD knows its own cluster already; it needs no mapping",
        ),
        (
            0,
            "prod.example.test",
            "The server must be an http or https address",
        ),
        (0, "", "Enter the server URL the Application names"),
    ] {
        cx.update_window(handle, |_, window, cx| {
            window.click("settings-destinations-map", cx);
            window.render_frame(cx);
            window.click(("settings-destination-by", by), cx);
            window.click("settings-destination-value", cx);
            if !value.is_empty() {
                window.input(value, cx);
            }
            window.click("settings-destination-save", cx);
            window.render_frame(cx);
            assert_eq!(window.find("settings-destination-error").label(), Some(why));
            window.click("settings-destination-cancel", cx);
            window.render_frame(cx);
        })
        .unwrap();
    }
    cx.run_until_parked();
    assert_eq!(
        std::fs::read(guard.path().join("workspace.json")).unwrap(),
        before
    );
}

#[gpui_kit::test]
async fn with_two_clusters_the_form_asks_which(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(
        guard.path(),
        r#"{"version":1,"clusters":[
            {"id":"mgmt","role":"core","context":"acme-mgmt"},
            {"id":"prod","role":"environment","context":"acme-prod"}]}"#,
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("settings-destinations-empty").visible());
        window.click("settings-destinations-map", cx);
        window.render_frame(cx);
        window.click("settings-destination-value", cx);
        window.input("https://prod.example.test", cx);
        window.click("settings-destination-save", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("settings-destination-error").label(),
            Some("Pick a workspace cluster")
        );
    })
    .unwrap();
}

/// A mapping whose cluster the workspace no longer lists is kept, marked
/// with the Unknown ring, and says why.
#[gpui_kit::test]
async fn a_mapping_to_a_cluster_no_longer_listed_is_marked(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(
        guard.path(),
        r#"{"version":1,"clusters":[
            {"id":"prod","role":"environment","context":"acme-prod"}],
          "destinations":[
            {"server":"https://prod.example.test","entry":"prod"},
            {"server":"https://old.example.test","entry":"old"}]}"#,
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        assert!(window.try_find("settings-destination-0-mark").is_none());
        let mark = window.find("settings-destination-1-mark");
        assert_eq!(mark.label(), Some("old isn’t in the workspace"));
        let list = view.read(cx).settings_page.read(cx).destinations.read(cx);
        assert_eq!(
            list.rows().collect::<Vec<_>>(),
            vec![
                ("https://prod.example.test", "prod", true),
                ("https://old.example.test", "old", false)
            ]
        );
    })
    .unwrap();
}
