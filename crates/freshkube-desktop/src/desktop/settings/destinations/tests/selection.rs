use super::*;

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
    saved(cx, handle, &view).await;
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

/// Every entry point reaches the same mapping, edit and removal flow. In
/// particular a row menu selects its row before it dispatches an action.
#[gpui_kit::test]
async fn toolbar_keys_and_row_menu_run_the_same_actions(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    for route in ["toolbar", "keys", "menu"] {
        let guard = tempfile::tempdir().unwrap();
        write(guard.path(), TWO);
        let (_runtime, handle, view) = launch(cx, guard.path());
        open_section(cx, handle, &view, SettingsSection::Destinations).await;
        for (action, key, menu_index, form) in [
            ("map", "a", 0usize, true),
            ("edit", "e", 1, true),
            ("remove", "backspace", 2, false),
        ] {
            cx.update_window(handle, |_, window, cx| {
                window.click("settings-destination-1", cx);
                if route == "menu" {
                    window.right_click("settings-destination-0", cx);
                } else {
                    window.click("settings-destination-0", cx);
                    if route == "toolbar" {
                        window.click(format!("settings-destinations-{action}"), cx);
                    } else {
                        window.press(key, cx);
                    }
                }
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                if route == "menu" {
                    window.render_frame(cx);
                    assert_eq!(list(&view, cx).selected_row().map(|(at, _)| at), Some(0));
                    let menu = window.within("popup-menu");
                    for (ix, label) in ["Map", "Edit", "Remove"].iter().enumerate() {
                        assert!(menu.find(ix).label().unwrap_or_default().starts_with(label));
                    }
                    window.within("popup-menu").click(menu_index, cx);
                }
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                if form {
                    assert!(window.find("settings-destination-form").visible());
                    window.click("settings-destination-value", cx);
                    window.press("secondary-a", cx);
                    window.input(
                        if action == "map" {
                            "https://new.example.test"
                        } else {
                            "https://edited.example.test"
                        },
                        cx,
                    );
                    let form = list(&view, cx).form().unwrap();
                    form.update(cx, |form, cx| form.pick("prod", window, cx));
                    window.click("settings-destination-save", cx);
                } else {
                    assert!(window.find("settings-destination-remove-dialog").visible());
                    window.click("settings-destination-remove-confirm", cx);
                }
            })
            .unwrap();
            saved(cx, handle, &view).await;
            let rows = destinations(guard.path());
            match action {
                "map" => {
                    assert_eq!(rows.len(), 3, "{route}");
                    assert_eq!(rows[2], server("https://new.example.test", "prod"));
                }
                "edit" => assert_eq!(rows[0], server("https://edited.example.test", "prod")),
                _ => {
                    assert_eq!(rows.len(), 2, "{route}");
                    assert!(
                        rows.iter()
                            .all(|row| row.key()
                                != Key::Server("https://edited.example.test".into()))
                    );
                }
            }
        }
    }
}
