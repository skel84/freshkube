//! The state the shell holds for one cluster, and the registry that owns it.
//!
//! One session is active: the one every page reads, through
//! `Registry::active`. Switching to another workspace entry parks the active
//! one, which keeps nothing running: its jobs and tasks go with it, and only
//! the last summary it read is kept, with the time it was read, for the
//! entry's next activation to show as last known. A parked summary is put
//! back only for the same entry, defined the same way and read with the same
//! applied configuration; any other answer would be another cluster's.
use std::{sync::Arc, time::SystemTime};

use freshkube_core::{
    AccessIdentity, ConfigurationRevision, kubernetes_summary::KubernetesSummary,
};
use gpui_kit::Task;

use super::kubernetes_summary::SummarySession;
use crate::{
    backend::{AppliedConfig, OwnedJob},
    screens::WorkloadData,
    state::Snapshot,
};

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

/// Whose session this is: the workspace entry's stable id, or the cluster a
/// window without a workspace file opened. Never an `AccessIdentity::key()`,
/// which is process-local and changes with the access.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SessionKey {
    Implicit,
    Entry(String),
}

/// What a parked session keeps: its last summary.
struct Parked {
    key: SessionKey,
    /// How the entry was defined when the summary was read.
    definition: String,
    identity: AppliedConfig,
    summary: Arc<KubernetesSummary>,
    taken: SystemTime,
}

/// The sessions the shell holds: the active one, and the last summary of
/// each entry parked since, at most one per entry.
pub(super) struct Registry {
    active_key: SessionKey,
    active: ClusterSession,
    parked: Vec<Parked>,
    /// Advances each time a summary session stops or the session is
    /// replaced, so a late answer from an older one is recognised. It lives
    /// here, not in a session, so a session made again never repeats a value.
    summary_epoch: u64,
}

