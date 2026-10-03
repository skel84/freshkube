//! Freshkube's desktop application, built on Longbridge GPUI Kit.
//!
//! The caller owns Tokio and keeps it alive for the native event loop.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct GpuiOptions {
    config_path: Option<PathBuf>,
    context: Option<String>,
    tail: i32,
    fixture: bool,
    kubeconfig_path: Option<PathBuf>,
    maintenance_endpoint: Option<String>,
    kubernetes_only: bool,
    kube_context: Option<String>,
    preferences: Option<PathBuf>,
}

impl GpuiOptions {
    pub fn new(config_path: Option<PathBuf>, context: Option<String>, tail: i32) -> Self {
        Self {
            config_path,
            context,
            tail: tail.clamp(0, 1_000),
            fixture: false,
            kubeconfig_path: None,
            maintenance_endpoint: None,
            kubernetes_only: false,
            kube_context: None,
            preferences: None,
        }
    }
    /// Opens without Talos: the Kubernetes pages read `kubeconfig`, or the
    /// files `KUBECONFIG` names, or `~/.kube/config`, connecting to `context`
    /// or else the current context. Talos pages wait for a talosconfig.
    pub fn kubernetes_only(
        kubeconfig: Option<PathBuf>,
        context: Option<String>,
        tail: i32,
    ) -> Self {
        Self {
            kubeconfig_path: kubeconfig,
            kubernetes_only: true,
            kube_context: context,
            ..Self::new(None, None, tail)
        }
    }
    /// Starts with this kubeconfig file selected (its current context), as if
    /// chosen in Settings. It's still checked against the cluster before use.
    pub fn with_kubeconfig(mut self, path: PathBuf) -> Self {
        self.kubeconfig_path = Some(path);
        self
    }
    /// Opens maintenance mode for the node at this endpoint instead of the
    /// cluster shell: insecure Talos access, no talosconfig, no cluster. The
    /// caller validates the endpoint with
    /// `freshkube_core::maintenance::validate_maintenance_endpoint`.
    pub fn with_maintenance_endpoint(mut self, endpoint: String) -> Self {
        self.maintenance_endpoint = Some(endpoint);
        self
    }
    pub fn maintenance_endpoint(&self) -> Option<&str> {
        self.maintenance_endpoint.as_deref()
    }
    pub fn kubeconfig_path(&self) -> Option<&Path> {
        self.kubeconfig_path.as_deref()
    }
    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }
    pub fn context(&self) -> Option<&str> {
        self.context.as_deref()
    }
    pub fn tail(&self) -> i32 {
        self.tail
    }
    /// Synthetic offline demonstration. No credentials or cluster are accessed.
    pub fn fixture() -> Self {
        Self {
            fixture: true,
            ..Self::new(None, None, 100)
        }
    }
    /// Remembers preferences between launches. A saved Talos selection is
    /// restored when no explicit config is supplied; fixture, maintenance and
    /// Kubernetes-only launches keep their requested mode.
    pub fn with_preferences(mut self, path: Option<PathBuf>) -> Self {
        if !self.fixture
            && !self.kubernetes_only
            && self.maintenance_endpoint.is_none()
            && self.config_path.is_none()
            && let Some(saved) = path.as_deref().and_then(connection_preferences::load)
        {
            self.config_path = Some(saved.path);
            if self.context.is_none() {
                self.context = Some(saved.context);
            }
        }
        self.preferences = path;
        self
    }
    pub fn is_fixture(&self) -> bool {
        self.fixture
    }
    pub fn is_kubernetes_only(&self) -> bool {
        self.kubernetes_only
    }
    /// The kubeconfig context Kubernetes-only mode asked for.
    pub fn kube_context(&self) -> Option<&str> {
        self.kube_context.as_deref()
    }
}

/// Whether a Talos launch is available when none is named: a remembered
/// selection, or `TALOSCONFIG` / `~/.talos/config`. A missing remembered file
/// stays in Talos mode so the app reports it and offers a new file picker.
pub fn default_talosconfig_exists() -> bool {
    preferences_path()
        .as_deref()
        .and_then(connection_preferences::load)
        .is_some()
        || talos_rs::config::TalosConfig::default_path().is_ok_and(|path| path.is_file())
}

