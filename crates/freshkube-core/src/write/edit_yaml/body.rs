//! What an edit sends: the fields the person changed or added, and the fields
//! `freshkube` already owns. Server-side apply removes a field its manager
//! owned and leaves out of the next apply, so a body of only the changes would
//! take back what an earlier edit set. A line the person deleted is left out,
//! which removes it when `freshkube` owns it; a field another manager owns
//! stays, and the dialog says so.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use super::locked::{HIDDEN, IDENTITY};

/// The keys down to a field. A list is one field: a changed list is sent
/// whole, never element by element.
pub(super) type Path = Vec<String>;

/// The body of the apply and what it says about the edit.
pub(super) struct Built {
    pub body: Value,
    /// The fields the person changed or added.
    pub changed: Vec<Path>,
    /// Fields the person changed that an edit never sends (the cluster's own
    /// fields, a Secret's values), as dotted paths.
    pub ignored: Vec<String>,
    /// Fields the person deleted that another manager owns, so the server
    /// keeps them: as dotted paths, never values.
    pub kept: Vec<String>,
}

pub(super) fn build(
    original: &Value,
    edited: &Value,
    owned: Option<&Value>,
    secret: bool,
) -> Built {
    let mut changed = vec![];
    changed_paths(edited, Some(original), &mut vec![], &mut changed);
    let mut body = owned
        .and_then(|trie| owned_subset(trie, edited))
        .unwrap_or_else(|| json!({}));
    for path in &changed {
        if let Some(value) = at(edited, path) {
            insert(&mut body, path, value.clone());
        }
    }
    // The identity is the object's own, whatever the text says.
    for path in IDENTITY {
        let path: Path = path.split('.').map(str::to_owned).collect();
        if let Some(value) = at(original, &path) {
            insert(&mut body, &path, value.clone());
        }
    }
    strip(&mut body, secret);
    let (ignored, changed): (Vec<Path>, Vec<Path>) = changed
        .into_iter()
        .partition(|path| is_locked(path, secret));
    let ignored = ignored.iter().map(|path| path.join(".")).collect();
    let owned_now: BTreeSet<Path> = owned
        .and_then(|trie| owned_subset(trie, original))
        .map(|owned| leaves(&owned))
        .unwrap_or_default();
    let mut gone = vec![];
    removed_paths(original, edited, &mut vec![], &mut gone);
    let kept = gone
        .into_iter()
        .flat_map(|path| match at(original, &path) {
            Some(value) => leaves_under(value, path),
            None => vec![],
        })
        .filter(|path| !owned_now.contains(path) && !is_locked(path, secret))
        .map(|path| path.join("."))
        .collect();
    Built {
        body,
        changed,
        ignored,
        kept,
    }
}

fn is_locked(path: &Path, secret: bool) -> bool {
    let dotted = path.join(".");
    HIDDEN
        .iter()
        .any(|hidden| dotted == *hidden || dotted.starts_with(&format!("{hidden}.")))
        || (secret
            && matches!(
                path.first().map(String::as_str),
                Some("data" | "stringData")
            ))
}

/// The leaves of `edited` that differ from `original`: a scalar, an empty
/// map or a whole list.
fn changed_paths(edited: &Value, original: Option<&Value>, path: &mut Path, out: &mut Vec<Path>) {
    match edited {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                path.push(key.clone());
                changed_paths(child, original.and_then(|o| o.get(key)), path, out);
                path.pop();
            }
        }
        other => {
            if original != Some(other) {
                out.push(path.clone());
            }
        }
    }
}

/// The keys of `original` that `edited` no longer has, as the topmost path of
/// each removed part.
fn removed_paths(original: &Value, edited: &Value, path: &mut Path, out: &mut Vec<Path>) {
    let (Value::Object(was), Value::Object(now)) = (original, edited) else {
        return;
    };
    for (key, child) in was {
        path.push(key.clone());
        match now.get(key) {
            None => out.push(path.clone()),
            Some(kept) => removed_paths(child, kept, path, out),
        }
        path.pop();
    }
}

fn at<'a>(value: &'a Value, path: &Path) -> Option<&'a Value> {
    path.iter().try_fold(value, |value, key| value.get(key))
}

fn insert(body: &mut Value, path: &Path, value: Value) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut here = body;
    for key in parents {
        here = object(here).entry(key.clone()).or_insert_with(|| json!({}));
    }
    object(here).insert(last.clone(), value);
}

