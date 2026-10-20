//! Navigation through the actual column, rail and keyboard. All files and
//! connection targets are invented and live under a temporary directory.
use super::{SettingsSection, edit_tests::write};
use crate::desktop::Page;
use freshkube_core::setup;
use freshkube_ui::real_io::RealIo;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, test::TestWindowExt};

const SETUP: &str = "version: 1\nclusters: []\nmetrics: {mode: automatic, store: {url: 'http://127.0.0.1:1'}}\ncoroot: {server: 'http://127.0.0.1:1', sign_in: anonymous}\n";
type Pilot = Entity<crate::desktop::Pilot>;

#[derive(Clone, Copy, Debug)]
enum Route {
    Column,
    Rail,
    Shortcut,
    ResourcesShortcut,
}

fn launch(
    cx: &mut TestAppContext,
    directory: &std::path::Path,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Pilot) {
    let options = crate::GpuiOptions::new(Some(directory.join("missing")), None, 100)
        .with_preferences(Some(directory.join("preferences.json")));
    crate::desktop::testing::mount(cx, options, 1280., 1600.)
}

fn open(cx: &mut TestAppContext, handle: AnyWindowHandle, pilot: &Pilot, section: SettingsSection) {
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        pilot.update(cx, |pilot, cx| pilot.show_settings(section, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn leave(cx: &mut TestAppContext, handle: AnyWindowHandle, route: Route) {
    cx.update_window(handle, |_, window, cx| match route {
        Route::Column => window.click("nav-settings-general", cx),
        Route::Rail => window.click("nav-overview", cx),
        Route::Shortcut => window.press("secondary-1", cx),
        Route::ResourcesShortcut => window.press("secondary-3", cx),
    })
    .unwrap();
    cx.run_until_parked();
}

fn edit(cx: &mut TestAppContext, handle: AnyWindowHandle, section: SettingsSection, value: &str) {
    cx.update_window(handle, |_, window, cx| {
        let input = match section {
            SettingsSection::Monitoring => "metrics-label",
            SettingsSection::Coroot => "coroot-project",
            _ => unreachable!(),
        };
        if section == SettingsSection::Monitoring {
            window.click("metrics-central", cx);
        }
        window.click(input, cx);
        window.press("secondary-a", cx);
        window.input(value, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn marker(cx: &mut TestAppContext, handle: AnyWindowHandle, section: SettingsSection, dirty: bool) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let marker = window.try_find(format!("nav-settings-{}-dirty", section.slug()));
        assert_eq!(marker.is_some(), dirty, "{section:?}");
        if let Some(marker) = marker {
            assert_eq!(marker.label(), Some("Unsaved changes"));
        }
    })
    .unwrap();
}

fn stayed(cx: &TestAppContext, pilot: &Pilot, section: SettingsSection) {
    cx.read(|cx| {
        assert_eq!(pilot.read(cx).navigation.page, Page::Settings);
        assert_eq!(pilot.read(cx).settings_page.read(cx).section(), section);
    });
}

fn arrived(cx: &TestAppContext, pilot: &Pilot, route: Route) {
    cx.read(|cx| {
        let pilot = pilot.read(cx);
        match route {
            Route::Column => {
                assert_eq!(pilot.navigation.page, Page::Settings);
                assert_eq!(
                    pilot.settings_page.read(cx).section(),
                    SettingsSection::General
                );
            }
            Route::Rail | Route::Shortcut => assert_eq!(pilot.navigation.page, Page::Overview),
            Route::ResourcesShortcut => {
                assert_eq!(pilot.navigation.page, Page::Resources);
                assert_eq!(pilot.navigation.resource_kind.key(), "namespaces");
            }
        }
    });
}

async fn choices(cx: &mut TestAppContext, section: SettingsSection) {
    cx.executor().allow_parking();
    for route in [
        Route::Column,
        Route::Rail,
        Route::Shortcut,
        Route::ResourcesShortcut,
    ] {
        for choice in ["stay", "discard", "save"] {
            let directory = tempfile::tempdir().unwrap();
            write(directory.path(), SETUP);
            let (_runtime, handle, pilot) = launch(cx, directory.path());
            open(cx, handle, &pilot, section);
            marker(cx, handle, section, false);
            edit(cx, handle, section, "cluster_example");
            marker(cx, handle, section, true);
            leave(cx, handle, route);
            stayed(cx, &pilot, section);
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(window.find("settings-unsaved-dialog").visible());
                if matches!(route, Route::Shortcut) && choice == "save" {
                    window.press("enter", cx);
                } else {
                    window.click(format!("settings-unsaved-{choice}"), cx);
                }
            })
            .unwrap();
            cx.run_until_parked();
            if choice == "save" {
                cx.wait_for_io(handle, |_, cx| {
                    !pilot
                        .read(cx)
                        .settings_page
                        .read(cx)
                        .unsaved_save_in_progress()
                })
                .await;
                cx.run_until_parked();
            }
            let saved =
                setup::parse(&std::fs::read(directory.path().join("freshkube.yaml")).unwrap())
                    .unwrap();
            let changed = match section {
                SettingsSection::Monitoring => saved.metrics.mode == setup::Mode::Central,
                SettingsSection::Coroot => {
                    saved.coroot.unwrap().project.as_deref() == Some("cluster_example")
                }
                _ => unreachable!(),
            };
            assert_eq!(changed, choice == "save", "{section:?} {route:?} {choice}");
            if choice == "stay" {
                stayed(cx, &pilot, section);
                marker(cx, handle, section, true);
                // Escape is also Stay; it must leave the next attempt usable.
                leave(cx, handle, route);
                cx.update_window(handle, |_, window, cx| window.press("escape", cx))
                    .unwrap();
                cx.run_until_parked();
                stayed(cx, &pilot, section);
                leave(cx, handle, route);
                cx.update_window(handle, |_, window, cx| {
                    window.click("settings-unsaved-discard", cx)
                })
                .unwrap();
                cx.run_until_parked();
            }
            arrived(cx, &pilot, route);
            open(cx, handle, &pilot, section);
            marker(cx, handle, section, false);
            cx.update_window(handle, |_, window, _| {
                let input = if section == SettingsSection::Monitoring {
                    "metrics-label"
                } else {
                    "coroot-project"
                };
                if section == SettingsSection::Coroot || choice == "save" {
                    assert_eq!(
                        window.find(input).value(),
                        Some(if choice == "save" {
                            "cluster_example"
                        } else {
                            ""
                        })
                    );
                }
            })
            .unwrap();
            leave(cx, handle, route);
            arrived(cx, &pilot, route);
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find("settings-unsaved-dialog").is_none());
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
async fn monitoring_save_discard_stay_and_clean_navigation(cx: &mut TestAppContext) {
    choices(cx, SettingsSection::Monitoring).await;
}

#[gpui_kit::test]
async fn coroot_save_discard_stay_and_clean_navigation(cx: &mut TestAppContext) {
    choices(cx, SettingsSection::Coroot).await;
}

#[gpui_kit::test]
async fn invalid_save_keeps_the_section_and_its_draft(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    for section in [SettingsSection::Monitoring, SettingsSection::Coroot] {
        let directory = tempfile::tempdir().unwrap();
        write(directory.path(), SETUP);
        let (_runtime, handle, pilot) = launch(cx, directory.path());
        open(cx, handle, &pilot, section);
        edit(cx, handle, section, "example");
        cx.update_window(handle, |_, window, cx| {
            let input = if section == SettingsSection::Monitoring {
                "metrics-url"
            } else {
                "coroot-server"
            };
            if section == SettingsSection::Monitoring {
                window.click("metrics-store-url", cx);
            }
            window.click(input, cx);
            window.press("secondary-a", cx);
            window.input("invalid", cx);
        })
        .unwrap();
        cx.run_until_parked();
        leave(cx, handle, Route::Column);
        cx.update_window(handle, |_, window, cx| {
            window.click("settings-unsaved-save", cx)
        })
        .unwrap();
        cx.run_until_parked();
        stayed(cx, &pilot, section);
        marker(cx, handle, section, true);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("freshkube.yaml")).unwrap(),
            SETUP
        );
        leave(cx, handle, Route::Rail);
        cx.update_window(handle, |_, window, cx| {
            window.click("settings-unsaved-discard", cx)
        })
        .unwrap();
        cx.run_until_parked();
        arrived(cx, &pilot, Route::Rail);
    }
}

#[gpui_kit::test]
fn general_saves_immediately_and_never_prompts(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = crate::desktop::testing::fixture(cx, 1280., 880.);
    open(cx, handle, &pilot, SettingsSection::General);
    cx.update_window(handle, |_, window, cx| {
        window.click("general-appearance-dark", cx)
    })
    .unwrap();
    cx.run_until_parked();
    marker(cx, handle, SettingsSection::General, false);
    leave(cx, handle, Route::Rail);
    arrived(cx, &pilot, Route::Rail);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-unsaved-dialog").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_refused_write_keeps_the_draft_and_can_be_discarded(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    for section in [SettingsSection::Monitoring, SettingsSection::Coroot] {
        let directory = tempfile::tempdir().unwrap();
        write(directory.path(), SETUP);
        let (_runtime, handle, pilot) = launch(cx, directory.path());
        open(cx, handle, &pilot, section);
        edit(cx, handle, section, "cluster_example");
        leave(cx, handle, Route::Rail);
        let outside = format!("{SETUP}outside_edit: true\n");
        write(directory.path(), &outside);
        cx.update_window(handle, |_, window, cx| {
            window.click("settings-unsaved-save", cx)
        })
        .unwrap();
        cx.run_until_parked();
        cx.wait_for_io(handle, |_, cx| {
            !pilot
                .read(cx)
                .settings_page
                .read(cx)
                .unsaved_save_in_progress()
        })
        .await;
        cx.run_until_parked();
        stayed(cx, &pilot, section);
        marker(cx, handle, section, true);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("freshkube.yaml")).unwrap(),
            outside
        );
        leave(cx, handle, Route::Rail);
        cx.update_window(handle, |_, window, cx| {
            window.click("settings-unsaved-discard", cx)
        })
        .unwrap();
        cx.run_until_parked();
        arrived(cx, &pilot, Route::Rail);
    }
}

