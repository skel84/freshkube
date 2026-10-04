//! Cache-backed collection with independent deadlines.
use std::time::Duration;

use chrono::Utc;
use k8s_openapi::api::{
    apps::v1::{DaemonSet, Deployment, StatefulSet},
    core::v1::{Event, Namespace, Node, PersistentVolume, PersistentVolumeClaim, Pod},
};
use kube::{
    Api, Client, Resource,
    api::{ListParams, ObjectList},
};
use serde::de::DeserializeOwned;

use super::{KubernetesSummary, Part, derive};

const DEADLINE: Duration = Duration::from_secs(15);

async fn list<K>(client: Client, name: &str, params: &ListParams) -> Part<Vec<K>>
where
    K: Clone + std::fmt::Debug + DeserializeOwned + Resource<DynamicType = ()>,
{
    match tokio::time::timeout(DEADLINE, Api::<K>::all(client).list(params)).await {
        Ok(Ok(ObjectList { items, .. })) => Part::Loaded(items),
        Ok(Err(kube::Error::Api(error))) if error.code == 403 => {
            Part::Refused(format!("Can't list {name}: forbidden").into())
        }
        Ok(Err(_)) => Part::Failed(format!("Can't list {name}: request failed").into()),
        Err(_) => Part::Failed(format!("Can't list {name}: timed out").into()),
    }
}

/// Lists each part concurrently, using the API server cache and an independent
/// deadline. Kept for one-shot callers; the desktop shell uses `Session`.
pub async fn collect_kubernetes_summary(client: Client) -> KubernetesSummary {
    let params = ListParams::default().match_any();
    let warnings = params.clone().fields("type=Warning");
    let version_client = client.clone();
    let (
        version,
        nodes,
        pods,
        deployments,
        statefulsets,
        daemonsets,
        namespaces,
        claims,
        volumes,
        events,
    ) = tokio::join!(
        tokio::time::timeout(DEADLINE, version_client.apiserver_version()),
        list::<Node>(client.clone(), "nodes", &params),
        list::<Pod>(client.clone(), "pods", &params),
        list::<Deployment>(client.clone(), "deployments", &params),
        list::<StatefulSet>(client.clone(), "statefulsets", &params),
        list::<DaemonSet>(client.clone(), "daemonsets", &params),
        list::<Namespace>(client.clone(), "namespaces", &params),
        list::<PersistentVolumeClaim>(client.clone(), "persistentvolumeclaims", &params),
        list::<PersistentVolume>(client.clone(), "persistentvolumes", &params),
        list::<Event>(client.clone(), "events", &warnings),
    );
    let version = match version {
        Ok(Ok(version)) => Part::Loaded(version.git_version),
        Ok(Err(kube::Error::Api(error))) if error.code == 403 => {
            Part::Refused("Can’t read Kubernetes version: forbidden".into())
        }
        _ => Part::Failed("Kubernetes API not answering".into()),
    };
    derive(
        version,
        nodes,
        pods,
        deployments,
        statefulsets,
        daemonsets,
        namespaces,
        claims,
        volumes,
        events,
        Utc::now(),
    )
}
