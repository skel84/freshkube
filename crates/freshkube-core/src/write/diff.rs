//! The diff a spec change shows: the object as it is against the server-side
//! dry run's reply, as changed field paths. A Secret's values are never in it.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::action::{Change, Target};

/// How one field differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Changed,
    Removed,
}

/// One field of a diff: where and how, never the value. A value can be a
/// credential in any kind (an `env[].value`, a ConfigMap, a custom
/// resource's token, the `last-applied-configuration` annotation), so none is
/// carried for any object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldChange {
    pub path: String,
    pub kind: ChangeKind,
}

const IGNORED: [&str; 6] = [
    "metadata.managedFields",
    "metadata.resourceVersion",
    "metadata.generation",
    "metadata.uid",
    "metadata.creationTimestamp",
    "metadata.selfLink",
];

fn flatten(value: &Value, prefix: String, out: &mut BTreeMap<String, Value>) {
    if IGNORED.iter().any(|ignored| prefix.starts_with(ignored)) {
        return;
    }
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(child, path, out);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (index, child) in items.iter().enumerate() {
                flatten(child, format!("{prefix}[{index}]"), out);
            }
        }
        _ if prefix.is_empty() => {}
        other => {
            out.insert(prefix, other.clone());
        }
    }
}

/// The fields that differ between `before` and `after`, in path order.
/// `status` is left out: a spec change does not write it.
pub fn diff(before: &Value, after: &Value) -> Vec<FieldChange> {
    let (mut old, mut new) = (BTreeMap::new(), BTreeMap::new());
    flatten(before, String::new(), &mut old);
    flatten(after, String::new(), &mut new);
    old.retain(|path, _| path != "status" && !path.starts_with("status."));
    new.retain(|path, _| path != "status" && !path.starts_with("status."));
    let mut changes = vec![];
    for (path, was) in &old {
        match new.get(path) {
            None => changes.push(FieldChange {
                path: path.clone(),
                kind: ChangeKind::Removed,
            }),
            Some(now) if now != was => changes.push(FieldChange {
                path: path.clone(),
                kind: ChangeKind::Changed,
            }),
            Some(_) => {}
        }
    }
    for path in new.keys() {
        if !old.contains_key(path) {
            changes.push(FieldChange {
                path: path.clone(),
                kind: ChangeKind::Added,
            });
        }
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    changes
}

/// `patch` laid over `base` as a merge patch is: objects merge key by key,
/// anything else (an array, a scalar) replaces. What example data's dry run
/// answers with.
pub fn overlay(base: &Value, patch: &Value) -> Value {
    match (base, patch) {
        (Value::Object(old), Value::Object(new)) => {
            let mut out = old.clone();
            for (key, value) in new {
                let merged = match old.get(key) {
                    Some(was) => overlay(was, value),
                    None => value.clone(),
                };
                out.insert(key.clone(), merged);
            }
            Value::Object(out)
        }
        // A list whose items are named (containers, ports, env) merges by
        // name, as server-side apply does; any other list is replaced.
        (Value::Array(old), Value::Array(new)) if named(old) && named(new) => {
            let mut out = old.clone();
            for item in new {
                match out
                    .iter()
                    .position(|was| was.get("name") == item.get("name"))
                {
                    Some(at) => out[at] = overlay(&out[at], item),
                    None => out.push(item.clone()),
                }
            }
            Value::Array(out)
        }
        (_, other) => other.clone(),
    }
}

fn named(items: &[Value]) -> bool {
    !items.is_empty()
        && items
            .iter()
            .all(|item| item.get("name").is_some_and(Value::is_string))
}

/// The diff of one object's write. A Secret is read as metadata only, so its
/// diff is its metadata's and, for the keys the write names under `data` and
/// `stringData`, those key names as changed: what each held before is not
/// known and is never read.
pub fn diff_for(
    target: &Target,
    change: &Change,
    before: &Value,
    after: &Value,
) -> Vec<FieldChange> {
    if !target.is_secret() {
        return diff(before, after);
    }
    let metadata = |object: &Value| json!({"metadata": object["metadata"]});
    let mut changes = diff(&metadata(before), &metadata(after));
    changes.extend(
        change
            .field_paths()
            .into_iter()
            .filter(|path| path.starts_with("data.") || path.starts_with("stringData."))
            .map(|path| FieldChange {
                path,
                kind: ChangeKind::Changed,
            }),
    );
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_overlay_merges_objects_and_named_lists_and_replaces_the_rest() {
        let base = json!({"spec": {"replicas": 2, "args": ["a"], "containers": [
            {"name": "web", "image": "a:1", "ports": [80]}, {"name": "side", "image": "b:1"}]}});
        let patch = json!({"spec": {"replicas": 5, "args": ["b", "c"],
            "containers": [{"name": "web", "image": "a:2"}]}});
        let after = overlay(&base, &patch);
        assert_eq!(
            after,
            json!({"spec": {"replicas": 5, "args": ["b", "c"], "containers": [
                {"name": "web", "image": "a:2", "ports": [80]}, {"name": "side", "image": "b:1"}]}})
        );
        let kinds: Vec<_> = diff(&base, &after)
            .into_iter()
            .map(|change| (change.path, change.kind))
            .collect();
        assert!(kinds.contains(&("spec.replicas".into(), ChangeKind::Changed)));
        assert!(kinds.contains(&("spec.args[1]".into(), ChangeKind::Added)));
        assert!(kinds.contains(&("spec.containers[0].image".into(), ChangeKind::Changed)));
    }

    #[test]
    fn safety_g13_a_secrets_diff_holds_its_metadata_and_the_key_names_it_writes_and_nothing_of_its_values()
     {
        let target = Target {
            group: String::new(),
            version: "v1".into(),
            kind: "Secret".into(),
            resource: "secrets".into(),
            namespace: Some("shop".into()),
            name: "db".into(),
            uid: "u".into(),
            resource_version: "1".into(),
            subresource: None,
        };
        let before = json!({"metadata": {"labels": {"a": "1"}}});
        // Even a reply that still carries values is not compared.
        let after = json!({"metadata": {"labels": {"a": "2"}}, "data": {"other": "c2VjcmV0"}});
        let change = Change::Apply(
            json!({"metadata": {"labels": {"a": "2"}}, "data": {"password": "bmV3"}}),
        );
        let lines: Vec<_> = diff_for(&target, &change, &before, &after)
            .into_iter()
            .map(|c| c.path)
            .collect();
        assert_eq!(lines, ["data.password", "metadata.labels.a"]);
    }

    #[test]
    fn it_lists_added_changed_and_removed_fields_in_path_order() {
        let before = json!({"spec": {"replicas": 2, "paused": true, "image": "a:1"}});
        let after = json!({"spec": {"replicas": 3, "image": "a:1", "strategy": "x"}});
        let lines: Vec<_> = diff(&before, &after)
            .into_iter()
            .map(|c| (c.path, c.kind))
            .collect();
        assert_eq!(
            lines,
            [
                ("spec.paused".to_owned(), ChangeKind::Removed),
                ("spec.replicas".to_owned(), ChangeKind::Changed),
                ("spec.strategy".to_owned(), ChangeKind::Added),
            ]
        );
    }

    #[test]
    fn server_bookkeeping_and_status_are_not_changes() {
        let before = json!({"metadata": {"resourceVersion": "1", "generation": 1, "managedFields": [{"a": 1}]}, "status": {"ready": 1}, "spec": {"n": 1}});
        let after = json!({"metadata": {"resourceVersion": "2", "generation": 2, "managedFields": [{"a": 2}]}, "status": {"ready": 2}, "spec": {"n": 1}});
        assert!(diff(&before, &after).is_empty());
    }

    #[test]
    fn no_kind_carries_a_value_not_a_secret_a_config_map_nor_an_env_var() {
        let before = json!({
            "data": {"password": "b2xk"},
            "spec": {"containers": [{"env": [{"name": "TOKEN", "value": "s3cr3t-old"}]}]},
            "metadata": {"annotations": {"kubectl.kubernetes.io/last-applied-configuration": "{\"token\":\"abc\"}"}}
        });
        let after = json!({
            "data": {"password": "bmV3", "token": "dG9r"},
            "spec": {"containers": [{"env": [{"name": "TOKEN", "value": "s3cr3t-new"}]}]},
            "metadata": {"annotations": {"kubectl.kubernetes.io/last-applied-configuration": "{\"token\":\"xyz\"}"}}
        });
        let changes = diff(&before, &after);
        assert_eq!(changes.len(), 4);
        let shown = format!("{changes:?}");
        for value in ["b2xk", "bmV3", "dG9r", "s3cr3t", "abc", "xyz"] {
            assert!(!shown.contains(value), "{value} in {shown}");
        }
        assert!(shown.contains("env[0].value"));
    }
}
