//! "Use the team's value" (#849): takes one of your own values out of your
//! `freshkube.yaml`, so the team's applies. The key is the flat key the team
//! file and your overrides use: `clusters[id].coroot.project`, or
//! `coroot.server` for a top-level one. A cluster's key is taken out of your
//! entry on the context you chose for that team cluster (the mapping), never
//! out of an entry that merely has the cluster's id.
//!
//! A level is never taken out: until the team's floor reaches the level
//! writes are gated on, deleting your `protection` line would let the
//! default, `guarded`, apply, which can be looser than what you set.
//!
//! It works on the setup's own text and parses the result again with the
//! setup file's rules, so an edit that would make a setup the app can't read
//! (a required key removed) is refused, never written.

use serde_yaml::Value;

use super::mapping::{Choice, ContextMap};
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
    /// A protection level, which is never taken out (see the module).
    Level,
    /// No entry of yours is on the context chosen for the team cluster, or
    /// none is chosen: there is nothing of yours to take the value out of.
    NoEntry,
}

/// Whether `key` is a cluster's protection level.
pub fn is_level_key(key: &str) -> bool {
    matches!(address(key), Some((Some(_), path)) if path == ["protection"])
}

/// The setup without the value at `key`. A cluster's key comes out of your
/// entries on the context `mapping` has for that team cluster.
pub fn without_key(setup: &Setup, key: &str, mapping: &ContextMap) -> Result<Setup, NotCleared> {
    if is_level_key(key) {
        return Err(NotCleared::Level);
    }
    cleared(setup, key, mapping).unwrap_or(Err(NotCleared::Refused))
}

