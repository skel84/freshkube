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
//! A Talos entry whose context is in none of the kubeconfig files given is
//! opened as the app opens it ([`connect_entry`]): through its talosconfig's
//! context, with the kubeconfig its control plane serves, under the same
//! connect deadline.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use kube::config::{Config, KubeConfigOptions};

use super::discover::ClusterSecrets;
use super::discover::NamespaceCounts;
use super::read::{
    ListRequest, Listing, MAX_PAGES, ReadOnlyClient, Reader, Resource, Scope, ServedGroup,
};
use super::source::printable;
use super::versions::resolve;
use super::{argocd, kargo, tekton, words};
use crate::applications::read::{PART_OF, argo_namespaces, read_argo_workloads};
use freshkube_core::exec_plugin::ConfigError;
use freshkube_core::resources::{Failure, FailureKind};
use freshkube_core::workspace::{Chosen, Entry, Part, Sources};

/// How long reading the kubeconfig, loading the credentials, running an
/// exec plugin and opening the connection may take together.
pub const CONNECT_DEADLINE: Duration = Duration::from_secs(5);
/// How long one GET may take, as the change page's own GETs.
pub const REQUEST_DEADLINE: Duration = super::change::live::pages::DEADLINE;
/// The most entries connected to at once, as the destinations are.
pub(crate) const MAX_AT_ONCE: usize = 4;

/// A reader whose every request ends by its deadline: one that doesn't
/// answer in time fails as timed out, which reads as unreachable.
#[derive(Clone)]
pub struct Deadline<R> {
    reader: R,
    limit: Duration,
    /// A whole listing's, which may read several pages; `limit` unless
    /// [`with_list_limit`](Self::with_list_limit) says otherwise.
    list_limit: Duration,
    /// Called when the cluster rejects the credentials.
    rejected: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl<R: Reader> Deadline<R> {
    pub fn new(reader: R, limit: Duration) -> Self {
        Self {
            reader,
            limit,
            list_limit: limit,
            rejected: None,
        }
    }

    /// The same, calling `rejected` whenever the cluster rejects the
    /// credentials (401).
    fn on_rejected(self, rejected: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            rejected: Some(Arc::new(rejected)),
            ..self
        }
    }

    /// `answer`, after telling whoever asked that the credentials were
    /// rejected.
    fn noticed<T>(&self, answer: Result<T, Failure>) -> Result<T, Failure> {
        if let (Err(failure), Some(rejected)) = (&answer, &self.rejected)
            && failure.kind == FailureKind::Unauthorized
        {
            rejected();
        }
        answer
    }

    /// How long each request may take.
    pub fn limit(&self) -> Duration {
        self.limit
    }

    /// The same, with a whole listing allowed `limit`.
    pub(crate) fn with_list_limit(self, limit: Duration) -> Self {
        Self {
            list_limit: limit,
            ..self
        }
    }

