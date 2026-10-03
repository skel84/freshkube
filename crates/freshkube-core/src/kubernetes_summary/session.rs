//! Session-owned reflectors. A collection commits only at InitDone; read
//! failures and incomplete replacement lists retain its previous evidence.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};
use kube::runtime::{
    reflector::{self, Lookup, Store},
    watcher::Event,
};
use tokio::sync::watch;

use super::{
    KubernetesSummary, Observation, ObservationFailure, Observations, Part, RetainedObject,
    SessionIdentity, Source, SubscriptionKey, SummaryResource,
};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub objects_per_kind: usize,
    pub bytes_per_generation: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            objects_per_kind: 100_000,
            bytes_per_generation: 128 * 1024 * 1024,
        }
    }
}

struct Collection {
    writer: reflector::store::Writer<RetainedObject>,
    reader: Store<RetainedObject>,
    staging: Option<BTreeMap<(String, String), usize>>,
    live_bytes: usize,
    staging_bytes: usize,
    observation: Observation,
}
impl Collection {
    fn new(source: Source) -> Self {
        let writer = reflector::store::Writer::new(source);
        Self {
            reader: writer.as_reader(),
            writer,
            staging: None,
            live_bytes: 0,
            staging_bytes: 0,
            observation: Observation::default(),
        }
    }
    fn discard_staging(&mut self) {
        self.writer.apply_watcher_event(&Event::Init);
        self.staging = None;
        self.staging_bytes = 0;
    }
}

struct State {
    generation: u64,
    publication_revision: u64,
    dirty_since: Option<std::time::Instant>,
    staging_peak: usize,
    collections: BTreeMap<Source, Collection>,
    version: Option<String>,
    version_observation: Observation,
}

/// Published on a capacity-one channel. Every consumer of this revision sees
/// the same evidence and observation metadata; resource versions are not a
/// globally atomic Kubernetes snapshot.
#[derive(Clone, Debug)]
pub struct Publication {
    pub identity: SessionIdentity,
    pub generation: u64,
    pub summary: Arc<KubernetesSummary>,
    pub revision: u64,
    pub retained_bytes: usize,
    pub staging_bytes: usize,
    pub pod_bytes: usize,
    pub pod_count: usize,
    pub derive_duration: std::time::Duration,
    pub changed_at: Option<std::time::Instant>,
    pub staging_peak: usize,
}

#[derive(Clone)]
pub struct Subscription {
    key: SubscriptionKey,
    receiver: watch::Receiver<Option<Arc<Publication>>>,
}
impl Subscription {
    pub fn key(&self) -> &SubscriptionKey {
        &self.key
    }
    pub fn latest(&self) -> Option<Arc<Publication>> {
        self.receiver.borrow().clone()
    }
    pub async fn changed(&mut self) -> Result<(), watch::error::RecvError> {
        self.receiver.changed().await
    }
}

/// Drop the task running `run` to cancel every watch. Cloning a handle shares
/// stores and subscriptions, and never creates a Kubernetes request.
#[derive(Clone)]
pub struct Session {
    identity: SessionIdentity,
    state: Arc<Mutex<State>>,
    limits: Limits,
    dirty: watch::Sender<u64>,
    refresh: watch::Sender<u64>,
    published: watch::Sender<Option<Arc<Publication>>>,
}

pub(super) struct Evidence {
    pub revision: u64,
    pub changed_at: Option<std::time::Instant>,
    pub staging_peak: usize,
    pub objects: BTreeMap<Source, Vec<Arc<RetainedObject>>>,
    pub observations: Observations,
    pub version: Part<String>,
    pub generation: u64,
    pub retained_bytes: usize,
    pub staging_bytes: usize,
}

