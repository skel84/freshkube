//! Experimental native WebView dashboard. Transport work uses the caller's Tokio runtime.

#[cfg(feature = "desktop")]
mod desktop;
#[cfg(feature = "desktop")]
mod diagnostics;
#[cfg(feature = "desktop")]
mod etcd_workloads;
#[cfg(feature = "desktop")]
mod feature;
#[cfg(feature = "desktop")]
mod logs;
#[cfg(feature = "desktop")]
mod maintenance;
#[cfg(any(feature = "desktop", test))]
mod model;
#[cfg(feature = "desktop")]
mod network;
#[cfg(feature = "desktop")]
mod operations;
#[cfg(feature = "desktop")]
mod processes_storage;
#[cfg(feature = "desktop")]
mod security_lifecycle;

use std::path::PathBuf;

/// CLI settings for the experimental desktop frontend.
#[derive(Debug, Clone, PartialEq)]
pub struct DioxusOptions {
    pub config_path: Option<PathBuf>,
    pub context: Option<String>,
    pub tail: i32,
    pub maintenance_endpoint: Option<String>,
}

/// Runs the native event loop on the calling (main) thread.
///
/// The caller owns the supplied Tokio runtime and must keep it alive for the
/// event loop's entire lifetime. The native desktop backend normally exits the
/// process rather than returning from its event loop, so mutations cannot be
/// drained after this call. Native window-close and the guarded Quit menu wait
/// for the shared mutation lease to finish (including compensation and audit)
/// before permitting the window/root scope to close. Forced process termination
/// is outside that guarantee.
#[cfg(feature = "desktop")]
pub fn run(options: DioxusOptions, runtime: tokio::runtime::Handle) -> color_eyre::Result<()> {
    desktop::run(options, runtime)
}

#[cfg(feature = "desktop")]
pub fn validate_maintenance_endpoint(endpoint: &str) -> color_eyre::Result<()> {
    talos_pilot_core::maintenance::validate_maintenance_endpoint(endpoint)
        .map_err(|error| color_eyre::eyre::eyre!(error.to_string()))
}