    async fn within<T>(
        &self,
        what: &str,
        request: impl Future<Output = Result<T, Failure>>,
    ) -> Result<T, Failure> {
        let answer = tokio::time::timeout(self.limit, request)
            .await
            .unwrap_or_else(|_| Err(Failure::timeout(what)));
        self.noticed(answer)
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

    /// The whole listing has its own limit, the request's unless set
    /// otherwise; [`reader`]'s is the deadline once for every page it may
    /// read ([`MAX_PAGES`]), and its client bounds each page by it.
    async fn list(&self, request: &ListRequest) -> Result<Listing, Failure> {
        let answer = tokio::time::timeout(self.list_limit, self.reader.list(request))
            .await
            .unwrap_or_else(|_| {
                Err(Failure::timeout(&format!(
                    "Listing {}",
                    request.resource.plural
                )))
            });
        self.noticed(answer)
    }

    async fn probe(&self, request: &ListRequest) -> Result<(), Failure> {
        self.within(
            &format!("Listing {}", request.resource.plural),
            self.reader.probe(request),
        )
        .await
    }

    async fn namespace_counts(&self, resource: &Resource) -> Result<NamespaceCounts, Failure> {
        self.within(
            &format!("Counting {}", resource.plural),
            self.reader.namespace_counts(resource),
        )
        .await
    }

    async fn any_in(&self, resource: &Resource, namespace: &str) -> Result<bool, Failure> {
        self.within(
            &format!("Reading {}", resource.plural),
            self.reader.any_in(resource, namespace),
        )
        .await
    }

    async fn cluster_secrets(&self, namespace: &str) -> Result<ClusterSecrets, Failure> {
        self.within(
            "Listing cluster Secrets",
            self.reader.cluster_secrets(namespace),
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

/// A client for one entry, every request and every page of a listing under
/// [`REQUEST_DEADLINE`], and a whole listing under it once for each page it
/// may read.
pub type EntryReader = Deadline<ReadOnlyClient>;

/// The reader for a client: [`EntryReader`]'s deadlines on it.
pub fn reader(client: kube::Client) -> EntryReader {
    bounded(ReadOnlyClient::new(client).with_page_deadline(REQUEST_DEADLINE))
}

/// [`EntryReader`]'s deadlines on `reader`, whose own pages are bounded.
fn bounded<R: Reader>(reader: R) -> Deadline<R> {
    Deadline::new(reader, REQUEST_DEADLINE).with_list_limit(REQUEST_DEADLINE * MAX_PAGES)
}

/// A client for the context `context` of the merged `kubeconfigs`, within
/// [`CONNECT_DEADLINE`]. The error is a short reason, without credentials.
pub async fn connect(kubeconfigs: Vec<PathBuf>, context: String) -> Result<EntryReader, Failure> {
    connect_within(kubeconfigs, context, CONNECT_DEADLINE).await
}

/// [`connect`] with its deadline given.
pub(crate) async fn connect_within(
    kubeconfigs: Vec<PathBuf>,
    context: String,
    limit: Duration,
) -> Result<EntryReader, Failure> {
    match open_listed(kubeconfigs, context.clone(), limit).await? {
        Some(client) => Ok(reader(client)),
        None => Err(Failure::new(
            FailureKind::Config,
            format!("the kubeconfig has no context {context}"),
        )),
    }
}

/// A client for `context` of the merged `kubeconfigs`, or `None` when none
/// of them has it, within `limit`; the files are loaded once.
async fn open_listed(
    kubeconfigs: Vec<PathBuf>,
    context: String,
    limit: Duration,
) -> Result<Option<kube::Client>, Failure> {
    // Without Talos nothing else may have chosen the TLS provider yet.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let runtime = tokio::runtime::Handle::current();
    // Files, credential files and exec plugins block; kube runs a plugin
    // while it makes the client, so all of it stays off the async workers
    // and inside the deadline.
    let setup = tokio::task::spawn_blocking(move || {
        let (report, kubeconfig) = freshkube_core::resources::load_kubeconfigs(&kubeconfigs);
        if report.context(&context).is_none() {
            return Ok(None);
        }
        let kubeconfig = kubeconfig
            .ok_or_else(|| Failure::new(FailureKind::Config, "no kubeconfig file could be read"))?;
        let options = KubeConfigOptions {
            context: Some(context),
            ..KubeConfigOptions::default()
        };
        runtime.block_on(async move {
            // kube's messages can quote credential sources; keep them out.
            let mut config =
                freshkube_core::exec_plugin::config_from_kubeconfig(kubeconfig, &options)
                    .await
                    .map_err(|error| match error {
                        ConfigError::Plugin(failure) => {
                            Failure::new(FailureKind::Config, failure.message)
                        }
                        ConfigError::Credentials => Failure::new(
                            FailureKind::Config,
                            "its kubeconfig entry couldn't be loaded",
                        ),
                    })?;
            config.connect_timeout = Some(limit);
            config.read_timeout = Some(REQUEST_DEADLINE);
            freshkube_core::kube_client::client(config)
                .map(Some)
                .map_err(|error| match error.is_exec() {
                    true => Failure::new(FailureKind::Unauthorized, "its credential plugin failed"),
                    false => Failure::new(
                        FailureKind::Other,
                        error.message("its client couldn't be made"),
                    ),
                })
        })
    });
    match tokio::time::timeout(limit, setup).await {
        Err(_) => Err(Failure::new(
            FailureKind::Timeout,
            format!(
                "no answer within {}; a credential plugin may be waiting for a sign-in",
                within(limit)
            ),
        )),
        Ok(Err(_)) => Err(stopped()),
        Ok(Ok(answer)) => answer,
    }
}

fn stopped() -> Failure {
    Failure::new(FailureKind::Other, "the connection worker stopped")
}

/// `5 s`, `200 ms`.
fn within(limit: Duration) -> String {
    match limit.subsec_millis() {
        0 => format!("{} s", limit.as_secs()),
        _ => format!("{} ms", limit.as_millis()),
    }
}

/// A client for `entry`, as the app opens it: its context in the merged
/// `kubeconfigs`; for a Talos entry whose context none of them has, the
/// kubeconfig its control plane serves, through its talosconfig's context
/// (`talos_context`, else `context`; never the file's current one). Within
/// [`CONNECT_DEADLINE`] either way.
pub async fn connect_entry(
    kubeconfigs: Vec<PathBuf>,
    entry: Entry,
) -> Result<EntryReader, Failure> {
    let Some(talosconfig) = entry.talosconfig.clone() else {
        return connect(kubeconfigs, entry.context).await;
    };
    let started = tokio::time::Instant::now();
    if let Some(client) = open_listed(kubeconfigs, entry.context.clone(), CONNECT_DEADLINE).await? {
        return Ok(reader(client));
    }
    let context = entry.talos_context.unwrap_or(entry.context);
    let left = CONNECT_DEADLINE.saturating_sub(started.elapsed());
    match tokio::time::timeout(left, through_talos(talosconfig, context, CONNECT_DEADLINE)).await {
        Err(_) => Err(Failure::new(
            FailureKind::Timeout,
            format!(
                "Talos gave no kubeconfig within {}",
                within(CONNECT_DEADLINE)
            ),
        )),
        Ok(Err(why)) => Err(why),
        // Credentials the cluster rejects are asked for again next time.
        Ok(Ok((client, key))) => Ok(reader(client).on_rejected(move || served::forget(&key))),
    }
}

/// The `kube::Client` for `entry`, connected as [`connect_entry`] does, for a
/// write (#855): the reader of the same name can only GET. Only the write
/// module sends anything through it, after its preflight.
pub async fn connect_writer(
    kubeconfigs: Vec<PathBuf>,
    entry: Entry,
) -> Result<kube::Client, Failure> {
    let Some(talosconfig) = entry.talosconfig.clone() else {
        return match open_listed(kubeconfigs, entry.context.clone(), CONNECT_DEADLINE).await? {
            Some(client) => Ok(client),
            None => Err(Failure::new(
                FailureKind::Config,
                format!("the kubeconfig has no context {}", entry.context),
            )),
        };
    };
    let started = tokio::time::Instant::now();
    if let Some(client) = open_listed(kubeconfigs, entry.context.clone(), CONNECT_DEADLINE).await? {
        return Ok(client);
    }
    let context = entry.talos_context.unwrap_or(entry.context);
    let left = CONNECT_DEADLINE.saturating_sub(started.elapsed());
    match tokio::time::timeout(left, through_talos(talosconfig, context, CONNECT_DEADLINE)).await {
        Err(_) => Err(Failure::new(
            FailureKind::Timeout,
            format!(
                "Talos gave no kubeconfig within {}",
                within(CONNECT_DEADLINE)
            ),
        )),
        Ok(Err(why)) => Err(why),
        Ok(Ok((client, _))) => Ok(client),
    }
}

/// The client for the kubeconfig a Talos context's control plane serves.
/// The errors name the file and context, never what they hold.
async fn through_talos(
    talosconfig: PathBuf,
    context: String,
    limit: Duration,
) -> Result<(kube::Client, served::Key), Failure> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let shown = talosconfig.display().to_string();
    let (talos_context, key) = tokio::task::spawn_blocking(move || {
        let config = talos_rs::config::TalosConfig::load_from(&talosconfig).map_err(|_| {
            Failure::new(
                FailureKind::Config,
                format!("its talosconfig {shown} couldn't be read"),
            )
        })?;
        let found = config.get_context(&context).cloned().map_err(|_| {
            Failure::new(
                FailureKind::Config,
                format!("its talosconfig has no context {context}"),
            )
        })?;
        Ok::<_, Failure>((found, served::key(talosconfig, context)))
    })
    .await
    .map_err(|_| stopped())??;
    // Talos signs a new admin certificate for every kubeconfig it serves:
    // one per talosconfig and context is kept for the session, in memory.
    let kubeconfig = match served::get(&key) {
        Some(kubeconfig) => kubeconfig,
        None => {
            let talos = talos_rs::client::TalosClient::from_context(&talos_context)
                .await
                .map_err(|error| {
                    Failure::new(
                        Failure::from_talos(&error).kind,
                        "its Talos endpoints couldn't be reached",
                    )
                })?;
            let served = talos.kubeconfig().await.map_err(|error| {
                Failure::new(
                    Failure::from_talos(&error).kind,
                    "its control plane gave no kubeconfig through Talos",
                )
            })?;
            let kubeconfig: kube::config::Kubeconfig =
                serde_yaml::from_str(&served).map_err(|_| {
                    Failure::new(
                        FailureKind::Config,
                        "the kubeconfig its control plane gave couldn't be read",
                    )
                })?;
            served::store(key.clone(), kubeconfig.clone());
            kubeconfig
        }
    };
    // The control plane's kubeconfig names one context, its own: named, so
    // the file's current context is never what is read.
    let options = KubeConfigOptions {
        context: kubeconfig.contexts.first().map(|named| named.name.clone()),
        ..KubeConfigOptions::default()
    };
    let mut config = Config::from_custom_kubeconfig(kubeconfig, &options)
        .await
        .map_err(|_| {
            Failure::new(
                FailureKind::Config,
                "the kubeconfig its control plane gave couldn't be loaded",
            )
        })?;
    config.connect_timeout = Some(limit);
    config.read_timeout = Some(REQUEST_DEADLINE);
    freshkube_core::kube_client::client(config)
        .map(|client| (client, key))
        .map_err(|error| {
            Failure::new(
                FailureKind::Other,
                error.message("its client couldn't be made"),
            )
        })
}

/// An entry's id, and its client or why there is none.
pub type Connected = (String, Result<EntryReader, Failure>);

/// Clients for `entries`, at most [`MAX_AT_ONCE`] connecting at a time, in
/// the order given, each with its entry's id. Each connects within its own
/// deadline, and a slow one never keeps the next from starting.
pub(crate) async fn connect_all(kubeconfigs: &[PathBuf], entries: Vec<Entry>) -> Vec<Connected> {
    connect_each(entries, |entry| connect_entry(kubeconfigs.to_vec(), entry)).await
}

/// [`connect_all`] through `connect`.
async fn connect_each<C, F, Fut>(entries: Vec<Entry>, connect: F) -> Vec<(String, C)>
where
    F: Fn(Entry) -> Fut,
    Fut: Future<Output = C>,
{
    let mut connected: Vec<(usize, (String, C))> =
        futures::stream::iter(entries.into_iter().enumerate())
            .map(|(number, entry)| {
                let id = entry.id.clone();
                let connecting = connect(entry);
                async move { (number, (id, connecting.await)) }
            })
            .buffer_unordered(MAX_AT_ONCE)
            .collect()
            .await;
    connected.sort_by_key(|(number, _)| *number);
    connected.into_iter().map(|(_, entry)| entry).collect()
}

/// What a part's entry answered. Each failure's kind agrees with its
/// variant ([`Reach::missing`] and the others make them so), and the
/// sentence comes from the variant's own kind ([`words::failed`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    /// No entry holds the part, or the one pinned is gone; the words say
    /// which. A [`FailureKind::Config`].
    Missing(Failure),
    /// The entry couldn't be reached: a deadline, DNS, TLS, or its exec
    /// plugin failed or waits for a sign-in. The words are why.
    Unreachable(Failure),
    /// 401 or 403, and the server's words.
    Refused(Failure),
    /// The part's kind isn't served there (404).
    NotServed(Failure),
    /// It answered.
    Readable,
}

impl Reach {
    /// No entry holds the part.
    pub fn missing(why: impl Into<String>) -> Self {
        Reach::Missing(Failure::new(FailureKind::Config, why))
    }

    /// The entry wasn't reached, for `failure`'s reason: a deadline stays a
    /// timeout, anything else is a failure to reach.
    pub(crate) fn unreached(failure: Failure) -> Self {
        let kind = match failure.kind {
            FailureKind::Timeout => FailureKind::Timeout,
            FailureKind::Config
            | FailureKind::Unreachable
            | FailureKind::Unauthorized
            | FailureKind::Forbidden
            | FailureKind::NotFound
            | FailureKind::Other => FailureKind::Unreachable,
        };
        Reach::Unreachable(Failure::new(kind, failure.message))
    }

    /// The entry wasn't reached, for a reason with no kind of its own.
    pub fn unreachable(why: impl Into<String>) -> Self {
        Reach::Unreachable(Failure::new(FailureKind::Unreachable, why))
    }

    /// The entry refused the read (401 or 403).
    pub fn refused(why: impl Into<String>) -> Self {
        Reach::Refused(Failure::new(FailureKind::Forbidden, why))
    }

    /// The entry doesn't serve the part's kind (404).
    pub fn not_served(why: impl Into<String>) -> Self {
        Reach::NotServed(Failure::new(FailureKind::NotFound, why))
    }

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
            Reach::Missing(failure) => {
                words::failed(FailureKind::Config, what, entry, &failure.message)
            }
            Reach::Unreachable(failure) => {
                words::failed(FailureKind::Unreachable, what, entry, &failure.message)
            }
            Reach::Refused(failure) => {
                words::failed(FailureKind::Forbidden, what, entry, &failure.message)
            }
            Reach::NotServed(failure) => {
                words::failed(FailureKind::NotFound, what, entry, &failure.message)
            }
            Reach::Readable => format!("{what} answered on {entry}"),
        }
    }

