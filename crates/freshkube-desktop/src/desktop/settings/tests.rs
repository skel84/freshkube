use super::{Origin, SettingsSection};
use crate::desktop::{
    Area, Page, layout_check,
    tests::{fixture, mount},
};
use crate::{DebugOptions, GpuiOptions};
use freshkube_core::workspace::{self, Invalid, Loaded};
use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};

const SETTINGS: layout_check::TablePage = layout_check::TablePage {
    page: "settings-page",
    title: "settings-title",
    title_text: "Clusters",
    table: "settings-table-scroll",
    list: "settings-list",
};

fn open_settings(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    window.click("settings", cx);
    window.render_frame(cx);
    window.click("settings-all", cx);
    window.render_frame(cx);
}

#[gpui_kit::test]
fn all_settings_opens_the_example_workspace(cx: &mut TestAppContext) {
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

/// Each list section is a table page, at both window sizes and the
/// smallest and largest text.
#[gpui_kit::test]
fn each_list_section_is_a_table_page_at_every_size(cx: &mut TestAppContext) {
    let pages = [
        (SettingsSection::Clusters, SETTINGS),
        (
            SettingsSection::Destinations,
            layout_check::TablePage {
                page: "settings-page",
                title: "settings-destinations-title",
                title_text: "Argo CD destinations",
                table: "settings-destinations-table-scroll",
                list: "settings-destinations-list",
            },
        ),
        (
            SettingsSection::Sources,
            layout_check::TablePage {
                page: "settings-page",
                title: "settings-sources-title",
                title_text: "Delivery sources",
                table: "settings-sources-table-scroll",
                list: "settings-sources-list",
            },
        ),
    ];
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        for size in [None, Some(20.)] {
            for (section, page) in &pages {
                cx.update_window(handle, |_, window, cx| {
                    if let Some(size) = size {
                        crate::text_size::set(size, cx);
                    }
                    view.update(cx, |pilot, cx| pilot.show_settings(*section, window, cx));
                    window.render_frame(cx);
                    layout_check::assert_table_page(window, cx, page);
                })
                .unwrap();
            }
        }
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

/// The sections in the column, App's then Workspace's, by their ids.
const SECTIONS: [(&str, SettingsSection, &str); 7] = [
    ("general", SettingsSection::General, "General"),
    ("connection", SettingsSection::Connection, "Connection"),
    ("monitoring", SettingsSection::Monitoring, "Monitoring"),
    ("coroot", SettingsSection::Coroot, "Coroot"),
    ("clusters", SettingsSection::Clusters, "Clusters"),
    (
        "destinations",
        SettingsSection::Destinations,
        "Argo CD destinations",
    ),
    ("sources", SettingsSection::Sources, "Delivery sources"),
];

/// The rail's Settings shows its column, App's sections then Workspace's,
/// and each row opens its section, the row current and its title shown.
#[gpui_kit::test]
fn the_rail_opens_settings_and_its_column_opens_each_section(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-settings", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).navigation.area, Area::Settings);
        assert_eq!(view.read(cx).navigation.page, Page::Settings);
        let mut tops = Vec::new();
        for id in [
            "app",
            "general",
            "coroot",
            "workspace",
            "clusters",
            "sources",
        ] {
            tops.push(window.find(format!("nav-settings-{id}")).bounds().top());
        }
        assert!(tops.is_sorted(), "{tops:?}");
        for (slug, section, title) in SECTIONS {
            window.click(format!("nav-settings-{slug}"), cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).settings_page.read(cx).section(), section);
            assert_eq!(view.read(cx).navigation.page, Page::Settings);
            // Clusters keeps the page's own prefix.
            let title_id = match section {
                SettingsSection::Clusters => "settings-title".to_owned(),
                _ => format!("settings-{slug}-title"),
            };
            // The title's text is the layout checks' to measure.
            assert!(window.find(title_id).visible(), "{slug}: {title}");
            assert!(window.find(format!("nav-settings-{slug}")).visible());
            assert_eq!(window.find("page-title").label(), Some("Settings"));
        }
    })
    .unwrap();
}

/// `FRESHKUBE_PAGE=settings-<section>` opens that section, and `settings`
/// Clusters.
#[gpui_kit::test]
fn each_section_opens_from_its_page_slug(cx: &mut TestAppContext) {
    let cases = SECTIONS
        .into_iter()
        .map(|(slug, section, _)| (format!("settings-{slug}"), section))
        .chain([("settings".to_owned(), SettingsSection::Clusters)]);
    for (page, section) in cases {
        let (_runtime, handle, view) = fixture(cx, 1280., 880.);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |pilot, cx| {
                pilot.startup_selection(&DebugOptions::default().with_page(&page), window, cx)
            });
            window.render_frame(cx);
            assert_eq!(view.read(cx).navigation.page, Page::Settings, "{page}");
            assert_eq!(view.read(cx).navigation.area, Area::Settings, "{page}");
            assert_eq!(
                view.read(cx).settings_page.read(cx).section(),
                section,
                "{page}"
            );
        })
        .unwrap();
    }
}

/// Command-comma and All settings… open the section shown last, not the
/// gear's menu.
#[gpui_kit::test]
fn open_settings_and_all_settings_open_the_section_shown_last(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Destinations, window, cx);
            pilot.navigate_from_keyboard(Page::Overview, window, cx);
        });
        window.render_frame(cx);
        window.dispatch_action(Box::new(freshkube_ui::platform::OpenSettings), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let pilot = view.read(cx);
        assert_eq!(pilot.navigation.page, Page::Settings);
        assert!(!pilot.navigation.settings_open);
        assert_eq!(
            pilot.settings_page.read(cx).section(),
            SettingsSection::Destinations
        );
        assert!(window.find("settings-destinations-list").visible());
        view.update(cx, |pilot, cx| {
            pilot.navigate_from_keyboard(Page::Overview, window, cx)
        });
        open_settings(window, cx);
        assert_eq!(view.read(cx).navigation.page, Page::Settings);
        assert!(!view.read(cx).navigation.settings_open);
        assert_eq!(
            view.read(cx).settings_page.read(cx).section(),
            SettingsSection::Destinations
        );
    })
    .unwrap();
}

/// A section whose controls aren't here yet says where they are, on a
/// padded page, and its button goes there: the gear's menu, or Coroot's
/// connection on Observability. It has no line in the status bar.
#[gpui_kit::test]
fn a_pointer_section_goes_where_its_controls_are(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    for (slug, section, title) in SECTIONS.into_iter().take(4) {
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |pilot, cx| pilot.show_settings(section, window, cx));
            window.render_frame(cx);
            layout_check::assert_page_frame(
                window,
                cx,
                &layout_check::PageFrame {
                    page: "settings-page",
                    title: Box::leak(format!("settings-{slug}-title").into_boxed_str()),
                    title_text: title,
                    content: Box::leak(format!("settings-{slug}-pointer").into_boxed_str()),
                },
            );
            assert!(window.try_find("settings-scope").is_none(), "{slug}");
            window.click(format!("settings-{slug}-open"), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let pilot = view.read(cx);
            if section == SettingsSection::Coroot {
                assert_eq!(pilot.navigation.page, Page::Observability);
            } else {
                assert_eq!(pilot.navigation.page, Page::Settings, "{slug}");
                assert!(pilot.navigation.settings_open, "{slug}");
            }
            view.update(cx, |pilot, _| pilot.navigation.settings_open = false);
        })
        .unwrap();
    }
}
