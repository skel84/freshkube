//! In-memory reuse of Talos connections and identity-validated Kubernetes clients.
//!
//! Everything here lives in process memory only. Entries hold credentials
//! (TLS channels, kubeconfig-derived clients), so none of these types
//! implement `Debug`, and nothing is ever logged or written to disk.
//!
//! Reuse is strictly keyed: an entry is returned only for an identical key, and
//! every creation path still runs the full validation it ran before caching.

use std::{
    collections::HashMap,
    hash::{BuildHasher, Hash},
    path::{Path, PathBuf},
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use kube::Client;
use talos_rs::TalosClient;

use crate::kubeconfig_selection::{KubeconfigSelection, PreparedKubeconfig};

/// How long a validated Kubernetes client or prepared kubeconfig is reused.
pub(crate) const KUBERNETES_TTL: Duration = Duration::from_secs(120);
const MAX_ENTRIES: usize = 16;

/// A bounded cache whose entries expire. Time is injected so tests need not sleep.
pub(crate) struct TtlCache<K, V> {
    entries: HashMap<K, (V, Instant)>,
    ttl: Option<Duration>,
    capacity: usize,
}

impl<K: Eq + Hash + Clone, V: Clone> TtlCache<K, V> {
    pub(crate) fn new(ttl: Option<Duration>, capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            ttl,
            capacity: capacity.max(1),
        }
    }

    fn expired(&self, stored: Instant, now: Instant) -> bool {
        self.ttl
            .is_some_and(|ttl| now.saturating_duration_since(stored) >= ttl)
    }

    /// The entry for exactly `key`, unless it has expired (which removes it).
    pub(crate) fn get(&mut self, key: &K, now: Instant) -> Option<V> {
        let stored = self.entries.get(key).map(|(_, stored)| *stored)?;
        if self.expired(stored, now) {
            self.entries.remove(key);
            return None;
        }
        self.entries.get(key).map(|(value, _)| value.clone())
    }

    pub(crate) fn insert(&mut self, key: K, value: V, now: Instant) {
        if !self.entries.contains_key(&key) && self.entries.len() >= self.capacity {
            let expired = self
                .entries
                .iter()
                .filter(|(_, (_, stored))| self.expired(*stored, now))
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            for key in expired {
                self.entries.remove(&key);
            }
            if self.entries.len() >= self.capacity
                && let Some(oldest) = self
                    .entries
                    .iter()
                    .min_by_key(|(_, (_, stored))| *stored)
                    .map(|(key, _)| key.clone())
            {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(key, (value, now));
    }

    pub(crate) fn remove(&mut self, key: &K) {
        self.entries.remove(key);
    }

    pub(crate) fn remove_where(&mut self, mut predicate: impl FnMut(&K) -> bool) {
        self.entries.retain(|key, _| !predicate(key));
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Opaque content fingerprint of a loaded talosconfig, for change detection only.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConfigIdentity(u64);

impl ConfigIdentity {
    /// Fingerprints the exact bytes a talosconfig was parsed from.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        use std::hash::Hasher;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write(bytes);
        Self(hasher.finish())
    }
}

/// One authenticated access lifetime. Cloning or retrying its transport keeps
/// this identity; replacing the access starts another lifetime. Node selection
/// and individual request generations are deliberately separate.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccessSessionId {
    origin: SessionOrigin,
    serial: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum SessionOrigin {
    Direct,
    Talos,
}

impl AccessSessionId {
    /// Starts a direct Kubernetes access session, before its first connection.
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            origin: SessionOrigin::Direct,
            serial: NEXT.fetch_add(1, Ordering::Relaxed),
        }
    }

    fn talos(client: &TalosClient) -> Self {
        Self {
            origin: SessionOrigin::Talos,
            serial: client.connection_id(),
        }
    }
}

impl Default for AccessSessionId {
    fn default() -> Self {
        Self::new()
    }
}

/// Opaque revision of local configuration and referenced credential files.
/// Reading a revision performs bounded local I/O and belongs on a worker.
/// Contents and their fingerprints are never formatted or logged.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConfigurationRevision(u64);

fn fingerprint(value: impl Hash) -> u64 {
    static HASHER: LazyLock<std::collections::hash_map::RandomState> =
        LazyLock::new(std::collections::hash_map::RandomState::new);
    HASHER.hash_one(value)
}

fn file_revision(path: &Path) -> Option<ConfigIdentity> {
    crate::kubeconfig_selection::read_bounded_regular_file(path, 4 * 1024 * 1024)
        .ok()
        .map(|bytes| ConfigIdentity::from_bytes(&bytes))
}

fn kubeconfig_revision(path: &Path) -> u64 {
    let references = crate::kubeconfig_selection::read_selected_file(path)
        .ok()
        .map(|config| {
            let authorities = config
                .clusters
                .into_iter()
                .filter_map(|entry| entry.cluster)
                .filter_map(|cluster| cluster.certificate_authority);
            let credentials = config
                .auth_infos
                .into_iter()
                .filter_map(|entry| entry.auth_info)
                .flat_map(|user| [user.client_certificate, user.client_key, user.token_file])
                .flatten();
            authorities
                .chain(credentials)
                .map(|path| {
                    let revision = file_revision(Path::new(&path));
                    (path, revision)
                })
                .collect::<Vec<_>>()
        });
    fingerprint((file_revision(path), references))
}

impl ConfigurationRevision {
    /// The local files that can replace Talos or Kubernetes access. Resolve and
    /// fingerprint these on a worker, including when connecting will fail.
    pub fn for_talos_sources(path: Option<&Path>, selection: &KubeconfigSelection) -> Self {
        let path = path
            .map(Path::to_path_buf)
            .or_else(|| talos_rs::TalosConfig::default_path().ok());
        Self(fingerprint((
            path.as_ref().map(|path| (path, file_revision(path))),
            selection_key(selection),
        )))
    }

    /// Includes merge order, paths, exact file contents and referenced CA,
    /// certificate, key and token files. Missing files have a distinct revision.
    pub fn from_kubeconfig_sources(paths: &[PathBuf]) -> Self {
        Self(fingerprint(
            paths
                .iter()
                .map(|path| (path, kubeconfig_revision(path)))
                .collect::<Vec<_>>(),
        ))
    }

    fn selection(selection: &KubeconfigSelection) -> Self {
        Self(fingerprint(selection_key(selection)))
    }
}

impl Default for ConfigurationRevision {
    fn default() -> Self {
        Self::from_kubeconfig_sources(&[])
    }
}

impl std::fmt::Debug for ConfigurationRevision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConfigurationRevision(..)")
    }
}

