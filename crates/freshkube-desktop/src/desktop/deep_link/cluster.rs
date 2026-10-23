//! Which cluster a link opens in (#873): the open cluster's own fingerprint
//! for Copy link, the candidates a link is matched against, Check, the
//! dialog that asks, and the switch to the chosen cluster.
//!
//! The window never switches context unasked: a switch is the one match, the
//! one open, or the reader's own choice in the dialog, and it goes through
//! the same switch as the header's, which a running shell does not hold up (it
//! keeps its own connection).
use super::super::settings::FingerprintLine;
use super::identity::{Cache, Candidate, Checked, Decision, Local, Record, Target, decide};
use super::picker::{LinkChoice, Why, rows};
use super::*;
use crate::backend::{self, OwnedJob};
use crate::debug_options::LinkShape;
use freshkube_core::fingerprint::{
    Fingerprint, StrongRead, ca_of_context, read_strong, strong_from_override, strong_from_uid,
};
use freshkube_core::resources::{
    connect, context_servers, kubeconfig_sources, load_kubeconfigs as load,
};
use freshkube_nav::{ClusterRef, FingerprintToken};
use freshkube_prefs::{Document, Layout};
use gpui_kit::component::WindowExt as _;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a Check may take, a sign-in included.
const CHECK_DEADLINE: Duration = Duration::from_secs(45);
/// How long a view held for a context switch waits for it.
const HOLD: Duration = Duration::from_secs(60);

type Continue = Box<dyn FnOnce(&mut Pilot, &mut Window, &mut Context<Pilot>)>;

/// A view waiting for the context the reader chose to open.
pub(super) struct Held {
    context: String,
    view: ViewState,
    at: Instant,
}

/// The dialog that asks which cluster, while it is open.
pub(super) struct Choosing {
    link: Link,
    why: Why,
    pub(super) entity: Entity<LinkChoice>,
}

impl Choosing {
    #[cfg(test)]
    pub(super) fn why_title(&self) -> &'static str {
        self.why.title()
    }
}

/// What the shell keeps of clusters' identities.
#[derive(Default)]
pub(super) struct Identities {
    cache: Cache,
    document: Option<Arc<Document<Cache>>>,
    /// What the kubeconfig says of each context, as last read.
    pub(super) local: BTreeMap<String, Local>,
    /// Why the last Check of a context failed.
    failed: BTreeMap<String, String>,
    /// The source whose cluster this window has identified.
    identified: Option<String>,
    checks: BTreeMap<String, (OwnedJob, Task<()>)>,
    reads: Vec<(OwnedJob, Task<()>)>,
    /// The kubeconfig read, apart from `reads` so that finishing it never
    /// drops a cluster's read still under way.
    local_read: Option<(OwnedJob, Task<()>)>,
    waiting: Vec<Continue>,
    reading_local: bool,
    pub(super) choosing: Option<Choosing>,
    held: Option<Held>,
    /// The source whose open cluster is being identified: links wait for the
    /// answer or the failure, or they would match against a cache that
    /// doesn't know the cluster yet.
    identifying: Option<String>,
    /// Counts the cache's saves; the file takes only the latest ([`save_latest`]).
    saves: u64,
    saved: Arc<std::sync::Mutex<u64>>,
    /// Makes the example read answer after this long, for tests.
    #[cfg(test)]
    pub(super) example_delay: Option<Duration>,
}

impl Identities {
    #[cfg(test)]
    pub(super) fn valid(&self, context: &str, local: Option<&Local>) -> Option<&Record> {
        self.cache.valid(context, local)
    }

    #[cfg(test)]
    pub(super) fn keep(&mut self, context: &str, record: Record) -> std::io::Result<()> {
        self.cache.put(context, record);
        match &self.document {
            Some(document) => save(document, self.cache.clone()),
            None => Ok(()),
        }
    }

