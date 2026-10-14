//! Network example adapters for `--fixture`.
use freshkube_core::example::{ExampleNode, kubespan, network};

use super::{KUBESPAN_PORT, KubeSpanState, NetworkData, ScreenSource};

/// Example network data for the selected, responding node.
pub(super) fn example(source: &ScreenSource, tick: u64) -> Result<NetworkData, String> {
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
    )
    .with_network(node.address.clone(), node.responding);
    let nodes = example_nodes(source);
    let snapshot = network(
        &node,
        source.inspection_target(),
        &nodes,
        tick,
        std::time::Instant::now(),
    );
    Ok(NetworkData::new(snapshot, tick))
}

/// Example KubeSpan peers for the selected node.
pub(super) fn example_kubespan(source: &ScreenSource, tick: u64) -> KubeSpanState {
    let Some(node) = source.node() else {
        return KubeSpanState::Unavailable("Example data has no such node".into());
    };
    // The unavailable example does not read a clock; core ignores its timestamp.
    let now = if node.name.contains("wk-fra1-02") {
        chrono::DateTime::UNIX_EPOCH
    } else {
        chrono::Utc::now()
    };
    let node = ExampleNode::new(
        node.name.clone(),
        node.role == crate::presentation::Role::ControlPlane,
        None,
    )
    .with_network(node.address.clone(), node.responding);
    match kubespan(&node, &example_nodes(source), tick, now, KUBESPAN_PORT) {
        Ok(peers) => KubeSpanState::Enabled(peers.into()),
        Err(message) => KubeSpanState::Unavailable(message),
    }
}

fn example_nodes(source: &ScreenSource) -> Vec<ExampleNode> {
    source
        .nodes
        .iter()
        .map(|node| {
            ExampleNode::new(
                node.name.clone(),
                node.role == crate::presentation::Role::ControlPlane,
                None,
            )
            .with_network(node.address.clone(), node.responding)
        })
        .collect()
}