fn object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = json!({});
    }
    value.as_object_mut().expect("just made an object")
}

/// The text the person edits: the object without the cluster's own fields and,
/// for a Secret, with its values read as [`NOT_READ`](super::NOT_READ).
pub(super) fn hide(document: &mut Value, secret: bool) {
    strip(document, secret);
    if secret {
        document["data"] = Value::String(super::NOT_READ.into());
    }
}

/// Takes out what an edit can never send: the cluster's own fields, and a
/// Secret's values, whatever put them there.
fn strip(body: &mut Value, secret: bool) {
    let Some(map) = body.as_object_mut() else {
        return;
    };
    for hidden in HIDDEN {
        let path: Vec<&str> = hidden.split('.').collect();
        let (last, parents) = path.split_last().expect("a path has a key");
        let mut here = Some(&mut *map);
        for key in parents {
            here = here
                .and_then(|map| map.get_mut(*key))
                .and_then(Value::as_object_mut);
        }
        if let Some(map) = here {
            map.remove(*last);
        }
    }
    if secret {
        map.remove("data");
        map.remove("stringData");
    }
}

/// The leaf paths of a value: scalars, empty maps and lists.
fn leaves(value: &Value) -> BTreeSet<Path> {
    leaves_under(value, vec![]).into_iter().collect()
}

fn leaves_under(value: &Value, prefix: Path) -> Vec<Path> {
    match value {
        Value::Object(map) if !map.is_empty() => map
            .iter()
            .flat_map(|(key, child)| {
                let mut path = prefix.clone();
                path.push(key.clone());
                leaves_under(child, path)
            })
            .collect(),
        _ => vec![prefix],
    }
}

/// What the `freshkube` manager owns, as a trie of `fieldsV1`, picked out of
/// `edited` with the values it has there. A field it owns that `edited` no
/// longer has is not in it.
fn owned_subset(trie: &Value, edited: &Value) -> Option<Value> {
    let trie = trie.as_object()?;
    match edited {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, sub) in trie {
                let Some(name) = key.strip_prefix("f:") else {
                    continue;
                };
                let Some(value) = map.get(name) else {
                    continue;
                };
                if is_leaf(sub) {
                    out.insert(name.to_owned(), value.clone());
                } else if let Some(inner) = owned_subset(sub, value) {
                    out.insert(name.to_owned(), inner);
                }
            }
            (!out.is_empty()).then_some(Value::Object(out))
        }
        Value::Array(items) => {
            let mut picked: Vec<(usize, Value)> = vec![];
            for (key, sub) in trie {
                let Some((index, item, keys)) = element(key, items) else {
                    continue;
                };
                let value = if is_leaf(sub) && keys.is_empty() {
                    item.clone()
                } else {
                    let mut part = owned_subset(sub, item)
                        .and_then(|part| match part {
                            Value::Object(map) => Some(map),
                            _ => None,
                        })
                        .unwrap_or_default();
                    for (key, value) in keys {
                        part.insert(key, value);
                    }
                    Value::Object(part)
                };
                picked.push((index, value));
            }
            picked.sort_by_key(|(index, _)| *index);
            picked.dedup_by_key(|(index, _)| *index);
            (!picked.is_empty()).then(|| Value::Array(picked.into_iter().map(|(_, v)| v).collect()))
        }
        _ => None,
    }
}

/// A trie node that ends there: the field is owned whole.
fn is_leaf(sub: &Value) -> bool {
    sub.as_object().is_none_or(Map::is_empty)
}

