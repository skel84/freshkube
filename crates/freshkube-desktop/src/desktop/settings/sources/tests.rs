//! The delivery sources list in the real app: where each part is read,
//! and what Check sources finds, in each state.
use super::super::edit_tests::{open_settings, write};
use super::super::tests::launch;
use crate::desktop::{Page, layout_check, tests::fixture};
use freshkube_core::delivery::entry::Reach;
use freshkube_core::workspace::{self, Entry, Loaded, Part, Role, Workspace};
use gpui_kit::{
    AppContext, TestAppContext,
    test::{TestAppContextExt, TestWindowExt},
};
use std::time::Duration;

const FRAME: layout_check::PageFrame = layout_check::PageFrame {
    page: "settings-page",
    title: "settings-sources-title",
    title_text: "Delivery sources",
    content: "settings-sources-table-scroll",
};

const TABLE: layout_check::Table = layout_check::Table {
    table: Some("settings-sources-table-scroll"),
    list: "settings-sources-list",
};

type View = gpui_kit::Entity<crate::desktop::Pilot>;

fn states(view: &View, cx: &gpui_kit::App) -> Vec<(Part, String, String)> {
    let sources = view.read(cx).settings_page.read(cx).sources.read(cx);
    sources
        .states()
        .into_iter()
        .map(|(part, entry, state)| (part, entry.to_owned(), state.to_owned()))
        .collect()
}

fn row(part: Part, entry: &str, state: &str) -> (Part, String, String) {
    (part, entry.to_owned(), state.to_owned())
}

fn open(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, view: &View) {
    view.update(cx, |pilot, cx| {
        pilot.navigate_from_keyboard(Page::Settings, window, cx)
    });
    window.render_frame(cx);
}

fn set_workspace(view: &View, workspace: Workspace, example: bool, cx: &mut gpui_kit::App) {
    view.update(cx, |pilot, cx| {
        pilot.settings_page.update(cx, |page, cx| {
            page.set_workspace(&Loaded::Workspace(workspace), example, cx)
        })
    });
}

