//! talos-pilot: terminal and native desktop management for Talos clusters.

use clap::{Parser, ValueEnum};
use color_eyre::Result;
use std::{fs::File, path::PathBuf};
use talos_pilot_gui::GuiOptions;
use talos_pilot_tui::App;
use tracing::Level;
use tracing_subscriber::{EnvFilter, prelude::*};

/// User interface launched by talos-pilot.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Ui {
    /// Terminal UI, suitable for every supported target.
    Tui,
    /// Native egui desktop dashboard.
    Gui,
    /// Experimental Dioxus desktop dashboard (requires feature dioxus-ui).
    Dioxus,
    /// Experimental GPUI Kit desktop frontend (requires gpui-ui).
    Gpui,
}
/// talos-pilot: manage and monitor Talos Linux clusters.
#[derive(Parser)]
#[command(name = "talos-pilot")]
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

    /// Log file path (default: <temp_dir>/talos-pilot.log)
    #[arg(long)]
    log_file: Option<String>,

    /// Number of log lines to fetch (default: 500)
    #[arg(short, long, default_value = "500")]
    tail: i32,

    /// Interface to launch.
    #[arg(long, value_enum, default_value_t = Ui::Tui)]
    ui: Ui,

    /// Kubeconfig file to use for Kubernetes data (GPUI only; uses its current
    /// context, which must belong to the same cluster as the Talos context)
    #[arg(long)]
    kubeconfig: Option<String>,

    /// Connect without TLS client certificates (for maintenance mode nodes)
    #[arg(short, long)]
    insecure: bool,

    /// Endpoint to connect to in insecure mode (e.g., 192.168.1.100 or 192.168.1.100:50000)
    #[arg(short, long, requires = "insecure")]
    endpoint: Option<String>,
}

fn main() -> Result<()> {
    // Parse CLI arguments
    let Cli {
        context,
        config,
        debug,
        log_file,
        tail,
        insecure,
        endpoint,
        ui,
        kubeconfig,
    } = Cli::parse();

    // Fail before opening files or connecting when this prototype is unavailable.
    validate_ui(ui, insecure, endpoint.as_deref())?;
    validate_kubeconfig(ui, insecure, kubeconfig.as_deref())?;

    // Initialize error handling
    color_eyre::install()?;

    // Initialize logging to file (not stdout, which would corrupt either UI).
    let log_path = resolve_log_path(log_file);
    let log_file = File::create(&log_path)?;

    // Build filter: set base level, but quiet down noisy HTTP/gRPC libraries
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

    tracing::info!("Starting talos-pilot");

    // Validate insecure mode requires endpoint
    if insecure && endpoint.is_none() {
        eprintln!("Error: --insecure requires --endpoint <ip>");
        eprintln!("Usage: talos-pilot --insecure --endpoint 192.168.1.100");
        std::process::exit(1);
    }

    if insecure {
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

    // Desktop frontends own their native event loops. Keep Tokio alive beside
    // them so Talos I/O never blocks the UI thread.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    match ui {
        Ui::Tui => {
            let mut app = App::new(config, context, tail, insecure, endpoint);
            runtime.block_on(app.run())?;
        }
        Ui::Gui => {
            talos_pilot_gui::run(
                GuiOptions {
                    config_path: config.map(PathBuf::from),
                    context,
                    maintenance_endpoint: insecure.then_some(endpoint).flatten(),
                },
                runtime.handle().clone(),
            )?;
        }
        Ui::Dioxus => {
            #[cfg(feature = "dioxus-ui")]
            talos_pilot_dioxus::run(
                dioxus_options(config, context, tail, insecure, endpoint),
                runtime.handle().clone(),
            )?;
            #[cfg(not(feature = "dioxus-ui"))]
            unreachable!("validate_ui rejects Dioxus without its build feature");
        }
        Ui::Gpui => {
            #[cfg(feature = "gpui-ui")]
            talos_pilot_gpui::run(
                gpui_options(config, context, tail, kubeconfig, insecure, endpoint),
                runtime.handle().clone(),
            )?;
            #[cfg(not(feature = "gpui-ui"))]
            color_eyre::eyre::bail!("GPUI is not enabled; rebuild with --features gpui-ui");
        }
    }

    tracing::info!("Goodbye!");
    Ok(())
}

