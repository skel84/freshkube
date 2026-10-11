//! Short read-only clients for the workspace entries a delivery read needs,
//! and what one small read says about each part's entry.
//!
//! A client is built from the entry's context in the kubeconfig the app
//! already uses, never the file's current context, and reads only through
//! [`ReadOnlyClient`]. It is not a session: no watch, no reflector, nothing
//! kept after the read that made it ends.
//!
//! Two deadlines bound it. [`connect`] has [`CONNECT_DEADLINE`] to read the
//! kubeconfig, load the credentials and run the context's exec plugin, as
//! kubectl would run it. kube runs the plugin while it builds the client,
//! on a blocking thread; a plugin that waits for a sign-in keeps that thread
//! until it exits, but the read stops waiting at the deadline and says why.
//! Every request then has [`REQUEST_DEADLINE`] ([`Deadline`]).
//!
//! [`check`] reads one item of a part's kind and says, in the words of
//! issue #564's States table, whether its entry is readable, refused,
//! unreachable, doesn't serve it, or isn't in the workspace. [`check_all`]
//! checks every part of [`Sources`] and each workloads entry at once,
//! connecting to each entry once, at most [`MAX_AT_ONCE`] at a time.
//!
//! Only the kubeconfig files given are read: a Talos entry whose context
//! is in none of them, which the app opens through its control plane's
//! kubeconfig, is unreachable here.

use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt;
use kube::config::{Config, KubeConfigOptions};

use super::read::{ListRequest, Listing, ReadOnlyClient, Reader, Resource, Scope, ServedGroup};
use super::source::Source;
use super::versions::resolve;
use super::{argocd, kargo, tekton};
use crate::applications::read::{PART_OF, argo_namespaces, read_argo_workloads};
use crate::resources::Failure;
use crate::workspace::{Chosen, Entry, Part, Sources};

/// How long reading the kubeconfig, loading the credentials, running an
/// exec plugin and opening the connection may take together.
pub const CONNECT_DEADLINE: Duration = Duration::from_secs(5);
/// How long one GET may take, as the change page's own GETs.
pub const REQUEST_DEADLINE: Duration = super::change::live::pages::DEADLINE;
/// The most entries connected to at once, as the destinations are.
pub const MAX_AT_ONCE: usize = 4;

/// A reader whose every request ends by its deadline: one that doesn't
/// answer in time fails as timed out, which reads as unreachable.
#[derive(Clone)]
pub struct Deadline<R> {
    reader: R,
    limit: Duration,
}

impl<R: Reader> Deadline<R> {
    pub fn new(reader: R, limit: Duration) -> Self {
        Self { reader, limit }
    }

    async fn within<T>(
        &self,
        what: &str,
        request: impl Future<Output = Result<T, Failure>>,
    ) -> Result<T, Failure> {
        tokio::time::timeout(self.limit, request)
            .await
            .unwrap_or_else(|_| Err(Failure::timeout(what)))
    }
}

