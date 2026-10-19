//! The destinations list in the real app: mapping, changing and removing a
//! destination, saved to the file, and the list beside the clusters.
use super::super::SettingsSection;
use super::super::edit_tests::{open_section, saved, write};
use super::super::tests::launch;
use crate::desktop::{layout_check, tests::fixture};
use freshkube_core::workspace::{self, Destination, Key, Loaded};
use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};
use std::path::Path;

const PAGE: layout_check::TablePage = layout_check::TablePage {
    page: "settings-page",
    title: "settings-destinations-title",
    title_text: "Argo CD destinations",
    table: "settings-destinations-table-scroll",
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
fn the_example_lists_its_destinations_on_a_table_page(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            view.update(cx, |pilot, cx| {
                pilot.show_settings(SettingsSection::Destinations, window, cx)
            });
            window.render_frame(cx);
            layout_check::assert_table_page(window, cx, &PAGE);
            for row in 0..3 {
                assert!(window.find(format!("settings-destination-{row}")).visible());
            }
            // The section holds one table: neither other list is drawn.
            assert!(window.try_find("settings-list").is_none());
            assert!(window.try_find("settings-sources-list").is_none());
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
fn the_glyph_column_stays_at_the_table_edge_and_contains_every_mark(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |pilot, cx| {
                pilot.show_settings(SettingsSection::Destinations, window, cx)
            });
            layout_check::assert_table_page(window, cx, &PAGE);
            let table = window.find(PAGE.table).bounds();
            let glyph_width = crate::ui::dp_px(freshkube_ui::table::GLYPH_WIDTH, window);
            let header = gpui_kit::base::test_support::snapshots(window)
                .into_iter()
                .find(|element| element.label() == Some("Destination"))
                .expect("Destination header")
                .bounds();
            assert!((header.left() - table.left() - glyph_width).abs() <= gpui_kit::px(1.));
            let destinations = view.read(cx).settings_page.read(cx).destinations.clone();
            for missing in [false, true] {
                if missing {
                    // Long destinations must leave every missing-cluster mark in its column.
                    destinations.update(cx, |list, cx| {
                        for row in &mut list.rows {
                            row.destination = "example-destination-with-a-long-name".into();
                            row.listed = false;
                        }
                        cx.notify();
                    });
                    window.render_frame(cx);
                }
                for row in 0..3 {
                    let glyph = window
                        .find(format!("settings-destination-{row}-glyph"))
                        .bounds();
                    assert!(
                        (glyph.left() - table.left() - gpui_kit::px(1.)).abs() <= gpui_kit::px(1.),
                        "glyph column moved at {width}×{height}: {glyph:?} in {table:?}"
                    );
                    assert!((glyph.size.width - glyph_width).abs() <= gpui_kit::px(1.));
                    let cell = window
                        .find(format!("settings-destination-{row}-destination"))
                        .bounds();
                    assert!((cell.left() - glyph.right()).abs() <= gpui_kit::px(1.));
                    let mark = window.try_find(format!("settings-destination-{row}-mark"));
                    if missing {
                        let mark = mark.expect("missing cluster has a mark");
                        assert!(mark.visible());
                        let mark = mark.bounds();
                        assert!(mark.left() >= glyph.left() && mark.right() <= glyph.right());
                        assert!((mark.center().x - glyph.center().x).abs() <= gpui_kit::px(1.));
                    } else {
                        assert!(mark.is_none());
                    }
                }
            }
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
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
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
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
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
        assert_eq!(
            list.selected(),
            Some(&Key::Server("https://prod.example.test".into()))
        );
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
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
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
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
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
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    for (by, value, why) in [
        (
            1usize,
            "prod-lon",
            "Argo CD's cluster prod-lon is already mapped to prod",
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
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
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
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
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

fn list<'a>(
    view: &gpui_kit::Entity<crate::desktop::Pilot>,
    cx: &'a gpui_kit::App,
) -> &'a super::Destinations {
    view.read(cx).settings_page.read(cx).destinations.read(cx)
}

fn name(name: &str) -> Key {
    Key::Name(name.into())
}

/// Each list's arrows stay in it, now that each is a section of its own:
/// Down at the last cluster and Up at the first mapping keep the row, and
/// neither list takes the other's selection. Escape clears this list's
/// selection before it lets go.
#[gpui_kit::test]
fn the_arrows_stay_in_their_list(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    let clusters = |cx: &gpui_kit::App| view.read(cx).settings_page.read(cx).selected.clone();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Clusters, window, cx)
        });
        window.render_frame(cx);
        window.click("settings-cluster-prod-ams", cx);
        window.press("down", cx);
        assert_eq!(clusters(cx).as_deref(), Some("prod-fra"));
        window.press("down", cx);
        assert_eq!(clusters(cx).as_deref(), Some("prod-fra"));
        assert!(list(&view, cx).selected().is_none());
        view.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Destinations, window, cx)
        });
        window.render_frame(cx);
        assert!(list(&view, cx).is_focused(window));
        window.press("down", cx);
        let first = list(&view, cx).rows[0].key.clone();
        assert_eq!(list(&view, cx).selected(), Some(&first));
        window.press("up", cx);
        assert_eq!(list(&view, cx).selected(), Some(&first));
        assert!(list(&view, cx).is_focused(window));
        assert_eq!(clusters(cx).as_deref(), Some("prod-fra"));
        window.press("escape", cx);
        assert!(list(&view, cx).selected().is_none());
        assert_eq!(clusters(cx).as_deref(), Some("prod-fra"));
    })
    .unwrap();
}

