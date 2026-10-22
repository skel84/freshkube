//! How an action describes itself to the framework: its tier, the objects it
//! touches, what it sends and how its outcome is observed. A page builds one
//! and hands it over; it never builds a request (`docs/WRITE_MODEL.md`).

use kube::api::ApiResource;
use serde_json::Value;

use super::requirement::Tier;

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

    /// Secret values are never shown in a diff.
    pub fn is_secret(&self) -> bool {
        self.group.is_empty() && self.resource == "secrets"
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
}

impl Change {
    /// The RBAC verb the access review asks about.
    pub fn verb(&self) -> &'static str {
        match self {
            Change::Apply(_) | Change::MergePatch(_) => "patch",
            Change::Delete => "delete",
        }
    }

    /// The field paths this change names, never their values. This is what
    /// the audit log records.
    pub fn field_paths(&self) -> Vec<String> {
        match self {
            Change::Apply(body) | Change::MergePatch(body) => {
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
    /// The object is gone.
    Gone,
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

/// One object's part of an action.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectChange {
    pub target: Target,
    pub change: Change,
    pub observe: Observation,
    /// For a reversible action: the way back.
    pub inverse: Option<Inverse>,
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
}

impl ActionDescription {
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
        assert!(target.is_secret());
        assert_eq!(target.label(), "Secret/s");
    }
}
