//! Metadata-only reads for object links and search.
use super::{Failure, ResourceKind};
use http::{Request, header};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
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
    let request = Request::get(kind.object_path(namespace, name))
        .header(
            header::ACCEPT,
            "application/json;as=PartialObjectMetadata;g=meta.k8s.io;v=v1",
        )
        .body(Vec::new())
        .map_err(|error| Failure::new(super::FailureKind::Other, error.to_string()))?;
    client
        .request::<Metadata>(request)
        .await
        .map(|object| object.metadata)
        .map_err(Failure::from_kube)
}

#[cfg(test)]
mod tests;