/// Identity shared by ordinary reads and caches, independent of the selected
/// node and each read's generation. A revision change invalidates old work even
/// when the selected path and context are unchanged.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccessIdentity {
    session: AccessSessionId,
    configuration: ConfigurationRevision,
}

impl AccessIdentity {
    pub fn new(session: AccessSessionId, configuration: ConfigurationRevision) -> Self {
        Self {
            session,
            configuration,
        }
    }

    /// Shares Talos's authenticated connection identity, including across
    /// `with_node` copies, and the cache's kubeconfig revision policy.
    pub fn for_talos(client: &TalosClient, selection: &KubeconfigSelection) -> Self {
        Self::new(
            AccessSessionId::talos(client),
            ConfigurationRevision::selection(selection),
        )
    }

    /// Adapter for existing resource/source keys. Process-local and opaque;
    /// never a path, context name, credential or raw credential fingerprint.
    pub fn key(&self) -> String {
        format!("access:{:016x}", fingerprint(self))
    }

    pub fn configuration(&self) -> ConfigurationRevision {
        self.configuration
    }
}

/// Everything that determines a Talos connection.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct TalosKey {
    pub config_path: Option<PathBuf>,
    pub identity: ConfigIdentity,
    pub context: String,
    pub endpoints: Vec<String>,
    pub nodes: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum SelectionKey {
    /// Automatic selection can use ambient `KUBECONFIG`, so its files are part of the key.
    Automatic(Vec<(PathBuf, u64)>),
    TalosControlPlane,
    File {
        path: PathBuf,
        context: Option<String>,
        revision: u64,
    },
}

fn ambient_files() -> Vec<(PathBuf, u64)> {
    let paths = match std::env::var_os("KUBECONFIG") {
        Some(value) if !value.is_empty() => std::env::split_paths(&value).collect(),
        _ => dirs_next::home_dir()
            .map(|home| vec![home.join(".kube").join("config")])
            .unwrap_or_default(),
    };
    paths
        .into_iter()
        .map(|path| {
            let revision = kubeconfig_revision(&path);
            (path, revision)
        })
        .collect()
}

