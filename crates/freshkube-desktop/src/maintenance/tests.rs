//! Offline UI tests. Every node interaction goes through the injected
//! [`Runner`], so no test touches a network or a node. The "generated"
//! configuration is a harmless fake written to a throwaway directory.

// Not `super::*`: it brings gpui_kit's `test` macro over the built-in one.
use super::{Cancelled, MaintenanceView, Runner};
use crate::mutation::{self, Operations};
use freshkube_core::maintenance::{
    BootstrapCommandOutcome, BootstrapPhase, BootstrapReadinessEvidence,
    ConfigurationApplicationOutcome, EtcdReadinessEvidence, GeneratedConfiguration,
    InsecureMaintenanceSnapshot, InstallTarget, KubernetesReadinessEvidence, MachineRole,
    MaintenanceAction, MaintenanceError, MaintenanceEvent, SourceAvailability,
    TalosReadinessEvidence, TalosVersionEvidence,
};
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, TestAppContext,
    component::{Root, Theme, ThemeMode},
    px, size,
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use talos_rs::{DiskInfo, InsecureVersionInfo, VolumeStatus};

const NODE: &str = "192.0.2.10";
const WAIT: Duration = Duration::from_secs(10);

/// A scripted stand-in for the node. It records which actions reached it.
#[derive(Clone)]
struct World {
    calls: Arc<Mutex<Vec<&'static str>>>,
    directory: PathBuf,
    version_unknown: bool,
    volumes_unknown: bool,
    fail_collection: bool,
    /// Bootstrap waits until released or cancelled.
    hold_bootstrap: bool,
    release_bootstrap: Arc<AtomicBool>,
}

