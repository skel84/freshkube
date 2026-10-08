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
    /// The kubeconfig and context came from the remembered choice, not from
    /// the command line; later context changes are remembered too.
    remembered_kubernetes: bool,
    /// The command line named a talosconfig, a Talos context, a kubeconfig or
    /// a kubeconfig context; set before a remembered choice fills any in.
    named_source: bool,
    preferences: Option<PathBuf>,
    keyring: bool,
    /// Example data holds the Talos overview and the summary
    /// (`fixture::hold`).
    hold_talos: bool,
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
            remembered_kubernetes: false,
            named_source: false,
            preferences: None,
            keyring: false,
            hold_talos: false,
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
            hold_talos: fixture::hold().talos,
            ..Self::new(None, None, 100)
        }
    }

    /// Example data that never answers Talos, as `FRESHKUBE_FIXTURE_HOLD=talos`.
    #[cfg(test)]
    pub(crate) fn holding_talos(mut self) -> Self {
        self.hold_talos = true;
        self
    }
    /// Remembers preferences between launches. A saved Talos selection is
    /// restored when no explicit config is supplied. A saved Kubernetes-only
    /// choice is restored to a Kubernetes-only launch that names neither a
    /// kubeconfig nor a context; fixture and maintenance launches keep their
    /// requested mode.
    ///
    /// A terminal's `KUBECONFIG` beats the remembered kubeconfig, as
    /// `TALOSCONFIG` beats the remembered talosconfig.
    pub fn with_preferences(self, path: Option<PathBuf>) -> Self {
        let environment = std::env::var_os("KUBECONFIG").is_some_and(|value| !value.is_empty());
        self.with_preferences_in(path, environment)
    }

    fn with_preferences_in(mut self, path: Option<PathBuf>, kubeconfig_environment: bool) -> Self {
        self.named_source = self.config_path.is_some()
            || self.context.is_some()
            || self.kubeconfig_path.is_some()
            || self.kube_context.is_some();
        if !self.fixture && self.maintenance_endpoint.is_none() {
            match path
                .as_deref()
                .and_then(connection_preferences::load_remembered)
            {
                Some(connection_preferences::Remembered::Talos(saved))
                    if !self.kubernetes_only && self.config_path.is_none() =>
                {
                    self.config_path = Some(saved.path);
                    if self.context.is_none() {
                        self.context = Some(saved.context);
                    }
                }
                Some(connection_preferences::Remembered::Kubernetes(saved))
                    if self.kubernetes_only
                        && !kubeconfig_environment
                        && self.kubeconfig_path.is_none()
                        && self.kube_context.is_none() =>
                {
                    self.kubeconfig_path = Some(saved.path);
                    self.kube_context = Some(saved.context);
                    self.remembered_kubernetes = true;
                }
                _ => {}
            }
        }
        self.preferences = path;
        self
    }
    /// Keeps the keys the user asks to remember in the system's credential
    /// store. Off by default, so tests never touch it.
    pub fn with_keyring(mut self) -> Self {
        self.keyring = true;
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
    /// Whether the kubeconfig and context were restored from the remembered
    /// choice rather than named on the command line.
    pub fn restored_kubernetes(&self) -> bool {
        self.remembered_kubernetes
    }
}

/// Whether a Talos launch is available when none is named: a remembered
/// selection, or `TALOSCONFIG` / `~/.talos/config`. A missing remembered file
/// stays in Talos mode so the app reports it and offers a new file picker. A
/// remembered Kubernetes-only choice means Talos is not wanted.
pub fn default_talosconfig_exists() -> bool {
    talos_available(preferences_path().as_deref(), || {
        talos_rs::config::TalosConfig::default_path()
            .is_ok_and(|path| default_talosconfig_present(&path))
    })
}

fn talos_available(preferences: Option<&Path>, default_file: impl FnOnce() -> bool) -> bool {
    match preferences.and_then(connection_preferences::load_remembered) {
        Some(connection_preferences::Remembered::Talos(_)) => true,
        Some(connection_preferences::Remembered::Kubernetes(_)) => false,
        None => default_file(),
    }
}

/// Largest default talosconfig read when deciding whether it counts.
const MAX_DEFAULT_TALOSCONFIG_BYTES: u64 = 4 * 1024 * 1024;

/// Whether the default talosconfig at `path` is a file to use. A missing file
/// is absent, and so is a stub with no contexts to choose from, which
/// `talosctl` can leave behind. A file that can't be read or parsed counts as
/// present: it may be a real config the user needs to fix, and Talos mode says
/// what is wrong with it.
fn default_talosconfig_present(path: &std::path::Path) -> bool {
    match freshkube_core::read_bounded_regular_file(path, MAX_DEFAULT_TALOSCONFIG_BYTES) {
        Ok(bytes) => !std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| talos_rs::config::TalosConfig::parse(text).ok())
            .is_some_and(|config| config.contexts.is_empty()),
        Err(freshkube_core::BoundedReadError::NotFound) => false,
        Err(_) => path.is_file(),
    }
}

#[cfg(test)]
mod default_talosconfig_tests {
    use super::default_talosconfig_present;
    use std::path::PathBuf;

    const VALID: &str = "context: alpha\ncontexts:\n  alpha:\n    endpoints: [\"192.0.2.10\"]\n    ca: Y2E=\n    crt: Y3J0\n    key: a2V5\n";

