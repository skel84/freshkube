//! The delivery sources list in the real app: where each part is read,
//! and what Check sources finds, in each state.
use super::super::SettingsSection;
use super::super::edit_tests::{open_section, write};
use super::super::tests::launch;
use super::Check;
use super::Read;
#[cfg(unix)]
use crate::desktop::Page;
use crate::desktop::{layout_check, tests::fixture};
use freshkube_core::workspace::{self, Entry, Loaded, Part, Role, Workspace};
#[cfg(unix)]
use freshkube_delivery::entry::Checked;
use freshkube_delivery::entry::Reach;
use freshkube_ui::real_io::RealIo;
use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};
#[cfg(unix)]
use std::time::Duration;

mod checks;
mod configuration;
mod inspector;
mod layout;
mod lifecycle;

const PAGE: layout_check::TablePage = layout_check::TablePage {
    page: "settings-page",
    title: "settings-sources-title",
    title_text: "Delivery sources",
    table: "settings-sources-table-scroll",
    list: "settings-sources-list",
};

const KARGO: Read = Read::Part(Part::Kargo);
const BUILDS: Read = Read::Part(Part::Builds);
const ARGOCD: Read = Read::Part(Part::ArgoCd);
const WORKLOADS: Read = Read::Workloads;
/// The example's destinations, in its order.
const MAPPED: &str = "dev-fra, stage-fra, prod-ams";

type View = gpui_kit::Entity<crate::desktop::Pilot>;
type List = gpui_kit::Entity<super::SourcesList>;

fn list(view: &View, cx: &gpui_kit::App) -> List {
    view.read(cx).settings_page.read(cx).sources.clone()
}

/// A completion token for answering the pending check in the test, without
/// replacing its work. Loader's test helper begins the synthetic answer.
#[cfg(unix)]
fn request(
    list: &List,
    cx: &mut gpui_kit::App,
) -> freshkube_core::snapshot::Request<super::CheckIdentity> {
    list.update(cx, |list, _| list.read.begin(list.identity()))
}

fn states(view: &View, cx: &gpui_kit::App) -> Vec<(Read, String, String)> {
    list(view, cx)
        .read(cx)
        .states()
        .into_iter()
        .map(|(read, entry, state)| (read, entry.to_owned(), state.to_owned()))
        .collect()
}

fn state(view: &View, read: Read, cx: &gpui_kit::App) -> String {
    let states = states(view, cx);
    let found = states.iter().find(|(row, _, _)| *row == read);
    found.map(|(_, _, state)| state.clone()).unwrap_or_default()
}

fn tooltip(view: &View, read: Read, cx: &gpui_kit::App) -> String {
    let list = list(view, cx);
    let list = list.read(cx);
    let row = list.rows.iter().find(|row| row.read == read).unwrap();
    row.tooltip.to_string()
}

fn row(read: Read, entry: &str, state: &str) -> (Read, String, String) {
    (read, entry.to_owned(), state.to_owned())
}