impl<R: Reader> Reader for Deadline<R> {
    async fn get(
        &self,
        resource: &Resource,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<serde_json::Value, Failure> {
        self.within(
            &format!("Reading {}", resource.plural),
            self.reader.get(resource, namespace, name),
        )
        .await
    }

    /// The deadline is the whole listing's, every page of it.
    async fn list(&self, request: &ListRequest) -> Result<Listing, Failure> {
        self.within(
            &format!("Listing {}", request.resource.plural),
            self.reader.list(request),
        )
        .await
    }

    async fn probe(&self, request: &ListRequest) -> Result<(), Failure> {
        self.within(
            &format!("Listing {}", request.resource.plural),
            self.reader.probe(request),
        )
        .await
    }

    async fn groups(&self) -> Result<Vec<ServedGroup>, Failure> {
        self.within("Reading the API groups", self.reader.groups())
            .await
    }

    async fn plurals(&self, group: &str, version: &str) -> Result<Vec<String>, Failure> {
        self.within(
            &format!("Reading {group}/{version}"),
            self.reader.plurals(group, version),
        )
        .await
    }
}

/// A client for one entry, every request under [`REQUEST_DEADLINE`].
pub type EntryReader = Deadline<ReadOnlyClient>;

/// A client for the context `context` of the merged `kubeconfigs`, within
/// [`CONNECT_DEADLINE`]. The error is a short reason, without credentials.
pub async fn connect(kubeconfigs: Vec<PathBuf>, context: String) -> Result<EntryReader, String> {
    connect_within(kubeconfigs, context, CONNECT_DEADLINE).await
}

/// [`connect`] with its deadline given.
pub async fn connect_within(
    kubeconfigs: Vec<PathBuf>,
    context: String,
    limit: Duration,
) -> Result<EntryReader, String> {
    // Without Talos nothing else may have chosen the TLS provider yet.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let runtime = tokio::runtime::Handle::current();
    // Files, credential files and exec plugins block; kube runs a plugin
    // while it makes the client, so all of it stays off the async workers
    // and inside the deadline.
    let setup = tokio::task::spawn_blocking(move || {
        let (report, kubeconfig) = crate::resources::contexts::load(&kubeconfigs);
        if report.context(&context).is_none() {
            return Err(format!("the kubeconfig has no context {context}"));
        }
        let kubeconfig = kubeconfig.ok_or("no kubeconfig file could be read")?;
        let options = KubeConfigOptions {
            context: Some(context),
            ..KubeConfigOptions::default()
        };
        runtime.block_on(async move {
            // kube's messages can quote credential sources; keep them out.
            let mut config = Config::from_custom_kubeconfig(kubeconfig, &options)
                .await
                .map_err(|_| "its kubeconfig entry couldn't be loaded".to_owned())?;
            config.connect_timeout = Some(limit);
            config.read_timeout = Some(REQUEST_DEADLINE);
            crate::kube_client::client(config).map_err(|error| match error.is_exec() {
                true => "its credential plugin failed".to_owned(),
                false => error.message("its client couldn't be made"),
            })
        })
    });
    match tokio::time::timeout(limit, setup).await {
        Err(_) => Err(format!(
            "no answer within {}; a credential plugin may be waiting for a sign-in",
            match limit.subsec_millis() {
                0 => format!("{} s", limit.as_secs()),
                _ => format!("{} ms", limit.as_millis()),
            }
        )),
        Ok(Err(_)) => Err("the connection worker stopped".into()),
        Ok(Ok(Err(why))) => Err(why),
        Ok(Ok(Ok(client))) => Ok(Deadline::new(ReadOnlyClient::new(client), REQUEST_DEADLINE)),
    }
}

/// Clients for `entries`, at most [`MAX_AT_ONCE`] connecting at a time, in
/// the order given, each with its entry's id.
pub async fn connect_all(
    kubeconfigs: &[PathBuf],
    entries: Vec<Entry>,
) -> Vec<(String, Result<EntryReader, String>)> {
    futures::stream::iter(entries)
        .map(|entry| {
            let kubeconfigs = kubeconfigs.to_vec();
            async move {
                let reader = connect(kubeconfigs, entry.context).await;
                (entry.id, reader)
            }
        })
        .buffered(MAX_AT_ONCE)
        .collect()
        .await
}

/// What a part's entry answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    /// No entry holds the part, or the one pinned is gone; the words say
    /// which.
    Missing(String),
    /// The entry couldn't be reached: a deadline, DNS, TLS, or its exec
    /// plugin failed or waits for a sign-in. The words are why.
    Unreachable(String),
    /// 401 or 403, and the server's words.
    Refused(String),
    /// The part's kind isn't served there (404).
    NotServed(String),
    /// It answered.
    Readable,
}

