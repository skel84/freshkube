//! What a table row shows beyond its printed cells, derived where the row
//! arrives, off the UI thread: who manages the object, where its name's
//! generated suffix starts, and for a pod, its state, readiness, restarts,
//! containers' squares, node and resources.

use std::collections::BTreeMap;
use std::sync::Arc;

use freshkube_core::resources::{Amounts, ContainerFacts, PodFacts, RunState};
use freshkube_ui::squares::{Square, Squares};
use freshkube_ui::ui::Tone;
use gpui_kit::SharedString;

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
    /// `ready` read as numbers, when it parses, for sorting.
    pub(crate) ready_counts: Option<(u64, u64)>,
    /// Whether every container is ready, or the count doesn't parse.
    pub(crate) all_ready: bool,
    pub(crate) restarts: u32,
    /// The Restarts cell's text, its accessible label and its emphasis.
    pub(crate) restarts_text: String,
    pub(crate) restarts_label: String,
    pub(crate) restarts_emphasis: Emphasis,
    pub(crate) node: String,
    pub(crate) requests: Amounts,
    pub(crate) limits: Amounts,
    pub(crate) containers: Vec<ContainerFacts>,
    pub(crate) init_containers: Vec<ContainerFacts>,
    /// One per container, init containers first (`squares`).
    pub(crate) squares: Squares,
    /// The worst square's [`severity`], for sorting.
    pub(crate) worst: u8,
    /// The squares' tooltip, a line per container, and their accessible
    /// label, the same lines joined.
    pub(crate) containers_tip: SharedString,
    pub(crate) containers_label: SharedString,
}

/// Waiting reasons that won't clear without a change to the pod or its
/// image: the container can't start.
const STUCK: [&str; 11] = [
    "CrashLoopBackOff",
    "ImagePullBackOff",
    "ErrImagePull",
    "ErrImageNeverPull",
    "ImageInspectError",
    "InvalidImageName",
    "CreateContainerConfigError",
    "CreateContainerError",
    "RunContainerError",
    "PreStartHookError",
    "SignatureValidationFailed",
];

/// A container's square. Running and ready is good; running unready or
/// waiting to start, a warning; waiting that won't clear or a failed exit,
/// critical; a clean exit, grey. Restarts outline it, as does a container
/// with no state reported yet.
///
/// The pod's state can say a container's own state means nothing, so the
/// square agrees with the row's glyph rather than raise a false alarm: a
/// terminating pod's containers are being stopped, and all draw grey; a
/// starting pod's containers waiting or not yet ready draw grey outlined;
/// a finished pod's sidecars were stopped when the app ended, so their
/// exit, often 137 or 143, draws grey.
pub(crate) fn square(container: &ContainerFacts, init: bool, pod: PodState) -> Square {
    let dim = init && !container.sidecar;
    let own = own_square(container);
    let square = match pod {
        PodState::Terminating => Square::new(Tone::Unknown),
        PodState::Pending if own.tone == Tone::Warn => Square::new(Tone::Unknown).outlined(true),
        PodState::Completed
            if container.sidecar && matches!(container.state, RunState::Terminated { .. }) =>
        {
            Square::new(Tone::Unknown)
        }
        _ => own,
    };
    square.dim(dim)
}

/// A container's square from its own state alone.
fn own_square(container: &ContainerFacts) -> Square {
    let restarted = container.restarts > 0;
    let (tone, outlined) = match &container.state {
        RunState::Running if container.ready => (Tone::Good, restarted),
        RunState::Running => (Tone::Warn, restarted),
        RunState::Waiting => {
            let stuck = container
                .waiting
                .as_deref()
                .is_some_and(|reason| STUCK.contains(&reason));
            (if stuck { Tone::Crit } else { Tone::Warn }, restarted)
        }
        RunState::Terminated { exit_code, reason } => {
            let failed = *exit_code != 0 || reason.as_deref() == Some("OOMKilled");
            (
                if failed { Tone::Crit } else { Tone::Unknown },
                failed && restarted,
            )
        }
        RunState::Unknown => (Tone::Unknown, true),
    };
    Square::new(tone).outlined(outlined)
}

