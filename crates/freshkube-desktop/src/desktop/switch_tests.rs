//! Switching between the clusters of the workspace file.
use super::session::SessionKey;
use super::tests::{fixture, mount, start_shell};
use super::{KubeconfigSelection, Page, Pilot};
use crate::GpuiOptions;
use crate::navigation_file::NavigationFile;
use crate::resources::{example, live, shell};

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
        // None of it passes for current: the parts, the sources, the error.
        let kept = summary.data().unwrap();
        assert!(!kept.nodes.is_current() && !kept.pods.is_current());
        assert!(kept.observations.values().all(|o| !o.is_current()));
        assert!(summary.error().is_some());
    })
    .unwrap();
    // What is drawn says so: the Overview's state reads "Last known" with its
    // age, in place of Connected, and the switcher's row has the cluster's age.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.navigate_from_keyboard(Page::Overview, window, cx)
        });
        window.render_frame(cx);
        let label = window
            .find("overview-last-known")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(label.starts_with("Last known · "), "{label}");
        assert!(window.try_find("overview-connection").is_none());
        window.click("context-switcher", cx);
        window.render_frame(cx);
        let parked = window
            .find(("cluster-last-known", 1usize))
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(parked.starts_with("last known "), "{parked}");
    })
    .unwrap();
    // A new read replaces it, and the mark goes with it.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.fixture_hold = false;
            // The held read still counts as in flight; let the refresh start one.
            pilot.overview = crate::state::Snapshot::default();
            pilot.refresh_now(window, cx);
        });
    })
    .unwrap();
    cx.wait_for(handle, Duration::from_secs(2), |window, cx| {
        window.try_find("overview-last-known").is_none()
            && !view
                .read(cx)
                .registry
                .active()
                .kubernetes_summary
                .is_stale()
    })
    .await;
}

#[gpui_kit::test]
async fn a_late_summary_from_the_cluster_just_left_is_dropped(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    pick(cx, handle, &view, 0);
    cx.wait_for(handle, Duration::from_secs(2), |_, cx| {
        view.read(cx).registry.active().summary_session.is_some()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            // The old session had derived this when the window moved on.
            let old = pilot
                .registry
                .active()
                .summary_session
                .as_ref()
                .unwrap()
                .core
                .clone();
            let late = old.derive(chrono::Utc::now());
            let health = crate::screens::WorkloadData::from_outcome(&late.summary.workloads);
            pilot.fixture_hold = true;
            pilot.activate_entry("cicd-fra", window, cx);
            pilot.apply_summary(late, health, window, cx);
            assert!(pilot.registry.active().kubernetes_summary.data().is_none());
            assert!(pilot.registry.active().summary_health.is_none());
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn typing_in_the_form_does_not_rebuild_the_switcher(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let revision = cx.read(|cx| view.read(cx).switcher_revision);
    assert!(revision > 0);
    // A redraw of the Settings page is not a new workspace.
    let page = cx.read(|cx| view.read(cx).settings_page.clone());
    cx.update(|cx| view.update(cx, |pilot, _| pilot.switcher.truncate(1)));
    cx.update(|cx| page.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| view.read(cx).switcher.len()), 1, "not rebuilt");
    assert_eq!(cx.read(|cx| view.read(cx).switcher_revision), revision);
    // A new workspace is.
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_workspace(
                &freshkube_core::workspace::Loaded::Workspace(freshkube_core::workspace::example()),
                true,
                cx,
            )
        })
    });
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| view.read(cx).switcher.len()), 6);
    _ = handle;
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

#[gpui_kit::test]
fn cancelling_the_shell_question_for_a_kubeconfig_pick_keeps_the_entry(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 800.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.switch_cluster("dev-fra".into(), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(active(cx, &view), key("dev-fra"));
    let context = cx.read(|cx| view.read(cx).applied.context.clone().unwrap());
    let (_, rows) = example::read(&context, "pods", None, live::now()).unwrap();
    let pod = rows
        .iter()
        .find(|row| row.cells[2] == "Running")
        .unwrap()
        .identity
        .clone();
    start_shell(handle, &view, &pod, cx);
    // Example mode ignores kubeconfig picks; let this one through to the question.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.fixture = false;
            pilot.apply_kubeconfig(KubeconfigSelection::TalosControlPlane, window, cx);
        });
    })
    .unwrap();
    assert!(cx.pending_prompt().is_some());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    cx.update(|cx| view.update(cx, |pilot, _| pilot.fixture = true));
    assert_eq!(active(cx, &view), key("dev-fra"));
}