/// `Some(Err(Absent))` for a well-formed key the setup doesn't hold.
fn cleared(setup: &Setup, key: &str, mapping: &ContextMap) -> Option<Result<Setup, NotCleared>> {
    let (cluster, path) = address(key)?;
    let mut root: Value = serde_yaml::from_str(&setup::to_text(setup)).ok()?;
    let top = root.as_mapping_mut()?;
    let removed = match cluster {
        None => remove(top, &path),
        Some(id) => {
            let Some(Choice::Context(context)) = mapping.choice(id) else {
                return Some(Err(NotCleared::NoEntry));
            };
            let Some(Value::Sequence(clusters)) = top.get_mut("clusters") else {
                return Some(Err(NotCleared::NoEntry));
            };
            let mut on_context = false;
            let mut removed = false;
            for item in clusters.iter_mut().filter_map(Value::as_mapping_mut) {
                if item.get("context").and_then(Value::as_str) == Some(context.as_str()) {
                    on_context = true;
                    removed |= remove(item, &path);
                }
            }
            if !on_context {
                return Some(Err(NotCleared::NoEntry));
            }
            removed
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
    use crate::team::Version;

    fn setup(text: &str) -> Setup {
        setup::parse(text.as_bytes()).expect("a setup")
    }

    /// The team's clusters, mapped as `choices` say.
    fn mapping(choices: &[(&str, Choice)]) -> ContextMap {
        let team = Version::parse(
            "v1",
            b"version: 1\nclusters:\n  - id: core-fra\n    role: core\n  - id: cicd-fra\n    role: cicd\n",
            Default::default(),
        )
        .unwrap();
        let mut map = ContextMap::for_version(&team, &ContextMap::default());
        let available: Vec<String> = ["ctx-a", "ctx-b"].map(String::from).into();
        for (id, choice) in choices {
            map.choose(id, choice.clone(), &available).unwrap();
        }
        map
    }

    fn on(context: &str) -> Choice {
        Choice::Context(context.into())
    }

    const TEXT: &str = "\
version: 1
clusters:
  - id: core-fra
    role: core
    context: ctx-a
    protection: open
    metrics:
      source:
        url: https://metrics.acme.example
  - id: elsewhere
    role: core
    context: ctx-b
    protection: protected
    metrics:
      source:
        url: https://metrics.other.example
coroot:
  server: https://coroot.acme.example
  sign_in: anonymous
  project: shop
";

    const URL: &str = "clusters[core-fra].metrics.source.url";

    #[test]
    fn a_cluster_value_comes_out_of_the_entry_on_the_mapped_context() {
        let before = setup(TEXT);
        // The team's core-fra is your ctx-b: the entry on ctx-b loses its
        // value, and the entry that merely has the id core-fra keeps its own.
        let map = mapping(&[("core-fra", on("ctx-b")), ("cicd-fra", Choice::LeftOut)]);
        let after = without_key(&before, URL, &map).expect("removed");
        let text = setup::to_text(&after);
        assert!(text.contains("metrics.acme.example"), "{text}");
        assert!(!text.contains("metrics.other.example"), "{text}");
        // And the other way round.
        let map = mapping(&[("core-fra", on("ctx-a")), ("cicd-fra", Choice::LeftOut)]);
        let text = setup::to_text(&without_key(&before, URL, &map).expect("removed"));
        assert!(!text.contains("metrics.acme.example"), "{text}");
        assert!(text.contains("metrics.other.example"), "{text}");
        assert!(text.contains("protection: open"), "the rest stays: {text}");
    }

    #[test]
    fn a_cluster_with_no_entry_on_its_context_is_refused() {
        let before = setup(TEXT);
        for map in [
            // Not chosen, or left out.
            mapping(&[]),
            mapping(&[("core-fra", Choice::LeftOut)]),
        ] {
            assert_eq!(without_key(&before, URL, &map), Err(NotCleared::NoEntry));
        }
        // A context none of your entries is on.
        let none = setup(&TEXT.replace("context: ctx-a", "context: ctx-c"));
        let map = mapping(&[("core-fra", on("ctx-a"))]);
        assert_eq!(without_key(&none, URL, &map), Err(NotCleared::NoEntry));
        // An entry there that holds no such value: nothing to write.
        let map = mapping(&[("core-fra", on("ctx-b"))]);
        let bare = setup(&TEXT.replace(
            "    metrics:\n      source:\n        url: https://metrics.other.example\n",
            "",
        ));
        assert_eq!(without_key(&bare, URL, &map), Err(NotCleared::Absent));
    }

    #[test]
    fn a_level_is_never_taken_out_and_the_line_stays() {
        let before = setup(TEXT);
        let map = mapping(&[("core-fra", on("ctx-a")), ("cicd-fra", on("ctx-b"))]);
        for key in [
            "clusters[core-fra].protection",
            "clusters[cicd-fra].protection",
            "clusters[anything].protection",
        ] {
            assert!(is_level_key(key), "{key}");
            assert_eq!(
                without_key(&before, key, &map),
                Err(NotCleared::Level),
                "{key}"
            );
        }
        assert!(!is_level_key(URL));
        assert!(!is_level_key("coroot.project"));
        // The setup it was asked about is as it was.
        assert!(setup::to_text(&before).contains("protection: open"));
    }

    #[test]
    fn a_top_level_value_goes_and_its_block_stays() {
        let before = setup(TEXT);
        let one = without_key(&before, "coroot.project", &mapping(&[])).expect("removed");
        let text = setup::to_text(&one);
        assert!(
            text.contains("server:") && !text.contains("project:"),
            "{text}"
        );
    }

    #[test]
    fn what_is_not_a_key_is_refused() {
        let before = setup(TEXT);
        let map = mapping(&[("core-fra", on("ctx-a"))]);
        for key in [
            "clusters[core-fra]",
            "clusters[].metrics",
            "..",
            "",
            "coroot..server",
        ] {
            assert_eq!(
                without_key(&before, key, &map),
                Err(NotCleared::Refused),
                "{key}"
            );
        }
        assert_eq!(
            without_key(&before, "coroot.nothing", &map),
            Err(NotCleared::Absent)
        );
    }

    #[test]
    fn a_removal_that_would_break_the_setup_is_refused() {
        let before = setup(TEXT);
        let map = mapping(&[("core-fra", on("ctx-a"))]);
        // A cluster entry can't lose its role.
        assert_eq!(
            without_key(&before, "clusters[core-fra].role", &map),
            Err(NotCleared::Refused)
        );
    }
}
