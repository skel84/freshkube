//! Security example adapter for `--fixture`.
use freshkube_core::example::{ExampleNode, security};

use super::{DateTime, ScreenSource, SecurityAuditSnapshot, Utc};

/// Example audit for the selected, responding node.
pub(super) fn example(
    source: &ScreenSource,
    now: DateTime<Utc>,
) -> Result<SecurityAuditSnapshot, String> {
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
        None,
    );
    let endpoints: Vec<String> = source
        .nodes
        .iter()
        .filter(|node| node.role == crate::presentation::Role::ControlPlane)
        .map(|node| node.address.clone())
        .collect();
    Ok(security(
        &node,
        &source.target.context,
        &source.target.address,
        endpoints,
        now,
    ))
}
