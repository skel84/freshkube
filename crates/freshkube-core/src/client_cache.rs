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
    hash::Hash,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant, SystemTime},
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

/// Cheap change detector for a file: length plus modification time. A missing
/// or unreadable file has no stamp, which never equals a present file's.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct FileStamp(Option<(u64, Option<SystemTime>)>);

impl FileStamp {
    pub(crate) fn of(path: &Path) -> Self {
        Self(
            std::fs::metadata(path)
                .ok()
                .map(|meta| (meta.len(), meta.modified().ok())),
        )
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
    Automatic(Vec<(PathBuf, FileStamp)>),
    TalosControlPlane,
    File {
        path: PathBuf,
        context: Option<String>,
        stamp: FileStamp,
    },
}

fn ambient_files() -> Vec<(PathBuf, FileStamp)> {
    let paths = match std::env::var_os("KUBECONFIG") {
        Some(value) if !value.is_empty() => std::env::split_paths(&value).collect(),
        _ => dirs_next::home_dir()
            .map(|home| vec![home.join(".kube").join("config")])
            .unwrap_or_default(),
    };
    paths
        .into_iter()
        .map(|path| {
            let stamp = FileStamp::of(&path);
            (path, stamp)
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
            stamp: FileStamp::of(path),
        },
    }
}

/// Everything that determines a validated Kubernetes client or prepared kubeconfig.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct KubernetesKey {
    connection_id: u64,
    context: String,
    node: Option<String>,
    selection: SelectionKey,
}

impl KubernetesKey {
    pub(crate) fn new(
        client: &TalosClient,
        context: &str,
        pinned_node: Option<&str>,
        selection: &KubeconfigSelection,
    ) -> Self {
        Self {
            connection_id: client.connection_id(),
            context: context.to_owned(),
            node: pinned_node.map(str::to_owned),
            selection: selection_key(selection),
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
    let id = client.connection_id();
    lock(&KUBERNETES_CLIENTS).remove_where(|key| key.connection_id == id && key.context == context);
    lock(&PREPARED).remove_where(|key| key.connection_id == id && key.context == context);
}

pub(crate) fn forget_all_kubernetes() {
    lock(&KUBERNETES_CLIENTS).remove_where(|_| true);
    lock(&PREPARED).remove_where(|_| true);
    lock(&NODE_CLIENTS).remove_where(|_| true);
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
