//! Metadata-only reads for object links and search.
use super::forward::api_resource;
use super::{Failure, ResourceKind};
use http::{Request, header};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::{Api, api::DynamicObject};
use serde::Deserialize;

#[derive(Deserialize)]
struct Metadata {
    metadata: ObjectMeta,
}

/// Resolve an object's identity without fetching its spec, status or Secret data.
pub async fn get_metadata(
    client: &kube::Client,
    kind: &ResourceKind,
    namespace: Option<&str>,
    name: &str,
) -> Result<ObjectMeta, Failure> {
    let resource = api_resource(kind);
    let api: Api<DynamicObject> = match namespace {
        Some(namespace) => Api::namespaced_with(client.clone(), namespace, &resource),
        None => Api::all_with(client.clone(), &resource),
    };
    api.get_metadata(name)
        .await
        .map(|object| object.metadata)
        .map_err(Failure::from_kube)
}

/// Bounded object names from a metadata-only list. A continuation means names were capped.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct MetadataNames {
    pub objects: Vec<ObjectMeta>,
    pub capped: bool,
}

/// Lists up to 2,000 names across all namespaces without an ordinary JSON fallback.
pub async fn list_metadata(
    client: &kube::Client,
    kind: &ResourceKind,
) -> Result<MetadataNames, Failure> {
    #[derive(Deserialize)]
    struct MetadataList {
        #[serde(default)]
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ListMeta,
        items: Vec<Metadata>,
    }
    let request = Request::get(format!(
        "{}?limit=2000&resourceVersion=0",
        kind.collection_path(None)
    ))
    .header(
        header::ACCEPT,
        "application/json;as=PartialObjectMetadataList;g=meta.k8s.io;v=v1",
    )
    .body(Vec::new())
    .map_err(|error| Failure::new(super::FailureKind::Other, error.to_string()))?;
    let list = client
        .request::<MetadataList>(request)
        .await
        .map_err(Failure::from_kube)?;
    let capped = list.items.len() >= 2_000
        || list
            .metadata
            .continue_
            .is_some_and(|value| !value.is_empty())
        || list
            .metadata
            .remaining_item_count
            .is_some_and(|count| count > 0);
    Ok(MetadataNames {
        objects: list
            .items
            .into_iter()
            .take(2_000)
            .map(|item| item.metadata)
            .collect(),
        capped,
    })
}

#[cfg(test)]
mod tests;
