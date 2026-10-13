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

/// Installs a hermetic value for a test process: the real proxy pair, since
/// kube reads it and the loopback test servers must be exempted from it, a
/// temporary home, no kubeconfig variables and a fixed user. Tests never read
/// the developer's `KUBECONFIG`, `~/.kube/config` or `~/.freshkube`, and the
/// value is the same whichever test installs it first.
#[cfg(test)]
pub(crate) fn install_for_tests() {
    let real = Ambient::from_lookup(|name| std::env::var_os(name), None);
    let home = std::env::temp_dir().join(format!("freshkube-test-home-{}", std::process::id()));
    ambient::install(Ambient {
        https_proxy: real.https_proxy,
        no_proxy: real.no_proxy,
        home: Some(home.clone()),
        home_variable: Some(home.into_os_string()),
        user: Some("test".into()),
        ..Ambient::default()
    });
}