    /// Makes the open cluster unidentified again, as on a cold start, with
    /// the example read answering `delay` later.
    #[cfg(test)]
    pub(super) fn go_cold(&mut self, delay: Duration) {
        self.cache = Cache::default();
        self.identified = None;
        self.example_delay = Some(delay);
    }

    #[cfg(test)]
    pub(super) fn snapshot(&self) -> Cache {
        self.cache.clone()
    }

    /// Saves `cache` as snapshot number `seq`, as `record`'s background save does.
    #[cfg(test)]
    pub(super) fn save_numbered(&self, cache: Cache, seq: u64) -> std::io::Result<()> {
        match &self.document {
            Some(document) => save_latest(document, cache, seq, &self.saved),
            None => Ok(()),
        }
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.cache.contexts.is_empty()
    }

    #[cfg(test)]
    pub(super) fn has_record(&self, context: &str) -> bool {
        self.cache.contexts.contains_key(context)
    }

    /// Loads `fingerprints.json` beside the preferences, if there are any.
    pub(super) fn open(preferences: Option<&std::path::Path>) -> Self {
        let document = preferences.map(|path| {
            Arc::new(
                Document::<Cache>::at(
                    &path.with_file_name("fingerprints.json"),
                    Layout::Compact,
                    0o600,
                )
                .limited_to(256 * 1024),
            )
        });
        let cache = document
            .as_ref()
            .map(|document| {
                let loaded = document.load();
                if let Some(problem) = loaded.problem {
                    tracing::warn!("fingerprints.json {problem}");
                }
                loaded.value
            })
            .unwrap_or_default();
        Self {
            cache,
            document,
            ..Self::default()
        }
    }
}

/// Writes the cache over the file's: blocking.
fn save(document: &Document<Cache>, cache: Cache) -> std::io::Result<()> {
    document.update(|saved| *saved = cache)
}

/// Writes snapshot number `seq` unless a later one is already on the file:
/// the saves run in the background and can finish in any order, but each
/// holds the whole cache, so the latest wins.
fn save_latest(
    document: &Document<Cache>,
    cache: Cache,
    seq: u64,
    last: &std::sync::Mutex<u64>,
) -> std::io::Result<()> {
    let mut last = last.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if seq <= *last {
        return Ok(());
    }
    save(document, cache)?;
    *last = seq;
    Ok(())
}

fn token(text: &Option<String>) -> Option<FingerprintToken> {
    text.as_deref()
        .and_then(|text| FingerprintToken::parse(text).ok())
}

impl Pilot {
    fn shape(&self) -> Option<LinkShape> {
        self.deep_link.shape.filter(|_| self.mode().is_example())
    }

    /// The context the window reads Kubernetes through, as its source names
    /// it: an entry's, a Kubernetes-only window's own, or the Talos context a
    /// Talos session derives its kubeconfig from.
    pub(super) fn open_kube_context(&self, _: &App) -> Option<String> {
        self.source().kube.map(|source| source.context)
    }

    fn kube_sources(&self, cx: &App) -> Vec<PathBuf> {
        if let Some(kube) = self.mode().kubernetes_only() {
            return kube.sources.clone();
        }
        kubeconfig_sources(
            self.settings_page
                .read(cx)
                .workspace()
                .kubeconfig
                .as_deref(),
        )
    }

    /// What this window knows of `context`'s identity, and how far.
    pub(super) fn identity_of(&self, context: &str, over: Option<&str>) -> (Fingerprint, Checked) {
        let local = self.deep_link.identities.local.get(context);
        let ca = local.and_then(|local| local.ca.clone());
        if let Some(over) = over.and_then(strong_from_override) {
            return (
                Fingerprint {
                    strong: Some(over),
                    ca,
                },
                Checked::Yes,
            );
        }
        let identities = &self.deep_link.identities;
        match identities.cache.valid(context, local) {
            Some(record) if record.refused => (Fingerprint { strong: None, ca }, Checked::Refused),
            Some(record) => (
                Fingerprint {
                    strong: record.strong.clone(),
                    ca,
                },
                Checked::Yes,
            ),
            None => (
                Fingerprint { strong: None, ca },
                identities
                    .failed
                    .get(context)
                    .map_or(Checked::No, |why| Checked::Failed(why.clone())),
            ),
        }
    }

