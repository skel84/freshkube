//! Maintenance mode with example data, for visual checks in debug builds
//! (`--fixture` with `FRESHKUBE_PAGE=maintenance`). An invented node answers
//! from memory: nothing reaches a network or a node. Its configuration is
//! written to a fresh temporary directory, and the view walks itself to the
//! review and stops there: it refuses to apply, bootstrap or poll before any
//! runner is asked.
use std::time::Duration;

use freshkube_core::maintenance::GeneratedConfiguration;
use gpui_kit::component::{Theme, ThemeMode};
use talos_rs::{DiskInfo, InsecureVersionInfo, VolumeStatus};

use super::*;

/// A documentation address (RFC 5737): it names no real node.
const NODE: &str = "192.0.2.10";
/// The disk the example installs to.
const DISK: &str = "/dev/nvme0n1";

impl MaintenanceView {
    /// The view on the example node, walking itself to the review.
    pub(crate) fn example(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // A fixed path, so captures show nothing of the machine they ran on.
        let output = if cfg!(unix) {
            std::path::PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        }
        .join("freshkube-maintenance-example");
        _ = std::fs::remove_dir_all(&output);
        // The shell applies FRESHKUBE_THEME; maintenance mode replaces it.
        // Deferred, as the shell does, so the window's first appearance
        // doesn't override it.
        let mode = match std::env::var("FRESHKUBE_THEME").as_deref() {
            Ok("light") => Some(ThemeMode::Light),
            Ok("dark") => Some(ThemeMode::Dark),
            _ => None,
        };
        if let Some(mode) = mode {
            cx.defer_in(window, move |_, window, cx| {
                Theme::change(mode, Some(window), cx)
            });
        }
        let held = crate::fixture::hold().talos;
        let mut view = Self::with_runner(NODE.into(), runtime, runner(held), window, cx);
        view.example = true;
        let output = output.display().to_string();
        view.fields
            .output
            .update(cx, |input, cx| input.set_value(output.clone(), window, cx));
        view.draft.output = output;
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                match this.update_in(cx, |view, window, cx| view.advance_example(window, cx)) {
                    Ok(false) => {}
                    Ok(true) | Err(_) => break,
                }
            }
        })
        .detach();
        view
    }

    /// Takes the next step towards the review; true once it shows, or once
    /// the session has gone anywhere else.
    pub(super) fn advance_example(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.review.is_some() || self.error.is_some() {
            return true;
        }
        if self.work.is_some() {
            return false;
        }
        match self.session.as_ref().map(|session| &session.phase) {
            None => self.start(window, cx),
            Some(BootstrapPhase::SelectingInstallTarget) => self.select_disk(DISK.into(), cx),
            Some(BootstrapPhase::Configuring) => self.generate(window, cx),
            Some(BootstrapPhase::ConfigurationReady) => self.read_review(window, cx),
            Some(BootstrapPhase::CollectingInsecureData)
            | Some(BootstrapPhase::GeneratingConfiguration) => {}
            Some(_) => return true,
        }
        false
    }
}

/// The example node: it inspects and generates, and refuses everything else.
/// `held` (`FRESHKUBE_FIXTURE_HOLD=talos`) leaves its first read unanswered.
pub(super) fn runner(held: bool) -> Runner {
    Arc::new(move |action, _| {
        Box::pin(async move {
            if held && matches!(action, MaintenanceAction::CollectInsecure { .. }) {
                std::future::pending::<()>().await;
            }
            answer(action)
        })
    })
}

pub(super) fn answer(action: MaintenanceAction) -> MaintenanceEvent {
    match action {
        MaintenanceAction::CollectInsecure { endpoint } => {
            MaintenanceEvent::InsecureDataCollected(InsecureMaintenanceSnapshot {
                endpoint,
                version: SourceAvailability::Available(InsecureVersionInfo {
                    tag: "v1.12.1".into(),
                    maintenance_mode: true,
                }),
                disks: disks(),
                volumes: SourceAvailability::Available(vec![VolumeStatus {
                    id: "STATE".into(),
                    encryption_provider: None,
                    phase: "waiting".into(),
                    size: "0 B".into(),
                    filesystem: None,
                    mount_location: None,
                }]),
            })
        }
        MaintenanceAction::GenerateConfiguration(request) => {
            let path = request
                .plan
                .output_dir
                .join(request.plan.role.configuration_filename());
            let written = std::fs::create_dir_all(&request.plan.output_dir)
                .and_then(|_| std::fs::write(&path, yaml(request.install_target.device_path())));
            match written {
                Ok(()) => {
                    MaintenanceEvent::ConfigurationGenerationFinished(GeneratedConfiguration {
                        machine_config_path: path,
                        talosconfig_path: request.plan.paths.talosconfig_path.clone(),
                        role: request.plan.role,
                        install_target: request.install_target,
                    })
                }
                Err(error) => MaintenanceEvent::OperationFailed(
                    MaintenanceError::ConfigurationGenerationFailed {
                        message: error.to_string(),
                    },
                ),
            }
        }
        // The view refuses these first; the example node refuses them too.
        MaintenanceAction::ApplyConfiguration(_)
        | MaintenanceAction::Bootstrap(_)
        | MaintenanceAction::PollReadiness(_) => MaintenanceEvent::Cancelled,
    }
}

fn disks() -> Vec<DiskInfo> {
    let disk = |id: &str, size: &str, model: &str, readonly: bool, cdrom: bool| DiskInfo {
        id: id.into(),
        dev_path: format!("/dev/{id}"),
        size: 0,
        size_pretty: size.into(),
        model: Some(model.into()),
        serial: Some(format!("EXAMPLE-{}", id.to_uppercase())),
        transport: None,
        rotational: false,
        readonly,
        cdrom,
        wwid: None,
        bus_path: None,
    };
    vec![
        disk("nvme0n1", "512 GB", "Example NVMe", false, false),
        disk("sda", "2.0 TB", "Example SATA", false, false),
        disk("sdb", "64 GB", "Example USB", true, false),
        disk("sr0", "1.0 GB", "Example Optical", false, true),
    ]
}

/// An invented machine configuration. Its long lines wrap in the review.
fn yaml(disk: &str) -> String {
    let fake = "EXAMPLE-NOT-A-SECRET-".repeat(8);
    format!(
        "version: v1alpha1\n\
         machine:\n  type: controlplane\n  token: example.notasecret0000\n  \
         ca:\n    crt: {fake}\n    key: {fake}\n  \
         install:\n    disk: {disk}\n    image: registry.example/talos/installer:v1.12.1\n    wipe: false\n  \
         network:\n    hostname: node-a\n\
         cluster:\n  clusterName: talos-cluster\n  \
         controlPlane:\n    endpoint: https://192.0.2.10:6443\n  \
         secretboxEncryptionSecret: {fake}\n"
    )
}
