//! External Secrets Operator's reported state. Never follows a Secret or a
//! provider/auth reference; store responses are deserialized into status only.
use chrono::{DateTime, Utc};
use futures::future::join_all;
use kube::Client;
use serde::Deserialize;
use serde_yaml::Value;

use super::object::{JSON, json_get};
use super::{Condition, Failure, FailureKind, ResourceKind, list_custom_groups, list_group_kinds};

pub const EXTERNAL_SECRETS_GROUP: &str = "external-secrets.io";
pub const MAX_STORES: usize = 20;

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ExternalSecret {
    pub conditions: Vec<Condition>,
    pub refreshed: Option<DateTime<Utc>>,
    pub interval: String,
    pub policy: String,
    pub interval_default: bool,
    pub policy_default: bool,
    pub stores: Vec<StoreRef>,
    pub unsupported_sources: bool,
    pub stores_omitted: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoreRef {
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct StoreStatus {
    pub ready: Option<Condition>,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct StoreRead {
    pub store: StoreRef,
    pub status: Result<StoreStatus, Failure>,
    pub api_absent: bool,
}

/// Only the ESO kind gets this summary, regardless of the served version.
pub(super) fn summary(
    kind: &ResourceKind,
    object: &Value,
    conditions: &[Condition],
) -> Option<ExternalSecret> {
    if kind.group != EXTERNAL_SECRETS_GROUP || kind.plural != "externalsecrets" {
        return None;
    }
    let spec = &object["spec"];
    let namespace = object["metadata"]["namespace"].as_str().unwrap_or_default();
    let mut stores = Vec::new();
    let mut unsupported_sources = false;
    let mut add = |value: &Value| {
        let Some(name) = value["name"].as_str().filter(|s| !s.is_empty()) else {
            return;
        };
        let kind = value["kind"].as_str().unwrap_or("SecretStore");
        if !matches!(kind, "SecretStore" | "ClusterSecretStore") {
            unsupported_sources = true;
            return;
        }
        let reference = StoreRef {
            kind: kind.into(),
            name: name.into(),
            namespace: (kind == "SecretStore").then(|| namespace.into()),
        };
        if !stores.contains(&reference) {
            stores.push(reference);
        }
    };
    add(&spec["secretStoreRef"]);
    for key in ["data", "dataFrom"] {
        if let Some(entries) = spec[key].as_sequence() {
            for entry in entries {
                add(&entry["sourceRef"]["storeRef"]);
            }
        }
    }
    let generators = ["data", "dataFrom"].iter().any(|key| {
        spec[*key].as_sequence().is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| !entry["sourceRef"]["generatorRef"].is_null())
        })
    });
    let stores_omitted = stores.len().saturating_sub(MAX_STORES);
    stores.truncate(MAX_STORES);
    Some(ExternalSecret {
        conditions: conditions
            .iter()
            .filter(|c| matches!(c.kind.as_str(), "Ready" | "SecretSynced"))
            .cloned()
            .collect(),
        refreshed: object["status"]["refreshTime"]
            .as_str()
            .and_then(|s| s.parse().ok()),
        interval: spec["refreshInterval"].as_str().unwrap_or("1h").into(),
        policy: spec["refreshPolicy"].as_str().unwrap_or("Periodic").into(),
        interval_default: spec["refreshInterval"].as_str().is_none(),
        policy_default: spec["refreshPolicy"].as_str().is_none(),
        stores,
        unsupported_sources: unsupported_sources || generators,
        stores_omitted,
    })
}

impl StoreRead {
    pub fn new(store: StoreRef, status: Result<StoreStatus, Failure>) -> Self {
        Self {
            store,
            status,
            api_absent: false,
        }
    }
}
impl StoreStatus {
    pub fn new(ready: Option<Condition>) -> Self {
        Self { ready }
    }
}

// Ignored fields (especially spec.provider and auth) are never materialized.
#[derive(Deserialize, Default)]
struct StoreResponse {
    #[serde(default)]
    status: Option<Status>,
}
#[derive(Deserialize, Default)]
struct Status {
    #[serde(default)]
    conditions: Option<Vec<ReportedCondition>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReportedCondition {
    #[serde(rename = "type")]
    kind: String,
    status: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    message: String,
    last_transition_time: Option<DateTime<Utc>>,
}
impl From<StoreResponse> for StoreStatus {
    fn from(response: StoreResponse) -> Self {
        Self {
            ready: response
                .status
                .and_then(|status| status.conditions)
                .unwrap_or_default()
                .into_iter()
                .find(|c| c.kind == "Ready")
                .map(|c| Condition {
                    kind: c.kind,
                    status: c.status,
                    reason: c.reason,
                    message: c.message,
                    changed: c.last_transition_time,
                }),
        }
    }
}

/// Discovers store versions independently of the ExternalSecret's version.
/// A failed store read is local to that store and says nothing about sync.
pub async fn read_external_secret_stores(
    client: &Client,
    secret: &ExternalSecret,
) -> Vec<StoreRead> {
    if secret.stores.is_empty() {
        return Vec::new();
    }
    let kinds = async {
        let group = list_custom_groups(client)
            .await?
            .into_iter()
            .find(|g| g.name == EXTERNAL_SECRETS_GROUP);
        match group {
            Some(group) => list_group_kinds(client, &group).await.map(Some),
            None => Ok(None),
        }
    }
    .await;
    join_all(secret.stores.iter().map(|store| async {
        let mut api_absent = false;
        let status = async {
            let kinds = kinds
                .as_ref()
                .map_err(Clone::clone)?
                .as_ref()
                .ok_or_else(|| {
                    api_absent = true;
                    Failure::new(FailureKind::NotFound, "External Secrets API not served")
                })?;
            let kind = kinds
                .kinds
                .iter()
                .find(|kind| kind.kind == store.kind)
                .ok_or_else(|| match kinds.failures.first() {
                    Some((version, failure)) => Failure::new(
                        failure.kind,
                        format!(
                            "Could not discover {} at {version}: {}",
                            store.kind, failure.message
                        ),
                    ),
                    None => {
                        api_absent = true;
                        Failure::new(
                            FailureKind::NotFound,
                            format!("{} API not served", store.kind),
                        )
                    }
                })?;
            let response: StoreResponse = client
                .request(json_get(
                    kind.object_path(store.namespace.as_deref(), &store.name),
                    JSON,
                )?)
                .await
                .map_err(Failure::from_kube)?;
            Ok(response.into())
        }
        .await;
        StoreRead {
            store: store.clone(),
            status,
            api_absent,
        }
    }))
    .await
}

#[cfg(test)]
mod tests;
