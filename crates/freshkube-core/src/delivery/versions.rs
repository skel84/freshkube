//! Which version of a kind a cluster serves, from its discovery documents.

use super::read::{Reader, Resource};
use crate::resources::{Failure, FailureKind};

/// The first of `preferred` that the cluster serves for `group`; failing
/// that, the group's own preferred version if it serves `plural`. A group the
/// cluster doesn't serve is a `NotFound` failure, which the caller reads as
/// "not installed", not as an empty list.
pub async fn resolve<R: Reader>(
    reader: &R,
    group: &str,
    plural: &str,
    preferred: &[&str],
    namespaced: bool,
) -> Result<Resource, Failure> {
    let [resource] = resolve_each(reader, group, [plural], preferred, namespaced).await?;
    resource
}

/// Each of `plurals` in one `group`, as [`resolve`] finds it, from one read
/// of `/apis` and at most one read of each version's document, so a read
/// that needs several kinds of a group discovers them once.
///
/// The outer failure is the group's: `/apis` could not be read, or the group
/// is not served. A kind that no version serves, or that was not yet found
/// when a version's document could not be read, fails on its own; the kinds
/// already found keep their versions.
pub async fn resolve_each<R: Reader, const N: usize>(
    reader: &R,
    group: &str,
    plurals: [&str; N],
    preferred: &[&str],
    namespaced: bool,
) -> Result<[Result<Resource, Failure>; N], Failure> {
    let groups = reader.groups().await?;
    let Some(served) = groups.iter().find(|served| served.name == group) else {
        return Err(Failure::new(
            FailureKind::NotFound,
            format!("API group {group} is not served"),
        ));
    };
    let mut candidates: Vec<&str> = Vec::new();
    for version in preferred
        .iter()
        .copied()
        .filter(|version| served.versions.iter().any(|v| v == version))
        .chain(served.versions.iter().map(String::as_str))
    {
        if !candidates.contains(&version) {
            candidates.push(version);
        }
    }
    let mut found: [Option<Result<Resource, Failure>>; N] = std::array::from_fn(|_| None);
    for version in candidates {
        if found.iter().all(Option::is_some) {
            break;
        }
        match reader.plurals(group, version).await {
            Ok(served) => {
                for (slot, plural) in found.iter_mut().zip(plurals) {
                    if slot.is_none() && served.iter().any(|served| served == plural) {
                        *slot = Some(Ok(Resource::new(group, version, plural, namespaced)));
                    }
                }
            }
            Err(failure) => {
                for slot in found.iter_mut().filter(|slot| slot.is_none()) {
                    *slot = Some(Err(failure.clone()));
                }
            }
        }
    }
    let mut found = found.into_iter();
    Ok(plurals.map(|plural| {
        found.next().flatten().unwrap_or_else(|| {
            Err(Failure::new(
                FailureKind::NotFound,
                format!("{plural}.{group} is not served at any version"),
            ))
        })
    }))
}
