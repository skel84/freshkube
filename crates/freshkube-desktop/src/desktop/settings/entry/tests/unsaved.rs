use super::*;

#[gpui_kit::test]
async fn leaving_a_group_with_changes_asks_to_save_discard_or_stay(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "acme-prod-3");
    let selected = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, _, cx| {
            view.read(cx).settings_page.read(cx).selected.clone()
        })
        .unwrap()
    };
    // Stay keeps the selection and the changes.
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-cluster-mgmt", cx);
    })
    .unwrap();
    let (message, _) = cx.pending_prompt().expect("a prompt");
    assert!(message.contains("prod"), "{message}");
    cx.simulate_prompt_answer("Stay");
    cx.run_until_parked();
    assert_eq!(selected(cx).as_deref(), Some("prod"));
    assert_eq!(values(cx, handle, &view).0.context, "acme-prod-3");
    // Discard moves on and drops them.
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-cluster-mgmt", cx);
    })
    .unwrap();
    cx.simulate_prompt_answer("Discard");
    cx.run_until_parked();
    assert_eq!(selected(cx).as_deref(), Some("mgmt"));
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert_eq!(values(cx, handle, &view).0.context, "acme-mgmt");
    assert_eq!(
        entry(&read_workspace(guard.path()), "prod").context,
        "acme-prod"
    );
    // Save writes them, then moves on.
    set(cx, handle, &view, Field::Context, "acme-mgmt-2");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-cluster-prod", cx);
    })
    .unwrap();
    cx.simulate_prompt_answer("Save");
    cx.run_until_parked();
    saved(cx, handle, &view).await;
    assert_eq!(
        entry(&read_workspace(guard.path()), "mgmt").context,
        "acme-mgmt-2"
    );
    assert_eq!(selected(cx).as_deref(), Some("prod"));
}

#[gpui_kit::test]
async fn a_save_for_the_prompt_that_fails_a_field_stays_on_the_entry(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-cluster-mgmt", cx);
    })
    .unwrap();
    cx.simulate_prompt_answer("Save");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        assert_eq!(page.selected.as_deref(), Some("prod"));
        assert!(window.find("settings-identity-context-row-error").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn unsaved_changes_hold_an_outside_edit_behind_reload(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "typed");
    cx.update_window(handle, |_, _, cx| {
        assert!(view.read(cx).settings_page.read(cx).has_unsaved_edits(cx));
    })
    .unwrap();
    // Saving the group leaves nothing unsaved.
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-identity-save", cx)
    })
    .unwrap();
    saved(cx, handle, &view).await;
    cx.update_window(handle, |_, _, cx| {
        assert!(!view.read(cx).settings_page.read(cx).has_unsaved_edits(cx));
    })
    .unwrap();
}

#[gpui_kit::test]
async fn add_with_unsaved_changes_asks_first_and_keeps_them_when_saved(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "acme-prod-4");
    let dialog = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, _| {
            window.try_find("settings-cluster-form").is_some()
        })
        .unwrap()
    };
    // Stay: no dialog, and the changes are still there.
    cx.update_window(handle, |_, window, cx| window.click("settings-add", cx))
        .unwrap();
    let (message, _) = cx.pending_prompt().expect("a prompt");
    assert!(message.contains("prod"), "{message}");
    cx.simulate_prompt_answer("Stay");
    cx.run_until_parked();
    assert!(!dialog(cx));
    assert_eq!(values(cx, handle, &view).0.context, "acme-prod-4");
    // Discard: the dialog opens, and the changes are gone.
    cx.update_window(handle, |_, window, cx| window.click("settings-add", cx))
        .unwrap();
    cx.simulate_prompt_answer("Discard");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(dialog(cx));
    assert_eq!(values(cx, handle, &view).0.context, "acme-prod");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-form-cancel", cx)
    })
    .unwrap();
    // Save: the changes are written, then the dialog opens; adding there
    // keeps the saved change.
    set(cx, handle, &view, Field::Context, "acme-prod-4");
    cx.update_window(handle, |_, window, cx| window.click("settings-add", cx))
        .unwrap();
    cx.simulate_prompt_answer("Save");
    cx.run_until_parked();
    saved(cx, handle, &view).await;
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(dialog(cx));
    assert_eq!(
        entry(&read_workspace(guard.path()), "prod").context,
        "acme-prod-4"
    );
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-form-context", cx);
        window.input("acme-next", cx);
        window.click("settings-form-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let written = read_workspace(guard.path());
    assert_eq!(entry(&written, "prod").context, "acme-prod-4");
    assert_eq!(written.clusters.len(), 3);
}

#[gpui_kit::test]
async fn reload_keeps_unsaved_changes_unless_the_file_changed_under_them(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "typed");
    cx.update_window(handle, |_, _, cx| {
        view.read(cx)
            .settings_page
            .clone()
            .update(cx, |page, cx| page.reload(cx));
    })
    .unwrap();
    saved(cx, handle, &view).await;
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).settings_page.read(cx).has_unsaved_edits(cx));
    })
    .unwrap();
    assert_eq!(values(cx, handle, &view).0.context, "typed");
    // Reloading to take a change made on disk is what drops them.
    cx.update_window(handle, |_, _, cx| {
        view.read(cx).settings_page.clone().update(cx, |page, cx| {
            page.pending_setup = true;
            page.reload(cx)
        });
    })
    .unwrap();
    saved(cx, handle, &view).await;
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!view.read(cx).settings_page.read(cx).has_unsaved_edits(cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(values(cx, handle, &view).0.context, "acme-prod");
}

#[gpui_kit::test]
async fn the_row_menu_offers_nothing_for_a_row_the_prompt_has_not_moved_to(
    cx: &mut TestAppContext,
) {
    use freshkube_ui::menu::MenuAction;
    use freshkube_ui::table::TableSource as _;
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    let menu = |cx: &mut TestAppContext, key: &'static str| {
        cx.update_window(handle, |_, window, cx| {
            view.read(cx).settings_page.clone().update(cx, |page, cx| {
                page.row_menu(&key.into(), window, cx)
                    .iter()
                    .filter_map(|action| match action {
                        MenuAction::Item { label, enabled, .. } => {
                            Some((label.to_string(), *enabled))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
        })
        .unwrap()
    };
    // Nothing is typed, so the menu follows the row it was opened on.
    let clean = menu(cx, "mgmt");
    assert!(
        clean
            .iter()
            .any(|(label, enabled)| label == "Remove" && *enabled)
    );
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "typed");
    let pending = menu(cx, "mgmt");
    assert!(!pending.is_empty());
    assert!(pending.iter().all(|(_, enabled)| !enabled), "{pending:?}");
    cx.simulate_prompt_answer("Stay");
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        assert_eq!(
            view.read(cx).settings_page.read(cx).selected.as_deref(),
            Some("prod")
        );
    })
    .unwrap();
    // The selected row's own menu still works.
    let own = menu(cx, "prod");
    assert!(
        own.iter()
            .any(|(label, enabled)| label == "Remove" && *enabled)
    );
}

#[gpui_kit::test]
async fn removing_the_selected_cluster_drops_its_unsaved_changes_without_a_prompt(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "typed");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-remove", cx);
        window.render_frame(cx);
        window.click("settings-remove-confirm", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    cx.run_until_parked();
    assert!(cx.pending_prompt().is_none());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        assert!(!page.has_unsaved_edits(cx));
        assert_eq!(page.selected, None);
        assert!(window.try_find("settings-detail").is_none());
    })
    .unwrap();
    let ids: Vec<_> = read_workspace(guard.path())
        .clusters
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(ids, vec!["mgmt"]);
}
