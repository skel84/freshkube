use super::Origin;
use crate::GpuiOptions;
use crate::desktop::{
    Page, layout_check,
    tests::{fixture, mount},
};
use freshkube_core::workspace::{self, Invalid, Loaded};
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
        assert_eq!(view.read(cx).navigation.page, Page::Settings);
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
        // The header's location reads Settings, and the line is in the bar.
        assert_eq!(window.find("page-title").label(), Some("Settings"));
        let scope = window.find("settings-scope");
        assert!(scope.label().unwrap_or_default().contains("6 clusters"));
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
        // The implicit workspace of one counts as a cluster, so the bar and
        // the empty table agree.
        let scope = window.find("settings-scope");
        assert!(
            scope
                .path()
                .contains(&gpui_kit::ElementId::from("status-bar"))
        );
        assert!(scope.label().unwrap_or_default().contains("1 cluster"));
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

pub(crate) fn launch(
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
    assert!(!guard.path().join("freshkube.yaml").exists());
    std::fs::write(guard.path().join("coroot.json"),
        r#"{"url":"https://remembered.example.test","auth":"anonymous","reconnect":false,"remember":false}"#).unwrap();
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
        assert_eq!(
            view.read(cx).observability.read(cx).coroot_url(cx),
            "https://remembered.example.test"
        );
    })
    .unwrap();
}

const SETUP: &str = "version: 1\nclusters:\n- {id: cluster-a, role: core, context: cluster-a}\n- {id: cluster-b, role: cicd, context: cluster-b}\nsources: {kargo: cluster-b}\ncoroot: {server: 'https://observations.example.test', sign_in: anonymous}\n";

fn poll_setup(cx: &mut TestAppContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn setup_reloads_settings_the_switcher_delivery_and_coroot(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("freshkube.yaml");
    std::fs::write(&path, SETUP).unwrap();
    std::fs::write(
        directory.path().join("workspace.json"),
        r#"{"version":1,"clusters":[]}"#,
    )
    .unwrap();
    let (_runtime, handle, view) = launch(cx, directory.path());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.navigate_from_keyboard(Page::Settings, window, cx)
        });
        window.render_frame(cx);
        assert!(window.find("settings-cluster-cluster-a").visible());
        assert!(
            window
                .find("settings-scope")
                .label()
                .unwrap()
                .contains("freshkube.yaml")
        );
        assert!(
            window
                .find("settings-banner")
                .label()
                .unwrap()
                .contains(super::HAND_EDITED)
        );
        assert_eq!(
            view.read(cx).observability.read(cx).coroot_url(cx),
            "https://observations.example.test"
        );
        window.click("settings-add", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings-form-context").is_none());
        let page = view.read(cx).settings_page.clone();
        page.update(cx, |page, cx| {
            assert!(!page.editable());
            page.commit(workspace::Workspace::default(), "Changed".into(), cx);
        });
    })
    .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), SETUP);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("workspace.json")).unwrap(),
        r#"{"version":1,"clusters":[]}"#
    );

    let changed = SETUP
        .replace("cluster-a", "cluster-c")
        .replace("observations.example.test", "reports.example.test");
    let replacement = directory.path().join("editor.tmp");
    std::fs::write(&replacement, &changed).unwrap();
    std::fs::rename(replacement, &path).unwrap();
    poll_setup(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let pilot = view.read(cx);
        assert_eq!(pilot.switcher[0].id.as_ref(), "cluster-c");
        assert!(window.find("settings-cluster-cluster-c").visible());
        assert!(
            window
                .find("setup-reload-status")
                .label()
                .unwrap()
                .starts_with("Reloaded freshkube.yaml · ")
        );
        assert_eq!(
            pilot.observability.read(cx).coroot_url(cx),
            "https://reports.example.test"
        );
        let workspace = pilot.settings_page.read(cx).workspace();
        assert_eq!(
            workspace.sources.pin(workspace::Part::Kargo),
            Some("cluster-b")
        );
    })
    .unwrap();

    std::fs::write(&path, "version: [").unwrap();
    poll_setup(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let banner = window.find("settings-banner");
        let words = banner.label().unwrap();
        assert!(
            words.contains("freshkube.yaml isn't used: invalid YAML at line"),
            "{words}"
        );
        assert!(words.contains("The last good setup stays in use."));
        assert!(window.find("settings-cluster-cluster-c").visible());
    })
    .unwrap();
    std::fs::write(&path, SETUP).unwrap();
    poll_setup(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-cluster-cluster-a").visible());
        assert!(
            !window
                .find("settings-banner")
                .label()
                .unwrap()
                .contains("invalid YAML")
        );
    })
    .unwrap();
    std::fs::remove_file(&path).unwrap();
    poll_setup(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("settings-banner")
                .label()
                .unwrap()
                .contains("is gone")
        );
        assert!(window.find("settings-cluster-cluster-a").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_broken_setup_at_launch_warns_and_keeps_the_no_file_behavior(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("freshkube.yaml"), "version: [").unwrap();
    let (_runtime, handle, view) = launch(cx, directory.path());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.navigate_from_keyboard(Page::Settings, window, cx)
        });
        window.render_frame(cx);
        assert!(window.find("settings-empty").visible());
        assert!(
            window
                .find("settings-banner")
                .label()
                .unwrap()
                .contains("invalid YAML")
        );
        assert!(view.read(cx).setup_file.read(cx).setup.is_none());
        assert!(!view.read(cx).settings_page.read(cx).editable());
    })
    .unwrap();
    assert!(!directory.path().join("workspace.json").exists());
}