fn selection_key(selection: &KubeconfigSelection) -> SelectionKey {
    match selection {
        KubeconfigSelection::Automatic => SelectionKey::Automatic(ambient_files()),
        KubeconfigSelection::TalosControlPlane => SelectionKey::TalosControlPlane,
        KubeconfigSelection::File { path, context } => SelectionKey::File {
            path: path.clone(),
            context: context.clone(),
            revision: kubeconfig_revision(path),
        },
    }
}

/// Everything that determines a validated Kubernetes client or prepared kubeconfig.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct KubernetesKey {
    access: AccessIdentity,
    context: String,
    node: Option<String>,
}

impl KubernetesKey {
    pub(crate) fn new(
        client: &TalosClient,
        context: &str,
        pinned_node: Option<&str>,
        selection: &KubeconfigSelection,
    ) -> Self {
        Self {
            access: AccessIdentity::for_talos(client, selection),
            context: context.to_owned(),
            node: pinned_node.map(str::to_owned),
        }
    }
}

/// A validated client together with the source description callers display.
#[derive(Clone)]
pub(crate) struct KubernetesEntry {
    pub client: Client,
    pub source: String,
    pub warning: Option<String>,
}

type Shared<K, V> = LazyLock<Mutex<TtlCache<K, V>>>;

static TALOS_CLIENTS: Shared<TalosKey, TalosClient> =
    LazyLock::new(|| Mutex::new(TtlCache::new(None, MAX_ENTRIES)));
static KUBERNETES_CLIENTS: Shared<KubernetesKey, KubernetesEntry> =
    LazyLock::new(|| Mutex::new(TtlCache::new(Some(KUBERNETES_TTL), MAX_ENTRIES)));
static PREPARED: Shared<KubernetesKey, PreparedKubeconfig> =
    LazyLock::new(|| Mutex::new(TtlCache::new(Some(KUBERNETES_TTL), MAX_ENTRIES)));
static NODE_CLIENTS: Shared<(u64, String), Client> =
    LazyLock::new(|| Mutex::new(TtlCache::new(Some(KUBERNETES_TTL), MAX_ENTRIES)));

fn lock<K, V>(cache: &Shared<K, V>) -> std::sync::MutexGuard<'_, TtlCache<K, V>> {
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn talos_client(key: &TalosKey) -> Option<TalosClient> {
    lock(&TALOS_CLIENTS).get(key, Instant::now())
}

pub(crate) fn store_talos_client(key: TalosKey, client: TalosClient) {
    lock(&TALOS_CLIENTS).insert(key, client, Instant::now());
}

pub(crate) fn forget_talos_client(key: &TalosKey) {
    lock(&TALOS_CLIENTS).remove(key);
}

pub(crate) fn kubernetes_entry(key: &KubernetesKey) -> Option<KubernetesEntry> {
    lock(&KUBERNETES_CLIENTS).get(key, Instant::now())
}

pub(crate) fn store_kubernetes_entry(key: KubernetesKey, entry: KubernetesEntry) {
    lock(&KUBERNETES_CLIENTS).insert(key, entry, Instant::now());
}

pub(crate) fn prepared_kubeconfig(key: &KubernetesKey) -> Option<PreparedKubeconfig> {
    lock(&PREPARED).get(key, Instant::now())
}

/// Only a prepared config with a verified pinned source is worth reusing; an
/// unavailable one usually reflects a transient fetch failure.
pub(crate) fn store_prepared_kubeconfig(key: KubernetesKey, prepared: &PreparedKubeconfig) {
    if prepared.config.is_some() {
        lock(&PREPARED).insert(key, prepared.clone(), Instant::now());
    }
}

/// Forgets every Kubernetes client and prepared kubeconfig for one Talos
/// connection and context, whatever the selection.
pub(crate) fn forget_kubernetes(client: &TalosClient, context: &str) {
    let session = AccessSessionId::talos(client);
    lock(&KUBERNETES_CLIENTS)
        .remove_where(|key| key.access.session == session && key.context == context);
    lock(&PREPARED).remove_where(|key| key.access.session == session && key.context == context);
}

