//! "Use the team's value" (#849): takes one of your own values out of your
//! `freshkube.yaml`, so the team's applies. The key is the flat key the team
//! file and your overrides use: `clusters[id].protection`, or `coroot.server`
//! for a top-level one.
//!
//! It works on the setup's own text and parses the result again with the
//! setup file's rules, so an edit that would make a setup the app can't read
//! (a required key removed) is refused, never written.

use serde_yaml::Value;

use crate::setup::{self, Setup};

/// Where a flat key points: the cluster it is under, if any, and the path of
/// names below that.
fn address(key: &str) -> Option<(Option<&str>, Vec<&str>)> {
    let (cluster, rest) = match key.strip_prefix("clusters[") {
        Some(after) => {
            let (id, rest) = after.split_once("].")?;
            (Some(id), rest)
        }
        None => (None, key),
    };
    let path: Vec<&str> = rest.split('.').collect();
    let named = |part: &&str| !part.is_empty() && !part.contains(['[', ']']);
    (path.iter().all(named) && cluster.is_none_or(|id| !id.is_empty())).then_some((cluster, path))
}

fn remove(map: &mut serde_yaml::Mapping, path: &[&str]) -> bool {
    let Some((first, rest)) = path.split_first() else {
        return false;
    };
    let name = Value::String((*first).to_string());
    if rest.is_empty() {
        return map.remove(&name).is_some();
    }
    let Some(Value::Mapping(inner)) = map.get_mut(&name) else {
        return false;
    };
    let removed = remove(inner, rest);
    if removed && inner.is_empty() {
        map.remove(&name);
    }
    removed
}

/// Why a value could not be taken out of the setup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotCleared {
    /// The setup holds no such value: nothing to write.
    Absent,
    /// Not a key, or what would be left is not a setup the app can read.
    Refused,
}

/// The setup without the value at `key`.
pub fn without_key(setup: &Setup, key: &str) -> Result<Setup, NotCleared> {
    cleared(setup, key)
        .ok_or(NotCleared::Refused)
        .and_then(|r| r)
}

/// `Some(Err(Absent))` for a well-formed key the setup doesn't hold.
fn cleared(setup: &Setup, key: &str) -> Option<Result<Setup, NotCleared>> {
    let (cluster, path) = address(key)?;
    let mut root: Value = serde_yaml::from_str(&setup::to_text(setup)).ok()?;
    let top = root.as_mapping_mut()?;
    let removed = match cluster {
        None => remove(top, &path),
        Some(id) => {
            let Some(Value::Sequence(clusters)) = top.get_mut("clusters") else {
                return Some(Err(NotCleared::Absent));
            };
            let entry = clusters.iter_mut().find_map(|item| {
                let item = item.as_mapping_mut()?;
                (item.get("id").and_then(Value::as_str) == Some(id)).then_some(item)
            });
            match entry {
                Some(entry) => remove(entry, &path),
                None => false,
            }
        }
    };
    if !removed {
        return Some(Err(NotCleared::Absent));
    }
    let text = serde_yaml::to_string(&root).ok()?;
    Some(setup::parse(text.as_bytes()).map_err(|_| NotCleared::Refused))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(text: &str) -> Setup {
        setup::parse(text.as_bytes()).expect("a setup")
    }

    const TEXT: &str = "\
version: 1
clusters:
  - id: core-fra
    role: core
    context: core-fra
    protection: open
  - id: cicd-fra
    role: core
    context: cicd-fra
    protection: protected
coroot:
  server: https://coroot.acme.example
  sign_in: anonymous
  project: shop
";

    #[test]
    fn a_cluster_value_goes_and_the_rest_stays() {
        let before = setup(TEXT);
        let after = without_key(&before, "clusters[core-fra].protection").expect("removed");
        let text = setup::to_text(&after);
        assert!(!text.contains("open"), "{text}");
        assert!(
            text.contains("protected"),
            "the other cluster keeps its own: {text}"
        );
        assert!(text.contains("context: core-fra"), "{text}");
    }

    #[test]
    fn a_nested_top_level_value_goes_and_its_block_stays() {
        let before = setup(TEXT);
        let one = without_key(&before, "coroot.project").expect("removed");
        let text = setup::to_text(&one);
        assert!(
            text.contains("server:") && !text.contains("project:"),
            "{text}"
        );
    }

    #[test]
    fn what_is_not_there_or_not_a_key_is_refused() {
        let before = setup(TEXT);
        for key in [
            "clusters[core-fra].fingerprint",
            "clusters[gone].protection",
            "coroot.nothing",
        ] {
            assert_eq!(without_key(&before, key), Err(NotCleared::Absent), "{key}");
        }
        for key in [
            "clusters[core-fra]",
            "clusters[].protection",
            "..",
            "",
            "coroot..server",
        ] {
            assert_eq!(without_key(&before, key), Err(NotCleared::Refused), "{key}");
        }
    }

    #[test]
    fn a_removal_that_would_break_the_setup_is_refused() {
        let before = setup(TEXT);
        // A cluster entry can't lose its role.
        assert_eq!(
            without_key(&before, "clusters[core-fra].role"),
            Err(NotCleared::Refused)
        );
    }
}
