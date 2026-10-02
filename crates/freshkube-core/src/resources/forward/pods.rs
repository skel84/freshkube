//! The pods forwards follow, watched once per namespace and selector and
//! shared by every forward of the connection that agrees on both. The watch
//! stops when its last forward lets go.

use std::collections::{BTreeMap, HashMap};
use std::pin::pin;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::runtime::watcher;
use kube::{Api, Client};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::target::PodInfo;
use crate::resources::events::classify;
use crate::resources::failure::Failure;

const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Which pods a watch follows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PodSelector {
    /// One pod by name, whichever incarnation.
    Name(String),
    /// A label selector.
    Labels(String),
}

/// The pods as last seen.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PodSnapshot {
    /// The first list has arrived, so a missing pod is really gone.
    pub(crate) listed: bool,
    /// By name.
    pub(crate) pods: Arc<Vec<PodInfo>>,
    /// The watch failed and lists again after a backoff, or stopped for
    /// good when the failure is permanent.
    pub(crate) failure: Option<Failure>,
}

/// The pod watches of one connection.
#[derive(Clone)]
pub struct PodWatches {
    client: Client,
    watches: Arc<Mutex<Running>>,
}

/// The watches running, by namespace and selector.
type Running = HashMap<(String, PodSelector), Weak<Watch>>;

/// One running watch; dropping the last subscription stops it.
struct Watch {
    snapshot: watch::Receiver<PodSnapshot>,
    task: JoinHandle<()>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A forward's hold on a watch.
pub(crate) struct Subscription {
    pub(crate) snapshot: watch::Receiver<PodSnapshot>,
    _watch: Arc<Watch>,
}

impl PodWatches {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            watches: Arc::default(),
        }
    }

    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    /// Follows the pods `selector` picks in `namespace`, joining a watch
    /// that already does. Call it on Tokio.
    pub(crate) fn subscribe(&self, namespace: &str, selector: PodSelector) -> Subscription {
        let mut watches = self.watches.lock().expect("pod watches lock");
        watches.retain(|_, watch| watch.strong_count() > 0);
        let key = (namespace.to_owned(), selector);
        if let Some(watch) = watches.get(&key).and_then(Weak::upgrade) {
            return Subscription {
                snapshot: watch.snapshot.clone(),
                _watch: watch,
            };
        }
        let (sender, snapshot) = watch::channel(PodSnapshot::default());
        let api: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        let config = match &key.1 {
            PodSelector::Name(name) => {
                watcher::Config::default().fields(&format!("metadata.name={name}"))
            }
            PodSelector::Labels(labels) => watcher::Config::default().labels(labels),
        };
        let task = tokio::spawn(follow(api, config, sender));
        let watch = Arc::new(Watch {
            snapshot: snapshot.clone(),
            task,
        });
        watches.insert(key, Arc::downgrade(&watch));
        Subscription {
            snapshot,
            _watch: watch,
        }
    }

    /// How many watches run, for tests.
    #[cfg(test)]
    pub(crate) fn running(&self) -> usize {
        let watches = self.watches.lock().expect("pod watches lock");
        watches
            .values()
            .filter(|watch| watch.strong_count() > 0)
            .count()
    }
}

/// Lists and watches the pods, publishing every change, until nobody holds
/// the watch or a failure is permanent.
async fn follow(api: Api<Pod>, config: watcher::Config, sink: watch::Sender<PodSnapshot>) {
    let mut backoff = MIN_BACKOFF;
    loop {
        let mut stream = pin!(watcher(api.clone(), config.clone()));
        let mut pods: BTreeMap<String, PodInfo> = BTreeMap::new();
        let mut listing = BTreeMap::new();
        let failure = loop {
            let Some(item) = stream.next().await else {
                return;
            };
            match item {
                Ok(watcher::Event::Init) => listing.clear(),
                Ok(watcher::Event::InitApply(pod)) => {
                    let info = PodInfo::of(&pod);
                    listing.insert(info.name.clone(), info);
                }
                Ok(watcher::Event::InitDone) => {
                    backoff = MIN_BACKOFF;
                    pods = std::mem::take(&mut listing);
                    publish(&sink, &pods);
                }
                Ok(watcher::Event::Apply(pod)) => {
                    let info = PodInfo::of(&pod);
                    pods.insert(info.name.clone(), info);
                    publish(&sink, &pods);
                }
                Ok(watcher::Event::Delete(pod)) => {
                    let info = PodInfo::of(&pod);
                    // A late delete of an older incarnation leaves the new
                    // one in place.
                    if pods
                        .get(&info.name)
                        .is_some_and(|known| known.uid == info.uid)
                    {
                        pods.remove(&info.name);
                    }
                    publish(&sink, &pods);
                }
                // An expired version: the watcher lists again by itself.
                Err(watcher::Error::WatchError(status)) if status.code == 410 => {}
                Err(error) => break classify(error),
            }
            if sink.is_closed() {
                return;
            }
        };
        let retrying = !failure.kind.is_permanent();
        sink.send_modify(|snapshot| snapshot.failure = Some(failure));
        if !retrying || sink.is_closed() {
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

fn publish(sink: &watch::Sender<PodSnapshot>, pods: &BTreeMap<String, PodInfo>) {
    sink.send_replace(PodSnapshot {
        listed: true,
        pods: Arc::new(pods.values().cloned().collect()),
        failure: None,
    });
}