    /// What `failure` says about its entry. Its message is made printable
    /// ([`printable`]); a kind with no refusal or 404 in it is a failure to
    /// reach.
    fn of_failure(failure: Failure) -> Self {
        let message = printable(&failure);
        match failure.kind {
            FailureKind::NotFound => Reach::NotServed(Failure::new(failure.kind, message)),
            FailureKind::Forbidden | FailureKind::Unauthorized => {
                Reach::Refused(Failure::new(failure.kind, message))
            }
            FailureKind::Timeout => Reach::Unreachable(Failure::new(failure.kind, message)),
            FailureKind::Config | FailureKind::Unreachable | FailureKind::Other => {
                Reach::Unreachable(Failure::new(FailureKind::Unreachable, message))
            }
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
pub(crate) async fn check_argo<R: Reader>(reader: &R) -> (Reach, Vec<String>) {
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
pub(crate) async fn check_workloads<R: Reader>(reader: &R) -> Reach {
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
    open: Option<Result<&R, Failure>>,
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
        .map(|open| open.as_ref().map_err(Failure::clone));
    let target = |chosen: &Chosen| -> Result<Target<'_, R>, Reach> {
        match chosen {
            Chosen::NoEntry(_) | Chosen::Gone(_) | Chosen::Skipped(_) => {
                Err(Reach::missing(chosen.missing().unwrap_or_default()))
            }
            Chosen::Open => match &open {
                Some(Ok(reader)) => Ok(Target::Open(reader)),
                Some(Err(why)) => Err(Reach::unreached(why.clone())),
                None => Err(Reach::unreachable("no cluster is open")),
            },
            Chosen::Entry { entry, .. } => match readers.iter().find(|(id, _)| id == &entry.id) {
                Some((_, Ok(reader))) => Ok(Target::Entry(reader)),
                Some((_, Err(why))) => Err(Reach::unreached(why.clone())),
                None => Err(Reach::unreachable("it wasn't connected to")),
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
            Chosen::Skipped(row) => row.label(),
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

pub mod pool;
mod served;
#[cfg(test)]
mod tests;