/// Checks a maintenance `--endpoint` with the same rule every frontend uses
/// (core's `MaintenanceEndpoint::parse`), before any window opens.
pub fn validate_maintenance_endpoint(endpoint: &str) -> color_eyre::Result<()> {
    freshkube_core::maintenance::validate_maintenance_endpoint(endpoint)
        .map_err(|error| color_eyre::eyre::eyre!(error.to_string()))
}

mod actions;
mod backend;
mod connection_preferences;
mod desktop;
mod fixture;
mod forwards;
mod logs;
mod maintenance;
mod monitoring;
mod mutation;
mod palette;
mod perf;
mod presentation;
mod resources;
// Framework pieces land before the screens that use them; drop this once
// every screen is built.
mod screens;
mod state;
#[cfg(feature = "stress")]
mod stress;
mod terminal;
mod text_size;
mod theme;
mod ui;

/// Where the app keeps its preferences:
/// `~/Library/Application Support/Freshkube/preferences.json`.
pub fn preferences_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty())?;
    Some(
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Freshkube")
            .join("preferences.json"),
    )
}

pub fn run(options: GpuiOptions, runtime: tokio::runtime::Handle) -> color_eyre::Result<()> {
    desktop::run(options, runtime)
}

#[cfg(feature = "stress")]
pub use stress::TerminalWorkload;

/// Opens a window with only a terminal, fed a synthetic stream, for the
/// stress harness and visual checks.
#[cfg(feature = "stress")]
pub fn run_terminal(workload: TerminalWorkload) -> color_eyre::Result<()> {
    stress::run_terminal(workload)
}

#[cfg(test)]
mod connection_tests {
    use super::*;
    use crate::connection_preferences::{ConnectionStore, Selection};

    #[test]
    fn explicit_launch_options_override_a_remembered_selection() {
        let directory =
            std::env::temp_dir().join(format!("freshkube-launch-selection-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.json");
        let selected = directory.join("selected-talosconfig");
        let store = ConnectionStore::new(&preferences);
        store.remember(Selection {
            path: selected.clone(),
            context: "remembered".into(),
        });
        store.save_latest().unwrap();

        // Even a missing remembered file stays selected for visible recovery.
        assert!(!selected.exists());
        let restored =
            GpuiOptions::new(None, None, 100).with_preferences(Some(preferences.clone()));
        assert_eq!(restored.config_path(), Some(selected.as_path()));
        assert_eq!(restored.context(), Some("remembered"));
        let named = GpuiOptions::new(None, Some("named".into()), 100)
            .with_preferences(Some(preferences.clone()));
        assert_eq!(named.config_path(), Some(selected.as_path()));
        assert_eq!(named.context(), Some("named"));

        let explicit = directory.join("explicit-talosconfig");
        let named_file = GpuiOptions::new(Some(explicit.clone()), None, 100)
            .with_preferences(Some(preferences.clone()));
        assert_eq!(named_file.config_path(), Some(explicit.as_path()));
        assert_eq!(named_file.context(), None);
        let kube = GpuiOptions::kubernetes_only(None, Some("kube".into()), 100)
            .with_preferences(Some(preferences.clone()));
        assert!(kube.is_kubernetes_only());
        assert_eq!(kube.config_path(), None);
        assert_eq!(kube.kube_context(), Some("kube"));
        let fixture = GpuiOptions::fixture().with_preferences(Some(preferences.clone()));
        assert!(fixture.is_fixture());
        assert_eq!(fixture.config_path(), None);
        let maintenance = GpuiOptions::new(None, None, 100)
            .with_maintenance_endpoint("192.0.2.1".into())
            .with_preferences(Some(preferences));
        assert_eq!(maintenance.maintenance_endpoint(), Some("192.0.2.1"));
        assert_eq!(maintenance.config_path(), None);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