#[gpui_kit::test]
async fn coroot_save_waits_for_a_new_or_replaced_key(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    for kept in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let setup = SETUP.replace(
            "sign_in: anonymous",
            if kept {
                "sign_in: api-key, key: keys.json"
            } else {
                "sign_in: api-key"
            },
        );
        write(directory.path(), &setup);
        let (_runtime, handle, pilot) = launch(cx, directory.path());
        cx.update(|cx| {
            let store = std::sync::Arc::new(freshkube_core::secrets::FileStore::new(
                &directory.path().join("preferences.json"),
                None,
            ));
            let (settings, observability) = {
                let view = pilot.read(cx);
                (view.settings_page.clone(), view.pages.observability.clone())
            };
            settings.update(cx, |page, _| page.metrics_secrets = Some(store.clone()));
            observability.update(cx, |page, _| {
                page.set_settings_secrets_for_test(
                    &directory.path().join("preferences.json"),
                    store,
                )
            });
        });
        open(cx, handle, &pilot, SettingsSection::Coroot);
        cx.update_window(handle, |_, window, cx| {
            if kept {
                window.click("coroot-credential-replace", cx);
            }
            window.click("coroot-credential-input", cx);
            window.input("invented-test-value", cx);
        })
        .unwrap();
        cx.run_until_parked();
        marker(cx, handle, SettingsSection::Coroot, true);
        leave(cx, handle, Route::Column);
        cx.update_window(handle, |_, window, cx| {
            window.click("settings-unsaved-save", cx)
        })
        .unwrap();
        cx.run_until_parked();
        cx.wait_for_io(handle, |_, cx| {
            !pilot
                .read(cx)
                .settings_page
                .read(cx)
                .unsaved_save_in_progress()
        })
        .await;
        cx.run_until_parked();
        arrived(cx, &pilot, Route::Column);
        marker(cx, handle, SettingsSection::Coroot, false);
        let setup =
            setup::parse(&std::fs::read(directory.path().join("freshkube.yaml")).unwrap()).unwrap();
        assert!(setup.coroot.unwrap().key.is_some());
        assert!(directory.path().join("keys.json").exists());
    }
}