pub(crate) fn node_client(client: &TalosClient, node: &str) -> Option<Client> {
    lock(&NODE_CLIENTS).get(&(client.connection_id(), node.to_owned()), Instant::now())
}

pub(crate) fn store_node_client(client: &TalosClient, node: &str, kube: Client) {
    lock(&NODE_CLIENTS).insert(
        (client.connection_id(), node.to_owned()),
        kube,
        Instant::now(),
    );
}

pub(crate) fn forget_node_client(client: &TalosClient, node: &str) {
    lock(&NODE_CLIENTS).remove(&(client.connection_id(), node.to_owned()));
}

/// Whether node-level outcomes show the shared Talos connection is unusable.
///
/// A failing etcd request goes straight to the endpoint, so a transport failure
/// there condemns the connection. Per-node failures condemn it only when every
/// queried node failed that way; one down node must not cost everyone a reconnect.
pub(crate) fn connection_unusable(endpoint_failed: bool, node_transport_failures: &[bool]) -> bool {
    endpoint_failed
        || (!node_transport_failures.is_empty() && node_transport_failures.iter().all(|f| *f))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            // macOS clocks tick in microseconds, so tests running in parallel
            // can read the same time; the counter keeps their names apart.
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "freshkube-access-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn same_size_same_timestamp_configuration_replacement_invalidates_the_cache() {
        let scratch = Scratch::new();
        let path = scratch.0.join("config");
        std::fs::write(&path, "old configuration").unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let selection = KubeconfigSelection::File {
            path: path.clone(),
            context: Some("lab".into()),
        };
        let before = selection_key(&selection);
        let mut cache = TtlCache::new(None, 8);
        let now = Instant::now();
        cache.insert(before.clone(), "old client", now);
        assert_eq!(
            cache.get(&selection_key(&selection), now),
            Some("old client")
        );
        std::fs::write(&path, "new configuration").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert!(before != selection_key(&selection));
        assert_eq!(cache.get(&selection_key(&selection), now), None);
    }

    #[test]
    fn referenced_credentials_and_missing_files_change_the_revision() {
        let scratch = Scratch::new();
        let path = scratch.0.join("config");
        std::fs::write(
            &path,
            "apiVersion: v1\nkind: Config\nusers:\n- name: lab\n  user:\n    tokenFile: token\n",
        )
        .unwrap();
        let sources = vec![path];
        let missing = ConfigurationRevision::from_kubeconfig_sources(&sources);
        let token = scratch.0.join("token");
        std::fs::write(&token, "synthetic first").unwrap();
        let first = ConfigurationRevision::from_kubeconfig_sources(&sources);
        assert_ne!(missing, first);
        assert_eq!(
            first,
            ConfigurationRevision::from_kubeconfig_sources(&sources)
        );
        std::fs::write(&token, "synthetic other").unwrap();
        assert_ne!(
            first,
            ConfigurationRevision::from_kubeconfig_sources(&sources)
        );
        std::fs::remove_file(token).unwrap();
        assert_eq!(
            missing,
            ConfigurationRevision::from_kubeconfig_sources(&sources)
        );
        assert_eq!(format!("{first:?}"), "ConfigurationRevision(..)");
    }

    #[test]
    fn access_keys_distinguish_sessions_and_revisions_without_request_generations() {
        let session = AccessSessionId::new();
        let revision = ConfigurationRevision::default();
        let before = AccessIdentity::new(session, revision);
        assert_eq!(before.key(), AccessIdentity::new(session, revision).key());
        assert_ne!(
            before.key(),
            AccessIdentity::new(AccessSessionId::new(), revision).key()
        );
        assert_ne!(
            before.key(),
            AccessIdentity::new(session, ConfigurationRevision(revision.0.wrapping_add(1))).key()
        );
        let mut cache = TtlCache::new(None, 8);
        let now = Instant::now();
        cache.insert(before, "previous client", now);
        assert_eq!(cache.get(&before, now), Some("previous client"));
        assert_eq!(
            cache.get(&AccessIdentity::new(AccessSessionId::new(), revision), now),
            None
        );
    }

    fn at(base: Instant, seconds: u64) -> Instant {
        base + Duration::from_secs(seconds)
    }

    #[test]
    fn identical_key_reuses_and_other_keys_miss() {
        let base = Instant::now();
        let mut cache = TtlCache::new(Some(Duration::from_secs(120)), 8);
        cache.insert(("ctx", "10.0.0.1"), 1, base);
        assert_eq!(cache.get(&("ctx", "10.0.0.1"), at(base, 5)), Some(1));
        assert_eq!(cache.get(&("other", "10.0.0.1"), at(base, 5)), None);
        assert_eq!(cache.get(&("ctx", "10.0.0.2"), at(base, 5)), None);
    }

    #[test]
    fn ttl_expiry_rebuilds_and_removes_the_entry() {
        let base = Instant::now();
        let mut cache = TtlCache::new(Some(Duration::from_secs(120)), 8);
        cache.insert("k", 1, base);
        assert_eq!(cache.get(&"k", at(base, 119)), Some(1));
        assert_eq!(cache.get(&"k", at(base, 120)), None);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn forgetting_removes_matching_entries_only() {
        let base = Instant::now();
        let mut cache = TtlCache::new(None, 8);
        cache.insert(("a", 1), 1, base);
        cache.insert(("a", 2), 2, base);
        cache.insert(("b", 1), 3, base);
        cache.remove_where(|key| key.0 == "a");
        assert_eq!(cache.get(&("a", 1), base), None);
        assert_eq!(cache.get(&("a", 2), base), None);
        assert_eq!(cache.get(&("b", 1), base), Some(3));
        cache.remove(&("b", 1));
        assert_eq!(cache.get(&("b", 1), base), None);
    }

    #[test]
    fn capacity_evicts_the_oldest_entry() {
        let base = Instant::now();
        let mut cache = TtlCache::new(None, 2);
        cache.insert("old", 1, base);
        cache.insert("mid", 2, at(base, 1));
        cache.insert("new", 3, at(base, 2));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&"old", at(base, 3)), None);
        assert_eq!(cache.get(&"new", at(base, 3)), Some(3));
    }

    #[test]
    fn file_selection_key_changes_with_path_context_and_file_identity() {
        let dir = std::env::temp_dir().join(format!("tp-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("kubeconfig");
        std::fs::write(&path, "a").unwrap();
        let file = |path: &Path, context: Option<&str>| KubeconfigSelection::File {
            path: path.to_path_buf(),
            context: context.map(str::to_owned),
        };

        let original = selection_key(&file(&path, Some("one")));
        assert!(original == selection_key(&file(&path, Some("one"))));
        assert!(original != selection_key(&file(&path, Some("two"))));
        assert!(original != selection_key(&file(&dir.join("other"), Some("one"))));
        std::fs::write(&path, "longer content").unwrap();
        assert!(original != selection_key(&file(&path, Some("one"))));
        assert!(
            selection_key(&KubeconfigSelection::TalosControlPlane)
                != selection_key(&file(&path, None))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn config_identity_follows_content() {
        assert!(ConfigIdentity::from_bytes(b"one") == ConfigIdentity::from_bytes(b"one"));
        assert!(ConfigIdentity::from_bytes(b"one") != ConfigIdentity::from_bytes(b"two"));
    }

    #[test]
    fn transport_failures_drop_the_connection_but_one_down_node_does_not() {
        assert!(connection_unusable(true, &[false, false]));
        assert!(connection_unusable(false, &[true, true]));
        assert!(!connection_unusable(false, &[true, false]));
        assert!(!connection_unusable(false, &[]));
    }

    #[tokio::test]
    async fn talos_connection_cache_drops_on_forget_and_key_change() {
        let key = |context: &str, identity: &[u8]| TalosKey {
            config_path: Some(PathBuf::from("/c")),
            identity: ConfigIdentity::from_bytes(identity),
            context: context.into(),
            endpoints: vec!["10.0.0.1".into()],
            nodes: vec![],
        };
        let base = Instant::now();
        let mut cache: TtlCache<TalosKey, u32> = TtlCache::new(None, 8);
        cache.insert(key("a", b"1"), 7, base);
        assert_eq!(cache.get(&key("a", b"1"), base), Some(7));
        assert_eq!(cache.get(&key("b", b"1"), base), None);
        assert_eq!(cache.get(&key("a", b"2"), base), None);
        cache.remove(&key("a", b"1"));
        assert_eq!(cache.get(&key("a", b"1"), base), None);
    }
}
