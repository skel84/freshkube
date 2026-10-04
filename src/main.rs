//! Freshkube: a native desktop app for Talos Linux and Kubernetes clusters.

use clap::Parser;
use color_eyre::Result;
use freshkube_desktop::GpuiOptions;
use std::{fs::File, path::PathBuf};
use tracing::Level;
use tracing_subscriber::{EnvFilter, prelude::*};

/// Freshkube: manage and monitor Talos Linux and Kubernetes clusters.
#[derive(Parser)]
#[command(name = "freshkube")]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Talos context to use (from talosconfig)
    #[arg(short, long)]
    context: Option<String>,

    /// Path to talosconfig file
    #[arg(long)]
    config: Option<String>,

    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,

    /// Log file path (default: <temp_dir>/freshkube.log)
    #[arg(long)]
    log_file: Option<String>,

    /// Number of log lines to fetch (default: 500)
    #[arg(short, long, default_value = "500")]
    tail: i32,

    /// Kubeconfig file to use for Kubernetes data. With Talos, its current
    /// context must belong to the same cluster as the Talos context; in
    /// Kubernetes-only mode its contexts are listed
    #[arg(long)]
    kubeconfig: Option<String>,

    /// Browse Kubernetes only, from a kubeconfig, without Talos. Chosen by
    /// itself when no Talos option or remembered selection is available and
    /// there is no talosconfig (TALOSCONFIG or ~/.talos/config)
    #[arg(long, conflicts_with_all = ["config", "context", "insecure"])]
    kubernetes_only: bool,

    /// Kubeconfig context to connect to; implies --kubernetes-only
    /// (default: the kubeconfig's current context)
    #[arg(long, conflicts_with_all = ["config", "context", "insecure"])]
    kube_context: Option<String>,

    /// Connect without TLS client certificates (for maintenance mode nodes)
    #[arg(short, long)]
    insecure: bool,

    /// Endpoint to connect to in insecure mode (e.g., 192.168.1.100 or 192.168.1.100:50000)
    #[arg(short, long, requires = "insecure")]
    endpoint: Option<String>,

    /// Show synthetic example data instead of connecting; no credentials or
    /// cluster are used
    #[arg(long, conflicts_with_all = [
        "insecure", "kubeconfig", "config", "context", "kubernetes_only", "kube_context",
    ])]
    fixture: bool,
}

fn main() -> Result<()> {
    let Cli {
        context,
        config,
        debug,
        log_file,
        tail,
        insecure,
        endpoint,
        kubeconfig,
        kubernetes_only,
        kube_context,
        fixture,
    } = Cli::parse();
    // A terminal's explicit Talos environment overrides a remembered GUI
    // selection. Finder launches use the saved selection or the default file.
    let config = if fixture || kubernetes_only || kube_context.is_some() || insecure {
        config
    } else {
        config.or_else(|| {
            std::env::var("TALOSCONFIG")
                .ok()
                .filter(|path| !path.is_empty())
        })
    };
    let kubernetes_only = !fixture
        && wants_kubernetes_only(
            kubernetes_only,
            kube_context.is_some(),
            config.is_some() || context.is_some() || insecure,
            freshkube_desktop::default_talosconfig_exists,
        );

    // Fail before opening files or connecting.
    validate(insecure, endpoint.as_deref(), kubeconfig.as_deref())?;

    color_eyre::install()?;

    // Log to a file; stdout belongs to the terminal that launched the app.
    let log_path = resolve_log_path(log_file);
    let log_file = File::create(&log_path)?;

    // Set the base level, but quiet down noisy HTTP/gRPC libraries.
    let filter = if debug {
        EnvFilter::from_default_env()
            .add_directive(Level::DEBUG.into())
            .add_directive("h2=info".parse().unwrap())
            .add_directive("hyper=info".parse().unwrap())
            .add_directive("tower=info".parse().unwrap())
            .add_directive("tonic=info".parse().unwrap())
            .add_directive("rustls=info".parse().unwrap())
    } else {
        EnvFilter::from_default_env().add_directive(Level::INFO.into())
    };

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(log_file)
                .with_ansi(true)
                .with_target(false),
        )
        .with(filter)
        .init();

    tracing::info!("Starting freshkube");
    if kubernetes_only {
        tracing::info!("Kubernetes-only mode");
        if let Some(context) = &kube_context {
            tracing::info!("Using kubeconfig context: {}", context);
        }
    } else if insecure {
        tracing::info!("Insecure mode enabled");
        if let Some(endpoint) = &endpoint {
            tracing::info!("Endpoint: {}", endpoint);
        }
    } else {
        if let Some(context) = &context {
            tracing::info!("Using context: {}", context);
        }
        if let Some(config) = &config {
            tracing::info!("Using config: {}", config);
        }
    }

    // The desktop app owns the native event loop. Keep Tokio alive beside it
    // so cluster I/O never blocks the UI thread.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    // Example data starts at the default text size, so captures are comparable.
    let options = if fixture {
        GpuiOptions::fixture()
    } else if kubernetes_only {
        GpuiOptions::kubernetes_only(kubeconfig.map(PathBuf::from), kube_context, tail)
            .with_preferences(freshkube_desktop::preferences_path())
            .with_keyring()
    } else {
        options(config, context, tail, kubeconfig, insecure, endpoint)
            .with_preferences(freshkube_desktop::preferences_path())
            .with_keyring()
    };
    freshkube_desktop::run(options, runtime.handle().clone())?;

    tracing::info!("Goodbye!");
    Ok(())
}

