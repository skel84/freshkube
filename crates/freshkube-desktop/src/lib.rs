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

/// Whether the talosconfig used when none is named exists: `TALOSCONFIG`,
/// else `~/.talos/config`.
pub fn default_talosconfig_exists() -> bool {
    talos_rs::config::TalosConfig::default_path().is_ok_and(|path| path.is_file())
}

/// Checks a maintenance `--endpoint` with the same rule every frontend uses
/// (core's `MaintenanceEndpoint::parse`), before any window opens.
pub fn validate_maintenance_endpoint(endpoint: &str) -> color_eyre::Result<()> {
    freshkube_core::maintenance::validate_maintenance_endpoint(endpoint)
        .map_err(|error| color_eyre::eyre::eyre!(error.to_string()))
}

mod actions;
mod backend;
mod desktop;
mod fixture;
mod logs;
mod maintenance;
mod mutation;
mod palette;
mod presentation;
mod resources;
// Framework pieces land before the screens that use them; drop this once
// every screen is built.
mod screens;
mod state;
mod theme;
mod ui;

pub fn run(options: GpuiOptions, runtime: tokio::runtime::Handle) -> color_eyre::Result<()> {
    desktop::run(options, runtime)
}