#[gpui_kit::test]
fn setup_waits_for_reload_while_a_settings_form_has_a_draft(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let (_runtime, handle, view) = launch(cx, directory.path());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.navigate_from_keyboard(Page::Settings, window, cx)
        });
        window.render_frame(cx);
        window.click("settings-add", cx);
        window.render_frame(cx);
        window.click("settings-form-context", cx);
        window.input("draft-cluster", cx);
    })
    .unwrap();
    std::fs::write(directory.path().join("freshkube.yaml"), SETUP).unwrap();
    poll_setup(cx);
    cx.update_window(handle, |_, window, cx| {
        let page = view.read(cx).settings_page.clone();
        assert!(page.read(cx).pending_setup);
        assert!(page.read(cx).workspace().clusters.is_empty());
        window.click("settings-form-save", cx);
        window.render_frame(cx);
        assert!(
            window
                .find("settings-form-error")
                .label()
                .unwrap()
                .contains("changed on disk")
        );
        window.press("escape", cx);
        window.render_frame(cx);
        window.click("settings-reload", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!view.read(cx).settings_page.read(cx).pending_setup);
        assert!(window.find("settings-cluster-cluster-a").visible());
        assert_eq!(
            view.read(cx).observability.read(cx).coroot_url(cx),
            "https://observations.example.test"
        );
    })
    .unwrap();
    assert!(!directory.path().join("workspace.json").exists());
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

#[gpui_kit::test]
fn a_row_tooltip_holds_the_full_talosconfig_path(cx: &mut TestAppContext) {
    use freshkube_ui::table::{Line, TableSource};
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    let path = std::env::temp_dir()
        .join("acme")
        .join("core-fra.talosconfig");
    let loaded = Loaded::Workspace(
        workspace::parse(
            serde_json::json!({
                "version": 1,
                "clusters": [
                    {"id": "core-fra", "role": "core", "context": "core-fra", "talosconfig": path},
                    {"id": "dev-fra", "role": "environment", "context": "dev-fra"},
                ],
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap(),
    );
    cx.update_window(handle, |_, _, cx| {
        let page = view.read(cx).settings_page.clone();
        page.update(cx, |page, cx| page.set_workspace(&loaded, false, cx));
        let page = page.read(cx);
        let tooltip = |ix| match page.line(ix, cx) {
            Some(Line::Row(row)) => row.tooltip.map(|t| t.to_string()),
            _ => None,
        };
        assert!(tooltip(0).unwrap().contains(&path.display().to_string()));
        assert!(!tooltip(1).unwrap().contains("Talosconfig"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_gear_panels_dividers_keep_their_pixel_in_a_short_window(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings", cx);
        window.render_frame(cx);
        // The panel scrolls in a short window; a divider that can shrink
        // would draw as nothing there.
        let mut found = 0;
        for ix in 1..=7usize {
            let Some(divider) = window.try_find(("settings-divider", ix)) else {
                continue;
            };
            found += 1;
            assert!(
                divider.bounds().size.height > gpui_kit::px(0.),
                "divider {ix}: {:?}",
                divider.bounds()
            );
        }
        assert!(found >= 5, "found {found} dividers");
    })
    .unwrap();
}

/// A `destinations` row for Argo CD's own cluster refuses the file, and
/// Settings › Workspace names the row and why, over the workspace of one.
#[gpui_kit::test]
fn a_destination_for_argo_cds_own_cluster_says_why_the_file_isnt_used(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    std::fs::write(
        guard.path().join("workspace.json"),
        r#"{"version":1,"clusters":[
            {"id":"core-fra","role":"core","context":"core-fra"}],
            "destinations":[{"server":"https://kubernetes.default.svc","entry":"core-fra"}]}"#,
    )
    .unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.navigate_from_keyboard(Page::Settings, window, cx)
        });
        window.render_frame(cx);
        assert!(matches!(
            view.read(cx).settings_page.read(cx).origin(),
            Origin::Refused(_)
        ));
        let banner = window.find("settings-banner");
        assert!(banner.visible());
        let words = banner.label().unwrap().to_owned();
        assert!(
            words.contains(
                "workspace.json isn’t used: destinations[0] (server https://kubernetes.default.svc) \
                 names Argo CD's own cluster, which is known already"
            ),
            "{words}"
        );
    })
    .unwrap();
}