    /// The fingerprint Copy link puts in a link: the open cluster's, as far
    /// as it is known. Without any, the link names no cluster.
    pub(super) fn link_for(&self, view: ViewState, cx: &App) -> Link {
        let cluster = self
            .open_kube_context(cx)
            .map(|context| {
                let over = self.override_of(&context, cx);
                self.identity_of(&context, over.as_deref()).0
            })
            .map(|identity| ClusterRef {
                strong: token(&identity.strong),
                ca: token(&identity.ca),
            })
            .unwrap_or_default();
        Link { cluster, view }
    }

    fn override_of(&self, context: &str, cx: &App) -> Option<String> {
        let workspace = self.settings_page.read(cx).workspace();
        // The active entry's own override wins; else an entry of the context.
        let active = self.switch.active_cluster();
        workspace
            .clusters
            .iter()
            .find(|entry| Some(entry.id.as_str()) == active && entry.context == context)
            .or_else(|| {
                workspace
                    .clusters
                    .iter()
                    .find(|entry| entry.context == context && entry.fingerprint.is_some())
            })
            .and_then(|entry| entry.fingerprint.as_deref().map(str::to_owned))
    }

    /// Tells Settings what each entry's link fingerprint is.
    pub(super) fn push_fingerprints(&mut self, cx: &mut Context<Self>) {
        let entries: Vec<_> = self
            .settings_page
            .read(cx)
            .workspace()
            .clusters
            .iter()
            .map(|entry| {
                (
                    entry.id.clone(),
                    entry.context.clone(),
                    entry.fingerprint.clone(),
                )
            })
            .collect();
        let lines = entries
            .into_iter()
            .map(|(id, context, over)| {
                let (identity, checked) = self.identity_of(&context, over.as_deref());
                let value = |token: &Option<String>| token.clone().map(SharedString::from);
                let line = match (&checked, over.is_some()) {
                    (Checked::Yes, true) => FingerprintLine {
                        value: value(&identity.strong),
                        note: "A hash of the fingerprint set in freshkube.yaml; the text itself is never in a link.".into(),
                    },
                    (Checked::Yes, false) => FingerprintLine {
                        value: value(&identity.strong),
                        note: "A salted hash of the cluster’s kube-system namespace UID, read with your own access.".into(),
                    },
                    (Checked::Refused, _) => FingerprintLine {
                        value: value(&identity.ca),
                        note: "CA only: your access can’t read kube-system, so links match on the cluster’s certificate authority and always ask. Set fingerprint in freshkube.yaml to match exactly.".into(),
                    },
                    (Checked::Failed(why), _) => FingerprintLine {
                        value: None,
                        note: format!("Can’t check: {why}").into(),
                    },
                    (Checked::No, _) => FingerprintLine {
                        value: None,
                        note: "Not checked yet. It is read when the cluster opens, or by Check when a link asks.".into(),
                    },
                };
                (id, line)
            })
            .collect();
        self.settings_page
            .update(cx, |page, cx| page.set_fingerprints(lines, cx));
    }

