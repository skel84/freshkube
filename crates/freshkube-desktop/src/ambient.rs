//! Reads the process environment core depends on, once, for the binaries.
//! Core reads no environment variable itself ([`freshkube_core::ambient`]).
use freshkube_core::ambient::{self, Ambient};

/// Reads `FRESHKUBE_KUBECONFIG`, `KUBECONFIG`, the proxy pair, the user and
/// the home directory from the environment and installs them for core.
pub fn install_ambient_from_env() {
    ambient::install(Ambient::from_lookup(
        |name| std::env::var_os(name),
        std::env::home_dir(),
    ));
}
