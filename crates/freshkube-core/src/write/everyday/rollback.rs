//! Roll back a Deployment to an earlier revision (#857). Its revisions are the
//! ReplicaSets it owns; rolling back makes its pod template the revision's,
//! as `kubectl rollout undo` does. A spec change: the dialog shows the dry
//! run's diff of the template (field paths and kinds, never a value).

use serde_json::{Map, Value, json};

use super::{
    ActionDescription, Change, Everyday, MOST_PAGES, NotOffered, ObjectChange, Observation, PAGE,
    Subject, Tier,
};

/// The label the Deployment controller adds to a ReplicaSet's pod template.
/// A Deployment's own template never has it.
const HASH_LABEL: &str = "pod-template-hash";

/// The annotation both a Deployment and its ReplicaSets carry: the revision.
const REVISION: &str = "deployment.kubernetes.io/revision";

/// The most revisions the picker offers; a Deployment keeps ten by default.
const MOST_REVISIONS: usize = 20;

/// One revision of a Deployment: one of the ReplicaSets it owns.
#[derive(Clone, Debug, PartialEq)]
pub struct Revision {
    pub number: u64,
    /// When the ReplicaSet was created, in seconds since the epoch.
    pub created: Option<i64>,
    /// The images of its containers, in order.
    pub images: Vec<String>,
    /// The Deployment runs this one now: it cannot be rolled back to.
    pub current: bool,
    /// Its pod template without the controller's hash label: what the
    /// Deployment's template becomes.
    template: Value,
}