    fn write(dir: &tempfile::TempDir, text: &str) -> PathBuf {
        let path = dir.path().join("config");
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn a_stub_with_no_contexts_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!default_talosconfig_present(&write(
            &dir,
            "context: \"\"\ncontexts: {}\n"
        )));
        assert!(!default_talosconfig_present(&write(
            &dir,
            "context: alpha\ncontexts: {}\n"
        )));
        assert!(!default_talosconfig_present(&write(
            &dir,
            "context: \"\"\ncontexts:\n"
        )));
    }

    #[test]
    fn a_missing_file_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!default_talosconfig_present(&dir.path().join("config")));
    }

    #[test]
    fn a_valid_file_is_present() {
        let dir = tempfile::tempdir().unwrap();
        assert!(default_talosconfig_present(&write(&dir, VALID)));
    }

    #[test]
    fn an_unparsable_file_is_present() {
        let dir = tempfile::tempdir().unwrap();
        assert!(default_talosconfig_present(&write(
            &dir,
            "context: [oops\n"
        )));
        assert!(default_talosconfig_present(&write(
            &dir,
            "not: a talosconfig\n"
        )));
    }
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
mod mutation;
mod navigation_file;
mod presentation;
mod resources;
// Framework pieces land before the screens that use them; drop this once
// every screen is built.
mod screens;
mod secrets;
mod state;
mod stream_status;
#[cfg(feature = "stress")]
mod stress;
mod ui;

use freshkube_probe::perf;
// Monitoring's dashboards and history charts, by their old path.
use freshkube_monitoring as monitoring;
// Observability's Coroot pages, by their old path.
use freshkube_observability as observability;
// The look lives in freshkube-ui; the app reaches it by its old paths.
use freshkube_ui::{meters, palette, text_size, theme};
// The pod shell's terminal view, by its old path.
use freshkube_terminal as terminal;

/// Where the app keeps its preferences: `preferences.json` in
/// `~/Library/Application Support/Freshkube` on macOS, `~/.config/freshkube`
/// on Linux and `%APPDATA%\Freshkube` on Windows.
pub fn preferences_path() -> Option<PathBuf> {
    let folder = if cfg!(any(target_os = "macos", windows)) {
        "Freshkube"
    } else {
        "freshkube"
    };
    Some(dirs::config_dir()?.join(folder).join("preferences.json"))
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
    use crate::connection_preferences::{ConnectionStore, KubeSelection, Selection};

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

    #[test]
    fn a_remembered_kubernetes_choice_opens_kubernetes_only_and_explicit_options_win() {
        let directory = std::env::temp_dir().join(format!(
            "freshkube-launch-kubernetes-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.json");
        let chosen = directory.join("kubeconfig");
        let store = ConnectionStore::new(&preferences);
        store.remember_kubernetes(KubeSelection {
            path: chosen.clone(),
            context: "example".into(),
        });
        store.save_latest().unwrap();

        // The remembered choice stands in for the Talos default.
        assert!(!talos_available(Some(&preferences), || true));
        let restored = GpuiOptions::kubernetes_only(None, None, 100)
            .with_preferences_in(Some(preferences.clone()), false);
        assert_eq!(restored.kubeconfig_path(), Some(chosen.as_path()));
        assert_eq!(restored.kube_context(), Some("example"));
        assert!(restored.restored_kubernetes());

        // A terminal's KUBECONFIG beats the remembered choice, as TALOSCONFIG
        // beats the remembered talosconfig.
        let environment = GpuiOptions::kubernetes_only(None, None, 100)
            .with_preferences_in(Some(preferences.clone()), true);
        assert_eq!(environment.kubeconfig_path(), None);
        assert_eq!(environment.kube_context(), None);
        assert!(!environment.restored_kubernetes());

        // A kubeconfig or context named on the command line wins.
        let other = directory.join("other");
        let named = GpuiOptions::kubernetes_only(Some(other.clone()), None, 100)
            .with_preferences_in(Some(preferences.clone()), false);
        assert_eq!(named.kubeconfig_path(), Some(other.as_path()));
        assert_eq!(named.kube_context(), None);
        assert!(!named.restored_kubernetes());
        let context = GpuiOptions::kubernetes_only(None, Some("named".into()), 100)
            .with_preferences_in(Some(preferences.clone()), false);
        assert_eq!(context.kubeconfig_path(), None);
        assert_eq!(context.kube_context(), Some("named"));
        assert!(!context.restored_kubernetes());
        // It never turns a Talos launch into a Kubernetes one.
        let talos =
            GpuiOptions::new(None, None, 100).with_preferences_in(Some(preferences.clone()), false);
        assert!(!talos.is_kubernetes_only() && talos.config_path().is_none());
        assert!(
            !GpuiOptions::fixture()
                .with_preferences_in(Some(preferences.clone()), false)
                .restored_kubernetes()
        );

        // Choosing a talosconfig again returns to Talos.
        store.remember(Selection {
            path: directory.join("talosconfig"),
            context: "alpha".into(),
        });
        store.save_latest().unwrap();
        assert!(talos_available(Some(&preferences), || false));
        let back = GpuiOptions::kubernetes_only(None, None, 100)
            .with_preferences_in(Some(preferences.clone()), false);
        assert_eq!(back.kubeconfig_path(), None);
        assert!(!back.restored_kubernetes());
        // Nothing remembered: the default talosconfig decides.
        assert!(talos_available(None, || true) && !talos_available(None, || false));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