#[gpui_kit::test]
async fn coroot_key_write_failure_keeps_the_draft(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    write(
        directory.path(),
        &SETUP.replace("sign_in: anonymous", "sign_in: api-key"),
    );
    let (_runtime, handle, pilot) = launch(cx, directory.path());
    cx.update(|cx| {
        let store = std::sync::Arc::new(freshkube_core::secrets::FileStore::new(
            &directory.path().join("preferences.json"),
            None,
        ));
        let (settings, observability) = {
            let view = pilot.read(cx);
            (view.settings_page.clone(), view.pages.observability.clone())
        };
        settings.update(cx, |page, _| page.metrics_secrets = Some(store.clone()));
        observability.update(cx, |page, _| {
            page.set_settings_secrets_for_test(&directory.path().join("preferences.json"), store)
        });
    });
    // An unusable temporary key file refuses writes without involving any
    // real credential store.
    std::fs::create_dir(directory.path().join("keys.json")).unwrap();
    open(cx, handle, &pilot, SettingsSection::Coroot);
    cx.update_window(handle, |_, window, cx| {
        window.click("coroot-credential-input", cx);
        window.input("invented-test-value", cx);
    })
    .unwrap();
    cx.run_until_parked();
    leave(cx, handle, Route::Column);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-unsaved-save", cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.wait_for_io(handle, |_, cx| {
        !pilot
            .read(cx)
            .settings_page
            .read(cx)
            .unsaved_save_in_progress()
    })
    .await;
    cx.run_until_parked();
    stayed(cx, &pilot, SettingsSection::Coroot);
    marker(cx, handle, SettingsSection::Coroot, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("coroot-state-text")
                .label()
                .unwrap()
                .contains("Couldn't save the key")
        );
    })
    .unwrap();
    leave(cx, handle, Route::Rail);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-unsaved-discard", cx)
    })
    .unwrap();
    cx.run_until_parked();
    arrived(cx, &pilot, Route::Rail);
}

