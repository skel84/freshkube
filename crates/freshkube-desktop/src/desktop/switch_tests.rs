//! Switching between the clusters of the workspace file.
use super::session::SessionKey;
use super::tests::{fixture, mount, start_shell};
use super::{Page, Pilot};
use crate::GpuiOptions;
use crate::navigation_file::NavigationFile;
use crate::resources::{example, live, shell};
use freshkube_core::workspace::{Entry, Role, Workspace};
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};
use std::path::Path;
use std::time::Duration;

fn key(id: &str) -> SessionKey {
    SessionKey::Entry(id.into())
}

fn active(cx: &mut TestAppContext, view: &Entity<Pilot>) -> SessionKey {
    cx.read(|cx| view.read(cx).registry.active_key().clone())
}

fn pick(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<Pilot>, ix: usize) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        if !view.read(cx).context_display.open {
            window.click("context-switcher", cx);
            window.render_frame(cx);
        }
        window.click(("cluster", ix), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[test]
fn a_launch_starts_the_remembered_entry_else_the_first_core_else_the_first() {
    let mut workspace = Workspace::default();
    workspace.clusters = vec![
        Entry::new("dev", Role::Environment, "acme-dev"),
        Entry::new("mgmt", Role::Core, "acme-mgmt"),
        Entry::new("ci", Role::Cicd, "acme-ci"),
    ];
    let start = |remembered| super::switch::start_entry(&workspace, remembered).map(|e| &*e.id);
    assert_eq!(start(Some("ci")), Some("ci"));
    // A remembered entry that is gone is not a reason to fail.
    assert_eq!(start(Some("gone")), Some("mgmt"));
    assert_eq!(start(None), Some("mgmt"));
    let mut without_core = workspace.clone();
    without_core.clusters.remove(1);
    assert_eq!(
        super::switch::start_entry(&without_core, None).map(|e| &*e.id),
        Some("dev")
    );
    assert!(super::switch::start_entry(&Workspace::default(), Some("ci")).is_none());
}

#[gpui_kit::test]
fn the_header_lists_the_clusters_and_picking_one_makes_it_active(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    assert_eq!(active(cx, &view), SessionKey::Implicit);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("context-switcher", cx);
        window.render_frame(cx);
        // The six example clusters, in the file's order.
        for ix in 0..6usize {
            assert!(window.find(("cluster", ix)).visible(), "{ix}");
        }
        assert_eq!(window.find(("cluster", 0usize)).label(), Some("core-fra"));
        assert_eq!(window.find(("cluster", 0usize)).selected(), Some(false));
    })
    .unwrap();
    pick(cx, handle, &view, 2);
    assert_eq!(active(cx, &view), key("dev-fra"));
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.applied.context.as_deref(), Some("dev-fra"));
        assert_eq!(
            pilot.active_definition,
            super::switch::definition(
                &pilot.settings_page.read(cx).workspace().clusters[2],
                pilot.settings_page.read(cx).workspace(),
            )
        );
    });
    assert_eq!(
        cx.read(|cx| NavigationFile::global(cx).active_cluster()),
        None,
        "example data touches no file"
    );
    cx.update_window(handle, |_, window, cx| {
        window.click("context-switcher", cx);
        window.render_frame(cx);
        assert_eq!(window.find(("cluster", 2usize)).selected(), Some(true));
        assert_eq!(window.find(("cluster", 0usize)).selected(), Some(false));
    })
    .unwrap();
}

#[gpui_kit::test]
fn picking_the_active_cluster_or_one_that_is_gone_changes_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    pick(cx, handle, &view, 1);
    let epoch = cx.read(|cx| view.read(cx).registry.summary_epoch());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.switch_cluster("cicd-fra".into(), window, cx);
            pilot.switch_cluster("not-listed".into(), window, cx);
            pilot.activate_entry("not-listed", window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(active(cx, &view), key("cicd-fra"));
    assert_eq!(cx.read(|cx| view.read(cx).registry.summary_epoch()), epoch);
}

