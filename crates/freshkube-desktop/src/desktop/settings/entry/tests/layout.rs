use super::*;

#[gpui_kit::test]
async fn example_data_shows_the_groups_and_changes_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = crate::desktop::tests::fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.show_settings(
                crate::desktop::settings::SettingsSection::Clusters,
                window,
                cx,
            )
        });
        window.render_frame(cx);
        window.click("settings-cluster-core-fra", cx);
        window.render_frame(cx);
        assert!(window.find("settings-detail").visible());
        assert!(window.find("settings-detail-note").visible());
        view.read(cx).settings_page.clone().update(cx, |page, cx| {
            let input = page
                .entry
                .as_ref()
                .unwrap()
                .input(Field::Context)
                .unwrap()
                .clone();
            input.update(cx, |input, cx| input.set_value("typed", window, cx));
        });
        window.render_frame(cx);
        window.click("settings-identity-save", cx);
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        assert!(page.saving.is_none());
        assert_eq!(page.workspace().clusters[0].context, "core-fra");
    })
    .unwrap();
    // Choosing Save for changes the page can't save says why, and stays.
    cx.update_window(handle, |_, window, cx| {
        view.read(cx)
            .settings_page
            .clone()
            .update(cx, |page, cx| page.entry_edited(cx));
        window.click("settings-cluster-cicd-fra", cx);
    })
    .unwrap();
    cx.simulate_prompt_answer("Save");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        assert_eq!(page.selected.as_deref(), Some("core-fra"));
        assert!(matches!(
            page.notice,
            Some(crate::desktop::settings::Notice::Failed(_))
        ));
        assert!(window.find("settings-save-failed").visible());
        let _ = Group::Identity;
    })
    .unwrap();
}

#[gpui_kit::test]
fn locked_fields_take_no_choices_either(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = crate::desktop::tests::fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.show_settings(
                crate::desktop::settings::SettingsSection::Clusters,
                window,
                cx,
            )
        });
        window.render_frame(cx);
        window.click("settings-cluster-core-fra", cx);
        window.render_frame(cx);
    })
    .unwrap();
    // Example data is read-only: the segments don't mark the group.
    cx.update_window(handle, |_, window, cx| {
        window.click(("settings-identity-role", 1usize), cx);
    })
    .unwrap();
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-source-service", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        assert!(!page.has_unsaved_edits(cx));
        assert!(window.try_find("settings-identity-dirty").is_none());
        assert!(window.try_find("settings-metrics-dirty").is_none());
        // And a change of row asks nothing.
        window.click("settings-cluster-cicd-fra", cx);
    })
    .unwrap();
    assert!(cx.pending_prompt().is_none());
}

#[gpui_kit::test]
async fn the_body_scrolls_below_the_heading_and_opens_at_the_top(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // A scrolled line is cut at the gap under the heading, not shown in it.
        let heading = window.find("settings-detail-heading").bounds();
        let body = window.find("settings-detail-body").bounds();
        let gap = crate::ui::dp_px(freshkube_ui::page::PANE_PADDING, window);
        assert!(
            body.top() - heading.bottom() >= gap - gpui_kit::px(1.),
            "{heading:?} over {body:?}"
        );
    })
    .unwrap();
    to_metrics(cx, handle);
    let scrolled = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, _, cx| {
            view.read(cx)
                .settings_page
                .read(cx)
                .detail_scroll
                .offset()
                .y
        })
        .unwrap()
    };
    assert!(scrolled(cx) < gpui_kit::px(0.), "the body scrolled");
    // Another cluster opens at the top.
    select(cx, handle, "mgmt");
    assert_eq!(scrolled(cx), gpui_kit::px(0.));
}

#[gpui_kit::test]
async fn an_error_under_a_field_moves_nothing_below_it(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    select(cx, handle, "prod");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-source-service", cx);
        window.render_frame(cx);
    })
    .unwrap();
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-port", cx);
        window.input("70000", cx);
    })
    .unwrap();
    cx.run_until_parked();
    to_metrics(cx, handle);
    let save = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, _| {
            window.find("settings-metrics-save").bounds().top()
        })
        .unwrap()
    };
    let before = save(cx);
    // The port loses the keyboard, and its error appears under it: a
    // button below it must not move between a press and its release.
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-namespace", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-metrics-port-row-error").visible());
    })
    .unwrap();
    assert_eq!(save(cx), before);
}

/// The table page is the shared one with nothing selected; with a cluster
/// selected the Inspector sits beside the table or under it as the shared
/// one does, at both window sizes and the smallest and largest text.
#[gpui_kit::test]
fn clusters_is_a_table_page_with_an_inspector_at_every_size(cx: &mut TestAppContext) {
    use crate::desktop::tests::fixture;
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        for size in [13., 20.] {
            cx.update_window(handle, |_, window, cx| {
                crate::text_size::set(size, cx);
                view.update(cx, |pilot, cx| {
                    pilot.show_settings(
                        crate::desktop::settings::SettingsSection::Clusters,
                        window,
                        cx,
                    )
                });
                window.render_frame(cx);
                layout_check::assert_table_page(
                    window,
                    cx,
                    &layout_check::TablePage {
                        page: "settings-page",
                        title: "settings-title",
                        title_text: "Clusters",
                        table: "settings-table-scroll",
                        list: "settings-list",
                    },
                );
                window.click("settings-cluster-core-fra", cx);
                window.render_frame(cx);
                layout_check::assert_inspector(
                    window,
                    cx,
                    "settings-split",
                    "settings-table",
                    "settings-detail",
                    "settings-detail-title",
                );
                let beside =
                    crate::screens::page_width(window) >= freshkube_ui::inspector::SPLIT_WIDTH;
                let table = window.find("settings-table").bounds();
                let detail = window.find("settings-detail").bounds();
                assert_eq!(beside, detail.left() >= table.right() - gpui_kit::px(1.));
                // Back to nothing selected for the next size.
                view.read(cx).settings_page.clone().update(cx, |page, cx| {
                    page.selected = None;
                    cx.notify();
                });
            })
            .unwrap();
        }
    }
}