fn open(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, view: &View) {
    view.update(cx, |pilot, cx| {
        pilot.show_settings(SettingsSection::Sources, window, cx)
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

const PIN_SETUP: &str = r#"version: 1
clusters:
- {id: cluster-a, role: core, context: acme-a, future_entry: {keep: true}}
- {id: cluster-b, role: cicd, context: acme-b}
sources: {builds: cluster-b, argocd: cluster-a, future_pin: {keep: true}}
destinations: [{name: acme, entry: cluster-a}]
metrics: {mode: automatic, future_metrics: keep}
coroot: {server: 'https://observations.example.test', sign_in: anonymous}
future_setup: {keep: [1, 2]}
"#;

fn dispatch_pin_from_menu(
    sources: &List,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
    entry: Option<&str>,
) {
    use freshkube_ui::{menu::MenuAction, table::TableSource};
    let actions = sources.update(cx, |list, cx| list.row_menu(&KARGO, window, cx));
    let action = if let Some(entry) = entry {
        let items = actions
            .into_iter()
            .find_map(|action| match action {
                MenuAction::Submenu { items, .. } => Some(items),
                _ => None,
            })
            .unwrap();
        items
            .into_iter()
            .find_map(|item| match item {
                MenuAction::Item {
                    label,
                    action,
                    enabled: true,
                    ..
                } if label.as_ref() == entry => Some(action),
                _ => None,
            })
            .unwrap()
    } else {
        actions
            .into_iter()
            .find_map(|item| match item {
                MenuAction::Item {
                    label,
                    action,
                    enabled: true,
                    ..
                } if label.as_ref() == "Follow roles" => Some(action),
                _ => None,
            })
            .unwrap()
    };
    sources
        .read(cx)
        .focus_handle()
        .dispatch_action(action.as_ref(), window, cx);
}

/// Count receipts only inside the status bar; the Saved banner stays on
/// the page, as it does on the other workspace sections.
fn assert_single_save_receipt(window: &gpui_kit::Window) {
    assert!(window.find("setup-reload-status").visible());
    let receipts = gpui_kit::base::test_support::snapshots(window)
        .into_iter()
        .filter(|snapshot| {
            snapshot.visible()
                && snapshot
                    .path()
                    .contains(&gpui_kit::ElementId::from("status-bar"))
                && snapshot
                    .label()
                    .is_some_and(|label| label.starts_with("Saved"))
        })
        .count();
    assert_eq!(
        receipts, 1,
        "the shell owns the only status-bar save receipt"
    );
}

#[gpui_kit::test]
async fn pin_and_follow_roles_keep_other_parts_and_unknown_keys(cx: &mut TestAppContext) {
    use super::super::edit_tests::{read_workspace, saved};
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), PIN_SETUP);
    let before = freshkube_core::setup::parse(PIN_SETUP.as_bytes()).unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    for (entry, inspector) in [
        (Some("cluster-a"), false),
        (None, true),
        (Some("cluster-a"), true),
        (None, false),
    ] {
        cx.update_window(handle, |_, window, cx| {
            let sources = list(&view, cx);
            if inspector {
                window.click("settings-source-kargo", cx);
                window.render_frame(cx);
                if entry.is_some() {
                    window.click("settings-source-pin", cx);
                    window.render_frame(cx);
                } else {
                    window.click("settings-source-follow-roles", cx);
                }
            } else if entry.is_none() {
                window.right_click("settings-source-kargo", cx);
            } else {
                dispatch_pin_from_menu(&sources, window, cx, entry);
            }
        })
        .unwrap();
        cx.run_until_parked();
        if inspector && entry.is_some() || !inspector && entry.is_none() {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window
                    .within("popup-menu")
                    .click(if inspector { 0usize } else { 3usize }, cx);
            })
            .unwrap();
            cx.run_until_parked();
        }
        saved(cx, handle, &view).await;
        assert_eq!(read_workspace(guard.path()).sources.pin(Part::Kargo), entry);
        let mut expected = before.clone();
        expected
            .workspace
            .sources
            .set(Part::Kargo, entry.map(str::to_owned));
        let after = freshkube_core::setup::parse(
            &std::fs::read(guard.path().join("freshkube.yaml")).unwrap(),
        )
        .unwrap();
        assert_eq!(after, expected);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("settings-saved").visible());
            let status = view.read(cx).settings_page.read(cx).status().unwrap();
            assert_eq!(status.text(None).as_ref(), "Delivery sources");
            assert_single_save_receipt(window);
            // Complete a check without cluster I/O: its time belongs to
            // the Sources segment while the shell keeps the save receipt.
            list(&view, cx).update(cx, |list, cx| {
                let request = list.read.start(list.identity());
                list.answer(&request, Ok(list.example_check()), cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let checked = list(&view, cx).read(cx).checked_at.clone().unwrap();
            let expected = format!("Delivery sources · Last checked {checked}");
            let status = view.read(cx).settings_page.read(cx).status().unwrap();
            assert_eq!(status.text(None).as_ref(), expected);
            assert_eq!(
                window.find("settings-scope").label(),
                Some(expected.as_str())
            );
            assert!(window.find("settings-saved").visible());
            assert_single_save_receipt(window);
        })
        .unwrap();
    }
    write(guard.path(), "version: [broken");
    cx.update_window(handle, |_, window, cx| {
        dispatch_pin_from_menu(&list(&view, cx), window, cx, Some("cluster-a"));
    })
    .unwrap();
    cx.run_until_parked();
    saved(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-save-failed").visible());
        let status = view.read(cx).settings_page.read(cx).status().unwrap();
        assert!(
            !status.text(None).contains("Saved"),
            "{}",
            status.text(None)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_broken_setup_refuses_pin_edits_without_replacing_the_file(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), PIN_SETUP);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    write(guard.path(), "version: [broken");
    cx.update_window(handle, |_, window, cx| {
        dispatch_pin_from_menu(&list(&view, cx), window, cx, Some("cluster-a"));
    })
    .unwrap();
    cx.run_until_parked();
    super::super::edit_tests::saved(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-save-failed").visible());
        assert_eq!(
            view.read(cx)
                .settings_page
                .read(cx)
                .workspace()
                .sources
                .pin(Part::Kargo),
            None
        );
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap(),
        "version: [broken"
    );
}

