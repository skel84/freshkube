//! Read-only pod relationships. Services are listed once in the pod's namespace;
//! a ReplicaSet controller is read once and checked against the owner's UID.
use super::{
    Failure, FailureKind, ObjectDocument, Owner, ResourceKind, builtin_by_gvk, get_object,
};
use k8s_openapi::api::core::v1::Service;
use kube::{Api, Client, api::ListParams};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ServiceSelector {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub selector: BTreeMap<String, String>,
}
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ControllerChain {
    pub via_uid: String,
    pub owners: Vec<Owner>,
}
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PodLinks {
    pub services: Result<Vec<ServiceSelector>, Failure>,
    pub controller: Option<ControllerChain>,
    pub controller_error: Option<Failure>,
}
impl PodLinks {
    /// Derive relationships from independently available answers.
    pub fn from_sources(
        pod: &ObjectDocument,
        services: Result<Vec<Service>, Failure>,
        replica_set: Option<Result<ObjectDocument, Failure>>,
    ) -> Self {
        let services = services.map(|services| {
            let mut selectors = services
                .into_iter()
                .filter_map(|service| {
                    let selector = service.spec?.selector?;
                    if selector.is_empty() {
                        return None;
                    }
                    Some(ServiceSelector {
                        namespace: service.metadata.namespace.unwrap_or_default(),
                        name: service.metadata.name.unwrap_or_default(),
                        uid: service.metadata.uid.unwrap_or_default(),
                        selector,
                    })
                })
                .collect::<Vec<_>>();
            selectors.sort_by(|left, right| left.name.cmp(&right.name));
            selectors
        });
        let owner = replica_set_owner(pod);
        let (controller, controller_error) = match (owner, replica_set) {
            (Some(owner), Some(Ok(document))) if document.uid == owner.uid => (
                Some(ControllerChain {
                    via_uid: owner.uid.clone(),
                    owners: document.overview.owners,
                }),
                None,
            ),
            (Some(_), Some(Ok(_))) => (
                None,
                Some(Failure::new(
                    FailureKind::NotFound,
                    "The ReplicaSet was replaced",
                )),
            ),
            (_, Some(Err(error))) => (None, Some(error)),
            _ => (None, None),
        };
        Self {
            services,
            controller,
            controller_error,
        }
    }
    /// A service selects a pod only when every entry in its nonempty selector matches.
    pub fn selected_by<'a>(
        &'a self,
        labels: &'a [(String, String)],
    ) -> impl Iterator<Item = &'a ServiceSelector> {
        self.services
            .as_ref()
            .ok()
            .into_iter()
            .flatten()
            .filter(move |service| {
                !service.selector.is_empty()
                    && service.selector.iter().all(|(key, value)| {
                        labels
                            .iter()
                            .any(|(name, selected)| name == key && selected == value)
                    })
            })
    }
}
fn replica_set_owner(pod: &ObjectDocument) -> Option<&Owner> {
    pod.overview.owners.iter().find(|owner| {
        owner.controller && owner.kind == "ReplicaSet" && owner.api_version.starts_with("apps/")
    })
}
/// Collects the two independent relationships concurrently. It never reads Secrets.
pub async fn collect_pod_links(client: &Client, pod: &ObjectDocument) -> PodLinks {
    let namespace = pod.namespace.as_deref().unwrap_or("default");
    let services = async {
        Api::<Service>::namespaced(client.clone(), namespace)
            .list(&ListParams::default())
            .await
            .map(|list| list.items)
            .map_err(Failure::from_kube)
    };
    let replica_set = async {
        let owner = replica_set_owner(pod)?;
        let kind = builtin_by_gvk(&owner.api_version, &owner.kind)?;
        Some(get_object(client, &kind, Some(namespace), &owner.name).await)
    };
    let (services, replica_set) = tokio::join!(services, replica_set);
    PodLinks::from_sources(pod, services, replica_set)
}
/// Resolves any owner kind from its actual discovery document, without guessing a plural.
pub async fn resolve_owner_kind(
    client: &Client,
    api_version: &str,
    kind: &str,
) -> Result<ResourceKind, Failure> {
    if let Some(kind) = builtin_by_gvk(api_version, kind) {
        return Ok(kind);
    }
    #[derive(serde::Deserialize)]
    struct ResourceList {
        resources: Vec<Entry>,
    }
    #[derive(serde::Deserialize)]
    struct Entry {
        name: String,
        kind: String,
        namespaced: bool,
    }
    let (group, version) = api_version.split_once('/').unwrap_or(("", api_version));
    let path = if group.is_empty() {
        format!("/api/{version}")
    } else {
        format!("/apis/{group}/{version}")
    };
    let list = client
        .request::<ResourceList>(super::object::json_get(path, super::object::JSON)?)
        .await
        .map_err(Failure::from_kube)?;
    let entry = list
        .resources
        .into_iter()
        .find(|entry| entry.kind == kind && !entry.name.contains('/'))
        .ok_or_else(|| {
            Failure::new(
                FailureKind::NotFound,
                format!("{api_version} does not serve {kind}"),
            )
        })?;
    Ok(ResourceKind::new(
        group,
        version,
        &entry.kind,
        &entry.name,
        entry.namespaced,
    ))
}

#[cfg(test)]
mod tests;
