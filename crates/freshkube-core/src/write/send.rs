//! The requests: a read of the object, a server-side dry run, and the send.
//! Server-side apply with field manager `freshkube`, never `force`; a merge
//! patch where apply doesn't fit; a delete with the UID and resourceVersion as
//! preconditions. A conflict is returned, never retried.

use kube::{
    Api,
    api::{DeleteParams, DynamicObject, Patch, PatchParams, PostParams, Preconditions},
};
use serde_json::{Value, json};

use super::{
    action::{Change, Target},
    audit::scrub,
};

/// The field manager every write carries.
pub const FIELD_MANAGER: &str = "freshkube";

/// A server-side apply conflict: other managers own fields the write sets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conflict {
    /// The field managers that own them.
    pub managers: Vec<String>,
    /// The fields in conflict, as the server names them.
    pub fields: Vec<String>,
}

/// What changed under the person since they opened the object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangedWhat {
    /// Someone wrote it: the resourceVersion moved.
    Modified,
    /// It was deleted.
    Gone,
    /// It was deleted and created again: the UID moved.
    Recreated,
    /// Something this write depends on is no longer as the page showed it.
    /// Reading the object again would not help: the page must be opened again.
    Moved,
}

impl ChangedWhat {
    pub fn text(self) -> &'static str {
        match self {
            ChangedWhat::Modified => "It was changed after you opened it.",
            ChangedWhat::Gone => "It no longer exists.",
            ChangedWhat::Recreated => "It was deleted and created again after you opened it.",
            ChangedWhat::Moved => {
                "It is no longer as the page showed it. Read the page again before going on."
            }
        }
    }
}

/// Why a request did not go through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A server-side apply conflict.
    Conflict(Conflict),
    /// The object is not as the person saw it.
    Changed(ChangedWhat),
    /// The API said the person may not.
    Denied(String),
    /// The API refused: admission, validation, anything else it answered.
    Rejected { code: u16, message: String },
    /// There was no answer. Whether the cluster took the write is not known.
    NoAnswer(String),
}

fn api(client: &kube::Client, target: &Target) -> Api<DynamicObject> {
    let resource = target.api_resource();
    match &target.namespace {
        Some(namespace) => Api::namespaced_with(client.clone(), namespace, &resource),
        None => Api::all_with(client.clone(), &resource),
    }
}

/// Reads the object as it is now. A Secret is read as metadata only, for every
/// action: the app never asks for a Secret's values on a write path.
pub async fn get(client: &kube::Client, target: &Target) -> Result<Value, Refusal> {
    let api = api(client, target);
    if target.is_secret() {
        let meta = api.get_metadata(&target.name).await.map_err(classify)?;
        let mut object = serde_json::to_value(meta).unwrap_or(Value::Null);
        // The metadata form answers as `PartialObjectMetadata`; the object is
        // still the Secret.
        object["apiVersion"] = json!(target.api_resource().api_version);
        object["kind"] = json!(target.kind);
        return Ok(object);
    }
    let object = api.get(&target.name).await.map_err(classify)?;
    Ok(serde_json::to_value(object).unwrap_or(Value::Null))
}

/// The server's reply as a value. The reply to a Secret's write carries its
/// values whatever was asked, so they are dropped here, before anything else
/// holds the reply: the diff, an observation, `Debug`, the audit and an error
/// all see a Secret without them.
fn reply_value(target: &Target, mut object: DynamicObject) -> Value {
    if target.is_secret()
        && let Some(map) = object.data.as_object_mut()
    {
        map.remove("data");
        map.remove("stringData");
    }
    serde_json::to_value(object).unwrap_or(Value::Null)
}

/// The body to send: the change's, with the resourceVersion the person saw, so
/// the server itself refuses it if the object moved since.
fn with_version(body: &Value, target: &Target) -> Value {
    let mut body = body.clone();
    if let Some(object) = body.as_object_mut() {
        let metadata = object.entry("metadata").or_insert_with(|| json!({}));
        if let Some(metadata) = metadata.as_object_mut() {
            metadata.insert("resourceVersion".into(), json!(target.resource_version));
        }
    }
    body
}