#[gpui_kit::test]
async fn a_talosconfig_whose_current_context_names_nothing_still_opens_an_entry(
    cx: &mut TestAppContext,
) {
    // Its `context:` is not among its contexts; the entry's own is.
    let config = "context: gone\ncontexts:\n  acme-mgmt:\n    endpoints: [127.0.0.1]\n    ca: YQ==\n    crt: Yg==\n    key: Yw==\n";
    let guard = talos_folder_with(config, true);
    let (_runtime, handle, view) = live_launch(cx, guard.path(), false);
    cx.wait_for(handle, Duration::from_secs(5), |_, cx| {
        let pilot = view.read(cx);
        !pilot.config_loading && pilot.applied.context.as_deref() == Some("acme-mgmt")
    })
    .await;
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert!(pilot.config_error.is_none(), "{:?}", pilot.config_error);
        assert!(pilot.kubernetes_only.is_none());
    });
}

#[gpui_kit::test]
fn leaving_an_entry_stops_the_last_known_label(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 800.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.restored_taken = Some(std::time::SystemTime::now());
            pilot
                .age_label
                .update(cx, |label, cx| label.set(pilot.restored_taken, cx));
            pilot.switch_cluster("dev-fra".into(), window, cx);
            pilot.restored_taken = Some(std::time::SystemTime::now());
            pilot
                .age_label
                .update(cx, |label, cx| label.set(pilot.restored_taken, cx));
            pilot.leave_entry(cx);
        });
    })
    .unwrap();
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.restored_taken, None);
        assert!(pilot.age_label.read(cx).is_idle());
    });
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
async fn switching_moves_between_a_talos_entry_and_a_kubeconfig_entry_and_remembers_it(
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
    let file = guard.path().join("navigation.json");
    cx.wait_for(handle, Duration::from_secs(2), |_, _| {
        std::fs::read_to_string(&file).is_ok_and(|text| text.contains("\"mgmt\""))
    })
    .await;
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(written.contains("\"active\":\"mgmt\""), "{written}");
}

const TALOSCONFIG_OTHER_CURRENT: &str = "context: other\ncontexts:\n  other:\n    endpoints: [127.0.0.1]\n    ca: YQ==\n    crt: Yg==\n    key: Yw==\n  acme-mgmt:\n    endpoints: [127.0.0.1]\n    ca: YQ==\n    crt: Yg==\n    key: Yw==\n";
const TALOSCONFIG_ONLY_OTHER: &str = "context: other\ncontexts:\n  other:\n    endpoints: [127.0.0.1]\n    ca: YQ==\n    crt: Yg==\n    key: Yw==\n";
/// A server that refuses at once, so nothing leaves this machine.
const KUBECONFIG: &str = "apiVersion: v1\nkind: Config\ncurrent-context: alpha\nclusters:\n- name: example\n  cluster:\n    server: https://127.0.0.1:1\ncontexts:\n- name: alpha\n  context:\n    cluster: example\n    user: example\n- name: beta\n  context:\n    cluster: example\n    user: example\nusers:\n- name: example\n  user:\n    token: not-a-real-token\n";

/// A folder with one Talos entry (`acme-mgmt`, no `talos_context`) over
/// `talosconfig`, and no workspace kubeconfig. A Kubernetes-only open with
/// none reads the home default, so `with_kubeconfig` names one in the folder.
fn talos_folder(talosconfig: &str) -> tempfile::TempDir {
    talos_folder_with(talosconfig, false)
}