fn validate_ui(ui: Ui, insecure: bool, endpoint: Option<&str>) -> Result<()> {
    if matches!(ui, Ui::Gpui) {
        #[cfg(not(feature = "gpui-ui"))]
        color_eyre::eyre::bail!(
            "GPUI is experimental and not enabled. Rebuild with: cargo run --features gpui-ui -- --ui gpui --config /path/to/talosconfig"
        );
        #[cfg(feature = "gpui-ui")]
        if insecure {
            let endpoint = endpoint
                .ok_or_else(|| color_eyre::eyre::eyre!("--insecure requires --endpoint <ip>"))?;
            talos_pilot_gpui::validate_maintenance_endpoint(endpoint)?;
        }
    }
    if matches!(ui, Ui::Dioxus) {
        #[cfg(not(feature = "dioxus-ui"))]
        color_eyre::eyre::bail!(
            "Dioxus is experimental and not enabled. Rebuild with: cargo run --features dioxus-ui -- --ui dioxus"
        );
        #[cfg(feature = "dioxus-ui")]
        if insecure {
            let endpoint = endpoint
                .ok_or_else(|| color_eyre::eyre::eyre!("--insecure requires --endpoint <ip>"))?;
            talos_pilot_dioxus::validate_maintenance_endpoint(endpoint)?;
        }
    }
    let _ = (insecure, endpoint);
    Ok(())
}

/// `--kubeconfig` selects Kubernetes credentials for a cluster shell. It is
/// GPUI-only, and meaningless in maintenance mode, which never has a cluster.
fn validate_kubeconfig(ui: Ui, insecure: bool, kubeconfig: Option<&str>) -> Result<()> {
    if kubeconfig.is_none() {
        return Ok(());
    }
    if !matches!(ui, Ui::Gpui) {
        color_eyre::eyre::bail!("--kubeconfig is only supported with --ui gpui");
    }
    if insecure {
        color_eyre::eyre::bail!(
            "--kubeconfig cannot be used with --insecure: maintenance mode talks only to the node at --endpoint and never uses a cluster's Kubernetes credentials"
        );
    }
    Ok(())
}

#[cfg(feature = "gpui-ui")]
fn gpui_options(
    config: Option<String>,
    context: Option<String>,
    tail: i32,
    kubeconfig: Option<String>,
    insecure: bool,
    endpoint: Option<String>,
) -> talos_pilot_gpui::GpuiOptions {
    let options = talos_pilot_gpui::GpuiOptions::new(config.map(PathBuf::from), context, tail);
    let options = match kubeconfig {
        Some(path) => options.with_kubeconfig(PathBuf::from(path)),
        None => options,
    };
    match insecure.then_some(endpoint).flatten() {
        Some(endpoint) => options.with_maintenance_endpoint(endpoint),
        None => options,
    }
}

#[cfg(feature = "dioxus-ui")]
fn dioxus_options(
    config: Option<String>,
    context: Option<String>,
    tail: i32,
    insecure: bool,
    endpoint: Option<String>,
) -> talos_pilot_dioxus::DioxusOptions {
    talos_pilot_dioxus::DioxusOptions {
        config_path: config.map(PathBuf::from),
        context,
        tail,
        maintenance_endpoint: insecure.then_some(endpoint).flatten(),
    }
}

