//! What reads the cluster for a follow, with the user's own access and
//! nothing but reads: a watch on one pod, a watch on one Kargo project's
//! Stages, and Coroot's incidents once a minute. Each runs on Tokio until
//! its job is dropped, and sends what it saw as [`Seen`].

use std::collections::BTreeMap;
use std::pin::pin;
use std::time::Duration;

use freshkube_core::cluster_source::ClusterAccess;
use freshkube_core::coroot::{IncidentState, Provider, Source, Status, TimeRange};
use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::{ApiResource, DynamicObject, GroupVersionKind};
use kube::runtime::watcher;
use tokio::sync::mpsc::Sender;

use crate::model::COROOT_EVERY;
use crate::rules::{IncidentSeen, PodSeen, Seen, StageSeen};

const FIRST_BACKOFF: Duration = Duration::from_secs(2);
const LAST_BACKOFF: Duration = Duration::from_secs(60);

/// Where Coroot is read: a provider and the project it was pointed at.
#[derive(Clone)]
pub struct Coroot {
    pub provider: Provider,
    pub source: Source,
}

/// A pod as the rules see it.
pub fn pod_seen(pod: &Pod) -> PodSeen {
    let status = pod.status.as_ref();
    let containers = status.into_iter().flat_map(|status| {
        status
            .container_statuses
            .iter()
            .flatten()
            .chain(status.init_container_statuses.iter().flatten())
    });
    let mut crash_looping = false;
    let mut restarts = 0;
    for container in containers {
        restarts = restarts.max(container.restart_count.max(0) as u32);
        crash_looping |= container
            .state
            .as_ref()
            .and_then(|state| state.waiting.as_ref())
            .and_then(|waiting| waiting.reason.as_deref())
            == Some("CrashLoopBackOff");
    }
    let ready = status
        .and_then(|status| status.conditions.as_ref())
        .is_some_and(|conditions| {
            conditions
                .iter()
                .any(|c| c.type_ == "Ready" && c.status == "True")
        });
    PodSeen {
        crash_looping,
        ready,
        restarts,
    }
}

/// A Stage as the rules see it.
pub fn stage_seen(value: &serde_json::Value) -> Option<StageSeen> {
    let stage = freshkube_delivery::kargo::parse_stage(value)?;
    Some(StageSeen {
        name: stage.name,
        current_freight: stage.current_freight,
        verifications: stage
            .verifications
            .into_iter()
            .map(|v| (v.freight, v.phase))
            .collect(),
    })
}

/// Why a read stopped.
enum Ended {
    /// Nothing listens any more.
    Closed,
    Failed(String),
}

fn failed(error: impl std::fmt::Display) -> Ended {
    Ended::Failed(error.to_string())
}

