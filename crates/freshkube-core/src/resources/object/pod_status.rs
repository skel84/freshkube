//! A pod's state as its status reports it: each container's instances, the
//! facts a detail shows beside them, and, when something is wrong, why.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_yaml::Value;

use super::{sequence, text, time};

/// The kubelet waits this long before the first restart of a crashed
/// container, doubles it each time and stops doubling at `MAX_BACKOFF`.
const FIRST_BACKOFF: Duration = Duration::from_secs(10);
const MAX_BACKOFF: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PodStatus {
    pub phase: String,
    /// The pod's own reason and message, such as `Evicted`.
    pub reason: String,
    pub message: String,
    pub service_account: String,
    pub ip: String,
    pub qos: String,
    pub started: Option<DateTime<Utc>>,
    /// The scheduler's message while it can't place the pod.
    pub unschedulable: Option<String>,
    /// The Ready condition's message while the pod isn't ready.
    pub not_ready: Option<String>,
    /// Init containers, then app containers, as the status lists them.
    pub containers: Vec<ContainerStatus>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContainerStatus {
    pub name: String,
    pub init: bool,
    pub ready: bool,
    pub restarts: u32,
    /// Why it isn't running, and the kubelet's message.
    pub waiting: Option<(String, String)>,
    pub running_since: Option<DateTime<Utc>>,
    /// The current instance, once it has ended.
    pub ended: Option<Instance>,
    /// The instance before the current one.
    pub last: Option<Instance>,
}

/// One run of a container.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Instance {
    pub started: Option<DateTime<Utc>>,
    pub finished: Option<DateTime<Utc>>,
    pub exit_code: i64,
    /// Such as `Error`, `OOMKilled` or `Completed`.
    pub reason: String,
    /// The container's termination message, when it wrote one.
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Failing,
    Waiting,
    NotReady,
}

/// What is wrong with a pod, from its status alone.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Diagnosis {
    pub severity: Severity,
    /// The container at fault, when one is.
    pub container: Option<String>,
    /// Such as `CrashLoopBackOff`, `Unschedulable` or `Evicted`.
    pub state: String,
    pub message: String,
    /// How the container's last instance ended.
    pub last: Option<Instance>,
    pub restarts: u32,
    /// When the kubelet will try again, estimated from its back-off.
    pub next_restart: Option<DateTime<Utc>>,
}

/// A waiting reason that only an error explains, as opposed to a container
/// that is still being created.
pub fn is_error_reason(reason: &str) -> bool {
    reason.ends_with("BackOff")
        || reason.contains("Err")
        || reason.contains("Error")
        || reason == "InvalidImageName"
}

/// The kubelet's next attempt after a crash: its back-off doubles from 10 s
/// with each restart, to at most 5 minutes.
pub fn next_restart(finished: DateTime<Utc>, restarts: u32) -> DateTime<Utc> {
    let doublings = restarts.saturating_sub(1).min(16);
    let delay = FIRST_BACKOFF
        .saturating_mul(1 << doublings)
        .min(MAX_BACKOFF);
    finished + chrono::Duration::from_std(delay).unwrap_or_default()
}

impl PodStatus {
    pub fn new(object: &Value) -> Self {
        let spec = object.get("spec");
        let status = object.get("status");
        let field = |key: &str| status.and_then(|status| status.get(key));
        let conditions = sequence(field("conditions"));
        let condition = |kind: &str| {
            conditions
                .iter()
                .find(|condition| text(condition.get("type")) == kind)
        };
        let unschedulable = condition("PodScheduled")
            .filter(|scheduled| text(scheduled.get("status")) == "False")
            .map(|scheduled| text(scheduled.get("message")));
        let not_ready = condition("Ready")
            .filter(|ready| text(ready.get("status")) != "True")
            .map(|ready| text(ready.get("message")));
        let mut containers = Vec::new();
        for (key, init) in [
            ("initContainerStatuses", true),
            ("containerStatuses", false),
        ] {
            containers.extend(
                sequence(field(key))
                    .iter()
                    .map(|status| container(status, init)),
            );
        }
        Self {
            phase: text(field("phase")),
            reason: text(field("reason")),
            message: text(field("message")),
            service_account: text(spec.and_then(|spec| spec.get("serviceAccountName"))),
            ip: text(field("podIP")),
            qos: text(field("qosClass")),
            started: time(field("startTime")),
            unschedulable,
            not_ready,
            containers,
        }
    }

    pub fn container(&self, name: &str) -> Option<&ContainerStatus> {
        self.containers
            .iter()
            .find(|container| container.name == name)
    }