impl Revision {
    fn of(replicaset: &Value, deployment: &Value) -> Option<Self> {
        let number: u64 = replicaset
            .pointer("/metadata/annotations")?
            .get(REVISION)?
            .as_str()?
            .parse()
            .ok()?;
        let mut template = replicaset.pointer("/spec/template")?.clone();
        if let Some(labels) = template
            .pointer_mut("/metadata/labels")
            .and_then(Value::as_object_mut)
        {
            labels.remove(HASH_LABEL);
        }
        let images = template
            .pointer("/spec/containers")
            .and_then(Value::as_array)
            .map(|containers| {
                containers
                    .iter()
                    .filter_map(|container| container["image"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let created = replicaset
            .pointer("/metadata/creationTimestamp")
            .and_then(Value::as_str)
            .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
            .map(|time| time.timestamp());
        let now = deployment.pointer("/spec/template");
        let running = deployment
            .pointer("/metadata/annotations")
            .and_then(|annotations| annotations.get(REVISION))
            .and_then(Value::as_str)
            .and_then(|revision| revision.parse::<u64>().ok());
        Some(Self {
            number,
            created,
            images,
            current: running == Some(number) || now == Some(&template),
            template,
        })
    }
}

/// What the read found of a Deployment's revisions.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Revisions {
    /// Not read: an object made from a document, not from the cluster.
    #[default]
    NotRead,
    /// Newest first, the current one among them.
    Read(Vec<Revision>),
    /// The ReplicaSets could not be read; rolling back is not on offer.
    Failed(String),
}

impl Revisions {
    /// The revisions among `replicasets`, newest first: those whose controller
    /// is the Deployment with this `uid` and which carry a revision number.
    /// Only an owner's UID makes a ReplicaSet a revision: a label selector
    /// alone could match another controller's, and its template is not this
    /// Deployment's history.
    pub fn from_replicasets(deployment: &Value, uid: &str, replicasets: &[Value]) -> Self {
        let mut found: Vec<Revision> = replicasets
            .iter()
            .filter(|replicaset| {
                replicaset
                    .pointer("/metadata/ownerReferences")
                    .and_then(Value::as_array)
                    .is_some_and(|owners| {
                        owners.iter().any(|owner| {
                            owner["controller"] == true && owner["uid"].as_str() == Some(uid)
                        })
                    })
            })
            .filter_map(|replicaset| Revision::of(replicaset, deployment))
            .collect();
        found.sort_by_key(|revision| std::cmp::Reverse(revision.number));
        found.truncate(MOST_REVISIONS);
        Self::Read(found)
    }
}

/// Reads the ReplicaSets of the Deployment's namespace that its selector
/// matches, a few pages at most.
pub(super) async fn find_revisions(client: &kube::Client, subject: &Subject) -> Revisions {
    use kube::api::{Api, ApiResource, DynamicObject, ListParams};
    let Some(namespace) = subject.target.namespace.as_deref() else {
        return Revisions::Failed("The Deployment has no namespace.".to_owned());
    };
    let resource = ApiResource {
        group: "apps".into(),
        version: "v1".into(),
        api_version: "apps/v1".into(),
        kind: "ReplicaSet".into(),
        plural: "replicasets".into(),
    };
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), namespace, &resource);
    let mut params = ListParams::default().limit(PAGE);
    if let Some(selector) = selector(&subject.object) {
        params = params.labels(&selector);
    }
    let mut all = vec![];
    for _ in 0..MOST_PAGES {
        let list = match api.list(&params).await {
            Ok(list) => list,
            Err(error) => {
                return Revisions::Failed(format!(
                    "The ReplicaSets could not be read: {}",
                    crate::write::scrub(&error.to_string())
                ));
            }
        };
        all.extend(
            list.items
                .iter()
                .filter_map(|replicaset| serde_json::to_value(replicaset).ok()),
        );
        match list
            .metadata
            .continue_
            .as_deref()
            .filter(|token| !token.is_empty())
        {
            Some(token) => params = params.continue_token(token),
            None => {
                return Revisions::from_replicasets(&subject.object, &subject.target.uid, &all);
            }
        }
    }
    Revisions::Failed("There are too many ReplicaSets to read its revisions.".to_owned())
}

/// The Deployment's `matchLabels` as a label selector; none when it has only
/// expressions, and every ReplicaSet of the namespace is then listed.
fn selector(deployment: &Value) -> Option<String> {
    let labels = deployment
        .pointer("/spec/selector/matchLabels")?
        .as_object()?;
    let pairs: Vec<String> = labels
        .iter()
        .filter_map(|(key, value)| Some(format!("{key}={}", value.as_str()?)))
        .collect();
    (!pairs.is_empty()).then(|| pairs.join(","))
}

/// The JSON merge patch that turns `from` into `to`: what differs, and a
/// null for every key `to` lacks. Arrays are replaced whole, as the patch
/// format does; a null in `to` is the same as a missing key.
fn merge_patch(from: &Value, to: &Value) -> Value {
    let (Value::Object(from), Value::Object(to)) = (from, to) else {
        return to.clone();
    };
    let mut patch = Map::new();
    for (key, new) in to {
        match from.get(key) {
            Some(old) if old == new => {}
            Some(old) => {
                patch.insert(key.clone(), merge_patch(old, new));
            }
            None if new.is_null() => {}
            None => {
                patch.insert(key.clone(), new.clone());
            }
        }
    }
    for (key, old) in from {
        if !to.contains_key(key) && !old.is_null() {
            patch.insert(key.clone(), Value::Null);
        }
    }
    Value::Object(patch)
}

impl Subject {
    /// Why Roll back can't start for this Deployment, or `None`: checked
    /// before the picker opens, and again by [`rollback`].
    pub fn rollback_refusal(&self) -> Option<String> {
        if !self.offered().contains(&Everyday::Rollback) {
            return Some("Only a Deployment can be rolled back.".to_owned());
        }
        // As `kubectl rollout undo` refuses it.
        if self.object.pointer("/spec/paused") == Some(&Value::Bool(true)) {
            return Some("A paused Deployment can't be rolled back. Resume it first.".to_owned());
        }
        match &self.revisions {
            Revisions::NotRead => Some("Its revisions were not read.".to_owned()),
            Revisions::Failed(why) => Some(why.clone()),
            Revisions::Read(revisions) if revisions.iter().all(|r| r.current) => {
                Some("It has no earlier revision to roll back to.".to_owned())
            }
            Revisions::Read(_) => None,
        }
    }
}

/// Roll the Deployment back to `revision`: a merge patch that makes its pod
/// template the revision's, observed when the template equals it. It is
/// refused when the Deployment is paused (as `kubectl rollout undo` does),
/// when the revision is not one of its own ReplicaSets, and when it is the
/// one it runs. There is no Undo (the spec change tier has none); rolling
/// forward is rolling back to the revision that was current.
pub fn rollback(subject: &Subject, revision: u64) -> Result<ActionDescription, NotOffered> {
    if !subject.offered().contains(&Everyday::Rollback) {
        return Err(NotOffered("Only a Deployment can be rolled back."));
    }
    if subject.object.pointer("/spec/paused") == Some(&Value::Bool(true)) {
        return Err(NotOffered(
            "A paused Deployment can't be rolled back. Resume it first.",
        ));
    }
    let revisions = match &subject.revisions {
        Revisions::Read(revisions) => revisions,
        Revisions::NotRead => return Err(NotOffered("Its revisions were not read.")),
        Revisions::Failed(_) => return Err(NotOffered("Its revisions could not be read.")),
    };
    let chosen = revisions
        .iter()
        .find(|candidate| candidate.number == revision)
        .ok_or(NotOffered("It has no such revision any more."))?;
    let now = subject
        .object
        .pointer("/spec/template")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if chosen.current || chosen.template == now {
        return Err(NotOffered("It is already at this revision."));
    }
    let patch = merge_patch(&now, &chosen.template);
    Ok(ActionDescription {
        kind: "rollback".into(),
        tier: Tier::SpecChange,
        verb: "Roll back".into(),
        sentence: subject.sentence(format!(
            "Roll back {} to revision {revision}: its pod template becomes that revision's, and the rollout replaces its pods",
            subject.target.label()
        )),
        destinations: vec![],
        no_confirmation: false,
        title: Some(format!(
            "Roll back {} to revision {revision}",
            subject.target.label()
        )),
        subjects: vec![],
        preview: vec![],
        changes: vec![ObjectChange {
            target: subject.target.clone(),
            change: Change::MergePatch(subject.body(json!({"template": patch}))),
            observe: Observation::FieldEquals {
                pointer: "/spec/template".into(),
                value: chosen.template.clone(),
            },
            inverse: None,
            depends: vec![],
        }],
    })
}