/// Sends `change` for `target`. With `dry_run` the server runs it all but
/// stores nothing. Returns the server's reply.
pub async fn send(
    client: &kube::Client,
    target: &Target,
    change: &Change,
    dry_run: bool,
) -> Result<Value, Refusal> {
    let api = api(client, target);
    let reply = match change {
        Change::Apply(body) | Change::Edit { body, .. } => {
            let mut params = PatchParams::apply(FIELD_MANAGER);
            params.dry_run = dry_run;
            let body = with_version(body, target);
            match &target.subresource {
                Some(sub) => {
                    api.patch_subresource(sub, &target.name, &params, &Patch::Apply(&body))
                        .await
                }
                None => api.patch(&target.name, &params, &Patch::Apply(&body)).await,
            }
            .map(|object| reply_value(target, object))
        }
        Change::MergePatch(body) => {
            let params = PatchParams {
                field_manager: Some(FIELD_MANAGER.to_owned()),
                dry_run,
                ..PatchParams::default()
            };
            let body = with_version(body, target);
            match &target.subresource {
                Some(sub) => {
                    api.patch_subresource(sub, &target.name, &params, &Patch::Merge(&body))
                        .await
                }
                None => api.patch(&target.name, &params, &Patch::Merge(&body)).await,
            }
            .map(|object| reply_value(target, object))
        }
        Change::Create(body) => {
            let params = PostParams {
                dry_run,
                field_manager: Some(FIELD_MANAGER.to_owned()),
            };
            // A new object has no version to check; a name taken is a plain
            // refusal, not a modified object.
            let object: DynamicObject = match serde_json::from_value(body.clone()) {
                Ok(object) => object,
                Err(error) => {
                    return Err(Refusal::Rejected {
                        code: 400,
                        message: scrub(&error.to_string()),
                    });
                }
            };
            return match api.create(&params, &object).await {
                Ok(created) => Ok(reply_value(target, created)),
                Err(kube::Error::Api(response)) if response.code == 409 => Err(Refusal::Rejected {
                    code: 409,
                    message: scrub(&response.message),
                }),
                Err(error) => Err(classify(error)),
            };
        }
        Change::Delete => {
            let params = DeleteParams {
                dry_run,
                preconditions: Some(Preconditions {
                    uid: Some(target.uid.clone()),
                    resource_version: Some(target.resource_version.clone()),
                }),
                ..DeleteParams::default()
            };
            api.delete(&target.name, &params).await.map(|either| {
                either.either(
                    |object| reply_value(target, object),
                    |status| serde_json::to_value(status).unwrap_or(Value::Null),
                )
            })
        }
    };
    reply.map_err(classify)
}

fn classify(error: kube::Error) -> Refusal {
    match error {
        kube::Error::Api(response) => match response.code {
            403 => Refusal::Denied(scrub(&response.message)),
            404 => Refusal::Changed(ChangedWhat::Gone),
            409 => match parse_conflict(&response.message) {
                Some(conflict) => Refusal::Conflict(conflict),
                None if response.message.contains("UID in precondition") => {
                    Refusal::Changed(ChangedWhat::Recreated)
                }
                None => Refusal::Changed(ChangedWhat::Modified),
            },
            code => Refusal::Rejected {
                code,
                message: scrub(&response.message),
            },
        },
        other => Refusal::NoAnswer(scrub(&other.to_string())),
    }
}

/// Reads the managers and fields out of a server-side apply conflict:
/// `Apply failed with 2 conflicts: conflicts with "kubectl" using apps/v1:\n-
/// .spec.replicas\n- .spec.paused`. `None` when the message is not one.
pub fn parse_conflict(message: &str) -> Option<Conflict> {
    if !message.contains("Apply failed with") {
        return None;
    }
    let mut conflict = Conflict::default();
    let mut rest = message;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('"') else { break };
        let manager = after[..end].to_owned();
        if !conflict.managers.contains(&manager) {
            conflict.managers.push(manager);
        }
        rest = &after[end + 1..];
    }
    for line in message.lines().skip(1) {
        let line = line.trim().trim_start_matches("- ").trim();
        if line.starts_with('.') {
            conflict.fields.push(line.to_owned());
        }
    }
    if conflict.fields.is_empty()
        && let Some((_, field)) = message.rsplit_once(": ")
        && field.trim().starts_with('.')
    {
        conflict.fields.push(field.trim().to_owned());
    }
    Some(conflict)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_conflict_names_its_manager_and_field() {
        let conflict = parse_conflict(
            "Apply failed with 1 conflict: conflict with \"kubectl-client-side-apply\" using apps/v1: .spec.replicas",
        )
        .unwrap();
        assert_eq!(conflict.managers, ["kubectl-client-side-apply"]);
        assert_eq!(conflict.fields, [".spec.replicas"]);
    }

    #[test]
    fn several_conflicts_are_all_named() {
        let conflict = parse_conflict(
            "Apply failed with 2 conflicts: conflicts with \"hpa\" using apps/v1:\n- .spec.replicas\nconflicts with \"kubectl\" using apps/v1:\n- .spec.paused",
        )
        .unwrap();
        assert_eq!(conflict.managers, ["hpa", "kubectl"]);
        assert_eq!(conflict.fields, [".spec.replicas", ".spec.paused"]);
    }

    #[test]
    fn another_message_is_not_a_conflict() {
        assert_eq!(parse_conflict("the object has been modified"), None);
    }

    #[test]
    fn the_version_the_person_saw_goes_in_the_body() {
        let target = Target {
            group: "apps".into(),
            version: "v1".into(),
            kind: "Deployment".into(),
            resource: "deployments".into(),
            namespace: Some("shop".into()),
            name: "web".into(),
            uid: "u".into(),
            resource_version: "41".into(),
            subresource: None,
        };
        let body = with_version(&json!({"spec": {"replicas": 3}}), &target);
        assert_eq!(body["metadata"]["resourceVersion"], "41");
        assert_eq!(body["spec"]["replicas"], 3);
    }
}
