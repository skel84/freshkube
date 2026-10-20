//! Offline example data for `GpuiOptions::fixture()`. The example cluster
//! lives in core's `example::cluster`; this owns its answers and clock.

pub(crate) use freshkube_core::cluster_source::Hold;
pub(crate) use freshkube_core::example::cluster::{CONTEXTS, cluster, logs, services, stream_line};

use crate::presentation::{self, LoadHistory};
use freshkube_core::cluster_overview::ClusterOverview;

/// The offline source's clock, hold and next overview answer.
pub(crate) struct Source {
    pub(crate) hold: Hold,
    pub(crate) tick: u64,
    fail_next: bool,
}

impl Source {
    pub(crate) fn new(hold: Hold) -> Self {
        Self {
            hold,
            tick: 0,
            fail_next: false,
        }
    }

    /// The example control replaces even a held answer with one failure.
    pub(crate) fn fail_next_overview(&mut self) {
        self.fail_next = true;
    }

    pub(crate) fn has_pending_failure(&self) -> bool {
        self.fail_next
    }

    /// `None` keeps the read in flight; a failure is consumed only once.
    pub(crate) fn overview(&mut self, context: &str) -> Option<Result<ClusterOverview, String>> {
        if std::mem::take(&mut self.fail_next) {
            return Some(Err(
                "Example: the Talos API didn't answer within 10 s".into()
            ));
        }
        if self.hold.talos {
            return None;
        }
        self.tick += 1;
        Some(Ok(cluster(context, self.tick)))
    }

    /// Five minutes of readings precede the first overview of each context,
    /// so the sparklines show a trend immediately, even while Talos is held.
    pub(crate) fn seed_history(&mut self, context: &str, history: &mut LoadHistory) {
        for _ in 1..presentation::LOAD_HISTORY_LEN {
            self.tick += 1;
            history.record(&presentation::node_summaries(&cluster(context, self.tick)));
        }
    }
}

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

    #[test]
    fn a_held_overview_can_fail_once_and_then_waits_again() {
        let mut source = Source::new(Hold::parse("talos"));
        assert!(source.overview("prod-fra").is_none());
        source.fail_next_overview();
        assert!(source.has_pending_failure());
        let error = source.overview("prod-fra").unwrap().err().unwrap();
        assert_eq!(error, "Example: the Talos API didn't answer within 10 s");
        assert!(!source.has_pending_failure());
        assert!(source.overview("prod-fra").is_none());
        assert_eq!(source.tick, 0, "waiting and failures add no readings");
        source.hold = Hold::NONE;
        assert!(source.overview("prod-fra").unwrap().is_ok());
        assert_eq!(source.tick, 1);
    }

    #[test]
    fn history_and_overviews_share_the_example_clock() {
        let mut source = Source::new(Hold::NONE);
        let mut history = LoadHistory::default();
        for context in ["prod-fra", "staging-eu"] {
            let first = source.tick + 1;
            source.seed_history(context, &mut history);
            let overview = source.overview(context).unwrap().unwrap();
            let nodes = presentation::node_summaries(&overview);
            history.record(&nodes);
            for node in nodes.iter().filter(|node| node.load.is_some()) {
                let expected: Vec<_> = (first..=source.tick)
                    .map(|tick| {
                        presentation::node_summaries(&cluster(context, tick))
                            .into_iter()
                            .find(|sample| sample.name == node.name)
                            .unwrap()
                            .load
                            .unwrap()[0]
                    })
                    .collect();
                assert_eq!(history.get(&node.name), expected);
                assert_eq!(expected.len(), presentation::LOAD_HISTORY_LEN);
            }
        }
    }
}
