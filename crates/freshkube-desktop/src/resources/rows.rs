//! What a table row shows beyond its printed cells, derived where the row
//! arrives, off the UI thread: who manages the object, where its name's
//! generated suffix starts, and for a pod, its state, readiness, restarts,
//! node and resources.

use std::collections::BTreeMap;
use std::sync::Arc;

use freshkube_core::resources::{Amounts, ContainerFacts, PodFacts};

use super::model::{ResourceColumn, ResourceRow, StatusTone, status_tone};

/// The object that manages a row's, as the Owner column shows it:
/// `deploy/` and the Deployment's name for a pod of one of its ReplicaSets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RowOwner {
    /// The column's short kind, such as `deploy` or `sts`.
    pub(crate) short: String,
    /// The kind in full, for its tooltip.
    pub(crate) kind: String,
    pub(crate) name: String,
}

impl RowOwner {
    /// `deploy/worker`, for sorting and accessible labels.
    pub(crate) fn label(&self) -> String {
        format!("{}/{}", self.short, self.name)
    }
}

/// The owner a row shows for its controller reference. A ReplicaSet named
/// after its Deployment and the `pod-template-hash` label the Deployment
/// sets on it and its pods is shown as that Deployment.
pub(crate) fn owner(
    controller: Option<(&str, &str)>,
    labels: &BTreeMap<String, String>,
) -> Option<RowOwner> {
    let (kind, name) = controller?;
    let deployment = (kind == "ReplicaSet")
        .then(|| labels.get("pod-template-hash"))
        .flatten()
        .and_then(|hash| name.strip_suffix(hash.as_str()))
        .and_then(|rest| rest.strip_suffix('-'))
        .filter(|deployment| !deployment.is_empty());
    let (kind, name) = match deployment {
        Some(deployment) => ("Deployment", deployment),
        None => (kind, name),
    };
    let short = match kind {
        "Deployment" => "deploy".to_owned(),
        "ReplicaSet" => "rs".to_owned(),
        "StatefulSet" => "sts".to_owned(),
        "DaemonSet" => "ds".to_owned(),
        "ReplicationController" => "rc".to_owned(),
        "CronJob" => "cj".to_owned(),
        other => other.to_lowercase(),
    };
    Some(RowOwner {
        short,
        kind: kind.to_owned(),
        name: name.to_owned(),
    })
}

/// Where the part of `name` its owner generated starts: `-6c4f8d-bbbbh`
/// after a Deployment's name. A StatefulSet's ordinal and a node's name in
/// a static pod's are meaningful, so they aren't dimmed.
pub(crate) fn generated_suffix(name: &str, owner: Option<&RowOwner>) -> Option<usize> {
    let owner = owner.filter(|owner| !matches!(owner.kind.as_str(), "StatefulSet" | "Node"))?;
    name.strip_prefix(owner.name.as_str())
        .filter(|rest| rest.len() > 1 && rest.starts_with('-'))
        .map(|_| owner.name.len())
}

/// How a pod is doing, from its printed status and readiness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PodState {
    /// Crashing, failing to pull, evicted or errored.
    Failing,
    /// Running, but not every container is ready.
    NotReady,
    /// Scheduled or starting: Pending, ContainerCreating, Init.
    Pending,
    Terminating,
    Running,
    Completed,
    /// A status this table doesn't know the meaning of.
    Unknown,
}

/// Whether a printed status says a container ran and stopped:
/// `CrashLoopBackOff`, `Error` or `OOMKilled`, in an init container too.
/// A pod that couldn't start, such as `ImagePullBackOff` or an
/// unschedulable one, has nothing that died.
pub(crate) fn died(status: &str) -> bool {
    let status = status.strip_prefix("Init:").unwrap_or(status);
    let reason = status.split([' ', '(']).next().unwrap_or_default();
    matches!(reason, "CrashLoopBackOff" | "Error" | "OOMKilled")
}

/// A pod's row, beyond the printed cells.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PodRow {
    pub(crate) state: PodState,
    /// The printed status, as `kubectl get pods` shows it.
    pub(crate) status: String,
    /// Shown after the name while the pod isn't healthy:
    /// `CrashLoopBackOff · exit 1`. Empty for a healthy pod.
    pub(crate) reason: String,
    /// Ready containers of all, as printed: `0/1`.
    pub(crate) ready: String,
    pub(crate) restarts: u32,
    pub(crate) node: String,
    pub(crate) requests: Amounts,
    pub(crate) limits: Amounts,
    pub(crate) containers: Vec<ContainerFacts>,
}