    /// Why the pod fails, waits or isn't ready; `None` when nothing in its
    /// status says so. Only what the status states counts: a running pod
    /// whose readiness passes isn't questioned.
    pub fn diagnose(&self) -> Option<Diagnosis> {
        let pod = |severity, state: &str, message: &str| Diagnosis {
            severity,
            container: None,
            state: state.to_owned(),
            message: message.to_owned(),
            last: None,
            restarts: 0,
            next_restart: None,
        };
        if self.phase == "Failed" && !self.reason.is_empty() {
            return Some(pod(Severity::Failing, &self.reason, &self.message));
        }
        for container in &self.containers {
            let at_fault =
                |severity, state: &str, message: &str, last: Option<&Instance>| Diagnosis {
                    severity,
                    container: Some(container.name.clone()),
                    state: state.to_owned(),
                    message: message.to_owned(),
                    last: last.cloned(),
                    restarts: container.restarts,
                    next_restart: None,
                };
            if let Some((reason, message)) = &container.waiting
                && is_error_reason(reason)
            {
                let last = container.last.as_ref();
                let message = match (message.is_empty(), last) {
                    (true, Some(last)) => last.message.as_str(),
                    _ => message.as_str(),
                };
                let mut diagnosis = at_fault(Severity::Failing, reason, message, last);
                if reason == "CrashLoopBackOff" {
                    diagnosis.next_restart = last
                        .and_then(|last| last.finished)
                        .map(|finished| next_restart(finished, container.restarts));
                }
                return Some(diagnosis);
            }
            if let Some(ended) = &container.ended
                && ended.exit_code != 0
                && (self.phase == "Failed" || container.init)
            {
                let state = if ended.reason.is_empty() {
                    "Error"
                } else {
                    &ended.reason
                };
                return Some(at_fault(
                    Severity::Failing,
                    state,
                    &ended.message,
                    Some(ended),
                ));
            }
        }
        if let Some(message) = &self.unschedulable {
            return Some(pod(Severity::Waiting, "Unschedulable", message));
        }
        if self.phase == "Pending" {
            let waiting = self.containers.iter().find_map(|container| {
                container
                    .waiting
                    .as_ref()
                    .map(|waiting| (container, waiting))
            });
            return Some(match waiting {
                Some((container, (reason, message))) => Diagnosis {
                    container: Some(container.name.clone()),
                    ..pod(Severity::Waiting, reason, message)
                },
                None => pod(Severity::Waiting, "Pending", &self.message),
            });
        }
        if self.phase == "Running"
            && let Some(message) = &self.not_ready
        {
            let unready = self
                .containers
                .iter()
                .find(|container| !container.init && !container.ready);
            return Some(Diagnosis {
                container: unready.map(|container| container.name.clone()),
                last: unready.and_then(|container| container.last.clone()),
                restarts: unready.map_or(0, |container| container.restarts),
                ..pod(Severity::NotReady, "Not ready", message)
            });
        }
        None
    }
}

fn container(status: &Value, init: bool) -> ContainerStatus {
    let state = status.get("state");
    ContainerStatus {
        name: text(status.get("name")),
        init,
        ready: status.get("ready").and_then(Value::as_bool) == Some(true),
        restarts: status
            .get("restartCount")
            .and_then(Value::as_u64)
            .map_or(0, |count| count.min(u64::from(u32::MAX)) as u32),
        waiting: state
            .and_then(|state| state.get("waiting"))
            .map(|waiting| (text(waiting.get("reason")), text(waiting.get("message")))),
        running_since: state
            .and_then(|state| state.get("running"))
            .and_then(|running| time(running.get("startedAt"))),
        ended: state
            .and_then(|state| state.get("terminated"))
            .map(instance),
        last: status
            .get("lastState")
            .and_then(|last| last.get("terminated"))
            .map(instance),
    }
}