    /// The clusters a link may open in: the workspace's entries, then, in a
    /// window without Talos, the kubeconfig's contexts no entry covers.
    pub(super) fn candidates(&self, cx: &App) -> Vec<Candidate> {
        let workspace = self.settings_page.read(cx).workspace();
        let open_context = self.open_kube_context(cx);
        let mut found: Vec<Candidate> = workspace
            .clusters
            .iter()
            .map(|entry| {
                let (identity, checked) =
                    self.identity_of(&entry.context, entry.fingerprint.as_deref());
                Candidate {
                    target: Target::Entry(entry.id.clone()),
                    label: entry.id.clone(),
                    context: entry.context.clone(),
                    identity,
                    checked,
                    open: open_context.as_deref() == Some(entry.context.as_str()),
                }
            })
            .collect();
        if !self.mode().has_talos() {
            for context in &self.contexts {
                if workspace
                    .clusters
                    .iter()
                    .any(|entry| &entry.context == context)
                {
                    continue;
                }
                let (identity, checked) = self.identity_of(context, None);
                found.push(Candidate {
                    target: Target::Context(context.clone()),
                    label: context.clone(),
                    context: context.clone(),
                    identity,
                    checked,
                    open: open_context.as_deref() == Some(context.as_str()),
                });
            }
        }
        found
    }