#[gpui_kit::test]
async fn coming_back_shows_the_last_summary_as_last_known_at_its_own_time(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    pick(cx, handle, &view, 0);
    cx.wait_for(handle, Duration::from_secs(2), |_, cx| {
        view.read(cx)
            .registry
            .active()
            .kubernetes_summary
            .data()
            .is_some()
    })
    .await;
    let (before, rows) = cx.read(|cx| {
        let pilot = view.read(cx);
        let summary = &pilot.registry.active().kubernetes_summary;
        assert!(!summary.is_stale());
        (summary.last_successful(), pilot.node_workspace.rows.len())
    });
    assert!(before.is_some() && rows > 0);

    pick(cx, handle, &view, 1);
    assert_eq!(active(cx, &view), key("cicd-fra"));
    cx.wait_for(handle, Duration::from_secs(2), |_, cx| {
        view.read(cx)
            .registry
            .active()
            .kubernetes_summary
            .data()
            .is_some()
    })
    .await;
    // Another cluster's summary is its own, never core-fra's.
    let other = cx.read(|cx| {
        view.read(cx)
            .registry
            .active()
            .kubernetes_summary
            .data()
            .cloned()
    });

    // Back on core-fra, before anything is read again (the example's answer
    // is held), its last answer is there, stale and with the time it was read.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.fixture_hold = true;
            pilot.activate_entry("core-fra", window, cx)
        });
        let pilot = view.read(cx);
        let summary = &pilot.registry.active().kubernetes_summary;
        assert!(summary.data().is_some());
        assert!(summary.is_stale());
        assert_eq!(summary.last_successful(), before);
        assert_ne!(summary.data().cloned(), other);
    })
    .unwrap();
}

#[gpui_kit::test]
fn switching_asks_before_ending_a_running_shell(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 800.);
    let context = cx.read(|cx| view.read(cx).applied.context.clone().unwrap());
    let (_, rows) = example::read(&context, "pods", None, live::now()).unwrap();
    let pod = rows
        .iter()
        .find(|row| row.cells[2] == "Running")
        .unwrap()
        .identity
        .clone();
    start_shell(handle, &view, &pod, cx);
    assert_eq!(cx.read(shell::running_anywhere).len(), 1);

    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.switch_cluster("dev-fra".into(), window, cx)
        });
    })
    .unwrap();
    let (message, _) = cx.pending_prompt().unwrap();
    assert_eq!(message, format!("End the shell in {}?", pod.name));
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(active(cx, &view), SessionKey::Implicit);
    assert_eq!(
        cx.read(|cx| view.read(cx).applied.context.clone()),
        Some(context)
    );
    assert_eq!(cx.read(shell::running_anywhere).len(), 1);

    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.switch_cluster("dev-fra".into(), window, cx)
        });
    })
    .unwrap();
    cx.simulate_prompt_answer("End the shell");
    cx.run_until_parked();
    assert_eq!(active(cx, &view), key("dev-fra"));
    assert!(cx.read(shell::running_anywhere).is_empty());
}

/// A preferences folder with a workspace file whose entries read files that
/// do not exist, so a launch connects to nothing and reads nothing outside
/// the folder.
fn folder(entries: &str) -> tempfile::TempDir {
    let guard = tempfile::tempdir().unwrap();
    let missing = guard.path().join("missing-kubeconfig");
    std::fs::write(
        guard.path().join("workspace.json"),
        format!(
            r#"{{"version":1,"kubeconfig":{},"clusters":[{entries}]}}"#,
            serde_json::to_string(&missing).unwrap()
        ),
    )
    .unwrap();
    guard
}

fn talos_entry(directory: &Path) -> String {
    format!(
        r#"{{"id":"mgmt","role":"core","context":"acme-mgmt","talosconfig":{},"talos_context":"acme-talos"}}"#,
        serde_json::to_string(&directory.join("missing-talosconfig")).unwrap()
    )
}

/// A folder whose file lists a kubeconfig entry and a Talos one.
fn both() -> tempfile::TempDir {
    rewrite(folder(""), |directory| {
        format!("{KUBE_ENTRY},{}", talos_entry(directory))
    })
}

const KUBE_ENTRY: &str = r#"{"id":"dev","role":"environment","context":"acme-dev"}"#;