impl World {
    fn new(name: &str) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "freshkube-desktop-maintenance-{}-{name}",
            std::process::id()
        ));
        _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        Self {
            calls: Arc::default(),
            directory,
            version_unknown: false,
            volumes_unknown: false,
            fail_collection: false,
            hold_bootstrap: false,
            release_bootstrap: Arc::default(),
        }
    }

    fn count(&self, name: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| **call == name)
            .count()
    }

    fn log(&self, name: &'static str) {
        self.calls.lock().unwrap().push(name);
    }

    fn disks() -> Vec<DiskInfo> {
        let disk = |id: &str, readonly: bool, cdrom: bool| DiskInfo {
            id: id.into(),
            dev_path: format!("/dev/{id}"),
            size: 100_000_000_000,
            size_pretty: "100 GB".into(),
            model: Some("Example Disk".into()),
            serial: None,
            transport: None,
            rotational: false,
            readonly,
            cdrom,
            wwid: None,
            bus_path: None,
        };
        vec![
            disk("sda", false, false),
            disk("sr0", false, true),
            disk("sdb", true, false),
        ]
    }

    /// A harmless stand-in for a generated machine configuration.
    fn yaml(role: MachineRole, disk: &str) -> String {
        let role = match role {
            MachineRole::ControlPlane => "controlplane",
            MachineRole::Worker => "worker",
        };
        format!(
            "machine:\n  type: {role}\n  install:\n    disk: {disk}\n  token: fake-test-token\n"
        )
    }

    async fn execute(&self, action: MaintenanceAction, cancelled: Cancelled) -> MaintenanceEvent {
        match action {
            MaintenanceAction::CollectInsecure { endpoint } => {
                self.log("collect");
                if self.fail_collection {
                    return MaintenanceEvent::OperationFailed(
                        MaintenanceError::InsecureDiskCollectionFailed {
                            message: "connection refused".into(),
                        },
                    );
                }
                MaintenanceEvent::InsecureDataCollected(InsecureMaintenanceSnapshot {
                    endpoint,
                    version: if self.version_unknown {
                        SourceAvailability::Unavailable {
                            reason: "version API unavailable".into(),
                        }
                    } else {
                        SourceAvailability::Available(InsecureVersionInfo {
                            tag: "v1.12.1".into(),
                            maintenance_mode: true,
                        })
                    },
                    disks: Self::disks(),
                    volumes: if self.volumes_unknown {
                        SourceAvailability::Unavailable {
                            reason: "volume API unavailable".into(),
                        }
                    } else {
                        SourceAvailability::Available(vec![VolumeStatus {
                            id: "STATE".into(),
                            encryption_provider: None,
                            phase: "waiting".into(),
                            size: "0 B".into(),
                            filesystem: None,
                            mount_location: None,
                        }])
                    },
                })
            }
            MaintenanceAction::GenerateConfiguration(request) => {
                self.log("generate");
                let path = request
                    .plan
                    .output_dir
                    .join(request.plan.role.configuration_filename());
                assert!(request.plan.output_dir.starts_with(&self.directory));
                std::fs::create_dir_all(&request.plan.output_dir).unwrap();
                std::fs::write(
                    &path,
                    Self::yaml(request.plan.role, request.install_target.device_path()),
                )
                .unwrap();
                MaintenanceEvent::ConfigurationGenerationFinished(GeneratedConfiguration {
                    machine_config_path: path,
                    talosconfig_path: request.plan.paths.talosconfig_path.clone(),
                    role: request.plan.role,
                    install_target: request.install_target,
                })
            }
            MaintenanceAction::ApplyConfiguration(_) => {
                self.log("apply");
                MaintenanceEvent::ConfigurationApplicationFinished(
                    ConfigurationApplicationOutcome {
                        applied: true,
                        message: "Configuration accepted (test double)".into(),
                    },
                )
            }
            MaintenanceAction::Bootstrap(_) => {
                self.log("bootstrap");
                while self.hold_bootstrap && !self.release_bootstrap.load(Ordering::SeqCst) {
                    if cancelled() {
                        return MaintenanceEvent::Cancelled;
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                MaintenanceEvent::BootstrapFinished(BootstrapCommandOutcome {
                    started: true,
                    message: "Bootstrap accepted (test double)".into(),
                })
            }
            MaintenanceAction::PollReadiness(request) => {
                self.log("poll");
                MaintenanceEvent::ReadinessPolled(BootstrapReadinessEvidence {
                    target: request.target,
                    // The Talos API answers; etcd and Kubernetes say nothing.
                    talos: TalosReadinessEvidence::Available {
                        versions: vec![TalosVersionEvidence {
                            node: NODE.into(),
                            version: "v1.12.1".into(),
                        }],
                        etcd: EtcdReadinessEvidence::Unavailable {
                            reason: "etcd API not answering yet".into(),
                        },
                    },
                    kubernetes: KubernetesReadinessEvidence::NotChecked,
                })
            }
        }
    }

    fn runner(&self) -> Runner {
        let world = self.clone();
        Arc::new(move |action, cancelled| {
            let world = world.clone();
            Box::pin(async move { world.execute(action, cancelled).await })
        })
    }
}

impl Drop for World {
    fn drop(&mut self) {
        // Clones share the directory; the last test thread cleans up.
        if Arc::strong_count(&self.calls) == 1 {
            _ = std::fs::remove_dir_all(&self.directory);
        }
    }
}

type Mounted = (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<MaintenanceView>,
);

fn mount(cx: &mut TestAppContext, world: &World, endpoint: &str) -> Mounted {
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        Theme::change(ThemeMode::Light, None, cx);
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut view = None;
    let window = cx.open_window(size(px(1400.), px(2600.)), |window, cx| {
        let created = cx.new(|cx| {
            MaintenanceView::with_runner(
                endpoint.to_owned(),
                runtime.handle().clone(),
                world.runner(),
                window,
                cx,
            )
        });
        view = Some(created.clone());
        Root::new(created, window, cx)
    });
    cx.run_until_parked();
    (runtime, window.into(), view.unwrap())
}

/// Types like a user: focus the field, replace its text, let events settle.
fn set_input(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str, value: &str) {
    let select_all = if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    };
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
        window.press(select_all, cx);
        if value.is_empty() {
            window.press("backspace", cx);
        } else {
            window.input(value, cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
}

fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
        window.render_frame(cx);
    })
    .unwrap();
}

