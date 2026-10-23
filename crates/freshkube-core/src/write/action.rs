//! How an action describes itself to the framework: its tier, the objects it
//! touches, what it sends and how its outcome is observed. A page builds one
//! and hands it over; it never builds a request (`docs/WRITE_MODEL.md`).

use kube::api::ApiResource;
use std::sync::Arc;

use serde_json::Value;

use super::{LevelCell, Protection, requirement::Tier};

/// One object an action touches, as the person saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// The API group; empty for the core group.
    pub group: String,
    pub version: String,
    pub kind: String,
    /// The plural resource name, as it appears in a URL.
    pub resource: String,
    pub namespace: Option<String>,
    pub name: String,
    /// The UID the person saw. A recreated object has another.
    pub uid: String,
    /// The resourceVersion the person saw.
    pub resource_version: String,
    /// A subresource to write to instead of the object, such as `scale`.
    pub subresource: Option<String>,
}

impl Target {
    pub fn api_resource(&self) -> ApiResource {
        let api_version = if self.group.is_empty() {
            self.version.clone()
        } else {
            format!("{}/{}", self.group, self.version)
        };
        ApiResource {
            group: self.group.clone(),
            version: self.version.clone(),
            api_version,
            kind: self.kind.clone(),
            plural: self.resource.clone(),
        }
    }

    /// `kind/name`, how a dialog and a toast name the object.
    pub fn label(&self) -> String {
        format!("{}/{}", self.kind, self.name)
    }
}

/// What is sent for one object.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// Server-side apply of this object, with field manager `freshkube` and
    /// no forced takeover.
    Apply(Value),
    /// A JSON merge patch, where apply doesn't fit (a subresource).
    MergePatch(Value),
    /// A delete, with the UID and resourceVersion as preconditions.
    Delete,
    /// A new object, created with field manager `freshkube`. There is no
    /// object to GET before it, so the target's UID and resourceVersion are
    /// empty and preflight reads nothing: the access review and the dry run
    /// are what it checks, and a name already taken refuses the dry run.
    Create(Value),
}

impl Change {
    /// The RBAC verb the access review asks about.
    pub fn verb(&self) -> &'static str {
        match self {
            Change::Apply(_) | Change::MergePatch(_) => "patch",
            Change::Delete => "delete",
            Change::Create(_) => "create",
        }
    }

    /// The field paths this change names, never their values. This is what
    /// the audit log records.
    pub fn field_paths(&self) -> Vec<String> {
        match self {
            Change::Apply(body) | Change::MergePatch(body) | Change::Create(body) => {
                let mut paths = vec![];
                leaf_paths(body, String::new(), &mut paths);
                // The object's identity is in `objects`; only what changes is a field.
                paths.retain(|path| {
                    !matches!(
                        path.as_str(),
                        "apiVersion" | "kind" | "metadata.name" | "metadata.namespace"
                    ) && !path.starts_with("metadata.resourceVersion")
                });
                paths.sort();
                paths
            }
            Change::Delete => vec![],
        }
    }
}

fn leaf_paths(value: &Value, prefix: String, out: &mut Vec<String>) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                leaf_paths(child, path, out);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (index, child) in items.iter().enumerate() {
                leaf_paths(child, format!("{prefix}[{index}]"), out);
            }
        }
        _ => out.push(prefix),
    }
}

/// How the cluster reaching the new state is seen.
#[derive(Clone, Debug, PartialEq)]
pub enum Observation {
    /// The value at this JSON pointer (`/status/readyReplicas`) equals this.
    FieldEquals { pointer: String, value: Value },
    /// As `FieldEquals`, and a field the object leaves out reads as 0: a
    /// count that is omitted at zero, such as `readyReplicas` after a scale
    /// to 0. Only a value of 0 is ever met by an absent field.
    FieldEqualsOrAbsent { pointer: String, value: Value },
    /// The object is gone.
    Gone,
    /// The object exists: what a created object is seen by.
    Exists,
    /// Nothing can be watched. The result says so; it is never success.
    Unobservable,
}

/// What puts one object back as it was, computed from the object before the
/// send. It is a change like any other.
#[derive(Clone, Debug, PartialEq)]
pub struct Inverse {
    pub change: Change,
    /// How the cluster is seen to be back.
    pub observe: Observation,
    /// What must still hold for Undo to be safe: the fields the write set
    /// still have the values it gave them. If someone else wrote them since,
    /// Undo is not offered a send; it says who.
    pub guard: Observation,
}

/// How to work out the way back from the object as it is just before the
/// send. It is run on the object preflight reads, so after "Review the new
/// version" the way back is worked out again from the new version, never
/// kept from the old one.
#[derive(Clone)]
pub struct InverseFrom(Arc<WayBack>);

type WayBack = dyn Fn(&Value) -> Option<Inverse> + Send + Sync;

impl InverseFrom {
    pub fn new(f: impl Fn(&Value) -> Option<Inverse> + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    pub fn of(&self, before: &Value) -> Option<Inverse> {
        (self.0)(before)
    }
}

impl std::fmt::Debug for InverseFrom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InverseFrom")
    }
}

impl PartialEq for InverseFrom {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// One object's part of an action.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectChange {
    pub target: Target,
    pub change: Change,
    pub observe: Observation,
    /// For a reversible action: how to work out the way back.
    pub inverse: Option<InverseFrom>,
    /// Other objects this change depends on, checked in preflight.
    pub depends: Vec<Depends>,
}