#[gpui_kit::test]
async fn an_example_pin_edit_leaves_the_setup_file_untouched(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), PIN_SETUP);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    cx.update_window(handle, |_, window, cx| {
        let workspace = view.read(cx).settings_page.read(cx).workspace().clone();
        set_workspace(&view, workspace, true, cx);
        window.click("settings-source-kargo", cx);
        window.render_frame(cx);
        window.click("settings-source-pin", cx);
        assert!(window.try_find("popup-menu").is_none());
        list(&view, cx).update(cx, |list, cx| list.pin(Some("cluster-a".into()), cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap(),
        PIN_SETUP
    );
    assert!(!guard.path().join("freshkube.yaml.bak").exists());
}

#[gpui_kit::test]
async fn a_hand_edit_refuses_a_pin_without_replacing_the_file(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), PIN_SETUP);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_section(cx, handle, &view, SettingsSection::Sources).await;
    let edited = PIN_SETUP.replace("acme-a", "acme-changed");
    write(guard.path(), &edited);
    cx.update_window(handle, |_, window, cx| {
        dispatch_pin_from_menu(&list(&view, cx), window, cx, Some("cluster-a"));
    })
    .unwrap();
    cx.run_until_parked();
    super::super::edit_tests::saved(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-save-failed").visible());
        assert!(view.read(cx).settings_page.read(cx).setup_is_pending());
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap(),
        edited
    );
}

fn settle_sources(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    window.render_frame(cx);
    for _ in 0..24 {
        if window.simulate_next_frame(cx) == 0 {
            break;
        }
        window.render_frame(cx);
    }
}

#[gpui_kit::test]
fn recovered_setup_file_is_used_with_warnings_and_all_skipped_sources_are_unavailable(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("freshkube.yaml"),
        "version: 1\nclusters: [[]]\n",
    )
    .unwrap();
    let (_runtime, handle, view) = super::super::tests::launch(cx, directory.path());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.show_settings(super::super::SettingsSection::Sources, window, cx)
        });
        window.render_frame(cx);
        let sources = list(&view, cx);
        let sources = sources.read(cx);
        let row = sources
            .rows
            .iter()
            .find(|row| row.read == Read::SetupFile)
            .unwrap();
        assert_eq!(row.state, "Used with warnings");
        assert_eq!(row.tone, Some(crate::ui::Tone::Warn));
        assert!(row.explanation.contains("skipped clusters[0]"));
        assert!(
            sources
                .sources
                .iter()
                .all(|(_, chosen)| matches!(chosen, workspace::Chosen::NoEntry(_)))
        );
        assert!(sources.sources.workloads().is_empty());
    })
    .unwrap();
}
