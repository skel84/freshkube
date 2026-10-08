//! The state the shell holds for one cluster, and the registry that owns it.
//!
//! Today the registry holds a workspace of one: the session for the applied
//! context or Kubernetes-only kubeconfig. The shell reads and replaces it
//! through `Registry::active`, so a later change can hold several without
//! the pages knowing.
use std::sync::Arc;

use freshkube_core::{
    AccessIdentity, ConfigurationRevision, kubernetes_summary::KubernetesSummary,
};
use gpui_kit::Task;

use super::kubernetes_summary::SummarySession;
use crate::{backend::OwnedJob, screens::WorkloadData, state::Snapshot};

/// One cluster's Kubernetes observation: the compact summary the shell
/// derives its pages from, and the session and tasks that keep it current.
/// Selecting a Talos node never replaces any of it.
pub(super) struct ClusterSession {
    pub(super) kubernetes_summary: Snapshot<Arc<KubernetesSummary>>,
    pub(super) summary_health: Option<Result<Arc<WorkloadData>, String>>,
    pub(super) summary_session: Option<SummarySession>,
    pub(super) summary_job: Option<OwnedJob>,
    pub(super) summary_task: Option<Task<()>>,
    /// The access this session reads with, once an overview has collected it.
    pub(super) access: Option<AccessIdentity>,
    /// The inspected local configuration it was collected under.
    pub(super) access_configuration: Option<ConfigurationRevision>,
    /// The replacement a running shell last held up, so it is asked once.
    pub(super) prompted_access: Option<(ConfigurationRevision, Option<AccessIdentity>)>,
}

impl ClusterSession {
    fn new() -> Self {
        Self {
            kubernetes_summary: Snapshot::default(),
            summary_health: None,
            summary_session: None,
            summary_job: None,
            summary_task: None,
            access: None,
            access_configuration: None,
            prompted_access: None,
        }
    }
}

/// The sessions the shell holds. One for now.
pub(super) struct Registry {
    active: ClusterSession,
    /// Advances each time a summary session stops or the session is
    /// replaced, so a late answer from an older one is recognised. It lives
    /// here, not in a session, so a session made again never repeats a value.
    summary_epoch: u64,
}

impl Registry {
    pub(super) fn new() -> Self {
        Self {
            active: ClusterSession::new(),
            summary_epoch: 0,
        }
    }

    pub(super) fn summary_epoch(&self) -> u64 {
        self.summary_epoch
    }

    /// Ends the current summary generation: nothing from an older one applies.
    pub(super) fn advance_summary_epoch(&mut self) {
        self.summary_epoch = self.summary_epoch.wrapping_add(1);
    }

    /// Replaces the active session as a whole, so nothing a later field adds
    /// can carry over to the next context. Its tasks are dropped with it.
    pub(super) fn reset_active(&mut self) {
        self.active = ClusterSession::new();
        self.advance_summary_epoch();
    }

    pub(super) fn active(&self) -> &ClusterSession {
        &self.active
    }

    pub(super) fn active_mut(&mut self) -> &mut ClusterSession {
        &mut self.active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_the_session_drops_all_of_it_and_never_repeats_an_epoch() {
        let mut registry = Registry::new();
        let first = registry.summary_epoch();
        registry.active_mut().summary_health = Some(Err("down".into()));
        registry.active_mut().prompted_access = None;
        registry.reset_active();
        assert!(registry.active().summary_health.is_none());
        assert!(registry.active().access.is_none());
        let second = registry.summary_epoch();
        registry.advance_summary_epoch();
        registry.reset_active();
        let third = registry.summary_epoch();
        assert!(first != second && second != third && first != third);
    }
}
