use super::*;

#[gpui_kit::test]
fn a_workspace_of_one_reads_the_open_cluster(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.settings_page.update(cx, |page, cx| {
                page.set_workspace(&Loaded::Missing, false, cx)
            })
        });
        open(window, cx, &view);
        assert!(
            states(&view, cx)
                .iter()
                .all(|(_, entry, state)| entry == "The open cluster" && state == "Not checked")
        );
        // Example data is open, so its example answers.
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert!(
            states(&view, cx)
                .iter()
                .all(|(_, _, state)| state == "Readable")
        );
        // In a sentence, the open cluster isn't a name.
        let kargo = tooltip(&view, KARGO, cx);
        assert!(
            kargo.contains("Kargo answered on the open cluster"),
            "{kargo}"
        );

        // Another open connection forgets what the check found.
        let sources = list(&view, cx);
        sources.update(cx, |list, cx| list.set_source(None, cx));
        assert!(
            states(&view, cx)
                .iter()
                .all(|(_, _, state)| state == "Not checked")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_new_workspace_forgets_the_check(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open(window, cx, &view);
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert_eq!(state(&view, KARGO, cx), "Readable");
        set_workspace(&view, workspace::example(), true, cx);
        window.render_frame(cx);
        assert_eq!(state(&view, KARGO, cx), "Not checked");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_stage_naming_of_the_setup_is_a_row_of_its_own_and_goes_when_removed(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open(window, cx, &view);
        settle_sources(window, cx);
        let sources = list(&view, cx);
        let named = |cx: &gpui_kit::App| {
            sources
                .read(cx)
                .rows
                .iter()
                .find(|row| row.read == Read::StageNaming)
                .map(|row| (row.entry.to_string(), row.state.to_string()))
        };
        assert_eq!(named(cx), None);
        let mut naming = freshkube_core::setup::StageNaming::new("{stage}-{project}");
        sources.update(cx, |list, cx| {
            list.set_stage_naming(Some(naming.clone()), cx)
        });
        settle_sources(window, cx);
        assert_eq!(
            named(cx),
            Some(("{stage}-{project}".into(), "By name only".into()))
        );
        assert!(window.find("settings-source-stage-naming").visible());
        naming.project_key = Some("example.test/project".into());
        sources.update(cx, |list, cx| list.set_stage_naming(Some(naming), cx));
        assert_eq!(
            named(cx).map(|(_, state)| state).as_deref(),
            Some("By project key")
        );
        sources.update(cx, |list, cx| list.set_stage_naming(None, cx));
        assert_eq!(named(cx), None);
    })
    .unwrap();
}