/// `--insecure` needs a valid `--endpoint`; `--kubeconfig` selects Kubernetes
/// credentials for a cluster, which maintenance mode never has.
fn validate(insecure: bool, endpoint: Option<&str>, kubeconfig: Option<&str>) -> Result<()> {
    if insecure {
        let endpoint = endpoint
            .ok_or_else(|| color_eyre::eyre::eyre!("--insecure requires --endpoint <ip>"))?;
        freshkube_desktop::validate_maintenance_endpoint(endpoint)?;
        if kubeconfig.is_some() {
            color_eyre::eyre::bail!(
                "--kubeconfig cannot be used with --insecure: maintenance mode talks only to the node at --endpoint and never uses a cluster's Kubernetes credentials"
            );
        }
    }
    Ok(())
}

/// Kubernetes-only when asked for, or when nothing names Talos and there is
/// no default talosconfig to use. A named talosconfig that is missing stays
/// Talos, which then says what's wrong with it.
fn wants_kubernetes_only(
    asked: bool,
    kube_context: bool,
    names_talos: bool,
    default_talosconfig_exists: impl FnOnce() -> bool,
) -> bool {
    asked || kube_context || (!names_talos && !default_talosconfig_exists())
}

fn options(
    config: Option<String>,
    context: Option<String>,
    tail: i32,
    kubeconfig: Option<String>,
    insecure: bool,
    endpoint: Option<String>,
) -> GpuiOptions {
    let options = GpuiOptions::new(config.map(PathBuf::from), context, tail);
    let options = match kubeconfig {
        Some(path) => options.with_kubeconfig(PathBuf::from(path)),
        None => options,
    };
    match insecure.then_some(endpoint).flatten() {
        Some(endpoint) => options.with_maintenance_endpoint(endpoint),
        None => options,
    }
}