/// Resolve the log file path, falling back to the platform temp directory.
fn resolve_log_path(log_file: Option<String>) -> PathBuf {
    match log_file {
        Some(path) => PathBuf::from(path),
        None => std::env::temp_dir().join("talos-pilot.log"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpui_cli_preserves_identity_and_existing_default() {
        let cli = Cli::try_parse_from([
            "talos-pilot",
            "--ui",
            "gpui",
            "--config",
            "/tmp/talosconfig",
            "--context",
            "lab",
            "--tail",
            "123",
        ])
        .unwrap();
        assert!(matches!(cli.ui, Ui::Gpui));
        assert_eq!(cli.config.as_deref(), Some("/tmp/talosconfig"));
        assert_eq!(cli.context.as_deref(), Some("lab"));
        assert_eq!(cli.tail, 123);
        assert!(matches!(
            Cli::try_parse_from(["talos-pilot"]).unwrap().ui,
            Ui::Tui
        ));
    }

    #[test]
    #[cfg(not(feature = "gpui-ui"))]
    fn gpui_disabled_has_actionable_error_before_launch() {
        let error = validate_ui(Ui::Gpui, false, None).unwrap_err().to_string();
        assert!(error.contains("cargo run --features gpui-ui -- --ui gpui"));
    }

    #[test]
    #[cfg(feature = "gpui-ui")]
    fn gpui_enabled_validates_readonly_options_without_launch() {
        assert!(validate_ui(Ui::Gpui, false, None).is_ok());
        let options = gpui_options(
            Some("/tmp/talosconfig".into()),
            Some("lab".into()),
            123,
            Some("/tmp/kubeconfig".into()),
            false,
            None,
        );
        assert_eq!(
            options.config_path(),
            Some(std::path::Path::new("/tmp/talosconfig"))
        );
        assert_eq!(options.context(), Some("lab"));
        assert_eq!(options.tail(), 123);
        assert_eq!(
            options.kubeconfig_path(),
            Some(std::path::Path::new("/tmp/kubeconfig"))
        );
        assert!(options.maintenance_endpoint().is_none());
        assert!(
            gpui_options(None, None, 1, None, false, None)
                .kubeconfig_path()
                .is_none()
        );
        // An endpoint without --insecure never enters maintenance mode.
        assert!(
            gpui_options(None, None, 1, None, false, Some("192.0.2.10".into()))
                .maintenance_endpoint()
                .is_none()
        );
    }

    #[test]
    #[cfg(feature = "gpui-ui")]
    fn gpui_accepts_valid_maintenance_endpoints_and_builds_options() {
        for endpoint in ["192.0.2.10", "node.example:50000", "[2001:db8::10]:50000"] {
            assert!(
                validate_ui(Ui::Gpui, true, Some(endpoint)).is_ok(),
                "{endpoint}"
            );
        }
        let cli = Cli::try_parse_from([
            "talos-pilot",
            "--ui",
            "gpui",
            "--insecure",
            "--endpoint",
            "192.0.2.10:50000",
        ])
        .unwrap();
        validate_ui(cli.ui, cli.insecure, cli.endpoint.as_deref()).unwrap();
        validate_kubeconfig(cli.ui, cli.insecure, cli.kubeconfig.as_deref()).unwrap();
        let options = gpui_options(
            cli.config,
            cli.context,
            cli.tail,
            cli.kubeconfig,
            cli.insecure,
            cli.endpoint,
        );
        assert_eq!(options.maintenance_endpoint(), Some("192.0.2.10:50000"));
        assert!(options.config_path().is_none());
        assert!(options.kubeconfig_path().is_none());
    }

    #[test]
    #[cfg(feature = "gpui-ui")]
    fn gpui_validates_maintenance_endpoint_before_launch() {
        assert!(
            validate_ui(Ui::Gpui, true, None)
                .unwrap_err()
                .to_string()
                .contains("--insecure requires --endpoint")
        );
        for endpoint in [
            "",
            " ",
            "not a valid endpoint",
            "https://node.example/path",
            "node.example:0",
            "node.example:65536",
        ] {
            assert!(
                validate_ui(Ui::Gpui, true, Some(endpoint)).is_err(),
                "invalid maintenance endpoint was accepted: {endpoint:?}"
            );
        }
    }

    #[test]
    fn kubeconfig_is_rejected_with_insecure_and_outside_gpui() {
        assert!(validate_kubeconfig(Ui::Gpui, false, Some("/tmp/kubeconfig")).is_ok());
        assert!(validate_kubeconfig(Ui::Gpui, false, None).is_ok());
        assert!(validate_kubeconfig(Ui::Gpui, true, None).is_ok());
        let error = validate_kubeconfig(Ui::Gpui, true, Some("/tmp/kubeconfig"))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("--kubeconfig cannot be used with --insecure"),
            "{error}"
        );
        for ui in [Ui::Tui, Ui::Gui, Ui::Dioxus] {
            assert_eq!(
                validate_kubeconfig(ui, false, Some("/tmp/kubeconfig"))
                    .unwrap_err()
                    .to_string(),
                "--kubeconfig is only supported with --ui gpui"
            );
        }
    }

    #[test]
    fn default_log_path_uses_temp_dir() {
        let path = resolve_log_path(None);
        let expected = std::env::temp_dir().join("talos-pilot.log");
        assert_eq!(path, expected);
    }

    #[test]
    fn default_log_path_parent_exists() {
        let path = resolve_log_path(None);
        assert!(
            path.parent().unwrap().exists(),
            "default log path parent directory does not exist: {}",
            path.display()
        );
    }

    #[test]
    fn explicit_log_path_is_used() {
        let custom = "/some/custom/path.log".to_string();
        let path = resolve_log_path(Some(custom.clone()));
        assert_eq!(path, PathBuf::from(custom));
    }

    #[test]
    fn gui_interface_is_selected_from_cli() {
        let cli = Cli::try_parse_from(["talos-pilot", "--ui", "gui"]).unwrap();
        assert!(matches!(cli.ui, Ui::Gui));
    }

    #[test]
    fn tui_remains_the_default_interface() {
        let cli = Cli::try_parse_from(["talos-pilot"]).unwrap();
        assert!(matches!(cli.ui, Ui::Tui));
        assert_eq!(cli.tail, 500);
        assert!(!cli.insecure);
        assert!(!cli.debug);
        assert!(cli.endpoint.is_none());
        assert!(cli.context.is_none());
        assert!(cli.config.is_none());
    }

    #[test]
    fn legacy_short_options_keep_tui_default() {
        let cli = Cli::try_parse_from([
            "talos-pilot",
            "-c",
            "lab",
            "--config",
            "/tmp/talosconfig",
            "-t",
            "200",
            "-d",
            "-i",
            "-e",
            "192.0.2.10",
        ])
        .unwrap();
        assert!(matches!(cli.ui, Ui::Tui));
        assert_eq!(cli.context.as_deref(), Some("lab"));
        assert_eq!(cli.config.as_deref(), Some("/tmp/talosconfig"));
        assert_eq!(cli.tail, 200);
        assert!(cli.debug);
        assert!(cli.insecure);
        assert_eq!(cli.endpoint.as_deref(), Some("192.0.2.10"));
    }

    #[test]
    fn dioxus_interface_is_selected_from_cli() {
        let cli = Cli::try_parse_from([
            "talos-pilot",
            "--ui",
            "dioxus",
            "--config",
            "/tmp/talosconfig",
            "--context",
            "lab",
            "--tail",
            "200",
        ])
        .unwrap();
        assert!(matches!(cli.ui, Ui::Dioxus));
        assert_eq!(cli.config.as_deref(), Some("/tmp/talosconfig"));
        assert_eq!(cli.context.as_deref(), Some("lab"));
        assert_eq!(cli.tail, 200);
    }

    #[test]
    fn endpoint_requires_insecure_mode_for_every_interface() {
        for ui in ["tui", "gui", "dioxus"] {
            assert!(
                Cli::try_parse_from(["talos-pilot", "--ui", ui, "--endpoint", "192.0.2.10"])
                    .is_err()
            );
        }
    }

    #[test]
    fn insecure_cli_preserves_endpoint_and_interface() {
        for ui in ["tui", "gui", "dioxus"] {
            let cli = Cli::try_parse_from([
                "talos-pilot",
                "--ui",
                ui,
                "--insecure",
                "--endpoint",
                "192.0.2.10:50000",
            ])
            .unwrap();
            assert!(cli.insecure);
            assert_eq!(cli.endpoint.as_deref(), Some("192.0.2.10:50000"));
            assert_eq!(cli.ui.to_possible_value().unwrap().get_name(), ui);
        }
    }

    #[test]
    fn existing_interfaces_remain_available() {
        for ui in [Ui::Tui, Ui::Gui] {
            assert!(validate_ui(ui, false, None).is_ok());
            assert!(validate_ui(ui, true, Some("192.0.2.10")).is_ok());
        }
    }

    #[test]
    #[cfg(not(feature = "dioxus-ui"))]
    fn dioxus_without_feature_preserves_unavailable_error() {
        const ERROR: &str = "Dioxus is experimental and not enabled. Rebuild with: cargo run --features dioxus-ui -- --ui dioxus";
        for (insecure, endpoint) in [
            (false, None),
            (true, None),
            (true, Some("192.0.2.10")),
            (true, Some("not a valid endpoint")),
        ] {
            assert_eq!(
                validate_ui(Ui::Dioxus, insecure, endpoint)
                    .unwrap_err()
                    .to_string(),
                ERROR
            );
        }
    }

    #[test]
    #[cfg(feature = "dioxus-ui")]
    fn enabled_dioxus_accepts_authenticated_and_valid_maintenance_modes() {
        assert!(validate_ui(Ui::Dioxus, false, None).is_ok());
        for endpoint in ["192.0.2.10", "node.example:50000", "[2001:db8::10]:50000"] {
            assert!(validate_ui(Ui::Dioxus, true, Some(endpoint)).is_ok());
        }
    }

    #[test]
    #[cfg(feature = "dioxus-ui")]
    fn dioxus_dispatch_options_preserve_cli_identity_without_launching_ui() {
        let cli = Cli::try_parse_from([
            "talos-pilot",
            "--ui",
            "dioxus",
            "--config",
            "/tmp/talosconfig",
            "--context",
            "lab",
            "--tail",
            "200",
            "--insecure",
            "--endpoint",
            "192.0.2.10:50000",
        ])
        .unwrap();
        validate_ui(cli.ui, cli.insecure, cli.endpoint.as_deref()).unwrap();
        let options = dioxus_options(
            cli.config,
            cli.context,
            cli.tail,
            cli.insecure,
            cli.endpoint,
        );
        assert_eq!(options.config_path, Some(PathBuf::from("/tmp/talosconfig")));
        assert_eq!(options.context.as_deref(), Some("lab"));
        assert_eq!(options.tail, 200);
        assert_eq!(
            options.maintenance_endpoint.as_deref(),
            Some("192.0.2.10:50000")
        );

        let defaults = dioxus_options(None, None, 500, false, None);
        assert!(defaults.config_path.is_none());
        assert!(defaults.context.is_none());
        assert_eq!(defaults.tail, 500);
        assert!(defaults.maintenance_endpoint.is_none());
        assert!(
            dioxus_options(None, None, 500, false, Some("192.0.2.10".into()))
                .maintenance_endpoint
                .is_none()
        );
    }

    #[test]
    #[cfg(feature = "dioxus-ui")]
    fn enabled_dioxus_validates_endpoint_before_launch() {
        assert!(
            validate_ui(Ui::Dioxus, true, None)
                .unwrap_err()
                .to_string()
                .contains("--insecure requires --endpoint")
        );
        for endpoint in [
            "",
            " ",
            "not a valid endpoint",
            "https://node.example/path",
            "node.example:0",
            "node.example:65536",
        ] {
            assert!(
                validate_ui(Ui::Dioxus, true, Some(endpoint)).is_err(),
                "invalid maintenance endpoint was accepted: {endpoint:?}"
            );
        }
    }
}