/// How bad a square is, worst first: critical, warning, no state, good,
/// then a clean exit.
pub(crate) fn severity(square: &Square) -> u8 {
    match square.tone {
        Tone::Crit | Tone::Died => 0,
        Tone::Warn => 1,
        Tone::Unknown if square.outlined => 2,
        Tone::Good => 3,
        _ => 4,
    }
}

/// A container's line in the squares' tooltip: its name, what it does
/// and why, and its restarts: `api (init): waiting · CrashLoopBackOff ·
/// 3 restarts`.
pub(crate) fn container_line(container: &ContainerFacts, init: bool) -> String {
    let mut line = container.name.clone();
    if container.sidecar {
        line.push_str(" (sidecar)");
    } else if init {
        line.push_str(" (init)");
    }
    line.push_str(": ");
    match &container.state {
        RunState::Running if container.ready => line.push_str("running"),
        RunState::Running => line.push_str("running, not ready"),
        RunState::Waiting => {
            line.push_str("waiting");
            if let Some(reason) = &container.waiting {
                line.push_str(&format!(" · {reason}"));
            }
        }
        RunState::Terminated { exit_code, reason } => {
            line.push_str(&format!("exited {exit_code}"));
            if let Some(reason) = reason {
                line.push_str(&format!(" · {reason}"));
            }
        }
        RunState::Unknown => line.push_str("no state reported"),
    }
    match container.restarts {
        0 => {}
        1 => line.push_str(" · 1 restart"),
        n => line.push_str(&format!(" · {n} restarts")),
    }
    line
}

/// How a count stands out: none muted, a few in ink, many in amber.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Emphasis {
    Muted,
    Plain,
    Warn,
}