impl Registry {
    pub(super) fn new() -> Self {
        Self {
            active_key: SessionKey::Implicit,
            active: ClusterSession::new(),
            parked: Vec::new(),
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

    pub(super) fn active_key(&self) -> &SessionKey {
        &self.active_key
    }

    /// Parks the active session and makes `key` the active one, empty. The
    /// parked session keeps its summary only, as read under the `definition`
    /// of its entry and the `applied` configuration it was left with; its
    /// jobs and tasks stop here. Activating the active key again changes
    /// nothing.
    pub(super) fn activate(
        &mut self,
        key: SessionKey,
        left_definition: &str,
        left_applied: &AppliedConfig,
    ) {
        if self.active_key == key {
            return;
        }
        let leaving = std::mem::replace(&mut self.active_key, key);
        let summary = &self.active.kubernetes_summary;
        if let (Some(data), Some(taken)) = (summary.data(), summary.last_successful()) {
            self.parked.retain(|parked| parked.key != leaving);
            self.parked.push(Parked {
                key: leaving,
                definition: left_definition.to_owned(),
                identity: left_applied.clone(),
                summary: data.clone(),
                taken,
            });
        }
        self.reset_active();
    }

    /// Drops what is parked for entries that are no longer listed.
    pub(super) fn retain_parked(&mut self, listed: impl Fn(&SessionKey) -> bool) {
        self.parked.retain(|parked| listed(&parked.key));
    }

    /// Puts the active entry's parked summary back, stale and with the time
    /// it was read, when it was read for this definition and configuration.
    /// One that does not fit stays until the entry is parked again or is no
    /// longer listed. True when it was put back.
    pub(super) fn restore_parked(&mut self, definition: &str, applied: &AppliedConfig) -> bool {
        let Some(ix) = self
            .parked
            .iter()
            .position(|parked| parked.key == self.active_key)
        else {
            return false;
        };
        if self.active.kubernetes_summary.data().is_some() {
            return false;
        }
        let parked = &self.parked[ix];
        if parked.definition != definition || &parked.identity != applied {
            return false;
        }
        let parked = self.parked.remove(ix);
        self.active
            .kubernetes_summary
            .restore(applied.clone(), parked.summary, parked.taken);
        true
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

    fn applied(context: &str) -> AppliedConfig {
        AppliedConfig {
            path: None,
            context: Some(context.into()),
        }
    }

    fn key(id: &str) -> SessionKey {
        SessionKey::Entry(id.into())
    }

    /// A registry whose active session read a summary of its own.
    fn read(registry: &mut Registry, applied: &AppliedConfig) {
        let request = registry
            .active_mut()
            .kubernetes_summary
            .begin(applied.clone());
        registry.active_mut().kubernetes_summary.apply(
            &request,
            Ok(Arc::new(crate::resources::example::summary(
                applied.context.as_deref().unwrap_or("none"),
                1_790_000_000,
            ))),
        );
    }

    #[test]
    fn a_parked_summary_comes_back_stale_for_the_same_entry_only() {
        let mut registry = Registry::new();
        registry.activate(key("a"), "", &applied("none"));
        read(&mut registry, &applied("acme-a"));
        let taken = registry.active().kubernetes_summary.last_successful();

        registry.activate(key("b"), "a:v1", &applied("acme-a"));
        assert!(registry.active().kubernetes_summary.data().is_none());
        assert_eq!(registry.active_key(), &key("b"));
        // Another entry's summary is never put into this one.
        assert!(!registry.restore_parked("b:v1", &applied("acme-a")));
        assert!(registry.active().kubernetes_summary.data().is_none());

        registry.activate(key("a"), "b:v1", &applied("acme-b"));
        // Defined differently since, or read with another configuration:
        // not put back, and kept for the right one.
        assert!(!registry.restore_parked("a:v2", &applied("acme-a")));
        assert!(!registry.restore_parked("a:v1", &applied("acme-other")));
        assert!(registry.active().kubernetes_summary.data().is_none());
        assert!(registry.restore_parked("a:v1", &applied("acme-a")));
        let summary = &registry.active().kubernetes_summary;
        assert!(summary.data().is_some() && summary.is_stale());
        assert_eq!(summary.last_successful(), taken);
        // Put back once: the entry's next parking stores the next answer.
        assert!(!registry.restore_parked("a:v1", &applied("acme-a")));
    }

    #[test]
    fn parking_stops_the_session_and_keeps_one_summary_per_entry() {
        let mut registry = Registry::new();
        registry.activate(key("a"), "", &applied("none"));
        let epoch = registry.summary_epoch();
        registry.active_mut().summary_health = Some(Err("down".into()));
        read(&mut registry, &applied("acme-a"));
        registry.activate(key("b"), "a:v1", &applied("acme-a"));
        assert!(registry.summary_epoch() != epoch);
        assert!(registry.active().summary_health.is_none());
        assert!(registry.active().summary_job.is_none());

        // Switching to the active entry again does nothing, and parks nothing.
        let epoch = registry.summary_epoch();
        registry.activate(key("b"), "b:v1", &applied("acme-b"));
        assert_eq!(registry.summary_epoch(), epoch);

        // Parking a with a newer answer replaces the older one.
        registry.activate(key("a"), "b:v1", &applied("acme-b"));
        read(&mut registry, &applied("acme-a"));
        registry.activate(key("b"), "a:v1", &applied("acme-a"));
        registry.activate(key("a"), "b:v1", &applied("acme-b"));
        assert!(registry.restore_parked("a:v1", &applied("acme-a")));
        assert_eq!(registry.parked.len(), 0);
    }

    #[test]
    fn an_entry_that_is_no_longer_listed_loses_what_was_parked() {
        let mut registry = Registry::new();
        registry.activate(key("a"), "", &applied("none"));
        read(&mut registry, &applied("acme-a"));
        registry.activate(key("b"), "a:v1", &applied("acme-a"));
        registry.retain_parked(|key| key == &self::key("b"));
        registry.activate(key("a"), "b:v1", &applied("acme-b"));
        assert!(!registry.restore_parked("a:v1", &applied("acme-a")));
    }
}