/// Map, Edit and Remove give the keyboard back to this list when their
/// dialog closes, even when the toolbar opened them.
#[gpui_kit::test]
async fn the_dialogs_hand_the_keyboard_back_to_this_list(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), ONE);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destinations-map", cx);
        window.render_frame(cx);
        window.click("settings-destination-cancel", cx);
        window.render_frame(cx);
        assert!(list(&view, cx).is_focused(window));
        window.click("settings-destination-0", cx);
        window.press("backspace", cx);
        window.render_frame(cx);
        window.click("settings-destination-remove-cancel", cx);
        window.render_frame(cx);
        assert!(list(&view, cx).is_focused(window));
        window.press("e", cx);
        window.render_frame(cx);
        assert!(window.find("settings-destination-form").visible());
    })
    .unwrap();
}

/// A save refused because the file changed on disk keeps the form and what
/// was typed, selects nothing new, and leaves the hand edit alone.
#[gpui_kit::test]
async fn a_file_changed_on_disk_refuses_a_mapping(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), ONE);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    let edited = ONE.replace("acme-prod", "acme-prod-2");
    write(guard.path(), &edited);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destination-0", cx);
        window.press("a", cx);
        window.render_frame(cx);
        window.click("settings-destination-value", cx);
        window.input("https://prod.example.test", cx);
        window.click("settings-destination-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let why = window
            .find("settings-destination-error")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(why.contains("changed on disk"), "{why}");
        assert_eq!(list(&view, cx).selected(), Some(&name("prod-lon")));
        assert_eq!(list(&view, cx).rows.len(), 1);
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(guard.path().join("workspace.json")).unwrap(),
        edited
    );
}

const TWO: &str = r#"{"version":1,"clusters":[
    {"id":"mgmt","role":"core","context":"acme-mgmt"},
    {"id":"prod","role":"environment","context":"acme-prod"}],
  "destinations":[
    {"server":"https://a.example.test","entry":"mgmt"},
    {"name":"prod-lon","entry":"prod"}]}"#;

/// An Edit that changes only the cluster keeps what the row matches, its
/// place, and its selection.
#[gpui_kit::test]
async fn an_edit_can_change_only_the_cluster(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destination-0", cx);
        window.press("e", cx);
        window.render_frame(cx);
        let form = list(&view, cx).form().expect("the form");
        form.update(cx, |form, cx| form.pick("prod", window, cx));
        window.click("settings-destination-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    assert_eq!(
        destinations(guard.path()),
        vec![
            server("https://a.example.test", "prod"),
            Destination::new(name("prod-lon"), "prod")
        ]
    );
    cx.update_window(handle, |_, _, cx| {
        assert_eq!(
            list(&view, cx).selected(),
            Some(&Key::Server("https://a.example.test".into()))
        );
    })
    .unwrap();
}

/// A server is a duplicate however it is written, as Argo CD matches it; a
/// duplicate whose cluster is gone says so.
#[gpui_kit::test]
async fn a_duplicate_is_found_as_argo_cd_matches(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(
        guard.path(),
        r#"{"version":1,"clusters":[
            {"id":"prod","role":"environment","context":"acme-prod"}],
          "destinations":[
            {"server":"https://a.example.test","entry":"prod"},
            {"server":"https://b.example.test","entry":"old"}]}"#,
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    for (value, why) in [
        (
            "https://A.example.test:443/",
            "Server https://A.example.test:443/ is already mapped to prod",
        ),
        (
            "https://b.example.test/",
            "Server https://b.example.test/ is already mapped to old, which the workspace no \
             longer lists: edit that mapping",
        ),
    ] {
        cx.update_window(handle, |_, window, cx| {
            window.click("settings-destinations-map", cx);
            window.render_frame(cx);
            window.click("settings-destination-value", cx);
            window.input(value, cx);
            window.click("settings-destination-save", cx);
            window.render_frame(cx);
            assert_eq!(window.find("settings-destination-error").label(), Some(why));
            window.click("settings-destination-cancel", cx);
            window.render_frame(cx);
        })
        .unwrap();
    }
}

