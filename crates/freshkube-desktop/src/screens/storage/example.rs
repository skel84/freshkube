//! Storage example adapter for `--fixture`.
use freshkube_core::example::{ExampleNode, storage};

#[cfg(test)]
pub(super) use freshkube_core::example::disk;

use super::{ScreenSource, StorageData};

/// Example disks and volumes for the selected, responding node.
pub(super) fn example(source: &ScreenSource) -> Result<StorageData, String> {
    let Some(node) = source.node() else {
        return Err("Example data has no such node".into());
    };
    if !node.responding {
        return Err(format!(
            "{} didn't answer the Talos API within 10 s (example)",
            node.name
        ));
    }
    let node = ExampleNode::new(
        node.name.clone(),
        node.role == crate::presentation::Role::ControlPlane,
        node.cores,
    )
    .with_storage(node.role == crate::presentation::Role::Worker);
    let (disks, volumes) = storage(&node);
    Ok(StorageData::new(Ok(disks), volumes))
}