#[gpui_kit::test]
fn the_example_lists_each_part_under_the_destinations_and_reads_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            open(window, cx, &view);
            // The page scrolls to its third list.
            window.scroll(
                "settings-page",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                    gpui_kit::px(0.),
                    gpui_kit::px(-5000.),
                )),
                cx,
            );
            window.render_frame(cx);
            layout_check::assert_edge_frame(window, cx, &FRAME);
            let rows = layout_check::assert_table(window, cx, &TABLE);
            assert!(rows.header.is_some(), "{rows:#?}");
            for part in ["kargo", "builds", "argocd"] {
                assert!(window.find(format!("settings-source-{part}")).visible());
            }
            let below = window.find("settings-sources-title").bounds().top();
            let above = window.find("settings-destinations-list").bounds();
            assert!(above.bottom() <= below, "{above:?} {below:?}");
            assert_eq!(
                states(&view, cx),
                vec![
                    row(Part::Kargo, "cicd-fra", "Not checked"),
                    row(Part::Builds, "cicd-fra", "Not checked"),
                    row(Part::ArgoCd, "core-fra", "Not checked"),
                ]
            );
            let sources = view.read(cx).settings_page.read(cx).sources.read(cx);
            assert!(!sources.is_checking());
            // Nothing is checked, so no glyph says anything yet.
            assert!(window.try_find("settings-source-kargo-mark").is_none());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn check_sources_finds_each_state(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open(window, cx, &view);
        let sources = view.read(cx).settings_page.read(cx).sources.clone();
        sources.update(cx, |list, _| {
            list.set_example_answer("cicd-fra", Part::Kargo, Reach::Refused("forbidden".into()));
            list.set_example_answer(
                "cicd-fra",
                Part::Builds,
                Reach::Unreachable("no answer within 5 s".into()),
            );
            list.set_example_answer(
                "core-fra",
                Part::ArgoCd,
                Reach::NotServed("not served".into()),
            );
        });
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert_eq!(
            states(&view, cx),
            vec![
                row(Part::Kargo, "cicd-fra", "Refused"),
                row(Part::Builds, "cicd-fra", "Unreachable"),
                row(Part::ArgoCd, "core-fra", "Not served"),
            ]
        );
        for part in ["kargo", "builds", "argocd"] {
            assert!(
                window
                    .find(format!("settings-source-{part}-mark"))
                    .visible(),
                "{part}"
            );
        }
        assert_eq!(
            window.find("settings-source-kargo-mark").label(),
            Some("Refused")
        );

        // Checked again, every part answers.
        sources.update(cx, |list, _| list.example_answers.clear());
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert!(
            states(&view, cx)
                .iter()
                .all(|(_, _, state)| state == "Readable")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_part_with_no_entry_says_so_before_any_check(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        // No core entry, and Builds pinned to an entry that is gone.
        let mut lone = Workspace::new(vec![
            Entry::new("cluster-b", Role::Cicd, "cluster-b"),
            Entry::new("cluster-c", Role::Environment, "cluster-c"),
        ]);
        lone.sources.set(Part::Builds, Some("cluster-x".into()));
        set_workspace(&view, lone, true, cx);
        open(window, cx, &view);
        assert_eq!(
            states(&view, cx),
            vec![
                row(Part::Kargo, "cluster-b", "Not checked"),
                row(Part::Builds, "cluster-x", "Not in the workspace"),
                row(Part::ArgoCd, "None", "Not in the workspace"),
            ]
        );
        assert!(window.find("settings-source-argocd-mark").visible());
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert_eq!(
            states(&view, cx),
            vec![
                row(Part::Kargo, "cluster-b", "Readable"),
                row(Part::Builds, "cluster-x", "Not in the workspace"),
                row(Part::ArgoCd, "None", "Not in the workspace"),
            ]
        );
        let tooltip = view.read(cx).settings_page.read(cx).sources.read(cx).rows[2]
            .tooltip
            .clone();
        assert!(
            tooltip.contains("No Core entry in the workspace"),
            "{tooltip}"
        );
    })
    .unwrap();
}

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
        assert_eq!(states(&view, cx)[0].2, "Readable");
        set_workspace(&view, workspace::example(), true, cx);
        window.render_frame(cx);
        assert_eq!(states(&view, cx)[0].2, "Not checked");
    })
    .unwrap();
}

#[gpui_kit::test]
fn c_checks_from_the_list(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open(window, cx, &view);
        window.click("settings-source-kargo", cx);
        window.press("c", cx);
        window.render_frame(cx);
        assert_eq!(states(&view, cx)[0].2, "Readable");
    })
    .unwrap();
}

/// An entry the kubeconfig doesn't define is unreachable, and says why; the
/// check reads only the workspace's own kubeconfig, which here is missing.
#[gpui_kit::test]
async fn a_live_entry_missing_from_the_kubeconfig_is_unreachable(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let kubeconfig = guard.path().join("kubeconfig");
    write(
        guard.path(),
        &serde_json::json!({
            "version": 1,
            "kubeconfig": kubeconfig,
            "clusters": [
                {"id": "cluster-a", "role": "core", "context": "acme-a"},
                {"id": "cluster-b", "role": "cicd", "context": "acme-b"},
            ],
        })
        .to_string(),
    );
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-sources-check", cx);
        window.render_frame(cx);
        assert!(
            states(&view, cx)
                .iter()
                .all(|(_, _, state)| state == "Checking…")
        );
    })
    .unwrap();
    let waiting = view.clone();
    cx.wait_for(handle, Duration::from_secs(10), move |_, cx| {
        !waiting
            .read(cx)
            .settings_page
            .read(cx)
            .sources
            .read(cx)
            .is_checking()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            states(&view, cx),
            vec![
                row(Part::Kargo, "cluster-b", "Unreachable"),
                row(Part::Builds, "cluster-b", "Unreachable"),
                row(Part::ArgoCd, "cluster-a", "Unreachable"),
            ]
        );
        let sources = view.read(cx).settings_page.read(cx).sources.read(cx);
        assert!(
            sources.rows[0]
                .tooltip
                .contains("Couldn't reach cluster-b: the kubeconfig has no context acme-b"),
            "{}",
            sources.rows[0].tooltip
        );
    })
    .unwrap();
}
