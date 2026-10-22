//! A pod's state from its printed status and readiness, and the verdict the
//! Resources page and the command line draw from it, so the two can't
//! disagree.

use super::state::{StatusTone, status_tone};

/// How a pod is doing, from its printed status and readiness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PodState {
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
pub fn died(status: &str) -> bool {
    let status = status.strip_prefix("Init:").unwrap_or(status);
    let reason = status.split([' ', '(']).next().unwrap_or_default();
    matches!(reason, "CrashLoopBackOff" | "Error" | "OOMKilled")
}

/// The state of a pod from the Status and Ready cells it prints.
pub fn pod_state(status: &str, ready: &str) -> PodState {
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

/// What a pod's glyph says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PodVerdict {
    Good,
    Warn,
    Crit,
    /// A container ran and stopped.
    Died,
    Unknown,
}

/// The verdict for a pod's state: a skull where a container ran and stopped,
/// critical where the pod can't run. A pod on a node that isn't ready can't
/// be confirmed, so it warns.
pub fn pod_verdict(state: PodState, status: &str, node_ready: bool) -> PodVerdict {
    match state {
        PodState::Failing if died(status) => PodVerdict::Died,
        PodState::Failing => PodVerdict::Crit,
        PodState::NotReady => PodVerdict::Warn,
        _ if !node_ready => PodVerdict::Warn,
        PodState::Running | PodState::Completed => PodVerdict::Good,
        PodState::Pending | PodState::Terminating | PodState::Unknown => PodVerdict::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_and_readiness_give_the_state() {
        for (status, ready, state) in [
            ("Running", "1/1", PodState::Running),
            ("Running", "1/2", PodState::NotReady),
            ("ContainerCreating", "0/1", PodState::Pending),
            ("Completed", "0/1", PodState::Completed),
            ("Terminating", "1/1", PodState::Terminating),
            ("ImagePullBackOff", "0/1", PodState::Failing),
            ("NodeAffinity", "0/1", PodState::Unknown),
        ] {
            assert_eq!(pod_state(status, ready), state, "{status}");
        }
    }

    #[test]
    fn a_pod_on_a_node_that_is_not_ready_warns_unless_it_is_failing() {
        assert_eq!(
            pod_verdict(PodState::Running, "Running", true),
            PodVerdict::Good
        );
        assert_eq!(
            pod_verdict(PodState::Running, "Running", false),
            PodVerdict::Warn
        );
        assert_eq!(
            pod_verdict(PodState::Failing, "CrashLoopBackOff", false),
            PodVerdict::Died
        );
        assert_eq!(
            pod_verdict(PodState::Failing, "ImagePullBackOff", true),
            PodVerdict::Crit
        );
    }
}