/// A pod's row from its printed cells and, when the table included the
/// whole pod, its facts. Columns are found by name, as the server and the
/// example data print them.
pub(crate) fn pod_row(
    columns: &[ResourceColumn],
    cells: &[String],
    facts: Option<PodFacts>,
    terminating: bool,
) -> PodRow {
    let cell = |name: &str| {
        columns
            .iter()
            .position(|column| column.name.eq_ignore_ascii_case(name))
            .and_then(|ix| cells.get(ix))
            .map(String::as_str)
            .unwrap_or("")
    };
    let facts = facts.unwrap_or_default();
    let status = match cell("Status") {
        "" if terminating => "Terminating".to_owned(),
        "" => facts.phase.clone(),
        printed => printed.to_owned(),
    };
    let ready = cell("Ready").to_owned();
    let restarts = cell("Restarts")
        .split_whitespace()
        .next()
        .and_then(|count| count.parse().ok())
        .unwrap_or_else(|| facts.containers.iter().map(|c| c.restarts).sum());
    let node = match cell("Node") {
        "" | "<none>" => facts.node.clone(),
        printed => printed.to_owned(),
    };
    let state = state(&status, &ready);
    let exit = facts
        .containers
        .iter()
        .filter(|container| !container.ready)
        .find_map(|container| container.last_exit_code.filter(|code| *code != 0));
    let reason = match state {
        PodState::Running | PodState::Completed => String::new(),
        PodState::NotReady => format!("Running · {ready} ready"),
        PodState::Failing => match exit {
            Some(code) if !status.contains("exit") => format!("{status} · exit {code}"),
            _ => status.clone(),
        },
        _ => status.clone(),
    };
    PodRow {
        state,
        status,
        reason,
        ready,
        restarts,
        node,
        requests: facts.requests,
        limits: facts.limits,
        containers: facts.containers,
    }
}

fn state(status: &str, ready: &str) -> PodState {
    if status == "Terminating" {
        return PodState::Terminating;
    }
    match (status, status_tone(status)) {
        ("Completed" | "Succeeded", _) => PodState::Completed,
        ("Running", _) => match ready.split_once('/') {
            Some((up, all)) if up != all => PodState::NotReady,
            _ => PodState::Running,
        },
        (_, StatusTone::Danger) => PodState::Failing,
        ("Unknown", _) => PodState::Unknown,
        (_, StatusTone::Warning) => PodState::Pending,
        _ => PodState::Unknown,
    }
}

/// Adds what a row shows beyond its cells: the owner and its generated
/// suffix for any kind, and for a pod its row.
pub(crate) fn derive(
    row: &mut ResourceRow,
    columns: &[ResourceColumn],
    controller: Option<(&str, &str)>,
    labels: &BTreeMap<String, String>,
    facts: Option<PodFacts>,
) {
    row.owner = owner(controller, labels);
    row.generated = generated_suffix(&row.identity.name, row.owner.as_ref());
    if row.identity.resource == "pods" {
        row.pod = Some(Arc::new(pod_row(
            columns,
            &row.cells,
            facts,
            row.terminating,
        )));
    }
}