fn phase(cx: &mut TestAppContext, view: &Entity<MaintenanceView>) -> Option<BootstrapPhase> {
    cx.update(|cx| view.read(cx).session.as_ref().map(|s| s.phase.clone()))
}

async fn wait_phase(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<MaintenanceView>,
    expected: BootstrapPhase,
) {
    let view = view.clone();
    cx.wait_for(handle, WAIT, move |_, cx| {
        view.read(cx).session.as_ref().map(|s| &s.phase) == Some(&expected)
    })
    .await;
}

fn label(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) -> String {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .find(id)
            .label()
            .map(str::to_owned)
            .unwrap_or_default()
    })
    .unwrap()
}

fn present(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

fn use_output_directory(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<MaintenanceView>,
    world: &World,
) {
    set_input(
        cx,
        handle,
        "maint-output",
        world.directory.to_str().unwrap(),
    );
    // Guard: a silent no-op here would let the fake write under the cwd.
    cx.update(|cx| {
        assert!(view.read(cx).draft.output.as_str() == world.directory.to_str().unwrap());
    });
}

/// Connects and inspects, leaving the session choosing an install disk.
async fn inspect(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<MaintenanceView>,
    world: &World,
) {
    use_output_directory(cx, handle, view, world);
    click(cx, handle, "maint-start");
    wait_phase(cx, handle, view, BootstrapPhase::SelectingInstallTarget).await;
}

/// Selects `/dev/sda`, generates and reviews: ends awaiting confirmation.
async fn reach_apply_confirmation(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<MaintenanceView>,
    world: &World,
) {
    inspect(cx, handle, view, world).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("maint-disk-select", 0usize), cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(phase(cx, view), Some(BootstrapPhase::Configuring));
    click(cx, handle, "maint-generate");
    wait_phase(cx, handle, view, BootstrapPhase::ConfigurationReady).await;
    click(cx, handle, "maint-review-read");
    let watched = view.clone();
    cx.wait_for(handle, WAIT, move |_, cx| watched.read(cx).review.is_some())
        .await;
    click(cx, handle, "maint-review-request");
    assert_eq!(
        phase(cx, view),
        Some(BootstrapPhase::AwaitingApplyConfirmation)
    );
}

/// Opens a confirmation dialog from `button` and waits for its animation.
fn open_confirmation(cx: &mut TestAppContext, handle: AnyWindowHandle, button: &'static str) {
    click(cx, handle, button);
    crate::mutation::settle_confirmation(cx, handle);
}

/// Applies and waits for the node to be waiting for its secure API.
async fn apply(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<MaintenanceView>) {
    open_confirmation(cx, handle, "maint-apply");
    click(cx, handle, "confirm-ok");
    wait_phase(cx, handle, view, BootstrapPhase::WaitingForTalos).await;
}

fn slot_free(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| Operations::current(cx).is_none())
}

