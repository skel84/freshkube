use super::Origin;
use crate::GpuiOptions;
use crate::desktop::{
    Page, layout_check,
    tests::{fixture, mount},
};
use freshkube_core::workspace::{Invalid, Loaded};
use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};

const SETTINGS: layout_check::TablePage = layout_check::TablePage {
    page: "settings-page",
    title: "settings-title",
    title_text: "Settings",
    table: "settings-table-scroll",
    list: "settings-list",
};

fn open_settings(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    window.click("settings", cx);
    window.render_frame(cx);
    window.click("settings-open-workspace", cx);
    window.render_frame(cx);
}

#[gpui_kit::test]
fn the_gear_opens_settings_with_the_example_workspace(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_settings(window, cx);
        assert_eq!(view.read(cx).page, Page::Settings);
        // Six invented clusters.
        for id in [
            "core-fra",
            "cicd-fra",
            "dev-fra",
            "stage-fra",
            "prod-ams",
            "prod-fra",
        ] {
            assert!(
                window.find(format!("settings-cluster-{id}")).visible(),
                "{id}"
            );
        }
        let page = view.read(cx).settings_page.read(cx);
        assert_eq!(page.origin(), &Origin::Example);
    })
    .unwrap();
}

#[gpui_kit::test]
fn settings_is_a_table_page_at_every_text_size(cx: &mut TestAppContext) {
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
            layout_check::assert_table_page(window, cx, &SETTINGS);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn no_file_is_a_workspace_of_one(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.settings_page.update(cx, |page, cx| {
                page.set_workspace(&Loaded::Missing, false, cx)
            });
            pilot.navigate_from_keyboard(Page::Settings, window, cx);
        });
        window.render_frame(cx);
        assert!(window.find("settings-empty").visible());
        assert!(window.find("settings-workspace-note").visible());
        assert!(window.try_find("settings-banner").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_refused_file_says_why_and_lists_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.settings_page.update(cx, |page, cx| {
                page.set_workspace(&Loaded::Refused(Invalid::UnknownVersion(2)), false, cx)
            });
            pilot.navigate_from_keyboard(Page::Settings, window, cx);
        });
        window.render_frame(cx);
        assert!(window.find("settings-banner").visible());
        assert!(window.find("settings-empty").visible());
        let page = view.read(cx).settings_page.read(cx);
        assert!(matches!(page.origin(), Origin::Refused(why) if why.contains("version 2")));
    })
    .unwrap();
}

fn launch(
    cx: &mut TestAppContext,
    directory: &std::path::Path,
) -> (
    tokio::runtime::Runtime,
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<crate::desktop::Pilot>,
) {
    let options = GpuiOptions::new(Some(directory.join("missing")), None, 100)
        .with_preferences(Some(directory.join("preferences.json")));
    mount(cx, options, 1280., 820.)
}

#[gpui_kit::test]
fn a_launch_reads_the_workspace_file_beside_the_preferences(cx: &mut TestAppContext) {
    // Tokio wakes GPUI from its own threads.
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    std::fs::write(
        guard.path().join("workspace.json"),
        r#"{"version":1,"clusters":[
            {"id":"core-fra","role":"core","context":"core-fra"},
            {"id":"prod-fra","role":"environment","context":"prod-fra"}]}"#,
    )
    .unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.navigate_from_keyboard(Page::Settings, window, cx)
        });
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        assert_eq!(page.origin(), &Origin::File);
        assert!(window.find("settings-cluster-prod-fra").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_launch_without_a_file_writes_none(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    cx.update_window(handle, |_, _, cx| {
        let page = view.read(cx).settings_page.read(cx);
        assert_eq!(page.origin(), &Origin::Alone);
    })
    .unwrap();
    assert!(!guard.path().join("workspace.json").exists());
}

#[gpui_kit::test]
fn a_refused_file_stays_where_it_is_after_a_launch(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let file = guard.path().join("workspace.json");
    std::fs::write(&file, "{\"version\":9}").unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    cx.update_window(handle, |_, _, cx| {
        let page = view.read(cx).settings_page.read(cx);
        assert!(matches!(page.origin(), Origin::Refused(_)));
    })
    .unwrap();
    // Reading never moves or rewrites it; only a save would set it aside.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{\"version\":9}");
    assert!(!guard.path().join("workspace.json.bak").exists());
}