/// The longest prefix every name shares up to a dash, such as `talos-`,
/// which a column of node names can leave out. Two names at least.
pub(crate) fn shared_prefix<'a>(names: impl IntoIterator<Item = &'a str>) -> usize {
    let mut names = names.into_iter().filter(|name| !name.is_empty());
    let Some(first) = names.next() else {
        return 0;
    };
    let mut common = first.len();
    let mut others = 0;
    for name in names {
        if name == first {
            continue;
        }
        others += 1;
        common = first
            .bytes()
            .zip(name.bytes())
            .take(common)
            .take_while(|(a, b)| a == b)
            .count();
    }
    if others == 0 {
        return 0;
    }
    // Up to and including the last dash of the shared part, and never a
    // whole name.
    first.as_bytes()[..common]
        .iter()
        .rposition(|byte| *byte == b'-')
        .map(|dash| dash + 1)
        .filter(|end| *end < first.len())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::model::ColumnKind;

    fn labels(hash: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("pod-template-hash".to_owned(), hash.to_owned())])
    }

    #[test]
    fn a_replicaset_named_by_its_template_hash_shows_its_deployment() {
        let owner = owner(Some(("ReplicaSet", "worker-6c4f8da0")), &labels("6c4f8da0")).unwrap();
        assert_eq!(owner.label(), "deploy/worker");
        assert_eq!(owner.kind, "Deployment");
        let name = "worker-6c4f8da0-bbbbh";
        assert_eq!(generated_suffix(name, Some(&owner)), Some(6));
        // Without the label, it stays a ReplicaSet.
        let bare = super::owner(Some(("ReplicaSet", "worker-6c4f8da0")), &BTreeMap::new());
        assert_eq!(bare.unwrap().label(), "rs/worker-6c4f8da0");
        let sts = super::owner(Some(("StatefulSet", "ledger-db")), &BTreeMap::new()).unwrap();
        assert_eq!(sts.label(), "sts/ledger-db");
        assert_eq!(generated_suffix("ledger-db-1", Some(&sts)), None);
        assert!(super::owner(None, &BTreeMap::new()).is_none());
    }

    fn columns() -> Vec<ResourceColumn> {
        ["Name", "Ready", "Status", "Restarts", "Age", "IP", "Node"]
            .iter()
            .map(|name| ResourceColumn::new(name, ColumnKind::Text, false))
            .collect()
    }

    fn cells(ready: &str, status: &str, restarts: &str) -> Vec<String> {
        ["p", ready, status, restarts, "", "10.0.0.1", "wk-1"]
            .map(str::to_owned)
            .to_vec()
    }

    #[test]
    fn pod_rows_read_state_reason_and_restarts() {
        let facts = PodFacts {
            containers: vec![ContainerFacts {
                last_exit_code: Some(1),
                ..ContainerFacts::default()
            }],
            ..PodFacts::default()
        };
        let crashing = pod_row(
            &columns(),
            &cells("0/1", "CrashLoopBackOff", "14 (3m ago)"),
            Some(facts),
            false,
        );
        assert_eq!(crashing.state, PodState::Failing);
        assert_eq!(crashing.reason, "CrashLoopBackOff · exit 1");
        assert_eq!(crashing.restarts, 14);
        assert_eq!(crashing.node, "wk-1");
        let running = pod_row(&columns(), &cells("1/1", "Running", "0"), None, false);
        assert_eq!(
            (running.state, running.reason.as_str()),
            (PodState::Running, "")
        );
        let unready = pod_row(&columns(), &cells("1/2", "Running", "0"), None, false);
        assert_eq!(unready.state, PodState::NotReady);
        assert_eq!(unready.reason, "Running · 1/2 ready");
        for (status, state) in [
            ("ContainerCreating", PodState::Pending),
            ("Init:0/2", PodState::Pending),
            ("Completed", PodState::Completed),
            ("Terminating", PodState::Terminating),
            ("ImagePullBackOff", PodState::Failing),
            ("Unknown", PodState::Unknown),
            ("NodeAffinity", PodState::Unknown),
        ] {
            let row = pod_row(&columns(), &cells("0/1", status, "0"), None, false);
            assert_eq!(row.state, state, "{status}");
        }
    }

    #[test]
    fn only_a_container_that_ran_and_stopped_died() {
        for status in [
            "CrashLoopBackOff",
            "Init:CrashLoopBackOff",
            "Error",
            "Error (exit 1)",
            "Init:Error",
            "OOMKilled",
        ] {
            assert!(died(status), "{status}");
        }
        for status in [
            "ImagePullBackOff",
            "ErrImagePull",
            "Init:ImagePullBackOff",
            "CreateContainerConfigError",
            "Pending",
            "Evicted",
            "Running",
            "Completed",
            "",
        ] {
            assert!(!died(status), "{status}");
        }
    }

    #[test]
    fn node_names_drop_only_a_prefix_they_all_share() {
        let nodes = ["talos-cp-fra1-01", "talos-wk-fra1-02", "talos-wk-fra1-03"];
        assert_eq!(shared_prefix(nodes), "talos-".len());
        assert_eq!(
            shared_prefix(["talos-wk-1", "talos-wk-2"]),
            "talos-wk-".len()
        );
        // One node, or no dash in common, keeps the names whole.
        assert_eq!(shared_prefix(["talos-home", "talos-home"]), 0);
        assert_eq!(shared_prefix(["alpha", "beta"]), 0);
        assert_eq!(shared_prefix(["a-b", "a-b-c"]), 2);
        assert_eq!(shared_prefix(["équipe-é1", "équipe-è2"]), "équipe-".len());
    }
}
