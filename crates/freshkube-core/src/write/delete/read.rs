//! Reading what a delete will touch: the object, and the objects of the kinds
//! that may own it. One get and at most [`MOST_LISTS`] lists of one page each,
//! under one deadline, with the person's own access.

use kube::api::{Api, ApiResource, DynamicObject, ListParams};
use serde_json::Value;
use tokio::time::Instant;

use super::{DEADLINE, Dependents, Impact, MOST_LISTS, Owned, PAGE, owned_kinds};
use crate::write::{Refusal, Target, everyday::Row, send::get};

impl Impact {
    /// Reads the object a row names and what owns objects of the kinds that
    /// may be its dependents. An object that is another one now (a new UID)
    /// is not read for: the list is stale.
    pub async fn read(client: &kube::Client, row: &Row) -> Result<Self, String> {
        let target = Target {
            group: row.group.clone(),
            version: row.version.clone(),
            kind: row.kind.clone(),
            resource: row.resource.clone(),
            namespace: row.namespace.clone(),
            name: row.name.clone(),
            uid: row.uid.clone(),
            resource_version: String::new(),
            subresource: None,
        };
        let object = get(client, &target)
            .await
            .map_err(|refusal| match refusal {
                Refusal::Changed(what) => format!("{}: {}", target.label(), what.text()),
                Refusal::Denied(why) => {
                    format!("You are not allowed to read {}: {why}", target.label())
                }
                _ => format!("{} could not be read.", target.label()),
            })?;
        if object["metadata"]["uid"].as_str() != Some(row.uid.as_str()) {
            return Err(format!(
                "{} is a different object now. Refresh the list.",
                target.label()
            ));
        }
        let dependents = find_dependents(client, row, &object).await;
        Self::new(row, &object, dependents)
            .ok_or_else(|| format!("{} has no identity to delete.", target.label()))
    }
}

/// What the lists of the possible dependents' kinds say. A list that is
/// refused, fails or runs past the deadline is unknown for its kind.
async fn find_dependents(client: &kube::Client, row: &Row, object: &Value) -> Vec<Dependents> {
    let uid = object["metadata"]["uid"].as_str().unwrap_or_default();
    let kinds: &[Owned] = owned_kinds(&row.group, &row.kind, row.namespace.is_some());
    let Some(namespace) = row.namespace.as_deref() else {
        // A cluster-wide object may own objects anywhere; they are not
        // looked for, and that is said, not read as none.
        return vec![Dependents::unknown(
            "objects",
            "not looked up for an object with no namespace",
        )];
    };
    let started = Instant::now();
    let mut found = vec![];
    for owned in kinds.iter().take(MOST_LISTS) {
        let left = DEADLINE.saturating_sub(started.elapsed());
        let resource = ApiResource {
            group: owned.group.into(),
            version: owned.version.into(),
            api_version: if owned.group.is_empty() {
                owned.version.into()
            } else {
                format!("{}/{}", owned.group, owned.version)
            },
            kind: owned.kind.into(),
            plural: owned.plural.into(),
        };
        let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), namespace, &resource);
        let list = tokio::time::timeout(left, api.list(&ListParams::default().limit(PAGE))).await;
        found.push(match list {
            Err(_) => Dependents::unknown(owned.kind, "no answer in time"),
            Ok(Err(kube::Error::Api(response))) if response.code == 403 => {
                Dependents::unknown(owned.kind, "you aren't allowed to list them")
            }
            Ok(Err(_)) => Dependents::unknown(owned.kind, "the list could not be read"),
            Ok(Ok(list)) => {
                let items: Vec<Value> = list
                    .items
                    .into_iter()
                    .map(|item| {
                        serde_json::json!({"metadata": {
                            "name": item.metadata.name,
                            "ownerReferences": item.metadata.owner_references,
                        }})
                    })
                    .collect();
                let more = list
                    .metadata
                    .continue_
                    .as_deref()
                    .is_some_and(|token| !token.is_empty());
                Dependents::of(owned.kind, uid, &items, more)
            }
        });
    }
    found
}
