//! Opening a part in Resources: which connection each cluster of a read
//! is, and which kind a part is there.
//!
//! A part's link is built as every cluster-qualified object link is: its
//! `ObjectRef` names the connection it was read through, and the shell
//! opens it through `Pilot::open_object`, which refuses a cluster that
//! isn't open. A live read's cluster is its connection. Example data maps
//! acme's `core-fra` to the example cluster that is open, whose Resources
//! list its objects; acme's other clusters are no connection, so their
//! links name their bare key and are refused.
//!
//! A cluster that isn't open but that the workspace file lists by name is
//! one switch away: its part's link goes to the shell with that entry, which
//! asks before switching to it and then opens the part there.
use std::collections::{BTreeMap, BTreeSet};

use freshkube_core::applications::{MemberKind, MemberRef, SessionKey};
use freshkube_core::delivery::{argocd, kargo};
use freshkube_core::resources::ResourceKind;
use freshkube_core::workloads::WorkloadKind;

use crate::resources::{ResourceLink, Tab, model::ObjectRef};

/// The version core reads Argo CD and Kargo at.
const VERSION: &str = "v1alpha1";

/// The connection each cluster of a read is, and the one open when it was
/// read.
#[derive(Clone, Debug, Default)]
pub(crate) struct Connections {
    open: String,
    ids: BTreeMap<SessionKey, String>,
    /// The workspace's entries other than the open one, by id.
    entries: BTreeSet<String>,
}

impl Connections {
    pub(crate) fn new(
        open: impl Into<String>,
        ids: impl IntoIterator<Item = (SessionKey, String)>,
    ) -> Self {
        Self {
            open: open.into(),
            ids: ids.into_iter().collect(),
            entries: BTreeSet::new(),
        }
    }

    /// The same, knowing the workspace's entries other than the open one.
    pub(crate) fn with_entries(mut self, entries: impl IntoIterator<Item = String>) -> Self {
        self.entries = entries.into_iter().collect();
        self
    }

    /// The connection a cluster is: its own, else its key, which no
    /// connection is.
    pub(crate) fn of(&self, session: &SessionKey) -> String {
        self.ids
            .get(session)
            .cloned()
            .unwrap_or_else(|| session.0.clone())
    }

    /// Whether the cluster is the one open, so its parts open in Resources.
    pub(crate) fn opens(&self, session: &SessionKey) -> bool {
        self.of(session) == self.open
    }

    /// The workspace entry a cluster that isn't open is, by its name: its
    /// parts open there once the window switches to it. A read's cluster
    /// key matches an entry's id only in example data, whose acme clusters
    /// are the fixture workspace's entries; a live read is the open
    /// connection alone until reads span clusters.
    pub(crate) fn switch_to(&self, session: &SessionKey) -> Option<&str> {
        if self.opens(session) {
            return None;
        }
        self.entries.get(&session.0).map(String::as_str)
    }
}

/// The kind a part is in Resources.
pub(crate) fn kind(kind: MemberKind) -> ResourceKind {
    match kind {
        MemberKind::KargoStage => ResourceKind::new(kargo::GROUP, VERSION, "Stage", "stages", true),
        MemberKind::KargoWarehouse => {
            ResourceKind::new(kargo::GROUP, VERSION, "Warehouse", "warehouses", true)
        }
        MemberKind::ArgoApplication => {
            ResourceKind::new(argocd::GROUP, VERSION, "Application", "applications", true)
        }
        MemberKind::Workload(WorkloadKind::Deployment) => {
            ResourceKind::new("apps", "v1", "Deployment", "deployments", true)
        }
        MemberKind::Workload(WorkloadKind::StatefulSet) => {
            ResourceKind::new("apps", "v1", "StatefulSet", "statefulsets", true)
        }
        MemberKind::Workload(WorkloadKind::DaemonSet) => {
            ResourceKind::new("apps", "v1", "DaemonSet", "daemonsets", true)
        }
    }
}

/// The link that opens a part on its Overview, in the cluster it was read
/// in.
pub(crate) fn link(member: &MemberRef, connections: &Connections) -> ResourceLink {
    ResourceLink::Object(
        kind(member.kind),
        ObjectRef {
            namespace: member.namespace.clone().unwrap_or_default(),
            name: member.name.clone(),
            uid: String::new(),
            connection: Some(connections.of(&member.session)),
        },
        Tab::Overview,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cluster_without_a_connection_names_its_key_and_never_opens() {
        let core = SessionKey::new("core-fra");
        let prod = SessionKey::new("prod-fra");
        let connections = Connections::new(
            "example:prod-fra",
            [(core.clone(), "example:prod-fra".into())],
        );
        assert!(connections.opens(&core));
        assert_eq!(connections.of(&prod), "prod-fra");
        assert!(!connections.opens(&prod));
        assert_eq!(connections.switch_to(&prod), None);
    }

    #[test]
    fn a_cluster_the_workspace_lists_is_a_switch_away_and_the_open_one_is_not() {
        let core = SessionKey::new("core-fra");
        let dev = SessionKey::new("dev-fra");
        let lon = SessionKey::new("prod-lon");
        let connections = Connections::new(
            "example:prod-fra",
            [(core.clone(), "example:prod-fra".into())],
        )
        .with_entries(["core-fra".to_owned(), "dev-fra".to_owned()]);
        assert_eq!(connections.switch_to(&core), None, "it is open");
        assert_eq!(connections.switch_to(&dev), Some("dev-fra"));
        assert_eq!(connections.switch_to(&lon), None, "not in the workspace");
    }

    #[test]
    fn each_part_kind_opens_the_kind_core_reads() {
        for (member, key) in [
            (MemberKind::KargoStage, "stages.kargo.akuity.io"),
            (MemberKind::KargoWarehouse, "warehouses.kargo.akuity.io"),
            (MemberKind::ArgoApplication, "applications.argoproj.io"),
            (
                MemberKind::Workload(WorkloadKind::Deployment),
                "deployments.apps",
            ),
            (
                MemberKind::Workload(WorkloadKind::StatefulSet),
                "statefulsets.apps",
            ),
            (
                MemberKind::Workload(WorkloadKind::DaemonSet),
                "daemonsets.apps",
            ),
        ] {
            assert_eq!(kind(member).key(), key);
        }
        // The workload kinds are Resources' own, so the rail finds them.
        for workload in [
            WorkloadKind::Deployment,
            WorkloadKind::StatefulSet,
            WorkloadKind::DaemonSet,
        ] {
            let kind = kind(MemberKind::Workload(workload));
            assert_eq!(
                Some(kind.clone()),
                freshkube_core::resources::builtin(&kind.key())
            );
        }
    }
}
