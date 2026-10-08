//! The state the shell holds for one cluster, and the registry that owns it.
//!
//! Today the registry holds a workspace of one: the session for the applied
//! context or Kubernetes-only kubeconfig. The shell reads and replaces it
//! through `Registry::active`, so a later change can hold several without
//! the pages knowing.
use std::sync::Arc;

use freshkube_core::kubernetes_summary::KubernetesSummary;
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
    /// Advances each time the session is stopped, so a late answer from an
    /// older one is recognised.
    pub(super) summary_epoch: u64,
    pub(super) summary_job: Option<OwnedJob>,
    pub(super) summary_task: Option<Task<()>>,
}

impl ClusterSession {
    fn new() -> Self {
        Self {
            kubernetes_summary: Snapshot::default(),
            summary_health: None,
            summary_session: None,
            summary_epoch: 0,
            summary_job: None,
            summary_task: None,
        }
    }
}

/// The sessions the shell holds. One for now.
pub(super) struct Registry {
    active: ClusterSession,
}

impl Registry {
    pub(super) fn new() -> Self {
        Self {
            active: ClusterSession::new(),
        }
    }

    pub(super) fn active(&self) -> &ClusterSession {
        &self.active
    }

    pub(super) fn active_mut(&mut self) -> &mut ClusterSession {
        &mut self.active
    }
}
