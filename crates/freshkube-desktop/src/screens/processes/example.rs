//! Processes example adapter for `--fixture`.
use freshkube_core::example::{ExampleNode, processes};

use super::{ProcessInspectionSnapshot, ScreenSource};

/// Example processes for the selected, responding node.
pub(super) fn example(
    source: &ScreenSource,
    tick: u64,
) -> Result<ProcessInspectionSnapshot, String> {
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
    );
    Ok(processes(&node, source.inspection_target(), tick))
}
