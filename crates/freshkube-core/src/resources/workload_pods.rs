//! The pods a workload runs, with their containers, for reading their logs
//! together. Read-only: it lists and watches pods by the workload's
//! selector, nothing else.
//!
//! This is its own watch rather than forward's [`super::PodWatches`]: that
//! one is shared by the app's forwards, lives as long as they do, and keeps
//! only readiness and ports; a log view needs each pod's containers and
//! stops watching when it hides.

use std::collections::BTreeMap;
use std::pin::pin;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use kube::runtime::watcher;
use kube::{Api, Client};
use serde_yaml::Value;
use tokio::sync::watch;

use super::WorkloadKind;
use super::events::classify;
use super::failure::Failure;
use super::forward::label_selector;
use super::kinds::{ResourceKind, builtin};
use super::object::{text, time};
use super::pod_logs::{PodContainers, pod_containers};

const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Whether objects of `kind` run pods from a template and find them by a
/// selector, so their pods' logs can be read together: Deployments,
/// StatefulSets, DaemonSets, ReplicaSets and Jobs.
pub fn runs_pods(kind: &ResourceKind) -> bool {
    WorkloadKind::of(kind).is_some() || builtin("jobs.batch").as_ref() == Some(kind)
}

/// A workload's `spec.selector` as a label selector, or `None` when it
/// selects nothing.
pub fn workload_selector(object: &Value) -> Option<String> {
    let selector = object.get("spec")?.get("selector")?;
    let selector: LabelSelector = serde_yaml::from_value(selector.clone()).ok()?;
    label_selector(Some(&selector))
}

/// One of a workload's pods as last seen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkloadPod {
    pub name: String,
    /// Tells a pod from a later one of the same name, as a StatefulSet's.
    pub uid: String,
    pub created: Option<DateTime<Utc>>,
    /// Deletion was asked for; its containers may still write.
    pub terminating: bool,
    pub containers: PodContainers,
}

impl WorkloadPod {
    /// Reads a pod written out as YAML or JSON.
    pub fn of_object(object: &Value) -> Self {
        let metadata = object.get("metadata");
        let field = |name: &str| metadata.and_then(|metadata| metadata.get(name));
        Self {
            name: text(field("name")),
            uid: text(field("uid")),
            created: time(field("creationTimestamp")),
            terminating: field("deletionTimestamp").is_some_and(|value| !value.is_null()),
            containers: pod_containers(object),
        }
    }

    fn of(pod: &Pod) -> Option<Self> {
        serde_yaml::to_value(pod)
            .ok()
            .map(|object| Self::of_object(&object))
    }
}

/// The workload's pods as last seen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkloadPods {
    /// The first list has arrived, so a missing pod is really gone.
    pub listed: bool,
    /// By name.
    pub pods: Vec<WorkloadPod>,
    /// The watch failed and lists again after a backoff, or stopped for
    /// good when the failure is permanent. The pods last seen stay.
    pub failure: Option<Failure>,
}

/// Lists and watches the pods `selector` picks in `namespace`, publishing
/// every change into `sink`, until the receiver is dropped or a failure is
/// permanent. Run it on Tokio and abort it to stop.
pub async fn follow_workload_pods(
    client: Client,
    namespace: String,
    selector: String,
    sink: watch::Sender<WorkloadPods>,
) {
    let api: Api<Pod> = Api::namespaced(client, &namespace);
    let config = watcher::Config::default().labels(&selector);
    let mut backoff = MIN_BACKOFF;
    let mut pods: BTreeMap<String, WorkloadPod> = BTreeMap::new();
    loop {
        let mut stream = pin!(watcher(api.clone(), config.clone()));
        let mut listing = BTreeMap::new();
        let failure = loop {
            let Some(item) = stream.next().await else {
                return;
            };
            match item {
                Ok(watcher::Event::Init) => listing.clear(),
                Ok(watcher::Event::InitApply(pod)) => {
                    if let Some(pod) = WorkloadPod::of(&pod) {
                        listing.insert(pod.name.clone(), pod);
                    }
                }
                Ok(watcher::Event::InitDone) => {
                    backoff = MIN_BACKOFF;
                    pods = std::mem::take(&mut listing);
                    publish(&sink, &pods);
                }
                Ok(watcher::Event::Apply(pod)) => {
                    if let Some(pod) = WorkloadPod::of(&pod) {
                        pods.insert(pod.name.clone(), pod);
                        publish(&sink, &pods);
                    }
                }
                Ok(watcher::Event::Delete(pod)) => {
                    let Some(pod) = WorkloadPod::of(&pod) else {
                        continue;
                    };
                    // A late delete of an older incarnation leaves the new
                    // one in place.
                    if pods
                        .get(&pod.name)
                        .is_some_and(|known| known.uid == pod.uid)
                    {
                        pods.remove(&pod.name);
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

fn publish(sink: &watch::Sender<WorkloadPods>, pods: &BTreeMap<String, WorkloadPod>) {
    sink.send_replace(WorkloadPods {
        listed: true,
        pods: pods.values().cloned().collect(),
        failure: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(text: &str) -> Value {
        serde_yaml::from_str(text).expect("example YAML")
    }

    #[test]
    fn workloads_and_jobs_run_pods() {
        for key in [
            "deployments.apps",
            "statefulsets.apps",
            "daemonsets.apps",
            "replicasets.apps",
            "jobs.batch",
        ] {
            assert!(runs_pods(&builtin(key).unwrap()), "{key}");
        }
        for key in ["pods", "services", "configmaps", "cronjobs.batch"] {
            assert!(!runs_pods(&builtin(key).unwrap()), "{key}");
        }
    }

    #[test]
    fn a_selector_reads_labels_and_expressions() {
        let object = yaml(
            "spec:\n  selector:\n    matchLabels:\n      app: web\n    matchExpressions:\n    - key: tier\n      operator: In\n      values: [front, edge]\n",
        );
        assert_eq!(
            workload_selector(&object).as_deref(),
            Some("app=web,tier in (front,edge)")
        );
        assert_eq!(workload_selector(&yaml("spec:\n  selector: {}\n")), None);
        assert_eq!(workload_selector(&yaml("spec: {}\n")), None);
    }

    #[test]
    fn a_pod_reads_its_identity_and_containers() {
        let pod = WorkloadPod::of_object(&yaml(
            "metadata:\n  name: web-0\n  uid: u1\n  creationTimestamp: '2026-10-06T10:00:00Z'\n  deletionTimestamp: '2026-10-06T11:00:00Z'\nspec:\n  containers:\n  - name: app\n  - name: proxy\n",
        ));
        assert_eq!(pod.name, "web-0");
        assert_eq!(pod.uid, "u1");
        assert!(pod.terminating);
        assert!(pod.created.is_some());
        let names: Vec<_> = pod
            .containers
            .containers
            .iter()
            .map(|container| container.name.as_str())
            .collect();
        assert_eq!(names, ["app", "proxy"]);
    }
}
