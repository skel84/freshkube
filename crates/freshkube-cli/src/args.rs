//! The subcommands and the options every one of them takes.

use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};

/// The read commands. They never start the window.
#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Run the cluster's health checks; exits non-zero when one fails
    Check(CheckArgs),
    /// List a kind, or read one object, with the columns the Resources page shows
    Get(GetArgs),
}

impl Command {
    pub fn cluster(&self) -> &ClusterArgs {
        match self {
            Command::Check(args) => &args.cluster,
            Command::Get(args) => &args.cluster,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Command::Check(_) => "check",
            Command::Get(_) => "get",
        }
    }
}

/// Which cluster to read, and how. Giving none is a usage error: the
/// current kubeconfig or talosconfig context is never used.
#[derive(Args, Debug, Clone, Default)]
pub struct ClusterArgs {
    /// A cluster entry of freshkube.yaml
    #[arg(long, conflicts_with_all = ["kubeconfig", "kube_context"])]
    pub cluster: Option<String>,

    /// The freshkube.yaml to read entries from, for CI (default: the app's)
    #[arg(long, requires = "cluster")]
    pub setup: Option<PathBuf>,

    /// Kubeconfig file to read, with --kube-context; Kubernetes only
    #[arg(long, requires = "kube_context")]
    pub kubeconfig: Option<PathBuf>,

    /// The kubeconfig context to read
    #[arg(long, requires = "kubeconfig")]
    pub kube_context: Option<String>,

    /// Let a context's credential plugin sign in, which may open a browser.
    /// Without it a context that needs one exits 5
    #[arg(long)]
    pub allow_login: bool,

    /// Write only JSON to stdout; progress and errors go to stderr
    #[arg(long)]
    pub json: bool,
}

/// What `check` counts as a failure besides a failed check.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FailOn {
    /// Only failures exit 1
    #[default]
    Fail,
    /// Warnings exit 1 too
    Warn,
}

#[derive(Args, Debug, Clone)]
pub struct CheckArgs {
    #[command(flatten)]
    pub cluster: ClusterArgs,

    /// Limit the pod and workload checks to one namespace
    #[arg(short, long)]
    pub namespace: Option<String>,

    /// Exit 1 on warnings too
    #[arg(long, value_enum, default_value_t = FailOn::Fail)]
    pub fail_on: FailOn,
}

#[derive(Args, Debug, Clone)]
pub struct GetArgs {
    #[command(flatten)]
    pub cluster: ClusterArgs,

    /// The kind: pods, deployments.apps, nodes… A `kind/name` reads one object
    pub kind: String,

    /// One object by name
    pub name: Option<String>,

    /// The namespace; without it, every namespace
    #[arg(short, long, conflicts_with = "all_namespaces")]
    pub namespace: Option<String>,

    /// Every namespace (the default)
    #[arg(short = 'A', long)]
    pub all_namespaces: bool,

    /// `wide` adds the columns kubectl shows with -o wide
    #[arg(short, long)]
    pub output: Option<String>,

    /// Refused for every kind: Secret values are never requested
    #[arg(long, hide = true)]
    pub show_values: bool,

    /// Refused for every kind: Secret values are never requested
    #[arg(long, hide = true)]
    pub reveal: bool,

    /// Refused for every kind: Secret values are never requested
    #[arg(long, hide = true)]
    pub decode: bool,

    /// Refused for every kind: a Secret's key is a value
    #[arg(long, hide = true)]
    pub key: Option<String>,
}
