use super::*;

#[gpui_kit::test]
fn selection_explains_roles_and_pins_in_the_shared_inspector(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open(window, cx, &view);
        window.click("settings-source-kargo", cx);
        window.render_frame(cx);
        assert_eq!(list(&view, cx).read(cx).selected, Some(KARGO));
        assert!(window.find("settings-source-choice").visible());
        assert!(
            list(&view, cx).read(cx).rows[0]
                .explanation
                .contains("first entry with the CI/CD role")
        );
        assert!(window.find("settings-source-reads").visible());
        let mut workspace = view.read(cx).settings_page.read(cx).workspace().clone();
        workspace.sources.set(Part::Kargo, Some("core-fra".into()));
        set_workspace(&view, workspace, true, cx);
        window.render_frame(cx);
        assert!(
            list(&view, cx).read(cx).rows[0]
                .explanation
                .contains("core-fra, pinned in freshkube.yaml")
        );
        assert_eq!(
            window.find("settings-source-inspector-title").label(),
            Some("Kargo")
        );
        // Example mode refuses both visible controls and dispatched actions.
        window.click("settings-source-follow-roles", cx);
        list(&view, cx).update(cx, |list, cx| list.pin(Some("cicd-fra".into()), cx));
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings-source-inspector").is_none());
        window.press("down", cx);
        window.render_frame(cx);
        assert!(window.find("settings-source-inspector").visible());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            view.read(cx)
                .settings_page
                .read(cx)
                .workspace()
                .sources
                .pin(Part::Kargo),
            Some("core-fra")
        );
    });
}

#[gpui_kit::test]
fn the_table_keeps_reads_in_its_room_beside_the_inspector(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open(window, cx, &view);
        window.click("settings-source-kargo", cx);
        settle_sources(window, cx);
        let sources = list(&view, cx);
        let split = sources.read(cx).split.beside_state().clone();
        for inspector_width in [460., 620.] {
            split.update(cx, |split, cx| {
                split.resize_panel(1, crate::ui::dp_px(inspector_width, window), window, cx)
            });
            for problem in [
                None,
                Some(
                    "freshkube.yaml isn't used: invalid YAML. The last good setup stays in use."
                        .to_owned(),
                ),
            ] {
                sources.update(cx, |list, cx| list.set_setup_problem(problem, cx));
                settle_sources(window, cx);
                let table = window.find(PAGE.table).bounds();
                let content = crate::ui::dp_px(sources.read(cx).width, window);
                assert!(
                    content <= table.size.width + gpui_kit::px(1.),
                    "{content:?} of content in {table:?}"
                );
                for read in sources.read(cx).rows.iter().map(|row| row.read) {
                    let id = format!("settings-source-{}-reads", read.key());
                    let cell = window.find(format!("{id}-cell"));
                    assert!(cell.visible());
                    assert!(
                        cell.bounds().right() <= table.right() + gpui_kit::px(1.),
                        "{read:?}: {:?} outside {table:?}",
                        cell.bounds()
                    );
                    let shown = freshkube_ui::table::word_cut_style(id).unwrap();
                    assert!(shown.text.ends_with('…'), "{}", shown.text);
                }
                window.scroll(
                    PAGE.table,
                    gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                        gpui_kit::px(-400.),
                        gpui_kit::px(0.),
                    )),
                    cx,
                );
                settle_sources(window, cx);
                let cell = window.find("settings-source-builds-reads-cell").bounds();
                assert!(cell.right() <= table.right() + gpui_kit::px(1.));
                assert!(
                    (cell.right() - table.right()).abs() <= gpui_kit::px(2.),
                    "a sideways wheel moved Reads: {cell:?} in {table:?}"
                );
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_first_right_click_keeps_its_menu_when_the_inspector_opens(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            open(window, cx, &view);
            assert!(window.try_find("settings-source-inspector").is_none());
            if width == 760. {
                window.blur(cx);
            }
            window.right_click("settings-source-kargo", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            settle_sources(window, cx);
            assert_eq!(list(&view, cx).read(cx).selected, Some(KARGO));
            assert!(window.find("popup-menu").visible());
            assert!(
                window
                    .within("popup-menu")
                    .find(0usize)
                    .label()
                    .unwrap()
                    .starts_with("Check sources")
            );
            window.within("popup-menu").click(0usize, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            settle_sources(window, cx);
            assert_eq!(state(&view, KARGO, cx), "Readable");
            assert!(window.try_find("popup-menu").is_none());
            assert!(window.find("settings-source-inspector").visible());
            window.right_click("settings-source-builds", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            settle_sources(window, cx);
            assert_eq!(list(&view, cx).read(cx).selected, Some(BUILDS));
            assert!(window.find("popup-menu").visible());
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("popup-menu").is_none());
            assert!(window.find("settings-source-inspector").visible());
        })
        .unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.press("escape", cx);
            settle_sources(window, cx);
            assert!(window.try_find("settings-source-inspector").is_none());
            window.right_click("settings-source-argocd", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            settle_sources(window, cx);
            assert!(window.find("popup-menu").visible());
            assert!(window.try_find("settings-source-inspector").is_none());
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            settle_sources(window, cx);
            assert!(window.try_find("popup-menu").is_none());
            assert_eq!(
                window.find("settings-source-inspector-title").label(),
                Some("Argo CD")
            );
        })
        .unwrap();
    }
}