/// Another object a change depends on, which preflight reads and finds as
/// the person saw it, so a page left open can't quietly act on a state that
/// has moved on (a Promotion on a Stage that runs other Freight now).
#[derive(Clone, Debug, PartialEq)]
pub struct Depends {
    /// The object, with the UID the person saw.
    pub target: Target,
    /// A verb the access review also asks about on it, such as Kargo's
    /// `promote` on a Stage.
    pub verb: Option<&'static str>,
    /// Fields that must still hold what the person saw: a JSON pointer and
    /// its value, where `null` means the field is absent.
    pub holds: Vec<(String, Value)>,
}

/// A second cluster an action also touches, such as the cluster a promotion
/// lands in. The action is held to the stricter of the two levels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    /// The destination entry's id.
    pub entry: String,
    /// Its level when the action was described.
    pub protection: Protection,
    /// The cell that holds its level now, when the app keeps one: the action
    /// follows it, as it follows the level of the cluster its objects are in,
    /// so a level changed in Settings reaches a dialog already open.
    pub cell: Option<LevelCell>,
    /// The subjects of the action it holds (`ActionDescription::subjects`),
    /// such as the Stages that deploy there; the dialog names them when the
    /// level asks for a name typed once for several.
    pub covers: Vec<String>,
}

impl Destination {
    /// The level now: the cell's when there is one, else the level it had.
    pub fn level(&self) -> Protection {
        self.cell.as_ref().map_or(self.protection, LevelCell::get)
    }
}

/// A titled list of lines the dialog shows under the sentence, which the
/// action supplies (a preview, a note). Lines hold no value that could be
/// secret, as the sentence does not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub title: String,
    pub lines: Vec<String>,
}

/// An action, described. Not sent until the framework has run its preflight
/// and the person has met the level's requirement.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionDescription {
    /// A stable id for the audit log, such as `scale`.
    pub kind: String,
    pub tier: Tier,
    /// What the confirm button says: `Scale`, `Delete`.
    pub verb: String,
    /// One sentence about what is sent, with no values that could be secret.
    pub sentence: String,
    pub changes: Vec<ObjectChange>,
    /// The other clusters the action reaches. It is held to the strictest of
    /// these and the cluster its objects are in.
    pub destinations: Vec<Destination>,
    /// The action declares that it needs no confirmation at any level but
    /// `read-only`, which still blocks it: it only adds an annotation. It is
    /// still audited.
    pub no_confirmation: bool,
    /// The dialog's title, when what the person does reads better than the
    /// verb and the objects' labels ("Promote wonky-otter to prod-ams").
    pub title: Option<String>,
    /// What a person calls the objects when a protected cluster asks for a
    /// name: the Stages a Promotion is for, not the Promotions' generated
    /// names. Empty means the objects' own names.
    pub subjects: Vec<String>,
    /// What the dialog shows under the sentence.
    pub preview: Vec<Preview>,
}

impl ActionDescription {
    /// The level the action is held to: the strictest of the cluster the
    /// objects are in (`own`) and its destination's.
    pub fn level(&self, own: Protection) -> Protection {
        self.destinations.iter().fold(own, |level, destination| {
            level.stricter(destination.level())
        })
    }

    /// The subjects a protected level asks to be named: the declared ones,
    /// else the objects' names.
    pub fn subject_names(&self) -> Vec<&str> {
        if self.subjects.is_empty() {
            self.names()
        } else {
            self.subjects.iter().map(String::as_str).collect()
        }
    }

    /// The subjects held to `protected`: all of them when the cluster the
    /// objects are in is, else those a protected destination covers.
    pub fn protected_subjects(&self, own: Protection) -> Vec<String> {
        if own == Protection::Protected {
            return self
                .subject_names()
                .into_iter()
                .map(str::to_owned)
                .collect();
        }
        let mut names: Vec<String> = vec![];
        for destination in &self.destinations {
            if destination.level() == Protection::Protected {
                for name in &destination.covers {
                    if !names.contains(name) {
                        names.push(name.clone());
                    }
                }
            }
        }
        names
    }

    pub fn names(&self) -> Vec<&str> {
        self.changes
            .iter()
            .map(|change| change.target.name.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn field_paths_name_the_leaves_and_never_a_value() {
        let change = Change::MergePatch(json!({
            "metadata": {"resourceVersion": "7"},
            "spec": {"replicas": 3, "containers": [{"image": "registry.example/app:2"}]},
            "data": {"password": "hunter2"}
        }));
        let paths = change.field_paths();
        assert_eq!(
            paths,
            ["data.password", "spec.containers[0].image", "spec.replicas"]
        );
        let joined = paths.join(" ");
        for value in ["hunter2", "registry.example", "3"] {
            assert!(!joined.contains(value), "{value}");
        }
    }

    #[test]
    fn a_delete_names_no_fields_and_asks_for_the_delete_verb() {
        assert!(Change::Delete.field_paths().is_empty());
        assert_eq!(Change::Delete.verb(), "delete");
        assert_eq!(Change::Apply(json!({})).verb(), "patch");
    }

    #[test]
    fn the_core_group_is_a_bare_version() {
        let target = Target {
            group: String::new(),
            version: "v1".into(),
            kind: "Secret".into(),
            resource: "secrets".into(),
            namespace: Some("a".into()),
            name: "s".into(),
            uid: "u".into(),
            resource_version: "1".into(),
            subresource: None,
        };
        assert_eq!(target.api_resource().api_version, "v1");
        assert_eq!(target.label(), "Secret/s");
    }
}
