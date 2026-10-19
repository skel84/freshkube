//! The Coroot section: its fields, Connect and Disconnect, and the file
//! they write.
use super::edit_tests::{open_section, saved, write};
use super::tests::launch;
use super::{Origin, SettingsSection};
use crate::desktop::{Page, layout_check};
use crate::observability::ConnectionState;
use freshkube_core::setup::{self, SignIn};
use freshkube_ui::real_io::RealIo;
use gpui_kit::{AppContext, Focusable, TestAppContext, test::TestWindowExt};
use std::path::Path;

/// A setup that names Coroot, with keys this version doesn't know.
const SETUP: &str = "version: 1\nfuture_key: kept\nclusters:\n- {id: cluster-a, role: core, context: cluster-a}\ncoroot: {server: 'https://observations.example.test', sign_in: anonymous, future: 1}\n";

type Window = gpui_kit::AnyWindowHandle;
type View = gpui_kit::Entity<crate::desktop::Pilot>;

fn read(directory: &Path) -> setup::Setup {
    setup::parse(&std::fs::read(directory.join("freshkube.yaml")).unwrap()).unwrap()
}

/// Opens the section, and lets its inputs be made.
async fn open(cx: &mut TestAppContext, handle: Window, view: &View) {
    open_section(cx, handle, view, SettingsSection::Coroot).await;
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.navigate_from_keyboard(Page::Settings, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

/// Sets a field as a person would have left it.
fn fill(cx: &mut TestAppContext, handle: Window, view: &View, field: &str, value: &str) {
    cx.update_window(handle, |_, window, cx| {
        let page = view.read(cx).settings_page.clone();
        page.update(cx, |page, cx| {
            let form = page.coroot.as_ref().unwrap();
            let input = match field {
                "server" => &form.server,
                "project" => &form.project,
                _ => &form.credential,
            };
            input.update(cx, |input, cx| {
                input.set_value(value.to_owned(), window, cx)
            });
        });
    })
    .unwrap();
}

#[gpui_kit::test]
async fn each_sign_in_shows_its_fields(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    write(directory.path(), SETUP);
    let (_runtime, handle, view) = launch(cx, directory.path());
    open(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("coroot-server").visible());
        assert!(window.find("coroot-project").visible());
        assert!(window.find(("coroot-sign-in", 2usize)).visible());
        // Anonymous needs no credential.
        assert!(window.try_find("coroot-credential-row").is_none());
        window.click(("coroot-sign-in", 0usize), cx);
        window.render_frame(cx);
        assert!(window.find("coroot-credential-row-label").visible());
        assert!(window.find("coroot-credential-input").visible());
        window.click(("coroot-sign-in", 1usize), cx);
        window.render_frame(cx);
        assert!(window.find("coroot-credential-row-label").visible());
        window.click(("coroot-sign-in", 2usize), cx);
        window.render_frame(cx);
        assert!(window.try_find("coroot-credential-row").is_none());
        // The foot names the file; the group saves with Connect and Revert.
        assert_eq!(window.find("coroot-connect").label(), Some("Connect"));
        assert!(window.find("coroot-connection-saved-in").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_kept_key_shows_where_it_is_kept_and_never_its_value(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    write(
        directory.path(),
        &SETUP.replace("sign_in: anonymous", "sign_in: api-key, key: keys.json"),
    );
    let (_runtime, handle, view) = launch(cx, directory.path());
    open(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("coroot-credential-kept").visible());
        assert!(window.try_find("coroot-credential-input").is_none());
        // Replace… opens the field, empty.
        window.click("coroot-credential-replace", cx);
        window.render_frame(cx);
        assert!(window.find("coroot-credential-input").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn connect_writes_the_block_keeping_the_rest_and_asks_observability(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    write(directory.path(), SETUP);
    let (_runtime, handle, view) = launch(cx, directory.path());
    open(cx, handle, &view).await;
    fill(cx, handle, &view, "server", "http://127.0.0.1:1");
    fill(cx, handle, &view, "project", "p2");
    cx.update_window(handle, |_, window, cx| window.click("coroot-connect", cx))
        .unwrap();
    saved(cx, handle, &view).await;
    let written = read(directory.path());
    let block = written.coroot.clone().unwrap();
    assert_eq!(block.server, "http://127.0.0.1:1");
    assert_eq!(block.sign_in, SignIn::Anonymous);
    assert_eq!(block.project.as_deref(), Some("p2"));
    assert_eq!(written.workspace.clusters.len(), 1);
    let text = std::fs::read_to_string(directory.path().join("freshkube.yaml")).unwrap();
    assert!(text.contains("future_key"), "{text}");
    assert!(
        text.contains("future: 1") || text.contains("future:"),
        "{text}"
    );
    // Observability was asked to connect, and its refusal is the group's
    // state line.
    let observed = view.clone();
    cx.wait_for_io(handle, move |_, cx| {
        matches!(
            observed
                .read(cx)
                .pages
                .observability
                .read(cx)
                .connection_state(),
            ConnectionState::Failed(_)
        )
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let line = window.find("coroot-state-text").label().unwrap().to_owned();
        assert!(!line.is_empty());
        assert_ne!(line, "Not connected");
        assert_eq!(
            view.read(cx).pages.observability.read(cx).coroot_url(cx),
            "http://127.0.0.1:1"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn disconnect_keeps_the_block_and_the_next_launch_stays_disconnected(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    write(directory.path(), SETUP);
    let (_runtime, handle, view) = launch(cx, directory.path());
    open(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        view.read(cx).settings_page.clone().update(cx, |page, cx| {
            page.set_coroot_state(ConnectionState::Connected, cx)
        });
        window.render_frame(cx);
        window.click("coroot-disconnect", cx);
    })
    .unwrap();
    let preferences = directory.path().join("preferences.json");
    cx.wait_for_io(handle, |_, _| {
        std::fs::read(&preferences)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|value| value["coroot_reconnect"] == false)
    })
    .await;
    // The block stays, byte for byte.
    assert_eq!(
        std::fs::read_to_string(directory.path().join("freshkube.yaml")).unwrap(),
        SETUP
    );
    let (_second_runtime, second, reopened) = launch(cx, directory.path());
    cx.run_until_parked();
    cx.update_window(second, |_, window, cx| {
        reopened.update(cx, |pilot, cx| {
            pilot.navigate_from_keyboard(Page::Observability, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let page = reopened.read(cx).pages.observability.read(cx);
        assert_eq!(page.connection_state(), ConnectionState::Disconnected);
        assert_eq!(page.coroot_url(cx), "https://observations.example.test");
    });
}

#[gpui_kit::test]
async fn revert_puts_the_files_values_back(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    write(directory.path(), SETUP);
    let (_runtime, handle, view) = launch(cx, directory.path());
    open(cx, handle, &view).await;
    fill(
        cx,
        handle,
        &view,
        "server",
        "https://elsewhere.example.test",
    );
    fill(cx, handle, &view, "project", "p9");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("coroot-revert", cx);
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        let form = page.coroot.as_ref().unwrap();
        assert_eq!(
            form.server.read(cx).value().as_ref(),
            "https://observations.example.test"
        );
        assert!(form.project.read(cx).value().is_empty());
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(directory.path().join("freshkube.yaml")).unwrap(),
        SETUP
    );
}

#[gpui_kit::test]
async fn field_errors_show_at_the_field_and_nothing_is_written(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    write(directory.path(), SETUP);
    let (_runtime, handle, view) = launch(cx, directory.path());
    open(cx, handle, &view).await;
    fill(cx, handle, &view, "server", "coroot.acme.example");
    cx.update_window(handle, |_, window, cx| {
        window.click("coroot-connect", cx);
        window.render_frame(cx);
        assert!(window.find("coroot-server-row-error").visible());
        // The first failing field takes focus.
        let page = view.read(cx).settings_page.read(cx);
        let server = page
            .coroot
            .as_ref()
            .unwrap()
            .server
            .read(cx)
            .focus_handle(cx);
        assert!(server.is_focused(window));
    })
    .unwrap();
    fill(cx, handle, &view, "server", "https://coroot.acme.example");
    cx.update_window(handle, |_, window, cx| {
        window.click(("coroot-sign-in", 0usize), cx);
        window.click("coroot-connect", cx);
        window.render_frame(cx);
        assert!(window.find("coroot-credential-row-error").visible());
        assert!(window.try_find("coroot-server-row-error").is_none());
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(directory.path().join("freshkube.yaml")).unwrap(),
        SETUP
    );
}

#[gpui_kit::test]
async fn a_broken_file_refuses_the_save_with_the_clusters_words(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    write(directory.path(), SETUP);
    let (_runtime, handle, view) = launch(cx, directory.path());
    open(cx, handle, &view).await;
    write(directory.path(), "version: [");
    super::tests::poll_setup(cx);
    fill(cx, handle, &view, "server", "https://coroot.acme.example");
    cx.update_window(handle, |_, window, cx| {
        window.click("coroot-connect", cx);
        window.render_frame(cx);
        let banner = window.find("settings-banner").label().unwrap().to_owned();
        assert!(banner.contains("freshkube.yaml isn't used"), "{banner}");
        let line = window.find("coroot-state-text").label().unwrap().to_owned();
        assert!(line.contains("freshkube.yaml isn't usable"), "{line}");
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(directory.path().join("freshkube.yaml")).unwrap(),
        "version: ["
    );
}

#[gpui_kit::test]
fn example_data_shows_the_example_connection_and_saves_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = crate::desktop::tests::fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Coroot, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        assert_eq!(page.origin(), &Origin::Example);
        let form = page.coroot.as_ref().unwrap();
        assert_eq!(
            form.server.read(cx).value().as_ref(),
            "https://coroot.acme.example"
        );
        assert!(window.find("coroot-credential-kept").visible());
        // Connect does nothing in example data.
        window.click("coroot-connect", cx);
        assert!(view.read(cx).settings_page.read(cx).saving.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn observability_points_to_the_section(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    write(
        directory.path(),
        "version: 1\nclusters:\n- {id: cluster-a, role: core, context: cluster-a}\n",
    );
    let (_runtime, handle, view) = launch(cx, directory.path());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.navigate_from_keyboard(Page::Observability, window, cx)
        });
        window.render_frame(cx);
        assert!(window.find("obs-not-connected").visible());
        window.click("obs-open-settings", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let pilot = view.read(cx);
        assert_eq!(pilot.navigation.page, Page::Settings);
        assert_eq!(
            pilot.settings_page.read(cx).section(),
            SettingsSection::Coroot
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn coroot_is_a_settings_form_at_each_window_and_text_size(cx: &mut TestAppContext) {
    use crate::{text_size, ui};
    use freshkube_ui::form;
    use gpui_kit::px;

    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, pilot) = crate::desktop::tests::fixture(cx, width, height);
        for size in [13., 20.] {
            cx.update_window(handle, |_, window, cx| {
                text_size::set(size, cx);
                pilot.update(cx, |pilot, cx| {
                    pilot.show_settings(SettingsSection::Coroot, window, cx)
                });
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                layout_check::assert_page_frame(
                    window,
                    cx,
                    &layout_check::PageFrame {
                        page: "settings-page",
                        title: "settings-coroot-title",
                        title_text: "Coroot",
                        content: "settings-coroot-form",
                    },
                );
                let dp = |n| ui::dp_px(n, window);
                let group = window.find("coroot-connection").bounds();
                assert!(group.size.width <= dp(form::MAX_WIDTH) + px(1.));
                let control = window.find(("coroot-sign-in", 0usize)).bounds();
                assert!(control.size.height > px(0.));
                let content = window.find("settings-coroot-form").bounds();
                assert!(group.right() <= content.right() + px(1.));
            })
            .unwrap();
        }
    }
}
