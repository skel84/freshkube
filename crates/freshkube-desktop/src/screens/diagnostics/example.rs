//! Diagnostics example adapter for `--fixture`.
use freshkube_core::diagnostic_runner::{
    LoadAverageSnapshot, MemorySnapshot, ServiceHealthSnapshot, ServiceSnapshot,
};
use freshkube_core::example::{ExampleNode, diagnostics};

use super::{DiagnosticSnapshot, ScreenSource, SourceState};
use crate::presentation::Role as NodeRole;

/// Example diagnostics for the selected, responding node.
pub(super) fn example(source: &ScreenSource) -> Result<DiagnosticSnapshot, String> {
    let Some(node) = source.node() else {
        return Err("Example data has no such node".into());
    };
    if !node.responding {
        return Err(format!(
            "{} didn't answer the Talos API within 10 s (example)",
            node.name
        ));
    }
    let control_plane = node.role == NodeRole::ControlPlane;
    let kubernetes_ok = source.target.context != "homelab";
    // System.
    let memory = node.memory.as_ref().map(|memory| MemorySnapshot {
        total_bytes: memory.total,
        available_bytes: memory.total.saturating_sub(memory.used),
        used_bytes: memory.used,
        usage_percent: memory.percent() as f32,
    });
    let load = node.load.map(|load| LoadAverageSnapshot {
        one_minute: load[0],
        five_minutes: load[1],
        fifteen_minutes: load[2],
    });
    // Services.
    let services: Vec<ServiceSnapshot> = node
        .services
        .iter()
        .map(|service| ServiceSnapshot {
            node: node.name.clone(),
            id: service.id.clone(),
            state: service.state.clone(),
            health: match &service.health {
                Some(health) if !health.unknown => SourceState::Available(ServiceHealthSnapshot {
                    healthy: health.healthy,
                    last_message: health.last_message.clone(),
                }),
                other => SourceState::unavailable(
                    "Talos service health",
                    match other {
                        Some(health) => format!(
                            "Talos reports service health as unknown: {}",
                            health.last_message
                        ),
                        None => "Talos did not return a health state for this service".to_owned(),
                    },
                ),
            },
        })
        .collect();
    let node = ExampleNode::new(node.name.clone(), control_plane, node.cores).with_diagnostics(
        node.address.clone(),
        memory,
        load,
        services,
    );
    let control_plane_address = source
        .nodes
        .iter()
        .find(|node| node.role == NodeRole::ControlPlane)
        .map(|node| node.address.as_str())
        .unwrap_or_default();
    Ok(diagnostics(
        &node,
        &source.target.context,
        kubernetes_ok,
        control_plane_address,
    ))
}