/// Resolve the log file path, falling back to the platform temp directory.
fn resolve_log_path(log_file: Option<String>) -> PathBuf {
    match log_file {
        Some(path) => PathBuf::from(path),
        None => std::env::temp_dir().join("freshkube.log"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_preserves_identity_and_defaults() {
        let cli = Cli::try_parse_from([
            "freshkube",
            "--config",
            "/tmp/talosconfig",
            "--context",
            "lab",
            "--tail",
            "123",
        ])
        .unwrap();
        assert_eq!(cli.config.as_deref(), Some("/tmp/talosconfig"));
        assert_eq!(cli.context.as_deref(), Some("lab"));
        assert_eq!(cli.tail, 123);
        let defaults = Cli::try_parse_from(["freshkube"]).unwrap();
        assert_eq!(defaults.tail, 500);
        assert!(!defaults.insecure && !defaults.debug && !defaults.fixture);
        assert!(defaults.endpoint.is_none() && defaults.kubeconfig.is_none());
    }

    #[test]
    fn short_options_are_kept() {
        let cli = Cli::try_parse_from([
            "freshkube",
            "-c",
            "lab",
            "-t",
            "200",
            "-d",
            "-i",
            "-e",
            "192.0.2.10",
        ])
        .unwrap();
        assert_eq!(cli.context.as_deref(), Some("lab"));
        assert_eq!(cli.tail, 200);
        assert!(cli.debug && cli.insecure);
        assert_eq!(cli.endpoint.as_deref(), Some("192.0.2.10"));
    }

    #[test]
    fn options_carry_kubeconfig_and_only_enter_maintenance_when_insecure() {
        let built = options(
            Some("/tmp/talosconfig".into()),
            Some("lab".into()),
            123,
            Some("/tmp/kubeconfig".into()),
            false,
            None,
        );
        assert_eq!(
            built.config_path(),
            Some(std::path::Path::new("/tmp/talosconfig"))
        );
        assert_eq!(built.context(), Some("lab"));
        assert_eq!(built.tail(), 123);
        assert_eq!(
            built.kubeconfig_path(),
            Some(std::path::Path::new("/tmp/kubeconfig"))
        );
        assert!(built.maintenance_endpoint().is_none());
        // An endpoint without --insecure never enters maintenance mode.
        assert!(
            options(None, None, 1, None, false, Some("192.0.2.10".into()))
                .maintenance_endpoint()
                .is_none()
        );
        assert_eq!(
            options(None, None, 1, None, true, Some("192.0.2.10:50000".into()))
                .maintenance_endpoint(),
            Some("192.0.2.10:50000")
        );
    }

    #[test]
    fn maintenance_endpoints_are_validated_before_launch() {
        assert!(
            validate(true, None, None)
                .unwrap_err()
                .to_string()
                .contains("--insecure requires --endpoint")
        );
        for endpoint in ["192.0.2.10", "node.example:50000", "[2001:db8::10]:50000"] {
            assert!(validate(true, Some(endpoint), None).is_ok(), "{endpoint}");
        }
        for endpoint in [
            "",
            " ",
            "not a valid endpoint",
            "https://node.example/path",
            "node.example:0",
            "node.example:65536",
        ] {
            assert!(
                validate(true, Some(endpoint), None).is_err(),
                "invalid maintenance endpoint was accepted: {endpoint:?}"
            );
        }
    }

    #[test]
    fn kubeconfig_is_rejected_with_insecure() {
        assert!(validate(false, None, Some("/tmp/kubeconfig")).is_ok());
        let error = validate(true, Some("192.0.2.10"), Some("/tmp/kubeconfig"))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("--kubeconfig cannot be used with --insecure"),
            "{error}"
        );
    }

    #[test]
    fn endpoint_requires_insecure_and_fixture_excludes_live_options() {
        assert!(Cli::try_parse_from(["freshkube", "--endpoint", "192.0.2.10"]).is_err());
        assert!(
            Cli::try_parse_from(["freshkube", "--fixture"])
                .unwrap()
                .fixture
        );
        assert!(Cli::try_parse_from(["freshkube", "--fixture", "--context", "lab"]).is_err());
    }

    #[test]
    fn kubernetes_only_is_asked_for_or_follows_a_missing_talosconfig() {
        let cli = Cli::try_parse_from([
            "freshkube",
            "--kubeconfig",
            "/tmp/kubeconfig",
            "--kube-context",
            "admin@lab",
        ])
        .unwrap();
        assert_eq!(cli.kube_context.as_deref(), Some("admin@lab"));
        assert!(!cli.kubernetes_only);
        assert!(
            Cli::try_parse_from(["freshkube", "--kubernetes-only"])
                .unwrap()
                .kubernetes_only
        );
        for talos in [
            &["--kubernetes-only", "--context", "lab"][..],
            &["--kubernetes-only", "--config", "/tmp/talosconfig"],
            &["--kube-context", "admin@lab", "-c", "lab"],
            &["--kube-context", "admin@lab", "-i", "-e", "192.0.2.10"],
            &["--fixture", "--kubernetes-only"],
            &["--fixture", "--kube-context", "admin@lab"],
        ] {
            let args = std::iter::once("freshkube").chain(talos.iter().copied());
            assert!(Cli::try_parse_from(args).is_err(), "{talos:?}");
        }

        let never = || -> bool { panic!("the talosconfig needn't be looked for") };
        assert!(wants_kubernetes_only(true, false, false, never));
        assert!(wants_kubernetes_only(false, true, false, never));
        // Nothing names Talos: the default talosconfig decides.
        assert!(wants_kubernetes_only(false, false, false, || false));
        assert!(!wants_kubernetes_only(false, false, false, || true));
        // A named talosconfig or context stays Talos even when missing.
        assert!(!wants_kubernetes_only(false, false, true, never));
    }

    #[test]
    fn log_path_defaults_to_temp_dir_and_honours_an_explicit_path() {
        let path = resolve_log_path(None);
        assert_eq!(path, std::env::temp_dir().join("freshkube.log"));
        assert!(path.parent().unwrap().exists());
        let custom = "/some/custom/path.log".to_string();
        assert_eq!(
            resolve_log_path(Some(custom.clone())),
            PathBuf::from(custom)
        );
    }
}