    /// Reads what the kubeconfig says of every context (its server and CA,
    /// locally, with no credential and no network), then runs `then`. Example
    /// data answers at once.
    fn with_local(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Pilot, &mut Window, &mut Context<Pilot>) + 'static,
    ) {
        if self.mode().is_example() {
            let shape = self.shape();
            let mut contexts: Vec<String> = self
                .settings_page
                .read(cx)
                .workspace()
                .clusters
                .iter()
                .map(|entry| entry.context.clone())
                .collect();
            contexts.extend(self.contexts.iter().cloned());
            for context in contexts {
                let local = example::local(&context, shape);
                self.deep_link.identities.local.insert(context, local);
            }
            self.push_fingerprints(cx);
            then(self, window, cx);
            return;
        }
        self.deep_link.identities.waiting.push(Box::new(then));
        if self.deep_link.identities.reading_local {
            return;
        }
        self.deep_link.identities.reading_local = true;
        let sources = self.kube_sources(cx);
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            Duration::from_secs(15),
            "Reading the kubeconfig timed out".to_owned(),
            async move {
                tokio::task::spawn_blocking(move || read_local(&sources))
                    .await
                    .map_err(|error| error.to_string())
            },
        );
        let task = cx.spawn_in(window, async move |this, cx| {
            let local = receiver
                .await
                .unwrap_or_else(|_| Err("The read stopped".into()));
            _ = this.update_in(cx, |this, window, cx| {
                this.deep_link.identities.reading_local = false;
                this.deep_link.identities.local_read = None;
                if let Ok(local) = local {
                    this.deep_link.identities.local.extend(local);
                }
                for step in std::mem::take(&mut this.deep_link.identities.waiting) {
                    step(this, window, cx);
                }
                this.push_fingerprints(cx);
            });
        });
        self.deep_link.identities.local_read = Some((job, task));
    }

    /// Identifies the cluster the window has just opened: its `kube-system`
    /// namespace read once, with the reader's own access, and kept until its
    /// server or CA changes. A cluster that refuses the read is "CA only".
    pub(in crate::desktop) fn identify_open_cluster(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.source().kube else {
            return;
        };
        if self.deep_link.identities.identified.as_deref() == Some(source.id.as_str()) {
            return;
        }
        let Some(context) = self.open_kube_context(cx) else {
            return;
        };
        self.deep_link.identities.identified = Some(source.id.clone());
        self.deep_link.identities.identifying = Some(source.id.clone());
        if self.shape() == Some(LinkShape::Unchecked) {
            let id = source.id.clone();
            return self.with_local(window, cx, move |this, window, cx| {
                this.identified(&id, window, cx);
            });
        }
        let over = self.override_of(&context, cx);
        self.with_local(window, cx, move |this, window, cx| {
            // A Talos session's Kubernetes access has no kubeconfig context
            // to read a server and CA from: its connection stands for them.
            let local = this
                .deep_link
                .identities
                .local
                .entry(context.clone())
                .or_insert_with(|| Local {
                    server: strong_from_uid(&source.id),
                    ca: None,
                })
                .clone();
            let local = Some(local);
            let known = over.is_some()
                || this
                    .deep_link
                    .identities
                    .cache
                    .valid(&context, local.as_ref())
                    .is_some();
            cx.notify();
            if known {
                this.identified(&source.id, window, cx);
                return;
            }
            if this.mode().is_example() {
                this.answer_example(&context, local, source.id.clone(), window, cx);
                return;
            }
            let wanted = source.id.clone();
            let access = source.access.clone();
            let (job, receiver) = backend::spawn_job(
                &this.runtime,
                freshkube_core::fingerprint::READ_DEADLINE + Duration::from_secs(20),
                "Reading the cluster timed out".to_owned(),
                async move {
                    let client = access.client().await.map_err(|failure| failure.message)?;
                    Ok(read_strong(&client).await)
                },
            );
            let task = cx.spawn_in(window, async move |this, cx| {
                let answer = receiver
                    .await
                    .unwrap_or_else(|_| Err("The read stopped".into()));
                _ = this.update_in(cx, |this, window, cx| {
                    // The window moved on to another cluster meanwhile.
                    if this.deep_link.identities.identified.as_deref() != Some(wanted.as_str()) {
                        return;
                    }
                    let answer = answer.unwrap_or_else(StrongRead::Failed);
                    this.record(&context, local, answer, cx);
                    this.identified(&wanted, window, cx);
                });
            });
            this.deep_link.identities.reads.push((job, task));
        });
    }

    /// The example cluster's read, answered at once (after a delay, in tests
    /// that need the read to be pending).
    fn answer_example(
        &mut self,
        context: &str,
        local: Option<Local>,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(test)]
        if let Some(delay) = self.deep_link.identities.example_delay {
            let context = context.to_owned();
            let task = cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(delay).await;
                _ = this.update_in(cx, |this, window, cx| {
                    let answer = example::read(&context, this.shape());
                    this.record(&context, local, answer, cx);
                    this.identified(&id, window, cx);
                });
            });
            task.detach();
            return;
        }
        let answer = example::read(context, self.shape());
        self.record(context, local, answer, cx);
        self.identified(&id, window, cx);
    }

    /// The open cluster `id` is identified, or its read failed: the links
    /// that waited for that can open now.
    fn identified(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.deep_link.identities.identifying.as_deref() == Some(id) {
            self.deep_link.identities.identifying = None;
        }
        self.open_waiting_links(window, cx);
    }

    /// Whether the open cluster's identity is not known yet: its read is
    /// under way, or the window has a Kubernetes source it hasn't started
    /// identifying (a Talos session installs its access before the summary
    /// that does).
    pub(in crate::desktop) fn identifying(&self) -> bool {
        let identities = &self.deep_link.identities;
        match self.source().kube {
            Some(source) => {
                identities.identified.as_deref() != Some(source.id.as_str())
                    || identities.identifying.as_deref() == Some(source.id.as_str())
            }
            None => false,
        }
    }

    /// Keeps what a read of `context` found, and saves it beside the
    /// preferences. A failure is shown, never saved.
    pub(super) fn record(
        &mut self,
        context: &str,
        local: Option<Local>,
        answer: StrongRead,
        cx: &mut Context<Self>,
    ) {
        let example = self.mode().is_example();
        let identities = &mut self.deep_link.identities;
        let (strong, refused) = match answer {
            StrongRead::Uid(uid) => (Some(strong_from_uid(&uid)), false),
            StrongRead::Refused => (None, true),
            StrongRead::Failed(why) => {
                identities.failed.insert(context.to_owned(), why);
                self.refresh_choice(cx);
                cx.notify();
                return;
            }
        };
        identities.failed.remove(context);
        if let Some(local) = local {
            identities.cache.put(
                context,
                Record {
                    server: local.server,
                    ca: local.ca,
                    strong,
                    refused,
                },
            );
            // Example data saves nothing.
            if let Some(document) = identities.document.clone().filter(|_| !example) {
                let cache = identities.cache.clone();
                identities.saves += 1;
                let (seq, last) = (identities.saves, identities.saved.clone());
                cx.background_executor()
                    .spawn(async move {
                        if let Err(error) = save_latest(&document, cache, seq, &last) {
                            tracing::warn!("fingerprints.json not saved: {error}");
                        }
                    })
                    .detach();
            }
        }
        self.refresh_choice(cx);
        self.push_fingerprints(cx);
        cx.notify();
    }

    /// Check on a row of the dialog: reads that context's `kube-system`
    /// once. It may run an exec plugin or a sign-in, which is why only the
    /// reader's click starts it.
    pub(super) fn check_context(
        &mut self,
        context: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.deep_link.identities.checks.contains_key(&context) {
            return;
        }
        self.deep_link.identities.failed.remove(&context);
        if self.mode().is_example() {
            let local = self.deep_link.identities.local.get(&context).cloned();
            let answer = example::read(&context, self.shape());
            self.record(&context, local, answer, cx);
            return;
        }
        let sources = self.kube_sources(cx);
        let local = self.deep_link.identities.local.get(&context).cloned();
        let wanted = context.clone();
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            CHECK_DEADLINE,
            "The check timed out".to_owned(),
            async move {
                let connection = connect(sources, wanted)
                    .await
                    .map_err(|failure| failure.message)?;
                Ok(read_strong(&connection.client).await)
            },
        );
        let key = context.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let answer = receiver
                .await
                .unwrap_or_else(|_| Err("The check stopped".into()));
            _ = this.update_in(cx, |this, _, cx| {
                this.deep_link.identities.checks.remove(&key);
                let answer = answer.unwrap_or_else(StrongRead::Failed);
                this.record(&key, local, answer, cx);
            });
        });
        self.deep_link
            .identities
            .checks
            .insert(context, (job, task));
        self.refresh_choice(cx);
    }

    /// Opens a link, in the cluster it names.
    ///
    /// A link with no fingerprint opens in the cluster that is open. One with
    /// a fingerprint is matched to this window's clusters (`identity::decide`):
    /// one match opens there, several ask unless one is open, a match on the
    /// CA alone always asks, and none says so and offers the contexts.
    pub(super) fn open_link(&mut self, link: Link, window: &mut Window, cx: &mut Context<Self>) {
        self.deep_link.identities.held = None;
        if link.cluster == ClusterRef::default() {
            self.apply_view(&link.view, window, cx);
            return;
        }
        self.with_local(window, cx, move |this, window, cx| {
            this.choose_cluster(link, window, cx)
        });
    }

    fn wanted(link: &Link) -> Fingerprint {
        Fingerprint {
            strong: link
                .cluster
                .strong
                .as_ref()
                .map(|token| token.as_str().to_owned()),
            ca: link
                .cluster
                .ca
                .as_ref()
                .map(|token| token.as_str().to_owned()),
        }
    }

    fn choose_cluster(&mut self, link: Link, window: &mut Window, cx: &mut Context<Self>) {
        let candidates = self.candidates(cx);
        let wanted = Self::wanted(&link);
        match decide(&wanted, &candidates) {
            Decision::Here => self.apply_view(&link.view, window, cx),
            Decision::Open(target) => self.go_to(target, link.view, window, cx),
            Decision::Ask => self.ask_cluster(link, Why::Choose, window, cx),
            Decision::NoMatch => self.ask_cluster(link, Why::NoMatch, window, cx),
        }
    }

    fn checking(&self) -> Vec<String> {
        self.deep_link.identities.checks.keys().cloned().collect()
    }

    fn ask_cluster(&mut self, link: Link, why: Why, window: &mut Window, cx: &mut Context<Self>) {
        // Another link replaces one still being asked about.
        if self.deep_link.identities.choosing.take().is_some() {
            window.close_dialog(cx);
        }
        let candidates = self.candidates(cx);
        let listed = rows(why, &Self::wanted(&link), &candidates, &self.checking());
        let pilot = cx.weak_entity();
        let entity = cx.new(|_| LinkChoice::new(pilot.clone(), why, listed));
        self.deep_link.identities.choosing = Some(Choosing {
            link,
            why,
            entity: entity.clone(),
        });
        let on_close = pilot.clone();
        window.open_dialog(cx, move |dialog, window, _| {
            let close = on_close.clone();
            let dialog = dialog.on_cancel(move |_, _, cx| {
                _ = close.update(cx, |pilot, _| pilot.deep_link.identities.choosing = None);
                true
            });
            freshkube_ui::dialog::frame(
                dialog,
                600.,
                why.title(),
                entity.clone(),
                // Closed by its own X or Escape; no button beside the rows.
                div(),
                window,
            )
        });
    }

    /// Refreshes the dialog's rows after a Check.
    fn refresh_choice(&mut self, cx: &mut Context<Self>) {
        let Some(choosing) = &self.deep_link.identities.choosing else {
            return;
        };
        let (why, entity, wanted) = (
            choosing.why,
            choosing.entity.clone(),
            Self::wanted(&choosing.link),
        );
        let candidates = self.candidates(cx);
        let listed = rows(why, &wanted, &candidates, &self.checking());
        entity.update(cx, |choice, cx| choice.set(why, listed, cx));
    }

    /// "Open here" on a row: the reader's own choice.
    pub(super) fn open_in_choice(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(choosing) = self.deep_link.identities.choosing.take() else {
            return;
        };
        window.close_dialog(cx);
        self.go_to(target, choosing.link.view, window, cx);
    }

    /// Opens `view` in `target`, switching to it through the header's own
    /// switch. A target that is already open only navigates, and a running
    /// shell is left alone either way.
    fn go_to(
        &mut self,
        target: Target,
        view: ViewState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            Target::Entry(id) => {
                let context = self.candidates(cx).into_iter().find_map(|candidate| {
                    (candidate.target == Target::Entry(id.clone())).then_some(candidate.context)
                });
                if self.switch.active_cluster() == Some(id.as_str())
                    || (context.is_some() && self.open_kube_context(cx) == context)
                {
                    self.apply_view(&view, window, cx);
                } else {
                    self.switch_cluster_to_view(id, view, window, cx);
                }
            }
            Target::Context(context) => {
                if self.open_kube_context(cx).as_deref() == Some(context.as_str()) {
                    self.apply_view(&view, window, cx);
                    return;
                }
                self.deep_link.identities.held = Some(Held {
                    context: context.clone(),
                    view,
                    at: cx.background_executor().now(),
                });
                self.select_context(context, window, cx);
            }
        }
    }

    /// Opens the view held for the context the window has switched to, once
    /// its Kubernetes source is there. A view held too long, or for a context
    /// the window isn't on, is dropped.
    pub(in crate::desktop) fn apply_held_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(held) = &self.deep_link.identities.held else {
            return;
        };
        if cx.background_executor().now().duration_since(held.at) > HOLD {
            self.deep_link.identities.held = None;
            return;
        }
        if self.applied.context.as_deref() != Some(held.context.as_str())
            || self.source().kube.is_none()
        {
            return;
        }
        if let Some(held) = self.deep_link.identities.held.take() {
            self.apply_view(&held.view, window, cx);
        }
    }
}

/// What the kubeconfig sources say of each context: a hash of its server, and
/// its CA value. Blocking file reads: run on a worker.
fn read_local(sources: &[PathBuf]) -> BTreeMap<String, Local> {
    let (_, kubeconfig) = load(sources);
    let Some(kubeconfig) = kubeconfig else {
        return BTreeMap::new();
    };
    context_servers(&kubeconfig)
        .into_iter()
        .map(|(context, server)| {
            let ca = ca_of_context(&kubeconfig, &context);
            (
                context,
                Local {
                    server: strong_from_uid(&server),
                    ca,
                },
            )
        })
        .collect()
}
