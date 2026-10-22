//! Which cluster a command reads. The choice is always explicit: the current
//! kubeconfig or talosconfig context is never used.

use std::path::{Path, PathBuf};

use freshkube_core::cluster_source::{ClusterAccess, ClusterSource, DirectAccess};
use freshkube_core::resources::{kubeconfig_sources, load_kubeconfigs};
use freshkube_core::workspace::MAX_BYTES;
use freshkube_core::{ConfigurationRevision, read_bounded_regular_file, setup};

use crate::args::ClusterArgs;

/// Why a command stops before it reads anything.
#[derive(Debug, PartialEq, Eq)]
pub enum Stop {
    /// The command line is wrong: exit 2.
    Usage(String),
    /// The context needs a sign-in that `--allow-login` did not allow: exit 5.
    SignIn(String),
}

/// What was asked for, once the options are resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The freshkube.yaml entry, when one was named.
    pub entry: Option<String>,
    /// The kubeconfig files to read the context from.
    pub sources: Vec<PathBuf>,
    /// The context to read, named by the person.
    pub context: String,
}

/// A cluster ready to read.
#[derive(Clone)]
pub struct Target {
    pub entry: Option<String>,
    pub context: String,
    pub source: ClusterSource,
}

/// Turns a resolved request into something to read. The real one builds a
/// kubeconfig connection; tests hand back a fake API.
pub trait Connector: Send + Sync {
    fn open(&self, request: &Request, allow_login: bool) -> Result<Target, Stop>;
}

/// Resolves the options into a request. `default_setup` is the app's
/// freshkube.yaml. Giving no choice, an unknown entry, or half of a pair is
/// a usage error that lists what could have been chosen.
pub fn resolve(args: &ClusterArgs, default_setup: Option<&Path>) -> Result<Request, Stop> {
    if let (Some(kubeconfig), Some(context)) = (&args.kubeconfig, &args.kube_context) {
        return Ok(Request {
            entry: None,
            sources: kubeconfig_sources(Some(kubeconfig)),
            context: context.clone(),
        });
    }
    let file = args.setup.as_deref().or(default_setup);
    let entries = read_entries(file);
    let Some(wanted) = &args.cluster else {
        return Err(Stop::Usage(format!(
            "choose a cluster with --cluster <entry>, or --kubeconfig <file> with --kube-context \
             <name>; the current context is never used.\n{}",
            list_entries(&entries)
        )));
    };
    let workspace = match &entries {
        Ok(found) => found,
        Err(why) => return Err(Stop::Usage(format!("can't read the setup file: {why}"))),
    };
    let Some(entry) = workspace.clusters.iter().find(|entry| &entry.id == wanted) else {
        return Err(Stop::Usage(format!(
            "there is no cluster entry \"{wanted}\".\n{}",
            list_entries(&entries)
        )));
    };
    Ok(Request {
        entry: Some(entry.id.clone()),
        sources: kubeconfig_sources(workspace.kubeconfig.as_deref()),
        context: entry.context.clone(),
    })
}

type Entries = Result<freshkube_core::workspace::Workspace, String>;

fn read_entries(file: Option<&Path>) -> Entries {
    let file = file.ok_or_else(|| "there is no setup file location".to_owned())?;
    let bytes = read_bounded_regular_file(file, MAX_BYTES)
        .map_err(|error| format!("{}: {error}", file.display()))?;
    let setup = setup::parse(&bytes).map_err(|invalid| format!("{}: {invalid}", file.display()))?;
    Ok(setup.workspace)
}

fn list_entries(entries: &Entries) -> String {
    match entries {
        Ok(workspace) if !workspace.clusters.is_empty() => {
            let mut text = String::from("Cluster entries:");
            for entry in &workspace.clusters {
                text.push_str(&format!("\n  {}  ({})", entry.id, entry.role.label()));
            }
            text
        }
        Ok(_) => "The setup file has no cluster entries.".to_owned(),
        Err(why) => format!("No cluster entries could be listed: {why}"),
    }
}

/// The real connector: a context of the kubeconfig files, by name.
pub struct Kubeconfig;

impl Connector for Kubeconfig {
    fn open(&self, request: &Request, allow_login: bool) -> Result<Target, Stop> {
        let (report, merged) = load_kubeconfigs(&request.sources);
        let Some(context) = report.context(&request.context) else {
            let names: Vec<_> = report.contexts.iter().map(|c| c.name.as_str()).collect();
            return Err(Stop::Usage(format!(
                "the kubeconfig has no context \"{}\". Contexts: {}",
                request.context,
                if names.is_empty() {
                    "none".to_owned()
                } else {
                    names.join(", ")
                }
            )));
        };
        let user = merged.as_ref().and_then(|config| {
            let named = config.contexts.iter().find(|c| c.name == context.name)?;
            let user = named.context.as_ref()?.user.as_ref()?;
            config.auth_infos.iter().find(|auth| &auth.name == user)
        });
        let signs_in = user
            .and_then(|named| named.auth_info.as_ref())
            .is_some_and(|auth| auth.exec.is_some());
        if signs_in && !allow_login {
            return Err(Stop::SignIn(format!(
                "context \"{}\" signs in with a credential plugin, which may open a browser. \
                 Run it again with --allow-login.",
                request.context
            )));
        }
        let access = DirectAccess::new(
            request.sources.clone(),
            request.context.clone(),
            ConfigurationRevision::from_kubeconfig_sources(&request.sources),
        );
        Ok(Target {
            entry: request.entry.clone(),
            context: request.context.clone(),
            source: ClusterSource {
                id: access.id(),
                context: request.context.clone(),
                access: ClusterAccess::Direct(access),
            },
        })
    }
}