#[gpui_kit::test]
async fn the_endpoint_is_validated_before_any_node_is_contacted(cx: &mut TestAppContext) {
    let world = World::new("endpoint");
    let (_runtime, handle, view) = mount(cx, &world, NODE);
    // The endpoint from the command line fills the form.
    cx.update(|cx| {
        assert_eq!(view.read(cx).draft.endpoint, NODE);
        assert_eq!(view.read(cx).draft.bootstrap_node, NODE);
        assert_eq!(
            view.read(cx).draft.kubernetes_api,
            "https://192.0.2.10:6443"
        );
    });
    for bad in [
        "",
        "node.example:0",
        "https://node.example/path",
        "not a valid endpoint",
    ] {
        set_input(cx, handle, "maint-endpoint", bad);
        click(cx, handle, "maint-start");
        let seen = cx.update(|cx| view.read(cx).draft.endpoint.clone());
        assert!(present(cx, handle, "maint-error"), "{bad:?} draft={seen:?}");
        assert!(phase(cx, &view).is_none(), "{bad:?}");
    }
    // A different node than the one to bootstrap is also refused up front.
    set_input(cx, handle, "maint-endpoint", "192.0.2.11");
    click(cx, handle, "maint-start");
    assert!(label(cx, handle, "maint-error").contains("does not match"));
    assert_eq!(world.count("collect"), 0);
    // The same node with a port is the same node.
    set_input(cx, handle, "maint-endpoint", "192.0.2.10:50000");
    use_output_directory(cx, handle, &view, &world);
    click(cx, handle, "maint-start");
    wait_phase(cx, handle, &view, BootstrapPhase::SelectingInstallTarget).await;
    assert_eq!(world.count("collect"), 1);
    assert!(!present(cx, handle, "maint-error"));
    // Once connected the form is frozen: connecting again does nothing.
    click(cx, handle, "maint-start");
    assert_eq!(world.count("collect"), 1);
}

#[gpui_kit::test]
async fn an_unanswered_source_shows_as_unknown_not_failed(cx: &mut TestAppContext) {
    let mut world = World::new("unknown");
    world.version_unknown = true;
    world.volumes_unknown = true;
    let (_runtime, handle, view) = mount(cx, &world, NODE);
    inspect(cx, handle, &view, &world).await;
    let version = label(cx, handle, "maint-version");
    let volumes = label(cx, handle, "maint-volumes");
    assert!(
        version.contains("Unknown") && version.contains("version API unavailable"),
        "{version}"
    );
    assert!(
        volumes.contains("Unknown") && volumes.contains("volume API unavailable"),
        "{volumes}"
    );
    // Nothing is reported as an error, and disks stay selectable.
    assert!(!present(cx, handle, "maint-error"));
    assert!(label(cx, handle, "maint-phase").contains("Choose an install disk"));
    // Optical and read-only disks can't be chosen.
    for ix in [1usize, 2] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(("maint-disk-select", ix), cx);
        })
        .unwrap();
    }
    assert_eq!(
        phase(cx, &view),
        Some(BootstrapPhase::SelectingInstallTarget)
    );
    assert_eq!(world.count("generate"), 0);
}

#[gpui_kit::test]
async fn a_node_that_cannot_be_inspected_is_reported_and_nothing_proceeds(cx: &mut TestAppContext) {
    let mut world = World::new("unreachable");
    world.fail_collection = true;
    let (_runtime, handle, view) = mount(cx, &world, NODE);
    use_output_directory(cx, handle, &view, &world);
    click(cx, handle, "maint-start");
    wait_phase(
        cx,
        handle,
        &view,
        BootstrapPhase::Failed(MaintenanceError::InsecureDiskCollectionFailed {
            message: "connection refused".into(),
        }),
    )
    .await;
    assert!(label(cx, handle, "maint-phase").contains("Failed"));
    assert!(label(cx, handle, "maint-message").contains("connection refused"));
    assert!(!present(cx, handle, "maint-snapshot"));
    assert!(!present(cx, handle, "maint-generate"));
    assert!(slot_free(cx));
    // Reset returns to the form, which can try again.
    click(cx, handle, "maint-reset");
    assert!(phase(cx, &view).is_none());
}

