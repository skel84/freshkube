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
    let groups = reader.groups().await?;
    let Some(served) = groups.iter().find(|served| served.name == group) else {
        return Err(Failure::new(
            FailureKind::NotFound,
            format!("API group {group} is not served"),
        ));
    };
    let candidates = preferred
        .iter()
        .filter(|version| served.versions.iter().any(|v| v == *version))
        .map(|version| (*version).to_owned())
        .chain(served.versions.iter().cloned());
    for version in candidates {
        let plurals = reader.plurals(group, &version).await?;
        if plurals.iter().any(|served| served == plural) {
            return Ok(Resource::new(group, &version, plural, namespaced));
        }
    }
    Err(Failure::new(
        FailureKind::NotFound,
        format!("{plural}.{group} is not served at any version"),
    ))
}
