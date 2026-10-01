//! Talos Pilot desktop frontend built on Longbridge GPUI Kit.
//!
//! The caller owns Tokio and keeps it alive for the native event loop. Native
//! dependencies are absent unless the `desktop` feature is selected.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct GpuiOptions {
    config_path: Option<PathBuf>,
    context: Option<String>,
    tail: i32,
    fixture: bool,
    kubeconfig_path: Option<PathBuf>,
    maintenance_endpoint: Option<String>,
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
    /// `talos_pilot_core::maintenance::validate_maintenance_endpoint`.
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
}

/// Checks a maintenance `--endpoint` with the same rule every frontend uses
/// (core's `MaintenanceEndpoint::parse`), before any window opens.
pub fn validate_maintenance_endpoint(endpoint: &str) -> color_eyre::Result<()> {
    talos_pilot_core::maintenance::validate_maintenance_endpoint(endpoint)
        .map_err(|error| color_eyre::eyre::eyre!(error.to_string()))
}

#[cfg(feature = "desktop")]
mod actions;
#[cfg(feature = "desktop")]
mod backend;
#[cfg(feature = "desktop")]
mod desktop;
#[cfg(feature = "desktop")]
mod fixture;
#[cfg(feature = "desktop")]
mod logs;
#[cfg(feature = "desktop")]
mod maintenance;
#[cfg(feature = "desktop")]
mod mutation;
#[cfg(feature = "desktop")]
mod palette;
#[cfg(feature = "desktop")]
mod presentation;
// Framework pieces land before the screens that use them; drop this once
// every screen is built.
#[cfg(feature = "desktop")]
mod screens;
#[cfg(feature = "desktop")]
mod state;
#[cfg(feature = "desktop")]
mod theme;
#[cfg(feature = "desktop")]
mod ui;

#[cfg(feature = "desktop")]
pub fn run(options: GpuiOptions, runtime: tokio::runtime::Handle) -> color_eyre::Result<()> {
    desktop::run(options, runtime)
}