fn talos_folder_with(talosconfig: &str, with_kubeconfig: bool) -> tempfile::TempDir {
    let guard = tempfile::tempdir().unwrap();
    let file = guard.path().join("talosconfig");
    std::fs::write(&file, talosconfig).unwrap();
    let kubeconfig = if with_kubeconfig {
        let path = guard.path().join("kubeconfig");
        std::fs::write(&path, KUBECONFIG.replace("alpha", "acme-mgmt")).unwrap();
        format!(r#","kubeconfig":{}"#, serde_json::to_string(&path).unwrap())
    } else {
        String::new()
    };
    std::fs::write(
        guard.path().join("workspace.json"),
        format!(
            r#"{{"version":1,"clusters":[{{"id":"mgmt","role":"core","context":"acme-mgmt","talosconfig":{}{kubeconfig}}}]}}"#,
            serde_json::to_string(&file).unwrap()
        ),
    )
    .unwrap();
    guard
}

#[gpui_kit::test]
async fn a_talos_entry_takes_the_context_it_names_never_the_files_current_one(
    cx: &mut TestAppContext,
) {
    let guard = talos_folder(TALOSCONFIG_OTHER_CURRENT);
    super::switch::search_kubeconfigs_in(vec![guard.path().join("none")]);
    let (_runtime, handle, view) = live_launch(cx, guard.path(), false);
    cx.wait_for(handle, Duration::from_secs(5), |_, cx| {
        view.read(cx).applied.context.is_some()
    })
    .await;
    cx.read(|cx| {
        let pilot = view.read(cx);
        // `other` is the file's current context; the entry's `context` is used.
        assert_eq!(pilot.applied.context.as_deref(), Some("acme-mgmt"));
        assert!(pilot.kubernetes_only.is_none());
        assert_eq!(pilot.registry.active_key(), &key("mgmt"));
    });
}

#[gpui_kit::test]
async fn a_talos_entry_the_file_has_no_context_for_opens_with_its_kubeconfig_context_alone(
    cx: &mut TestAppContext,
) {
    let guard = talos_folder_with(TALOSCONFIG_ONLY_OTHER, true);
    let (_runtime, handle, view) = live_launch(cx, guard.path(), false);
    cx.wait_for(handle, Duration::from_secs(5), |_, cx| {
        view.read(cx).kubernetes_only.is_some()
    })
    .await;
    cx.read(|cx| {
        let pilot = view.read(cx);
        // The file's current context was not applied.
        assert_ne!(pilot.applied.context.as_deref(), Some("other"));
        assert_eq!(pilot.applied.context, None);
        assert_eq!(pilot.registry.active_key(), &key("mgmt"));
        let note = pilot.settings_page.read(cx).note("mgmt").unwrap();
        assert!(
            note.starts_with("No Talos context named acme-mgmt in "),
            "{note}"
        );
        assert!(note.contains("set talos_context"), "{note}");
    });
}

#[gpui_kit::test]
async fn a_talos_entry_without_a_workspace_kubeconfig_reads_the_file_that_defines_its_context(
    cx: &mut TestAppContext,
) {
    let guard = talos_folder(TALOSCONFIG_OTHER_CURRENT);
    let file = guard.path().join("ambient-kubeconfig");
    std::fs::write(&file, KUBECONFIG).unwrap();
    // The entry's context is `acme-mgmt`; the ambient file has alpha and
    // beta, with alpha current. Point the entry at beta.
    std::fs::write(
        guard.path().join("workspace.json"),
        std::fs::read_to_string(guard.path().join("workspace.json"))
            .unwrap()
            .replace(
                "\"context\":\"acme-mgmt\"",
                "\"context\":\"beta\",\"talos_context\":\"acme-mgmt\"",
            ),
    )
    .unwrap();
    super::switch::search_kubeconfigs_in(vec![file.clone()]);
    let (_runtime, handle, view) = live_launch(cx, guard.path(), false);
    cx.wait_for(handle, Duration::from_secs(5), |_, cx| {
        matches!(
            view.read(cx).kubeconfig,
            freshkube_core::KubeconfigSelection::File { .. }
        )
    })
    .await;
    cx.read(|cx| {
        assert_eq!(
            view.read(cx).kubeconfig,
            freshkube_core::KubeconfigSelection::File {
                path: file.clone(),
                context: Some("beta".into())
            },
            "the entry's context, not alpha, the file's current one"
        );
    });
}

#[gpui_kit::test]
async fn a_context_no_default_kubeconfig_defines_is_said_and_never_replaced_by_the_current_one(
    cx: &mut TestAppContext,
) {
    let guard = talos_folder(TALOSCONFIG_OTHER_CURRENT);
    let file = guard.path().join("ambient-kubeconfig");
    std::fs::write(&file, KUBECONFIG).unwrap();
    super::switch::search_kubeconfigs_in(vec![file]);
    let (_runtime, handle, view) = live_launch(cx, guard.path(), false);
    cx.wait_for(handle, Duration::from_secs(5), |_, cx| {
        view.read(cx).settings_page.read(cx).note("mgmt").is_some()
    })
    .await;
    cx.read(|cx| {
        let pilot = view.read(cx);
        let note = pilot.settings_page.read(cx).note("mgmt").unwrap();
        assert!(
            note.contains("acme-mgmt is in none of the default kubeconfig files"),
            "{note}"
        );
        // The control plane's kubeconfig, not the ambient files.
        assert_eq!(
            pilot.kubeconfig,
            freshkube_core::KubeconfigSelection::TalosControlPlane
        );
    });
}

#[gpui_kit::test]
async fn choosing_a_context_by_hand_leaves_the_entry_and_the_next_launch_forgets_it(
    cx: &mut TestAppContext,
) {
    let guard = both();
    let (_runtime, handle, view) = live_launch(cx, guard.path(), false);
    assert_eq!(active(cx, &view), key("mgmt"));
    assert_eq!(
        cx.read(|cx| NavigationFile::global(cx).active_cluster())
            .as_deref(),
        Some("mgmt")
    );
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.select_context("acme-other".into(), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(active(cx, &view), SessionKey::Implicit);
    assert_eq!(
        cx.read(|cx| NavigationFile::global(cx).active_cluster()),
        None
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("context-switcher", cx);
        window.render_frame(cx);
        for ix in 0..2usize {
            assert_eq!(window.find(("cluster", ix)).selected(), Some(false), "{ix}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_launch_naming_the_source_an_entry_describes_belongs_to_that_entry(cx: &mut TestAppContext) {
    let guard = both();
    cx.executor().allow_parking();
    let options = GpuiOptions::new(
        Some(guard.path().join("missing-talosconfig")),
        Some("acme-talos".into()),
        100,
    )
    .with_preferences(Some(guard.path().join("preferences.json")));
    let (_runtime, _handle, view) = mount(cx, options, 1280., 820.);
    assert_eq!(active(cx, &view), key("mgmt"));
    // Adopted, not started: nothing was remembered by it.
    assert_eq!(
        cx.read(|cx| NavigationFile::global(cx).active_cluster()),
        None
    );
}

/// A Talos folder whose workspace names `kubeconfig` (written from `body`).
fn talos_folder_with_workspace_kubeconfig(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let guard = talos_folder(TALOSCONFIG_OTHER_CURRENT);
    let file = guard.path().join("kubeconfig");
    std::fs::write(&file, body).unwrap();
    std::fs::write(
        guard.path().join("workspace.json"),
        std::fs::read_to_string(guard.path().join("workspace.json"))
            .unwrap()
            .replace(
                "\"version\":1,",
                &format!(
                    "\"version\":1,\"kubeconfig\":{},",
                    serde_json::to_string(&file).unwrap()
                ),
            ),
    )
    .unwrap();
    (guard, file)
}

/// Starts the window on a Kubernetes-only entry listed before `mgmt`.
fn start_on_dev_first(guard: &tempfile::TempDir) {
    let file = guard.path().join("workspace.json");
    let text = std::fs::read_to_string(&file).unwrap().replace(
        "\"clusters\":[",
        "\"clusters\":[{\"id\":\"dev\",\"role\":\"core\",\"context\":\"acme-dev\"},",
    );
    std::fs::write(file, text).unwrap();
}

/// Opens `entry` and holds a link for it at once, before its kubeconfig
/// read has answered.
fn open_and_hold_link(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<Pilot>,
    entry: &str,
) {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.activate_entry(entry, window, cx))
    })
    .unwrap();
    cx.update(|cx| {
        view.update(cx, |pilot, _| {
            assert!(
                pilot.kubeconfig_draft.inspecting,
                "the entry's kubeconfig read is still out"
            );
            let link = crate::resources::model::ObjectRef {
                namespace: "ns".into(),
                name: "p".into(),
                uid: String::new(),
                connection: Some("c".into()),
            };
            pilot.pending_link = Some(super::switch::PendingLink::for_test(
                entry,
                pilot.entry_generation,
                link,
                super::switch::LinkWork::Open {
                    kind: freshkube_core::resources::builtin("pods").unwrap(),
                    tab: crate::resources::Tab::Overview,
                },
            ));
        })
    });
}

#[gpui_kit::test]
async fn a_talos_entry_with_a_kubeconfig_file_stays_the_entry_and_keeps_a_held_link(
    cx: &mut TestAppContext,
) {
    let (guard, wanted) =
        talos_folder_with_workspace_kubeconfig(&KUBECONFIG.replace("alpha", "acme-mgmt"));
    start_on_dev_first(&guard);
    let (_runtime, handle, view) = live_launch(cx, guard.path(), false);
    assert_eq!(active(cx, &view), key("dev"));
    cx.wait_for(handle, Duration::from_secs(5), |_, cx| {
        !view.read(cx).config_loading
    })
    .await;
    open_and_hold_link(cx, handle, &view, "mgmt");
    let generation = cx.read(|cx| view.read(cx).entry_generation);
    cx.wait_for(handle, Duration::from_secs(5), |_, cx| {
        matches!(
            &view.read(cx).kubeconfig,
            freshkube_core::KubeconfigSelection::File { path, .. } if *path == wanted
        )
    })
    .await;
    cx.run_until_parked();
    cx.read(|cx| {
        let pilot = view.read(cx);
        // Its own kubeconfig applied; the window never left the entry, and
        // the link held before the apply is the one still held.
        assert_eq!(pilot.registry.active_key(), &key("mgmt"));
        assert_eq!(
            NavigationFile::global(cx).active_cluster().as_deref(),
            Some("mgmt")
        );
        assert!(pilot.kubernetes_only.is_none());
        assert_eq!(pilot.entry_generation, generation);
        let held = pilot.pending_link.as_ref().unwrap();
        assert_eq!(
            (held.entry.as_str(), held.object.name.as_str()),
            ("mgmt", "p")
        );
        assert_eq!(held.object.connection.as_deref(), Some("c"));
    });
}

#[gpui_kit::test]
async fn a_held_link_is_dropped_when_the_entrys_kubeconfig_lacks_its_context(
    cx: &mut TestAppContext,
) {
    // alpha and beta; the entry's context is acme-mgmt.
    let (guard, _) = talos_folder_with_workspace_kubeconfig(KUBECONFIG);
    start_on_dev_first(&guard);
    let (_runtime, handle, view) = live_launch(cx, guard.path(), false);
    cx.wait_for(handle, Duration::from_secs(5), |_, cx| {
        !view.read(cx).config_loading
    })
    .await;
    open_and_hold_link(cx, handle, &view, "mgmt");
    cx.wait_for(handle, Duration::from_secs(5), |_, cx| {
        !view.read(cx).kubeconfig_draft.inspecting
    })
    .await;
    cx.run_until_parked();
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert!(
            pilot.pending_link.is_none(),
            "never left to be refused later"
        );
        assert_eq!(pilot.registry.active_key(), &key("mgmt"));
        assert_eq!(
            pilot.kubeconfig,
            freshkube_core::KubeconfigSelection::TalosControlPlane
        );
    });
}

#[gpui_kit::test]
async fn an_entry_that_cannot_read_its_kubeconfig_drops_a_held_link(cx: &mut TestAppContext) {
    let guard = both();
    let (_runtime, handle, view) = live_launch(cx, guard.path(), false);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.activate_entry("dev", window, cx);
            let link = crate::resources::model::ObjectRef {
                namespace: "ns".into(),
                name: "p".into(),
                uid: String::new(),
                connection: Some("c".into()),
            };
            pilot.pending_link = Some(super::switch::PendingLink::for_test(
                "dev",
                pilot.entry_generation,
                link,
                super::switch::LinkWork::Open {
                    kind: freshkube_core::resources::builtin("pods").unwrap(),
                    tab: crate::resources::Tab::Overview,
                },
            ));
        });
    })
    .unwrap();
    cx.wait_for(handle, Duration::from_secs(5), |_, cx| {
        view.read(cx).config_error.is_some()
    })
    .await;
    cx.read(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.registry.active_key(), &key("dev"));
        assert!(pilot.pending_link.is_none());
    });
}