/// Restarts' emphasis: none muted, five or more amber (#291).
pub(crate) fn restarts_emphasis(restarts: u32) -> Emphasis {
    match restarts {
        0 => Emphasis::Muted,
        1..5 => Emphasis::Plain,
        _ => Emphasis::Warn,
    }
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
    let ready_counts = ready
        .split_once('/')
        .and_then(|(up, all)| Some((up.trim().parse().ok()?, all.trim().parse().ok()?)));
    // The printed status says Terminating, or the object has its deletion
    // time and the status hasn't caught up.
    let whole = if terminating {
        PodState::Terminating
    } else {
        state
    };
    let squares = Squares::new(
        (facts.init_containers.iter().map(|c| square(c, true, whole)))
            .chain(facts.containers.iter().map(|c| square(c, false, whole)))
            .collect(),
    );
    let lines: Vec<String> = (facts.init_containers.iter())
        .map(|c| container_line(c, true))
        .chain(facts.containers.iter().map(|c| container_line(c, false)))
        .collect();
    PodRow {
        state,
        status,
        reason,
        all_ready: ready_counts.is_none_or(|(up, all)| up == all),
        ready_counts,
        ready,
        restarts,
        restarts_text: restarts.to_string(),
        restarts_label: format!("{restarts} restart{}", if restarts == 1 { "" } else { "s" }),
        restarts_emphasis: restarts_emphasis(restarts),
        node,
        requests: facts.requests,
        limits: facts.limits,
        worst: squares.iter().map(severity).min().unwrap_or(u8::MAX),
        squares,
        containers_tip: lines.join("\n").into(),
        containers_label: lines.join("; ").into(),
        containers: facts.containers,
        init_containers: facts.init_containers,
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

    #[test]
    fn restarts_and_ready_are_derived_with_the_row() {
        let emphasis: Vec<_> = [0, 1, 4, 5, 14].map(restarts_emphasis).to_vec();
        assert_eq!(
            emphasis,
            [
                Emphasis::Muted,
                Emphasis::Plain,
                Emphasis::Plain,
                Emphasis::Warn,
                Emphasis::Warn
            ]
        );
        let one = pod_row(&columns(), &cells("1/2", "Running", "1"), None, false);
        assert_eq!(
            (one.restarts_text.as_str(), one.restarts_label.as_str()),
            ("1", "1 restart")
        );
        assert_eq!((one.ready_counts, one.all_ready), (Some((1, 2)), false));
        let many = pod_row(&columns(), &cells("1/1", "Running", "14"), None, false);
        assert_eq!(many.restarts_label, "14 restarts");
        assert_eq!(many.restarts_emphasis, Emphasis::Warn);
        assert!(many.all_ready);
        let odd = pod_row(&columns(), &cells("", "Pending", "0"), None, false);
        assert_eq!((odd.ready_counts, odd.all_ready), (None, true));
    }

    fn exit(code: i32, reason: &str) -> RunState {
        RunState::Terminated {
            exit_code: code,
            reason: Some(reason.into()),
        }
    }

    fn container(
        state: RunState,
        ready: bool,
        restarts: u32,
        waiting: Option<&str>,
    ) -> ContainerFacts {
        ContainerFacts {
            name: "c".into(),
            ready,
            restarts,
            state,
            waiting: waiting.map(str::to_owned),
            ..ContainerFacts::default()
        }
    }

    #[test]
    fn a_containers_line_names_its_state_reason_and_restarts() {
        let waiting = container(RunState::Waiting, false, 3, Some("CrashLoopBackOff"));
        assert_eq!(
            container_line(&waiting, false),
            "c: waiting · CrashLoopBackOff · 3 restarts"
        );
        let exited = container(
            RunState::Terminated {
                exit_code: 137,
                reason: Some("OOMKilled".into()),
            },
            false,
            1,
            None,
        );
        assert_eq!(
            container_line(&exited, true),
            "c (init): exited 137 · OOMKilled · 1 restart"
        );
        let unready = container(RunState::Running, false, 0, None);
        assert_eq!(container_line(&unready, false), "c: running, not ready");
        let unknown = container(RunState::Unknown, false, 0, None);
        assert_eq!(container_line(&unknown, false), "c: no state reported");
    }

    #[test]
    fn each_container_state_has_its_square() {
        let exit = |code: i32, reason: &str| RunState::Terminated {
            exit_code: code,
            reason: Some(reason.to_owned()),
        };
        let cases = [
            (
                container(RunState::Running, true, 0, None),
                Tone::Good,
                false,
            ),
            (
                container(RunState::Running, true, 3, None),
                Tone::Good,
                true,
            ),
            (
                container(RunState::Running, false, 0, None),
                Tone::Warn,
                false,
            ),
            (
                container(RunState::Waiting, false, 0, Some("ContainerCreating")),
                Tone::Warn,
                false,
            ),
            (
                container(RunState::Waiting, false, 14, Some("CrashLoopBackOff")),
                Tone::Crit,
                true,
            ),
            (
                container(RunState::Waiting, false, 0, Some("ImagePullBackOff")),
                Tone::Crit,
                false,
            ),
            (
                container(exit(0, "Completed"), false, 0, None),
                Tone::Unknown,
                false,
            ),
            (
                container(exit(1, "Error"), false, 2, None),
                Tone::Crit,
                true,
            ),
            (
                container(exit(0, "OOMKilled"), false, 0, None),
                Tone::Crit,
                false,
            ),
            (
                container(RunState::Unknown, false, 0, None),
                Tone::Unknown,
                true,
            ),
        ];
        for (facts, tone, outlined) in cases {
            let drawn = square(&facts, false, PodState::Running);
            assert_eq!((drawn.tone, drawn.outlined), (tone, outlined), "{facts:?}");
            assert!(!drawn.dim);
        }
        assert!(square(&cases_init(), true, PodState::Running).dim);
        // A sidecar runs beside the app, so it isn't dimmed as an init
        // container is.
        let sidecar = ContainerFacts {
            sidecar: true,
            ..cases_init()
        };
        assert!(!square(&sidecar, true, PodState::Running).dim);
    }

    /// Squares agree with the row's glyph: a pod's state can say a
    /// container's own state means nothing, and the square then draws grey
    /// rather than a false alarm (#329 review).
    #[test]
    fn a_pods_state_greys_what_its_containers_cannot_say() {
        let drawn = |status: &str, terminating: bool, init: Vec<ContainerFacts>, app| {
            let facts = PodFacts {
                init_containers: init,
                containers: app,
                ..PodFacts::default()
            };
            let pod = pod_row(
                &columns(),
                &cells("0/1", status, "0"),
                Some(facts),
                terminating,
            );
            pod.squares
                .iter()
                .map(|s| (s.tone, s.outlined, s.dim))
                .collect::<Vec<_>>()
        };
        let killed = || container(exit(143, "Error"), false, 0, None);
        let sidecar = |state| ContainerFacts {
            sidecar: true,
            ..container(state, false, 0, None)
        };
        // A terminating pod's stopped containers, whether the status says
        // so or only the object's deletion time does.
        for (status, terminating) in [("Terminating", false), ("Running", true)] {
            assert_eq!(
                drawn(status, terminating, vec![], vec![killed()]),
                [(Tone::Unknown, false, false)],
                "{status}"
            );
        }
        // A starting pod: an init container running but not yet ready,
        // and an app container being created.
        assert_eq!(
            drawn(
                "PodInitializing",
                false,
                vec![container(RunState::Running, false, 0, None)],
                vec![container(
                    RunState::Waiting,
                    false,
                    0,
                    Some("ContainerCreating")
                )],
            ),
            [(Tone::Unknown, true, true), (Tone::Unknown, true, false)]
        );
        // A Job's pod that succeeded: its sidecar was stopped with the app.
        let done = || container(exit(0, "Completed"), false, 0, None);
        assert_eq!(
            drawn(
                "Completed",
                false,
                vec![sidecar(exit(137, "Error"))],
                vec![done()]
            ),
            [(Tone::Unknown, false, false), (Tone::Unknown, false, false)]
        );
        // While the pod runs, a sidecar that exits has failed.
        assert_eq!(
            drawn(
                "Running",
                false,
                vec![sidecar(exit(137, "Error"))],
                vec![done()]
            )[0],
            (Tone::Crit, false, false)
        );
        // A stuck container stays critical while the pod is starting.
        assert_eq!(
            drawn(
                "Pending",
                false,
                vec![],
                vec![container(
                    RunState::Waiting,
                    false,
                    0,
                    Some("ErrImageNeverPull")
                )],
            ),
            [(Tone::Crit, false, false)]
        );
    }

    fn cases_init() -> ContainerFacts {
        container(RunState::Running, true, 0, None)
    }

    #[test]
    fn a_pods_squares_put_init_containers_first_and_rank_by_the_worst() {
        let facts = PodFacts {
            init_containers: vec![container(
                RunState::Terminated {
                    exit_code: 0,
                    reason: Some("Completed".into()),
                },
                false,
                0,
                None,
            )],
            containers: vec![
                container(RunState::Running, true, 0, None),
                container(RunState::Waiting, false, 4, Some("CrashLoopBackOff")),
            ],
            ..PodFacts::default()
        };
        let pod = pod_row(
            &columns(),
            &cells("1/2", "Running", "4"),
            Some(facts),
            false,
        );
        let drawn: Vec<_> = pod.squares.iter().map(|s| (s.tone, s.dim)).collect();
        assert_eq!(
            drawn,
            [
                (Tone::Unknown, true),
                (Tone::Good, false),
                (Tone::Crit, false)
            ]
        );
        assert_eq!(pod.worst, 0);
        // Ready and restarts stay the app containers'.
        assert_eq!((pod.ready.as_str(), pod.restarts), ("1/2", 4));
        let bare = pod_row(&columns(), &cells("1/1", "Running", "0"), None, false);
        assert!(bare.squares.is_empty());
        assert_eq!(bare.worst, u8::MAX);
    }
}