#[gpui_kit::test]
async fn apply_needs_an_explicit_confirmation_of_the_reviewed_yaml(cx: &mut TestAppContext) {
    let world = World::new("apply");
    let (_runtime, handle, view) = mount(cx, &world, NODE);
    reach_apply_confirmation(cx, handle, &view, &world).await;
    // Reaching confirmation changed nothing on the node.
    assert_eq!(world.count("apply"), 0);
    // What was reviewed is exactly what the node's config holds.
    let expected = World::yaml(MachineRole::ControlPlane, "/dev/sda");
    cx.update(|cx| {
        let review = view.read(cx).review.as_ref().unwrap();
        // Not assert_eq!: this stands in for secrets and must not print.
        assert!(review.text == expected);
    });
    assert!(present(cx, handle, "maint-review"));

    open_confirmation(cx, handle, "maint-apply");
    let title = label(cx, handle, "confirm-dialog");
    assert!(title.contains("Install Talos on 192.0.2.10"), "{title}");
    assert_eq!(world.count("apply"), 0);
    click(cx, handle, "confirm-cancel");
    cx.run_until_parked();
    assert_eq!(world.count("apply"), 0);
    assert_eq!(
        phase(cx, &view),
        Some(BootstrapPhase::AwaitingApplyConfirmation)
    );
    assert!(slot_free(cx));

    open_confirmation(cx, handle, "maint-apply");
    click(cx, handle, "confirm-ok");
    wait_phase(cx, handle, &view, BootstrapPhase::WaitingForTalos).await;
    assert_eq!(world.count("apply"), 1);
    assert!(slot_free(cx));
    // Reviewed secrets are dropped once they have been sent.
    cx.update(|cx| assert!(view.read(cx).review.is_none()));
    assert!(!present(cx, handle, "maint-review"));
    assert!(label(cx, handle, "maint-progress").contains("WaitingForTalos"));
}

