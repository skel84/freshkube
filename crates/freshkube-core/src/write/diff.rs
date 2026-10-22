//! The diff a spec change shows: the object as it is against the server-side
//! dry run's reply, as changed field paths. A Secret's values are never in it.

use std::collections::BTreeMap;

use serde_json::Value;

/// How one field differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Changed,
    Removed,
}

/// One field of a diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldChange {
    pub path: String,
    pub kind: ChangeKind,
    /// The value before, cut for display. `None` for a Secret, or when added.
    pub before: Option<String>,
    pub after: Option<String>,
}

const IGNORED: [&str; 6] = [
    "metadata.managedFields",
    "metadata.resourceVersion",
    "metadata.generation",
    "metadata.uid",
    "metadata.creationTimestamp",
    "metadata.selfLink",
];

const SHOWN: usize = 160;

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

fn shown(value: &Value) -> String {
    let text = match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    if text.chars().count() > SHOWN {
        let mut cut: String = text.chars().take(SHOWN - 1).collect();
        cut.push('…');
        cut
    } else {
        text
    }
}

/// The fields that differ between `before` and `after`, in path order.
/// `status` is left out: a spec change does not write it. With `secret`, no
/// value is carried, only the paths.
pub fn diff(before: &Value, after: &Value, secret: bool) -> Vec<FieldChange> {
    let (mut old, mut new) = (BTreeMap::new(), BTreeMap::new());
    flatten(before, String::new(), &mut old);
    flatten(after, String::new(), &mut new);
    old.retain(|path, _| path != "status" && !path.starts_with("status."));
    new.retain(|path, _| path != "status" && !path.starts_with("status."));
    let value = |v: &Value| (!secret).then(|| shown(v));
    let mut changes = vec![];
    for (path, was) in &old {
        match new.get(path) {
            None => changes.push(FieldChange {
                path: path.clone(),
                kind: ChangeKind::Removed,
                before: value(was),
                after: None,
            }),
            Some(now) if now != was => changes.push(FieldChange {
                path: path.clone(),
                kind: ChangeKind::Changed,
                before: value(was),
                after: value(now),
            }),
            Some(_) => {}
        }
    }
    for (path, now) in &new {
        if !old.contains_key(path) {
            changes.push(FieldChange {
                path: path.clone(),
                kind: ChangeKind::Added,
                before: None,
                after: value(now),
            });
        }
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn it_lists_added_changed_and_removed_fields_in_path_order() {
        let before = json!({"spec": {"replicas": 2, "paused": true, "image": "a:1"}});
        let after = json!({"spec": {"replicas": 3, "image": "a:1", "strategy": "x"}});
        let changes = diff(&before, &after, false);
        let lines: Vec<_> = changes
            .iter()
            .map(|c| (c.path.as_str(), c.kind, c.before.clone(), c.after.clone()))
            .collect();
        assert_eq!(
            lines,
            [
                (
                    "spec.paused",
                    ChangeKind::Removed,
                    Some("true".into()),
                    None
                ),
                (
                    "spec.replicas",
                    ChangeKind::Changed,
                    Some("2".into()),
                    Some("3".into())
                ),
                ("spec.strategy", ChangeKind::Added, None, Some("x".into())),
            ]
        );
    }

    #[test]
    fn server_bookkeeping_and_status_are_not_changes() {
        let before = json!({"metadata": {"resourceVersion": "1", "generation": 1, "managedFields": [{"a": 1}]}, "status": {"ready": 1}, "spec": {"n": 1}});
        let after = json!({"metadata": {"resourceVersion": "2", "generation": 2, "managedFields": [{"a": 2}]}, "status": {"ready": 2}, "spec": {"n": 1}});
        assert!(diff(&before, &after, false).is_empty());
    }

    #[test]
    fn a_secret_carries_paths_and_no_value() {
        let before = json!({"data": {"password": "b2xk"}});
        let after = json!({"data": {"password": "bmV3", "token": "dG9r"}});
        let changes = diff(&before, &after, true);
        assert_eq!(changes.len(), 2);
        for change in &changes {
            assert_eq!(
                (change.before.as_ref(), change.after.as_ref()),
                (None, None)
            );
        }
        assert_eq!(changes[0].path, "data.password");
    }

    #[test]
    fn long_values_are_cut() {
        let after = json!({"spec": {"command": "x".repeat(1000)}});
        let changes = diff(&json!({}), &after, false);
        assert_eq!(changes[0].after.as_ref().unwrap().chars().count(), SHOWN);
    }
}