fn live_launch(
    cx: &mut TestAppContext,
    directory: &Path,
    named: bool,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    // Tokio wakes GPUI from its own threads.
    cx.executor().allow_parking();
    let options = if named {
        GpuiOptions::new(Some(directory.join("missing-talosconfig")), None, 100)
    } else {
        GpuiOptions::new(None, None, 100)
    }
    .with_preferences(Some(directory.join("preferences.json")));
    mount(cx, options, 1280., 820.)
}

#[gpui_kit::test]
fn a_launch_with_no_named_source_opens_the_first_core_entry_through_its_talosconfig(
    cx: &mut TestAppContext,
) {
    let guard = both();
    let (_runtime, _handle, view) = live_launch(cx, guard.path(), false);
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.registry.active_key(), &key("mgmt"));
        assert!(pilot.kubernetes_only.is_none());
        assert_eq!(
            pilot.applied.path.as_deref(),
            Some(guard.path().join("missing-talosconfig").as_path())
        );
        assert_eq!(pilot.applied.context.as_deref(), Some("acme-talos"));
        assert_eq!(
            NavigationFile::global(cx).active_cluster().as_deref(),
            Some("mgmt")
        );
    });
}

/// Writes the workspace file again with entries that name `directory`.
fn rewrite(guard: tempfile::TempDir, entries: impl FnOnce(&Path) -> String) -> tempfile::TempDir {
    let missing = guard.path().join("missing-kubeconfig");
    std::fs::write(
        guard.path().join("workspace.json"),
        format!(
            r#"{{"version":1,"kubeconfig":{},"clusters":[{}]}}"#,
            serde_json::to_string(&missing).unwrap(),
            entries(guard.path())
        ),
    )
    .unwrap();
    guard
}

#[gpui_kit::test]
fn the_remembered_entry_wins_and_a_kubeconfig_entry_opens_without_talos(cx: &mut TestAppContext) {
    let guard = both();
    std::fs::write(
        guard.path().join("navigation.json"),
        r#"{"workspace":{"active":"dev"},"collapsed":true}"#,
    )
    .unwrap();
    let (_runtime, _handle, view) = live_launch(cx, guard.path(), false);
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.registry.active_key(), &key("dev"));
        let kube = pilot.kubernetes_only.as_ref().unwrap();
        assert_eq!(
            kube.explicit_file(),
            Some(guard.path().join("missing-kubeconfig").as_path())
        );
        assert_eq!(pilot.applied.path, None);
        assert_eq!(pilot.page, Page::Overview);
    });
}

#[gpui_kit::test]
fn a_launch_that_names_a_source_ignores_the_workspace_start(cx: &mut TestAppContext) {
    let guard = both();
    let (_runtime, _handle, view) = live_launch(cx, guard.path(), true);
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.registry.active_key(), &SessionKey::Implicit);
        assert_eq!(NavigationFile::global(cx).active_cluster(), None);
        // The switcher still lists the file's clusters.
        assert_eq!(pilot.switcher.len(), 2);
    });
}

#[gpui_kit::test]
fn switching_moves_between_a_talos_entry_and_a_kubeconfig_entry_and_remembers_it(
    cx: &mut TestAppContext,
) {
    let guard = both();
    let (_runtime, handle, view) = live_launch(cx, guard.path(), false);
    // The first core entry is the Talos one.
    assert_eq!(active(cx, &view), key("mgmt"));
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.activate_entry("dev", window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.registry.active_key(), &key("dev"));
        assert!(pilot.kubernetes_only.is_some());
        assert_eq!(pilot.applied.path, None);
        assert_eq!(
            NavigationFile::global(cx).active_cluster().as_deref(),
            Some("dev")
        );
    });
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.activate_entry("mgmt", window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert!(pilot.kubernetes_only.is_none());
        assert_eq!(pilot.applied.context.as_deref(), Some("acme-talos"));
        assert_eq!(
            NavigationFile::global(cx).active_cluster().as_deref(),
            Some("mgmt")
        );
    });
    // The choice reaches the file.
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    let written = std::thread::scope(|_| {
        for _ in 0..200 {
            if let Ok(text) = std::fs::read_to_string(guard.path().join("navigation.json"))
                && text.contains("\"mgmt\"")
            {
                return text;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        String::new()
    });
    assert!(written.contains("\"active\":\"mgmt\""), "{written}");
}