#[gpui_kit::test]
async fn the_review_draws_each_row_as_a_document_line(cx: &mut TestAppContext) {
    let world = World::new("review-lines");
    let (_runtime, handle, view) = mount(cx, &world, NODE);
    reach_apply_confirmation(cx, handle, &view, &world).await;
    // No line of the fake configuration is long enough to wrap.
    let rows = World::yaml(MachineRole::ControlPlane, "/dev/sda")
        .lines()
        .count();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for ix in 0..rows {
            let line = window.find(("maint-review-line", ix)).bounds();
            assert_eq!(line.size.height, px(freshkube_ui::document::LINE_HEIGHT));
        }
        assert!(window.try_find(("maint-review-line", rows)).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_stale_apply_preview_is_refused_and_sends_nothing(cx: &mut TestAppContext) {
    let world = World::new("stale");
    let (_runtime, handle, view) = mount(cx, &world, NODE);
    reach_apply_confirmation(cx, handle, &view, &world).await;
    open_confirmation(cx, handle, "maint-apply");
    // The workflow moved on while the dialog was open.
    view.update(cx, |view, cx| view.reset(cx));
    cx.update_window(handle, |_, window, cx| {
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert!(window.find("confirm-blocked").visible());
        assert!(window.find("confirm-dialog").visible());
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(world.count("apply"), 0);
    assert!(slot_free(cx));
}

#[gpui_kit::test]
async fn a_busy_operation_slot_refuses_apply_until_it_is_free(cx: &mut TestAppContext) {
    let world = World::new("busy");
    let (_runtime, handle, view) = mount(cx, &world, NODE);
    reach_apply_confirmation(cx, handle, &view, &world).await;
    open_confirmation(cx, handle, "maint-apply");
    let operations = cx.update(Operations::global);
    let ticket = operations
        .update(cx, |operations, cx| {
            operations.begin("Reboot another-node", cx)
        })
        .ok()
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert!(window.find("confirm-blocked").visible());
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(world.count("apply"), 0);
    // The holder of the slot is untouched.
    cx.update(|cx| {
        assert_eq!(
            Operations::current(cx)
                .map(|running| running.label.to_string())
                .as_deref(),
            Some("Reboot another-node")
        );
    });
    operations.update(cx, |operations, cx| operations.finish(ticket, cx));
    click(cx, handle, "confirm-ok");
    wait_phase(cx, handle, &view, BootstrapPhase::WaitingForTalos).await;
    assert_eq!(world.count("apply"), 1);
    assert!(slot_free(cx));
}

#[gpui_kit::test]
async fn connecting_is_refused_while_another_operation_runs(cx: &mut TestAppContext) {
    let world = World::new("start-busy");
    let (_runtime, handle, view) = mount(cx, &world, NODE);
    use_output_directory(cx, handle, &view, &world);
    let operations = cx.update(Operations::global);
    let ticket = operations
        .update(cx, |operations, cx| operations.begin("Restart etcd", cx))
        .ok()
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.start(window, cx));
    })
    .unwrap();
    assert_eq!(world.count("collect"), 0);
    assert!(label(cx, handle, "maint-error").contains("Restart etcd is still running"));
    operations.update(cx, |operations, cx| operations.finish(ticket, cx));
    click(cx, handle, "maint-start");
    wait_phase(cx, handle, &view, BootstrapPhase::SelectingInstallTarget).await;
}

#[gpui_kit::test]
async fn bootstrap_shows_progress_blocks_closing_and_cancel_frees_the_slot(
    cx: &mut TestAppContext,
) {
    let mut world = World::new("bootstrap");
    world.hold_bootstrap = true;
    let (_runtime, handle, view) = mount(cx, &world, NODE);
    reach_apply_confirmation(cx, handle, &view, &world).await;
    apply(cx, handle, &view).await;
    // Bootstrap isn't offered until the secure Talos API has answered.
    assert!(!present(cx, handle, "maint-bootstrap"));
    click(cx, handle, "maint-poll");
    wait_phase(cx, handle, &view, BootstrapPhase::ReadyToBootstrap).await;
    assert_eq!(world.count("bootstrap"), 0);

    open_confirmation(cx, handle, "maint-bootstrap");
    assert!(label(cx, handle, "confirm-dialog").contains("Bootstrap etcd on 192.0.2.10"));
    click(cx, handle, "confirm-ok");
    assert_eq!(phase(cx, &view), Some(BootstrapPhase::Bootstrapping));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let running = Operations::current(cx).expect("bootstrap holds the slot");
        assert!(running.label.contains("bootstrapping"));
        assert!(
            window
                .find("maint-status")
                .label()
                .unwrap()
                .contains("bootstrapping")
        );
        // The window refuses to close under a half-done change.
        assert!(!mutation::may_close(window, cx));
        // Nothing else can start meanwhile.
        assert!(
            window
                .find("maint-progress")
                .label()
                .unwrap()
                .contains("Bootstrapping")
        );
    })
    .unwrap();
    click(cx, handle, "maint-reset");
    click(cx, handle, "maint-poll");
    assert_eq!(phase(cx, &view), Some(BootstrapPhase::Bootstrapping));
    assert_eq!(world.count("poll"), 1);
    // The prompt that explains the refusal can be dismissed.
    cx.simulate_prompt_answer("Keep running");

    click(cx, handle, "maint-cancel");
    wait_phase(cx, handle, &view, BootstrapPhase::Cancelled).await;
    cx.wait_for(handle, WAIT, |_, cx| Operations::current(cx).is_none())
        .await;
    assert!(slot_free(cx));
    assert!(label(cx, handle, "maint-phase").contains("Cancelled"));
    cx.update_window(handle, |_, window, cx| {
        assert!(mutation::may_close(window, cx));
    })
    .unwrap();
    cx.update(|cx| assert!(!view.read(cx).auto_poll));
}

