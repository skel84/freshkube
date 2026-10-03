//! Tokio driver. One task owns all read tasks and one debounced publisher.
use std::{fmt::Debug, pin::pin, time::Duration};

use chrono::Utc;
use futures::StreamExt;
use k8s_openapi::api::{
    apps::v1::{DaemonSet, Deployment, StatefulSet},
    core::v1::{Event, Namespace, Node, PersistentVolume, PersistentVolumeClaim, Pod},
};
use kube::{Api, Client, runtime::watcher};
use serde::de::DeserializeOwned;
use tokio::{task::JoinSet, time::Instant};

use super::{ObservationFailure, Session, Source, SummaryResource};
use crate::resources::FailureKind;

pub const DEBOUNCE: Duration = Duration::from_millis(500);
const READ_DEADLINE: Duration = Duration::from_secs(15);

impl Session {
    /// Cancellation of this future drops the JoinSet and aborts every read,
    /// including an in-progress initial list or resync. No task is detached.
    pub async fn run(&self, client: Client) {
        let mut changes = self.changes();
        let mut refreshes = self.refreshes();
        let mut jobs = self.start_reads(client.clone(), self.generation());
        let mut due = Some(Instant::now() + DEBOUNCE);
        let mut expiry = None;
        loop {
            let deadline = due.unwrap_or_else(|| {
                expiry.unwrap_or_else(|| Instant::now() + Duration::from_secs(24 * 60 * 60))
            });
            tokio::select! {
                _ = refreshes.changed() => {
                    jobs.shutdown().await;
                    let generation = *refreshes.borrow_and_update();
                    jobs = self.start_reads(client.clone(), generation);
                    due.get_or_insert(Instant::now() + DEBOUNCE);
                }
                _ = changes.changed() => {
                    changes.borrow_and_update();
                    // A burst cannot keep moving the deadline into the future.
                    due.get_or_insert(Instant::now() + DEBOUNCE);
                }
                _ = tokio::time::sleep_until(deadline), if due.is_some() || expiry.is_some() => {
                    // The driver is the only derivation worker. Events arriving
                    // during derivation leave a new dirty revision for the next window.
                    changes.borrow_and_update();
                    let publication = self.derive(Utc::now());
                    self.publish(publication);
                    due = None;
                    expiry = self.next_warning_change(Utc::now()).map(|duration| Instant::now() + duration);
                }
            }
        }
    }

    fn start_reads(&self, client: Client, generation: u64) -> JoinSet<()> {
        let mut jobs = JoinSet::new();
        macro_rules! watch_kind {
            ($kind:ty) => {{
                let (session, client) = (self.clone(), client.clone());
                jobs.spawn(async move {
                    session.watch::<$kind>(client, generation).await;
                });
            }};
        }
        watch_kind!(Node);
        watch_kind!(Pod);
        watch_kind!(Event);
        watch_kind!(Deployment);
        watch_kind!(DaemonSet);
        watch_kind!(StatefulSet);
        watch_kind!(PersistentVolume);
        watch_kind!(PersistentVolumeClaim);
        watch_kind!(Namespace);
        if self.version_is_current() {
            return jobs;
        }
        let session = self.clone();
        jobs.spawn(async move {
            match tokio::time::timeout(READ_DEADLINE, client.apiserver_version()).await {
                Ok(Ok(version)) => session.version(generation, version.git_version, Utc::now()),
                Ok(Err(error)) => session.fail(
                    generation,
                    Source::Version,
                    ObservationFailure::from_kube(error),
                ),
                Err(_) => session.fail(
                    generation,
                    Source::Version,
                    ObservationFailure::Read(FailureKind::Timeout),
                ),
            }
        });
        jobs
    }

    async fn watch<K>(&self, client: Client, generation: u64)
    where
        K: SummaryResource + Clone + Debug + DeserializeOwned + Send + 'static,
    {
        let api = Api::<K>::all(client);
        let mut config = watcher::Config::default();
        if K::SOURCE == Source::Events {
            config = config.fields("type=Warning");
        }
        let mut backoff = Duration::from_secs(1);
        loop {
            let mut stream = pin!(watcher(api.clone(), config.clone()));
            let mut initial_deadline = Some(Instant::now() + READ_DEADLINE);
            let (failure, expired) = loop {
                let item = match initial_deadline {
                    Some(deadline) => {
                        match tokio::time::timeout_at(deadline, stream.next()).await {
                            Ok(item) => item,
                            Err(_) => {
                                break (ObservationFailure::Read(FailureKind::Timeout), false);
                            }
                        }
                    }
                    None => stream.next().await,
                };
                match item {
                    Some(Ok(event)) => {
                        if matches!(event, watcher::Event::Init | watcher::Event::InitApply(_)) {
                            // Bound a stalled read, not the total time needed
                            // for a large collection that is still progressing.
                            initial_deadline = Some(Instant::now() + READ_DEADLINE);
                        }
                        if matches!(event, watcher::Event::InitDone) {
                            initial_deadline = None;
                            backoff = Duration::from_secs(1);
                        }
                        if let Err(failure) = self.apply(generation, event, Utc::now()) {
                            break (failure, false);
                        }
                    }
                    Some(Err(error)) => break classify(error),
                    None => break (ObservationFailure::Read(FailureKind::Unreachable), false),
                }
            };
            if self.generation() != generation {
                return;
            }
            self.fail(generation, K::SOURCE, failure.clone());
            if failure.is_permanent() {
                return;
            }
            // Recreate on both event 410 and HTTP watch-start 410. kube 0.98
            // resets only the former itself. Other failures back off and relist.
            if !expired {
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
            } else {
                tokio::task::yield_now().await;
            }
        }
    }
}

fn classify(error: watcher::Error) -> (ObservationFailure, bool) {
    let error = match error {
        watcher::Error::InitialListFailed(error)
        | watcher::Error::WatchStartFailed(error)
        | watcher::Error::WatchFailed(error) => error,
        watcher::Error::WatchError(status) => kube::Error::Api(status),
        watcher::Error::NoResourceVersion => return (ObservationFailure::InvalidObject, false),
    };
    let expired = matches!(&error, kube::Error::Api(status) if status.code == 410);
    (ObservationFailure::from_kube(error), expired)
}