/// Runs `once` again after a failure, with a growing wait, and tells the
/// owner what went wrong each time: the rules tell the person only once.
async fn keep_reading<F, Fut>(tx: &Sender<Seen>, mut once: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = (Ended, bool)>,
{
    let mut backoff = FIRST_BACKOFF;
    loop {
        let (ended, healthy) = once().await;
        if healthy {
            backoff = FIRST_BACKOFF;
        }
        let Ended::Failed(why) = ended else {
            return;
        };
        if tx.send(Seen::Unreadable(why)).await.is_err() {
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(LAST_BACKOFF);
    }
}

/// Watches one pod by name until the job is dropped.
pub async fn watch_pod(access: ClusterAccess, namespace: String, name: String, tx: Sender<Seen>) {
    let sender = tx.clone();
    keep_reading(&tx, move || {
        let (access, namespace, name, tx) = (
            access.clone(),
            namespace.clone(),
            name.clone(),
            sender.clone(),
        );
        async move {
            let client = match access.client().await {
                Ok(client) => client,
                Err(failure) => {
                    access.forget();
                    return (failed(failure.message), false);
                }
            };
            let api: Api<Pod> = Api::namespaced(client, &namespace);
            let config = watcher::Config::default().fields(&format!("metadata.name={name}"));
            let mut stream = pin!(watcher(api, config));
            let mut healthy = false;
            loop {
                let Some(item) = stream.next().await else {
                    return (Ended::Closed, healthy);
                };
                let seen = match item {
                    Ok(watcher::Event::InitApply(pod) | watcher::Event::Apply(pod)) => {
                        Seen::Pod(Some(pod_seen(&pod)))
                    }
                    Ok(watcher::Event::Delete(_)) => Seen::Pod(None),
                    Ok(watcher::Event::InitDone) => {
                        healthy = true;
                        continue;
                    }
                    Ok(watcher::Event::Init) => continue,
                    // An expired version: the watcher lists again by itself.
                    Err(watcher::Error::WatchError(status)) if status.code == 410 => continue,
                    Err(error) => return (failed(error), healthy),
                };
                healthy = true;
                if tx.send(seen).await.is_err() {
                    return (Ended::Closed, healthy);
                }
            }
        }
    })
    .await;
}

/// Watches the Stages of one Kargo project until the job is dropped. One
/// watch serves every follow in the project.
pub async fn watch_project(access: ClusterAccess, project: String, tx: Sender<Seen>) {
    let sender = tx.clone();
    keep_reading(&tx, move || {
        let (access, project, tx) = (access.clone(), project.clone(), sender.clone());
        async move {
            let client = match access.client().await {
                Ok(client) => client,
                Err(failure) => {
                    access.forget();
                    return (failed(failure.message), false);
                }
            };
            let resource = ApiResource::from_gvk_with_plural(
                &GroupVersionKind::gvk(freshkube_delivery::kargo::GROUP, "v1alpha1", "Stage"),
                "stages",
            );
            let api: Api<DynamicObject> = Api::namespaced_with(client, &project, &resource);
            let mut stream = pin!(watcher(api, watcher::Config::default()));
            let mut stages: BTreeMap<String, StageSeen> = BTreeMap::new();
            let mut listing: BTreeMap<String, StageSeen> = BTreeMap::new();
            let mut healthy = false;
            loop {
                let Some(item) = stream.next().await else {
                    return (Ended::Closed, healthy);
                };
                let of = |object: &DynamicObject| {
                    serde_json::to_value(object)
                        .ok()
                        .as_ref()
                        .and_then(stage_seen)
                };
                match item {
                    Ok(watcher::Event::Init) => listing.clear(),
                    Ok(watcher::Event::InitApply(object)) => {
                        if let Some(stage) = of(&object) {
                            listing.insert(stage.name.clone(), stage);
                        }
                        continue;
                    }
                    Ok(watcher::Event::InitDone) => stages = std::mem::take(&mut listing),
                    Ok(watcher::Event::Apply(object)) => {
                        if let Some(stage) = of(&object) {
                            stages.insert(stage.name.clone(), stage);
                        }
                    }
                    Ok(watcher::Event::Delete(object)) => {
                        if let Some(stage) = of(&object) {
                            stages.remove(&stage.name);
                        }
                    }
                    Err(watcher::Error::WatchError(status)) if status.code == 410 => continue,
                    Err(error) => return (failed(error), healthy),
                }
                healthy = true;
                let seen = Seen::Stages(stages.values().cloned().collect());
                if tx.send(seen).await.is_err() {
                    return (Ended::Closed, healthy);
                }
            }
        }
    })
    .await;
}

/// An incident as the rules see it.
pub fn incident_seen(incident: &freshkube_core::coroot::Incident) -> IncidentSeen {
    IncidentSeen {
        key: incident.key.clone(),
        application: incident.app.name().to_owned(),
        critical: incident.severity == Status::Critical,
        open: incident.state == IncidentState::Open,
        description: incident.description.clone(),
    }
}

/// Reads Coroot's incidents once a minute until the job is dropped.
pub async fn poll_incidents(coroot: Coroot, tx: Sender<Seen>) {
    loop {
        let seen = match coroot
            .provider
            .incidents(&coroot.source, TimeRange::default())
            .await
        {
            Ok(incidents) => Seen::Incidents(incidents.iter().map(incident_seen).collect()),
            Err(error) => Seen::Unreadable(error.to_string()),
        };
        if tx.send(seen).await.is_err() {
            return;
        }
        tokio::time::sleep(COROOT_EVERY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(json: serde_json::Value) -> Pod {
        serde_json::from_value(json).expect("a pod")
    }

    #[test]
    fn a_waiting_container_in_crash_loop_back_off_is_a_crash_loop() {
        let seen = pod_seen(&pod(serde_json::json!({
            "metadata": {"name": "api"},
            "status": {
                "conditions": [{"type": "Ready", "status": "False"}],
                "containerStatuses": [{
                    "name": "api", "image": "i", "imageID": "", "ready": false,
                    "restartCount": 7,
                    "state": {"waiting": {"reason": "CrashLoopBackOff"}}
                }]
            }
        })));
        assert!(seen.crash_looping && !seen.ready);
        assert_eq!(seen.restarts, 7);
    }

    #[test]
    fn a_ready_pod_is_ready_and_not_crash_looping() {
        let seen = pod_seen(&pod(serde_json::json!({
            "metadata": {"name": "api"},
            "status": {
                "conditions": [{"type": "Ready", "status": "True"}],
                "containerStatuses": [{
                    "name": "api", "image": "i", "imageID": "", "ready": true,
                    "restartCount": 1, "state": {"running": {}}
                }]
            }
        })));
        assert!(seen.ready && !seen.crash_looping);
    }

    #[test]
    fn a_stage_carries_its_freight_and_verifications() {
        let seen = stage_seen(&serde_json::json!({
            "metadata": {"name": "prod-ams", "namespace": "checkout", "uid": "u", "resourceVersion": "1"},
            "status": {"currentFreight": {"name": "wonky-otter"}}
        }))
        .expect("a stage");
        assert_eq!(seen.name, "prod-ams");
        assert_eq!(seen.current_freight, vec!["wonky-otter".to_owned()]);
    }
}