#[gpui_kit::test]
async fn readiness_that_is_not_proven_is_unknown_and_never_complete(cx: &mut TestAppContext) {
    let world = World::new("readiness");
    let (_runtime, handle, view) = mount(cx, &world, NODE);
    reach_apply_confirmation(cx, handle, &view, &world).await;
    apply(cx, handle, &view).await;
    click(cx, handle, "maint-poll");
    wait_phase(cx, handle, &view, BootstrapPhase::ReadyToBootstrap).await;
    open_confirmation(cx, handle, "maint-bootstrap");
    click(cx, handle, "confirm-ok");
    wait_phase(cx, handle, &view, BootstrapPhase::WaitingForKubernetes).await;
    assert_eq!(world.count("bootstrap"), 1);
    click(cx, handle, "maint-poll");
    let watched = view.clone();
    cx.wait_for(handle, WAIT, move |_, cx| {
        watched
            .read(cx)
            .session
            .as_ref()
            .is_some_and(|s| s.poll_attempts >= 2)
    })
    .await;
    // etcd and Kubernetes gave no evidence: unknown, so not complete.
    assert_eq!(phase(cx, &view), Some(BootstrapPhase::WaitingForKubernetes));
    let evidence = label(cx, handle, "maint-readiness");
    assert!(evidence.contains("etcd: Unknown"), "{evidence}");
    assert!(
        evidence.contains("Kubernetes: Not checked yet"),
        "{evidence}"
    );
    assert!(!present(cx, handle, "maint-error"));
    assert!(slot_free(cx));
}

#[gpui_kit::test]
async fn waiting_for_the_secure_api_polls_on_a_timer(cx: &mut TestAppContext) {
    let world = World::new("timer");
    let (_runtime, handle, view) = mount(cx, &world, NODE);
    reach_apply_confirmation(cx, handle, &view, &world).await;
    apply(cx, handle, &view).await;
    assert_eq!(world.count("poll"), 0);
    cx.executor()
        .advance_clock(super::BOOTSTRAP_POLL_INTERVAL + Duration::from_secs(1));
    wait_phase(cx, handle, &view, BootstrapPhase::ReadyToBootstrap).await;
    assert_eq!(world.count("poll"), 1);
    // Polling stops by itself once there is nothing to wait for.
    cx.executor()
        .advance_clock(super::BOOTSTRAP_POLL_INTERVAL * 2);
    cx.run_until_parked();
    assert_eq!(world.count("poll"), 1);
}

#[gpui_kit::test]
async fn a_worker_is_never_offered_bootstrap(cx: &mut TestAppContext) {
    let world = World::new("worker");
    let (_runtime, handle, view) = mount(cx, &world, NODE);
    click(cx, handle, "maint-role-worker");
    reach_apply_confirmation(cx, handle, &view, &world).await;
    apply(cx, handle, &view).await;
    click(cx, handle, "maint-poll");
    wait_phase(cx, handle, &view, BootstrapPhase::ReadyToBootstrap).await;
    assert!(!present(cx, handle, "maint-bootstrap"));
    assert!(present(cx, handle, "maint-worker-note"));
    assert_eq!(world.count("bootstrap"), 0);
    cx.update(|cx| {
        assert!(view.read(cx).bootstrap_fingerprint().is_none());
    });
}

#[gpui_kit::test]
async fn changing_the_form_after_connecting_is_not_possible(cx: &mut TestAppContext) {
    let world = World::new("locked");
    let (_runtime, handle, view) = mount(cx, &world, NODE);
    inspect(cx, handle, &view, &world).await;
    click(cx, handle, "maint-start");
    assert_eq!(world.count("collect"), 1);
    click(cx, handle, "maint-role-worker");
    cx.update(|cx| {
        assert_eq!(view.read(cx).draft.role, MachineRole::ControlPlane);
    });
    // The chosen disk is mandatory before anything can be generated.
    assert!(!present(cx, handle, "maint-generate"));
    let target = cx.update(|cx| {
        view.read(cx)
            .session
            .as_ref()
            .unwrap()
            .install_target
            .clone()
    });
    assert!(target.is_none());
    let _ = InstallTarget::select(&World::disks(), "/dev/sda").unwrap();
}
