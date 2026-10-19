//! Offline example data for `GpuiOptions::fixture()`. The example cluster
//! lives in core's `example::cluster`; this keeps its old path, and the tests
//! that read it through `presentation`.

pub(crate) use freshkube_core::cluster_source::Hold;
pub(crate) use freshkube_core::example::cluster::{CONTEXTS, cluster, logs, services, stream_line};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::{Health, cluster_summary, node_summaries, service_health};

    #[test]
    fn example_contexts_cover_healthy_degraded_and_unreachable_states() {
        let prod = cluster("prod-fra", 0);
        let nodes = node_summaries(&prod);
        assert_eq!(nodes.len(), 6);
        assert_eq!(nodes.iter().filter(|node| !node.responding).count(), 1);
        let summary = cluster_summary(&prod, &nodes);
        assert_eq!(summary.services.unhealthy, 1);
        assert_eq!(
            summary.first_unhealthy,
            Some(("talos-wk-fra1-02".into(), "kubelet".into()))
        );
        assert!(summary.peak_memory.unwrap().1 > 85.);
        let staging = cluster("staging-eu", 0);
        assert!(staging.discovery_warning.is_some());
        let staging_nodes = node_summaries(&staging);
        assert_eq!(cluster_summary(&staging, &staging_nodes).versions.len(), 2);
        assert!(services("prod-fra", "talos-wk-fra1-03").is_empty());
        assert!(
            services("prod-fra", "talos-cp-fra1-01")
                .iter()
                .all(|service| service_health(service) != Health::Unhealthy)
        );
    }
}