#[gpui_kit::test]
async fn typing_in_a_dirty_section_draws_the_column_once_at_most(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    write(directory.path(), SETUP);
    let (_runtime, handle, pilot) = launch(cx, directory.path());
    let section = SettingsSection::Monitoring;
    open(cx, handle, &pilot, section);
    edit(cx, handle, section, "c");
    marker(cx, handle, section, true);
    let before = freshkube_probe::probe::count("column.derive");
    for key in ["l", "u", "s"] {
        cx.update_window(handle, |_, window, cx| window.input(key, cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }
    let drawn = freshkube_probe::probe::count("column.derive") - before;
    assert!(drawn <= 1, "drawn {drawn}");
    marker(cx, handle, section, true);
}

#[gpui_kit::test]
async fn a_navigation_that_is_not_the_keyboard_leaves_focus_alone(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    write(directory.path(), SETUP);
    let (_runtime, handle, pilot) = launch(cx, directory.path());
    let _other = cx
        .update_window(handle, |_, window, cx| {
            window.activate_window();
            let other = cx.focus_handle();
            window.focus(&other, cx);
            other
        })
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Nodes, window, cx));
        assert!(!pilot.read(cx).focus.is_focused(window));
        pilot.update(cx, |pilot, cx| {
            pilot.navigate_from_keyboard(Page::Overview, window, cx)
        });
        assert!(pilot.read(cx).focus.is_focused(window));
    })
    .unwrap();
}