impl Reach {
    /// The state, as the Sources view names it.
    pub fn label(&self) -> &'static str {
        match self {
            Reach::Missing(_) => "Not in the workspace",
            Reach::Unreachable(_) => "Unreachable",
            Reach::Refused(_) => "Refused",
            Reach::NotServed(_) => "Not served",
            Reach::Readable => "Readable",
        }
    }

    /// The state in a sentence, for what was read (`Kargo`, `Workloads`)
    /// on `entry`, as the view names both.
    pub fn words(&self, what: &str, entry: &str) -> String {
        match self {
            Reach::Missing(why) => why.clone(),
            Reach::Unreachable(why) => format!("Couldn't reach {entry}: {why}"),
            Reach::Refused(why) => format!("{what} refused on {entry}: {why}"),
            Reach::NotServed(_) => format!("{what} isn't served on {entry}"),
            Reach::Readable => format!("{what} answered on {entry}"),
        }
    }

    fn of_failure(failure: Failure) -> Self {
        match Source::<()>::from_failure(failure) {
            Source::NotInstalled(why) => Reach::NotServed(why),
            Source::Refused(why) => Reach::Refused(why),
            Source::Unreadable(why) => Reach::Unreachable(why),
            Source::Read(()) | Source::Capped(..) => Reach::Readable,
        }
    }
}

/// The one small read that says whether `part` can be read: its kind's
/// version from discovery, then one item (`limit=1`).
///
/// - Kargo: Projects, which are cluster-scoped.
/// - Builds: PipelineRuns that Pipelines as Code labelled, in every
///   namespace, as a build is found by its commit.
/// - Argo CD: Applications where Argo CD runs, as the Applications read
///   finds it ([`check_argo`]).
pub async fn check<R: Reader>(reader: &R, part: Part) -> Reach {
    let (resource, scope) = match part {
        Part::ArgoCd => return check_argo(reader).await.0,
        Part::Kargo => (
            resolve(reader, kargo::GROUP, "projects", kargo::VERSIONS, false).await,
            Scope::Cluster,
        ),
        Part::Builds => (
            resolve(
                reader,
                tekton::TEKTON_GROUP,
                "pipelineruns",
                tekton::TEKTON_VERSIONS,
                true,
            )
            .await,
            Scope::Labels {
                namespace: None,
                selector: tekton::REPOSITORY_LABEL.to_owned(),
            },
        ),
    };
    let request = async {
        reader
            .probe(&ListRequest {
                resource: resource?,
                scope,
            })
            .await
    };
    reach(request.await)
}

fn reach(answer: Result<(), Failure>) -> Reach {
    match answer {
        Ok(()) => Reach::Readable,
        Err(failure) => Reach::of_failure(failure),
    }
}

/// Argo CD's Applications where it runs: the namespaces of the workloads
/// labelled `app.kubernetes.io/part-of=argocd`, at most three, else
/// `argocd`, as the Applications read chooses them
/// ([`argo_namespaces`]). One item is read in each; the first that fails
/// says the state. Returns the namespaces read too.
pub async fn check_argo<R: Reader>(reader: &R) -> (Reach, Vec<String>) {
    let (namespaces, _) = argo_namespaces(&read_argo_workloads(reader).await);
    let resource = match resolve(
        reader,
        argocd::GROUP,
        "applications",
        argocd::VERSIONS,
        true,
    )
    .await
    {
        Ok(resource) => resource,
        Err(failure) => return (Reach::of_failure(failure), namespaces),
    };
    for namespace in &namespaces {
        let request = ListRequest {
            resource: resource.clone(),
            scope: Scope::Namespace(namespace.clone()),
        };
        if let Err(failure) = reader.probe(&request).await {
            return (Reach::of_failure(failure), namespaces);
        }
    }
    (Reach::Readable, namespaces)
}

/// Whether an entry's workloads can be read: one labelled Deployment, in
/// every namespace, as the Applications read lists them.
pub async fn check_workloads<R: Reader>(reader: &R) -> Reach {
    let request = ListRequest {
        resource: Resource::new("apps", "v1", "deployments", true),
        scope: Scope::Labels {
            namespace: None,
            selector: PART_OF.to_owned(),
        },
    };
    reach(reader.probe(&request).await)
}