/// The element of a list a `k:`, `v:` or `i:` key names, with the key fields
/// an element must carry to be found again.
fn element<'a>(key: &str, items: &'a [Value]) -> Option<(usize, &'a Value, Map<String, Value>)> {
    if let Some(keys) = key.strip_prefix("k:") {
        let Value::Object(want) = serde_json::from_str::<Value>(keys).ok()? else {
            return None;
        };
        let (index, item) = items
            .iter()
            .enumerate()
            .find(|(_, item)| want.iter().all(|(k, v)| item.get(k) == Some(v)))?;
        Some((index, item, want))
    } else if let Some(value) = key.strip_prefix("v:") {
        let want: Value = serde_json::from_str(value).ok()?;
        let (index, item) = items.iter().enumerate().find(|(_, item)| **item == want)?;
        Some((index, item, Map::new()))
    } else {
        let index: usize = key.strip_prefix("i:")?.parse().ok()?;
        Some((index, items.get(index)?, Map::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dotted(paths: &[Path]) -> Vec<String> {
        paths.iter().map(|path| path.join(".")).collect()
    }

    #[test]
    fn only_what_changed_is_sent_with_the_identity() {
        let was = json!({"apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "a", "namespace": "n", "labels": {"x": "1"}}, "data": {"k": "v", "j": "w"}});
        let mut now = was.clone();
        now["data"]["k"] = json!("v2");
        now["metadata"]["labels"]["y"] = json!("2");
        let built = build(&was, &now, None, false);
        assert_eq!(dotted(&built.changed), ["metadata.labels.y", "data.k"]);
        assert_eq!(
            built.body,
            json!({"apiVersion": "v1", "kind": "ConfigMap",
                "metadata": {"name": "a", "namespace": "n", "labels": {"y": "2"}},
                "data": {"k": "v2"}})
        );
    }

    #[test]
    fn what_freshkube_owns_is_kept_and_a_deleted_owned_line_is_left_out() {
        let was = json!({"apiVersion": "apps/v1", "kind": "Deployment",
            "metadata": {"name": "web", "annotations": {"a": "1", "b": "2"}},
            "spec": {"replicas": 3, "paused": true}});
        // freshkube owns replicas and annotation b; someone else owns the rest.
        let owned =
            json!({"f:spec": {"f:replicas": {}}, "f:metadata": {"f:annotations": {"f:b": {}}}});
        let mut now = was.clone();
        now["metadata"]["annotations"]["a"] = json!("changed");
        let built = build(&was, &now, Some(&owned), false);
        assert_eq!(built.body["spec"], json!({"replicas": 3}));
        assert_eq!(
            built.body["metadata"]["annotations"],
            json!({"a": "changed", "b": "2"})
        );
        // Deleting owned replicas and unowned paused: replicas is not sent
        // (the server removes it), paused is kept by the server.
        let mut now = was.clone();
        now["spec"] = json!({});
        now["spec"].as_object_mut().unwrap().clear();
        now.as_object_mut().unwrap().remove("spec");
        let built = build(&was, &now, Some(&owned), false);
        assert!(built.body.get("spec").is_none());
        assert_eq!(built.kept, ["spec.paused"]);
    }

    #[test]
    fn a_list_the_edit_changes_is_sent_whole_and_owned_elements_are_found_by_key() {
        let was = json!({"apiVersion": "apps/v1", "kind": "Deployment", "metadata": {"name": "web"},
            "spec": {"template": {"spec": {"containers": [
                {"name": "web", "image": "a:1", "args": ["x"]},
                {"name": "side", "image": "b:1"}]}}}});
        let owned = json!({"f:spec": {"f:template": {"f:spec": {"f:containers": {
            "k:{\"name\":\"side\"}": {".": {}, "f:name": {}, "f:image": {}}}}}}});
        let built = build(&was, &was, Some(&owned), false);
        assert_eq!(
            built.body["spec"]["template"]["spec"]["containers"],
            json!([{"name": "side", "image": "b:1"}])
        );
        let mut now = was.clone();
        now["spec"]["template"]["spec"]["containers"][0]["image"] = json!("a:2");
        let built = build(&was, &now, Some(&owned), false);
        assert_eq!(
            built.body["spec"]["template"]["spec"]["containers"],
            now["spec"]["template"]["spec"]["containers"]
        );
        assert_eq!(dotted(&built.changed), ["spec.template.spec.containers"]);
    }

    #[test]
    fn the_clusters_fields_and_a_secrets_values_never_reach_the_body() {
        let was = json!({"apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": "db", "labels": {"a": "1"}}, "data": "‹not read›"});
        // A trie that owns data and a hidden field, and an edit that adds a status.
        let owned = json!({"f:data": {}, "f:status": {}, "f:metadata": {"f:resourceVersion": {}, "f:labels": {"f:a": {}}}});
        let mut now = was.clone();
        now["status"] = json!({"x": 1});
        now["metadata"]["resourceVersion"] = json!("9");
        now["data"] = json!({"k": "dmFsdWU="});
        let built = build(&was, &now, Some(&owned), true);
        assert_eq!(
            built.body,
            json!({"apiVersion": "v1", "kind": "Secret",
                "metadata": {"name": "db", "labels": {"a": "1"}}})
        );
        assert!(
            built
                .changed
                .iter()
                .all(|p| p[0] != "data" && p[0] != "status")
        );
    }
}