impl Session {
    pub fn new(identity: SessionIdentity) -> Self {
        Self::with_limits(identity, Limits::default())
    }
    pub fn with_limits(identity: SessionIdentity, limits: Limits) -> Self {
        Self {
            identity,
            state: Arc::new(Mutex::new(State {
                generation: 0,
                publication_revision: 0,
                dirty_since: None,
                staging_peak: 0,
                collections: Source::WATCHED
                    .into_iter()
                    .map(|source| (source, Collection::new(source)))
                    .collect(),
                version: None,
                version_observation: Observation::default(),
            })),
            limits,
            dirty: watch::channel(0).0,
            refresh: watch::channel(0).0,
            published: watch::channel(None).0,
        }
    }
    pub fn identity(&self) -> &SessionIdentity {
        &self.identity
    }
    pub fn generation(&self) -> u64 {
        self.state.lock().unwrap().generation
    }
    pub fn publications(&self) -> watch::Receiver<Option<Arc<Publication>>> {
        self.published.subscribe()
    }
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.dirty.subscribe()
    }
    pub(super) fn refreshes(&self) -> watch::Receiver<u64> {
        self.refresh.subscribe()
    }
    pub fn subscribe(&self, key: SubscriptionKey) -> Option<Subscription> {
        (key == SubscriptionKey::summary(self.identity.clone(), key.source())).then(|| {
            Subscription {
                key,
                receiver: self.publications(),
            }
        })
    }
    fn changed(&self) {
        self.state
            .lock()
            .unwrap()
            .dirty_since
            .get_or_insert_with(std::time::Instant::now);
        self.dirty.send_modify(|revision| *revision += 1);
    }

    /// Invalidates pending work synchronously, even before the driver wakes.
    pub fn relist(&self) -> u64 {
        let generation = {
            let mut state = self.state.lock().unwrap();
            state.generation += 1;
            for collection in state.collections.values_mut() {
                collection.discard_staging();
                collection.observation.syncing();
            }
            state.version_observation.syncing();
            state.generation
        };
        self.refresh.send_replace(generation);
        self.changed();
        generation
    }

    /// Shared reducer for typed Kubernetes events and deterministic fixtures.
    pub fn apply<K: SummaryResource>(
        &self,
        generation: u64,
        event: Event<K>,
        now: DateTime<Utc>,
    ) -> Result<(), ObservationFailure> {
        let event = match event {
            Event::Init => Event::Init,
            Event::InitDone => Event::InitDone,
            Event::InitApply(object) => Event::InitApply(object.retain()),
            Event::Apply(object) => Event::Apply(object.retain()),
            Event::Delete(object) => Event::Delete(object.retain()),
        };
        self.apply_retained(generation, K::SOURCE, event, now)
    }

    fn apply_retained(
        &self,
        generation: u64,
        source: Source,
        event: Event<RetainedObject>,
        now: DateTime<Utc>,
    ) -> Result<(), ObservationFailure> {
        let mut state = self.state.lock().unwrap();
        if state.generation != generation {
            return Ok(());
        }
        let live_total: usize = state.collections.values().map(|c| c.live_bytes).sum();
        let staging_total: usize = state.collections.values().map(|c| c.staging_bytes).sum();
        let collection = state.collections.get_mut(&source).expect("watched source");
        let result = (|| {
            match &event {
                Event::Init => {
                    collection.staging = Some(BTreeMap::new());
                    collection.staging_bytes = 0;
                    collection.observation.syncing();
                }
                Event::InitApply(object) => {
                    object.validate(source)?;
                    let staging = collection
                        .staging
                        .as_mut()
                        .ok_or(ObservationFailure::InvalidObject)?;
                    let key = (object.namespace.clone(), object.name.clone());
                    let previous = staging.get(&key).copied().unwrap_or(0);
                    if staging.len() + usize::from(previous == 0) > self.limits.objects_per_kind
                        || staging_total - previous + object.bytes
                            > self.limits.bytes_per_generation
                    {
                        return Err(ObservationFailure::Capacity);
                    }
                    staging.insert(key, object.bytes);
                    collection.staging_bytes = collection.staging_bytes - previous + object.bytes;
                }
                Event::InitDone => {
                    if collection.staging.take().is_none() {
                        return Err(ObservationFailure::InvalidObject);
                    }
                    if live_total - collection.live_bytes + collection.staging_bytes
                        > self.limits.bytes_per_generation
                    {
                        return Err(ObservationFailure::Capacity);
                    }
                    collection.live_bytes = collection.staging_bytes;
                    collection.staging_bytes = 0;
                    collection.observation.success(now, true);
                }
                Event::Apply(object) => {
                    object.validate(source)?;
                    if !collection.observation.has_data() {
                        return Err(ObservationFailure::InvalidObject);
                    }
                    let old = collection.reader.get(&object.to_object_ref(source));
                    let previous = old.as_ref().map_or(0, |o| o.bytes);
                    if collection.reader.len() + usize::from(old.is_none())
                        > self.limits.objects_per_kind
                        || live_total - previous + object.bytes > self.limits.bytes_per_generation
                    {
                        return Err(ObservationFailure::Capacity);
                    }
                    collection.live_bytes = collection.live_bytes - previous + object.bytes;
                    collection.observation.success(now, false);
                }
                Event::Delete(object) => {
                    object.validate(source)?;
                    let Some(old) = collection.reader.get(&object.to_object_ref(source)) else {
                        return Ok(false);
                    };
                    // Reflector keys omit UID: a late delete must not erase a replacement.
                    if old.uid != object.uid {
                        return Ok(false);
                    }
                    collection.live_bytes -= old.bytes;
                    collection.observation.success(now, false);
                }
            }
            collection.writer.apply_watcher_event(&event);
            Ok(!matches!(event, Event::InitApply(_)))
        })();
        if let Err(failure) = &result {
            collection.discard_staging();
            collection.observation.failed(failure.clone());
        }
        state.staging_peak = state
            .staging_peak
            .max(state.collections.values().map(|c| c.staging_bytes).sum());
        drop(state);
        if !matches!(result, Ok(false)) {
            self.changed();
        }
        result.map(|_| ())
    }

    pub fn fail(&self, generation: u64, source: Source, failure: ObservationFailure) {
        let mut state = self.state.lock().unwrap();
        if state.generation != generation {
            return;
        }
        if source == Source::Version {
            state.version_observation.failed(failure);
        } else if let Some(collection) = state.collections.get_mut(&source) {
            collection.discard_staging();
            collection.observation.failed(failure);
        }
        drop(state);
        self.changed();
    }
    pub fn version(&self, generation: u64, version: String, now: DateTime<Utc>) {
        let mut state = self.state.lock().unwrap();
        if state.generation != generation {
            return;
        }
        state.version = Some(version);
        state.version_observation.success(now, true);
        drop(state);
        self.changed();
    }
    pub(super) fn version_is_current(&self) -> bool {
        self.state.lock().unwrap().version_observation.is_current()
    }
    pub(super) fn warning_objects(&self) -> Vec<Arc<RetainedObject>> {
        self.state.lock().unwrap().collections[&Source::Events]
            .reader
            .state()
    }
    fn evidence(&self) -> Evidence {
        let mut state = self.state.lock().unwrap();
        state.publication_revision += 1;
        let mut observations: Observations = state
            .collections
            .iter()
            .map(|(s, c)| (*s, c.observation.clone()))
            .collect();
        observations.insert(Source::Version, state.version_observation.clone());
        Evidence {
            revision: state.publication_revision,
            changed_at: state.dirty_since.take(),
            staging_peak: state.staging_peak,
            objects: state
                .collections
                .iter()
                .map(|(s, c)| (*s, c.reader.state()))
                .collect(),
            version: Part::observed(
                state.version.clone().unwrap_or_default(),
                &state.version_observation,
            ),
            observations,
            generation: state.generation,
            retained_bytes: state.collections.values().map(|c| c.live_bytes).sum(),
            staging_bytes: state.collections.values().map(|c| c.staging_bytes).sum(),
        }
    }
    /// Called off the UI executor for live data, once per debounce window.
    pub fn derive(&self, now: DateTime<Utc>) -> Arc<Publication> {
        let started = std::time::Instant::now();
        let evidence = self.evidence();
        let pods = &evidence.objects[&Source::Pods];
        let (pod_count, pod_bytes) = (pods.len(), pods.iter().map(|pod| pod.bytes).sum());
        let mut publication = Publication {
            identity: self.identity.clone(),
            generation: evidence.generation,
            retained_bytes: evidence.retained_bytes,
            staging_bytes: evidence.staging_bytes,
            pod_count,
            pod_bytes,
            derive_duration: std::time::Duration::ZERO,
            revision: evidence.revision,
            changed_at: evidence.changed_at,
            staging_peak: evidence.staging_peak,
            summary: Arc::new(super::project::derive(evidence, now)),
        };
        publication.derive_duration = started.elapsed();
        Arc::new(publication)
    }
    pub fn publish(&self, publication: Arc<Publication>) -> bool {
        let state = self.state.lock().unwrap();
        if publication.identity != self.identity || publication.generation != state.generation {
            return false;
        }
        if self
            .published
            .borrow()
            .as_ref()
            .is_some_and(|previous| previous.revision >= publication.revision)
        {
            return false;
        }
        self.published.send_replace(Some(publication));
        true
    }
}