/// Removing the selected mapping clears the selection rather than passing
/// it to the next row; Reload finds the selected mapping wherever it moved.
#[gpui_kit::test]
async fn the_selection_follows_the_mapping_not_its_place(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    // Reordered by hand: Reload keeps prod-lon selected, now first.
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-destination-1", cx);
        write(
            guard.path(),
            r#"{"version":1,"clusters":[
                {"id":"mgmt","role":"core","context":"acme-mgmt"},
                {"id":"prod","role":"environment","context":"acme-prod"}],
              "destinations":[
                {"name":"prod-lon","entry":"prod"},
                {"server":"https://a.example.test","entry":"mgmt"}]}"#,
        );
        view.update(cx, |pilot, cx| {
            pilot.settings_page.update(cx, |page, cx| page.reload(cx))
        });
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(list(&view, cx).selected(), Some(&name("prod-lon")));
        assert_eq!(list(&view, cx).selected_row().map(|(at, _)| at), Some(0));
        window.render_frame(cx);
        window.press("backspace", cx);
        window.render_frame(cx);
        window.click("settings-destination-remove-confirm", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    cx.update_window(handle, |_, _, cx| {
        assert_eq!(list(&view, cx).rows.len(), 1);
        assert!(list(&view, cx).selected().is_none());
    })
    .unwrap();
}

/// Editing a mapping whose cluster is gone names it in the picker, and the
/// status bar counts only mappings to listed clusters.
#[gpui_kit::test]
async fn a_gone_cluster_is_named_and_not_counted(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(
        guard.path(),
        r#"{"version":1,"clusters":[
            {"id":"prod","role":"environment","context":"acme-prod"},
            {"id":"stage","role":"environment","context":"acme-stage"}],
          "destinations":[
            {"server":"https://prod.example.test","entry":"prod"},
            {"server":"https://old.example.test","entry":"old"}]}"#,
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    cx.update_window(handle, |_, window, cx| {
        let scope = window
            .find("settings-scope")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(scope.contains("1 destination mapped"), "{scope}");
        window.click("settings-destination-1", cx);
        window.press("e", cx);
        window.render_frame(cx);
        let form = list(&view, cx).form().expect("the form");
        assert_eq!(
            form.read(cx).gone(),
            Some("old is gone, no longer in the workspace: pick a cluster")
        );
    })
    .unwrap();
}

/// With no mapping to a listed cluster the status bar says nothing of them,
/// and without a cluster Map opens nothing.
#[gpui_kit::test]
async fn without_a_cluster_there_is_nothing_to_map(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(
        guard.path(),
        r#"{"version":1,"clusters":[],
          "destinations":[{"name":"prod-lon","entry":"old"}]}"#,
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Destinations).await;
    cx.update_window(handle, |_, window, cx| {
        let scope = window
            .find("settings-scope")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(!scope.contains("destination"), "{scope}");
        window.click("settings-destinations-map", cx);
        window.click("settings-destination-0", cx);
        for key in ["a", "e"] {
            window.press(key, cx);
        }
        window.render_frame(cx);
        assert!(window.try_find("settings-destination-form").is_none());
        assert!(!list(&view, cx).mappable());
    })
    .unwrap();
}

/// In a short window a list section scrolls its frame, and its list keeps
/// `SHORT_LIST_HEIGHT`, with twenty mappings as with three.
#[gpui_kit::test]
fn a_short_window_keeps_each_list_usable(cx: &mut TestAppContext) {
    use freshkube_ui::page::SHORT_LIST_HEIGHT;
    use gpui_kit::px;
    let (_runtime, handle, view) = fixture(cx, 760., 480.);
    for (section, table) in [
        (SettingsSection::Clusters, "settings-table-scroll"),
        (
            SettingsSection::Destinations,
            "settings-destinations-table-scroll",
        ),
    ] {
        for many in [false, true] {
            cx.update_window(handle, |_, window, cx| {
                crate::text_size::set(20., cx);
                let mut example = workspace::example();
                if many {
                    example.destinations = (0..20)
                        .map(|n| Destination::new(name(&format!("shop-api-{n}")), "dev-fra"))
                        .collect();
                }
                view.update(cx, |pilot, cx| {
                    pilot.settings_page.update(cx, |page, cx| {
                        page.set_workspace(&Loaded::Workspace(example), true, cx)
                    });
                    pilot.show_settings(section, window, cx)
                });
                crate::desktop::tests::settle_header(window, cx);
                assert!(freshkube_ui::page::is_short(window));
                let least = crate::ui::dp_px(SHORT_LIST_HEIGHT, window) - px(2.);
                let bounds = window.find(table).bounds();
                assert!(bounds.size.height >= least, "{table} is {bounds:?}");
            })
            .unwrap();
        }
    }
}