fn instance(terminated: &Value) -> Instance {
    Instance {
        started: time(terminated.get("startedAt")),
        finished: time(terminated.get("finishedAt")),
        exit_code: terminated
            .get("exitCode")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        reason: text(terminated.get("reason")),
        message: text(terminated.get("message")).trim_end().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(yaml: &str) -> PodStatus {
        PodStatus::new(&serde_yaml::from_str(yaml).unwrap())
    }

    const CRASHING: &str = "
spec:
  serviceAccountName: api
status:
  phase: Running
  podIP: 10.244.1.7
  qosClass: Burstable
  startTime: '2026-10-01T10:00:00Z'
  conditions:
  - type: Ready
    status: 'False'
    message: 'containers with unready status: [api]'
  initContainerStatuses:
  - name: migrate
    ready: true
    restartCount: 0
    state:
      terminated: {exitCode: 0, reason: Completed}
  containerStatuses:
  - name: api
    ready: false
    restartCount: 14
    state:
      waiting:
        reason: CrashLoopBackOff
        message: back-off 5m0s restarting failed container
    lastState:
      terminated:
        exitCode: 1
        reason: Error
        message: |
          panic: connect to postgres: connection refused
        startedAt: '2026-10-01T10:20:00Z'
        finishedAt: '2026-10-01T10:20:12Z'
";

    #[test]
    fn a_crash_loop_names_its_container_last_exit_and_next_try() {
        let status = status(CRASHING);
        assert_eq!(status.service_account, "api");
        assert_eq!(status.ip, "10.244.1.7");
        assert_eq!(status.qos, "Burstable");
        assert_eq!(status.containers.len(), 2);
        assert!(status.containers[0].init);
        let diagnosis = status.diagnose().unwrap();
        assert_eq!(diagnosis.severity, Severity::Failing);
        assert_eq!(diagnosis.container.as_deref(), Some("api"));
        assert_eq!(diagnosis.state, "CrashLoopBackOff");
        assert_eq!(
            diagnosis.message,
            "back-off 5m0s restarting failed container"
        );
        let last = diagnosis.last.unwrap();
        assert_eq!((last.exit_code, last.reason.as_str()), (1, "Error"));
        assert_eq!(
            last.message,
            "panic: connect to postgres: connection refused"
        );
        assert_eq!(diagnosis.restarts, 14);
        // Fourteen restarts: the back-off has reached its 5 minute cap.
        assert_eq!(
            diagnosis.next_restart.unwrap().to_rfc3339(),
            "2026-10-01T10:25:12+00:00"
        );
    }

    #[test]
    fn the_back_off_doubles_from_ten_seconds_to_five_minutes() {
        let finished = DateTime::parse_from_rfc3339("2026-10-01T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let delay = |restarts| (next_restart(finished, restarts) - finished).num_seconds();
        assert_eq!(
            [0, 1, 2, 3, 5, 6, 40].map(delay),
            [10, 10, 20, 40, 160, 300, 300]
        );
    }

    #[test]
    fn waiting_unschedulable_evicted_and_not_ready_pods_say_why() {
        let pending = status(
            "
status:
  phase: Pending
  conditions:
  - type: PodScheduled
    status: 'False'
    reason: Unschedulable
    message: '0/3 nodes are available: 3 Insufficient memory.'
",
        );
        let diagnosis = pending.diagnose().unwrap();
        assert_eq!(diagnosis.severity, Severity::Waiting);
        assert_eq!(diagnosis.state, "Unschedulable");
        assert!(diagnosis.message.contains("Insufficient memory"));

        let creating = status(
            "
status:
  phase: Pending
  containerStatuses:
  - name: web
    state:
      waiting: {reason: ContainerCreating}
",
        );
        let diagnosis = creating.diagnose().unwrap();
        assert_eq!(diagnosis.severity, Severity::Waiting);
        assert_eq!(diagnosis.container.as_deref(), Some("web"));

        let pull = status(
            "
status:
  phase: Pending
  containerStatuses:
  - name: web
    state:
      waiting: {reason: ImagePullBackOff, message: 'Back-off pulling image \"web:9\"'}
",
        );
        let diagnosis = pull.diagnose().unwrap();
        assert_eq!(diagnosis.severity, Severity::Failing);
        assert_eq!(diagnosis.next_restart, None);

        let evicted = status(
            "
status:
  phase: Failed
  reason: Evicted
  message: 'The node was low on resource: memory.'
",
        );
        let diagnosis = evicted.diagnose().unwrap();
        assert_eq!(
            (diagnosis.severity, diagnosis.state.as_str()),
            (Severity::Failing, "Evicted")
        );

        let unready = status(
            "
status:
  phase: Running
  conditions:
  - type: Ready
    status: 'False'
    message: 'containers with unready status: [web]'
  containerStatuses:
  - name: web
    ready: false
    restartCount: 0
    state:
      running: {startedAt: '2026-10-01T10:00:00Z'}
",
        );
        let diagnosis = unready.diagnose().unwrap();
        assert_eq!(diagnosis.severity, Severity::NotReady);
        assert_eq!(diagnosis.container.as_deref(), Some("web"));
    }

    #[test]
    fn a_healthy_or_finished_pod_has_nothing_to_explain() {
        let healthy = status(
            "
status:
  phase: Running
  conditions:
  - type: Ready
    status: 'True'
  containerStatuses:
  - name: web
    ready: true
    restartCount: 3
    state:
      running: {startedAt: '2026-10-01T10:00:00Z'}
    lastState:
      terminated: {exitCode: 137, reason: OOMKilled}
",
        );
        assert_eq!(healthy.diagnose(), None);
        let done = status(
            "
status:
  phase: Succeeded
  containerStatuses:
  - name: job
    state:
      terminated: {exitCode: 0, reason: Completed}
",
        );
        assert_eq!(done.diagnose(), None);
    }
}