/// What a check found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Checked {
    /// Each part, in [`Part::ALL`]'s order.
    pub parts: Vec<(Part, Reach)>,
    /// The namespaces Argo CD's Applications were read in; empty when its
    /// entry wasn't reached.
    pub argocd_in: Vec<String>,
    /// Each workloads entry by id, in [`Sources::workloads`]' order; the
    /// open connection is [`OPEN`].
    pub workloads: Vec<(String, Reach)>,
}

/// How a check names the open connection, for the workspace of one.
pub const OPEN: &str = "the open cluster";

/// Checks every part of `sources`, and each workloads entry, at once. Each
/// entry is connected to once, however much it holds, the parts' entries
/// first; a part with no entry is [`Reach::Missing`] without a read.
///
/// `open` is the open connection, which the workspace of one reads
/// ([`Chosen::Open`]): its reader, or why it couldn't be had. Without one,
/// no cluster is open.
pub async fn check_all<R: Reader>(
    sources: &Sources,
    kubeconfigs: &[PathBuf],
    open: Option<Result<&R, String>>,
) -> Checked {
    let mut entries: Vec<Entry> = Vec::new();
    let chosen = sources.iter().map(|(_, chosen)| chosen);
    for chosen in chosen.chain(sources.workloads()) {
        if let Some(entry) = chosen.entry()
            && !entries.iter().any(|seen| seen.id == entry.id)
        {
            entries.push(entry.clone());
        }
    }
    let readers = connect_all(kubeconfigs, entries).await;
    let open = open
        .as_ref()
        .map(|open| open.as_ref().map_err(String::clone));
    let target = |chosen: &Chosen| -> Result<Target<'_, R>, Reach> {
        match chosen {
            Chosen::NoEntry(_) | Chosen::Gone(_) => {
                Err(Reach::Missing(chosen.missing().unwrap_or_default()))
            }
            Chosen::Open => match &open {
                Some(Ok(reader)) => Ok(Target::Open(reader)),
                Some(Err(why)) => Err(Reach::Unreachable(why.clone())),
                None => Err(Reach::Unreachable("no cluster is open".into())),
            },
            Chosen::Entry { entry, .. } => match readers.iter().find(|(id, _)| id == &entry.id) {
                Some((_, Ok(reader))) => Ok(Target::Entry(reader)),
                Some((_, Err(why))) => Err(Reach::Unreachable(why.clone())),
                None => Err(Reach::Unreachable("it wasn't connected to".into())),
            },
        }
    };
    let parts = sources.iter().map(|(part, chosen)| {
        let target = target(chosen);
        async move {
            let (reach, namespaces) = match target {
                Err(reach) => (reach, Vec::new()),
                Ok(target) if part == Part::ArgoCd => match target {
                    Target::Open(reader) => check_argo(reader).await,
                    Target::Entry(reader) => check_argo(reader).await,
                },
                Ok(Target::Open(reader)) => (check(reader, part).await, Vec::new()),
                Ok(Target::Entry(reader)) => (check(reader, part).await, Vec::new()),
            };
            (part, reach, namespaces)
        }
    });
    let workloads = sources.workloads().iter().map(|chosen| {
        let target = target(chosen);
        let id = match chosen {
            Chosen::Entry { entry, .. } => entry.id.clone(),
            Chosen::Gone(id) => id.clone(),
            Chosen::Open | Chosen::NoEntry(_) => OPEN.to_owned(),
        };
        async move {
            let reach = match target {
                Err(reach) => reach,
                Ok(Target::Open(reader)) => check_workloads(reader).await,
                Ok(Target::Entry(reader)) => check_workloads(reader).await,
            };
            (id, reach)
        }
    });
    let (parts, workloads) = futures::future::join(
        futures::future::join_all(parts),
        futures::future::join_all(workloads),
    )
    .await;
    let mut checked = Checked {
        workloads,
        ..Checked::default()
    };
    for (part, reach, namespaces) in parts {
        if part == Part::ArgoCd {
            checked.argocd_in = namespaces;
        }
        checked.parts.push((part, reach));
    }
    checked
}

/// The reader a part or workloads entry is checked through.
enum Target<'a, R> {
    Open(&'a R),
    Entry(&'a EntryReader),
}

#[cfg(test)]
mod tests;
