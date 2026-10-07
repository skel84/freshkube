//! What a pods table row needs beyond its printed cells: the pod's node,
//! its containers' states and its resource requests and limits, read from
//! the full object the table includes for pods. Usage comes from
//! metrics-server ([`list_pod_usage`]), which a cluster may not run.

use std::collections::BTreeMap;

use kube::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};

use super::failure::Failure;
use super::object::{JSON, json_get};
use super::table::nullable;

/// CPU in millicores and memory in bytes. `None` where a container leaves
/// it unset, which for a sum means at least one container did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Amounts {
    pub cpu_millis: Option<f64>,
    pub memory_bytes: Option<f64>,
}

/// One container's state as the pod's status reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContainerFacts {
    pub name: String,
    pub ready: bool,
    pub restarts: u32,
    /// What it is doing now.
    pub state: RunState,
    /// Why it waits, such as `CrashLoopBackOff` or `ImagePullBackOff`.
    pub waiting: Option<String>,
    /// How its previous run ended, when it ended.
    pub last_exit_code: Option<i32>,
    pub last_reason: Option<String>,
    /// A native sidecar: an init container with `restartPolicy: Always`,
    /// which runs beside the app containers and is stopped when they end.
    pub sidecar: bool,
}

/// What a container is doing now, from its status's `state`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RunState {
    /// The status names no state, or the container has no status yet.
    #[default]
    Unknown,
    Running,
    /// Waiting to start or start again; its reason is
    /// [`ContainerFacts::waiting`].
    Waiting,
    /// Ran and stopped: `Completed` with 0, or `Error`, `OOMKilled`.
    Terminated {
        exit_code: i32,
        reason: Option<String>,
    },
}

/// A pod's facts for its row. Requests and limits sum its app containers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PodFacts {
    pub node: String,
    pub phase: String,
    pub qos: String,
    pub requests: Amounts,
    pub limits: Amounts,
    /// The app containers, as the status lists them.
    pub containers: Vec<ContainerFacts>,
    /// The init containers, in the order they run. Kept apart, so readiness
    /// and restarts stay the app containers', as `kubectl` prints them.
    pub init_containers: Vec<ContainerFacts>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PodSpec {
    #[serde(default, deserialize_with = "nullable")]
    node_name: String,
    #[serde(default, deserialize_with = "nullable")]
    containers: Vec<SpecContainer>,
    #[serde(default, deserialize_with = "nullable")]
    init_containers: Vec<SpecContainer>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
struct SpecContainer {
    #[serde(default, deserialize_with = "nullable")]
    name: String,
    #[serde(default, deserialize_with = "nullable")]
    resources: Requirements,
    #[serde(default, rename = "restartPolicy", deserialize_with = "nullable")]
    restart_policy: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
struct Requirements {
    #[serde(default, deserialize_with = "nullable")]
    requests: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "nullable")]
    limits: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PodStatus {
    #[serde(default, deserialize_with = "nullable")]
    phase: String,
    #[serde(default, deserialize_with = "nullable")]
    qos_class: String,
    #[serde(default, deserialize_with = "nullable")]
    container_statuses: Vec<ContainerStatus>,
    #[serde(default, deserialize_with = "nullable")]
    init_container_statuses: Vec<ContainerStatus>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContainerStatus {
    #[serde(default, deserialize_with = "nullable")]
    name: String,
    #[serde(default)]
    ready: bool,
    #[serde(default)]
    restart_count: u32,
    #[serde(default, deserialize_with = "nullable")]
    state: State,
    #[serde(default, deserialize_with = "nullable")]
    last_state: State,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
struct State {
    #[serde(default)]
    running: Option<serde_json::Value>,
    #[serde(default)]
    waiting: Option<Reason>,
    #[serde(default)]
    terminated: Option<Terminated>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
struct Reason {
    #[serde(default, deserialize_with = "nullable")]
    reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Terminated {
    #[serde(default)]
    exit_code: i32,
    #[serde(default, deserialize_with = "nullable")]
    reason: String,
}

/// Reads a field of a row's object, or nothing when it has another shape:
/// any kind but Pod has its own `spec` and `status`.
pub(crate) fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

/// A pod's facts from its object's spec and status.
pub(crate) fn facts(spec: &PodSpec, status: Option<&PodStatus>) -> PodFacts {
    let empty = PodStatus::default();
    let status = status.unwrap_or(&empty);
    let sum = |pick: fn(&Requirements) -> &BTreeMap<String, String>| {
        let total = |key: &str, parse: fn(&str) -> Option<f64>| {
            spec.containers
                .iter()
                .map(|container| pick(&container.resources).get(key).and_then(|q| parse(q)))
                .sum::<Option<f64>>()
                .filter(|_| !spec.containers.is_empty())
        };
        Amounts {
            cpu_millis: total("cpu", cpu_millis),
            memory_bytes: total("memory", quantity),
        }
    };
    PodFacts {
        node: spec.node_name.clone(),
        phase: status.phase.clone(),
        qos: status.qos_class.clone(),
        requests: sum(|resources| &resources.requests),
        limits: sum(|resources| &resources.limits),
        containers: containers(&spec.containers, &status.container_statuses, false),
        init_containers: containers(&spec.init_containers, &status.init_container_statuses, true),
    }
}

/// The spec's containers in its order, each with its status, or with an
/// unknown state while it has none (a pod not yet scheduled), then any
/// status the spec doesn't name. Only an init container restarted Always
/// is a sidecar: an app container's restart rules, newer and alpha, don't
/// make it one.
fn containers(
    spec: &[SpecContainer],
    statuses: &[ContainerStatus],
    init: bool,
) -> Vec<ContainerFacts> {
    let named = |name: &str| spec.iter().any(|container| container.name == name);
    spec.iter()
        .map(|declared| {
            let facts = statuses
                .iter()
                .find(|status| status.name == declared.name)
                .map(container)
                .unwrap_or_else(|| ContainerFacts {
                    name: declared.name.clone(),
                    ..ContainerFacts::default()
                });
            ContainerFacts {
                sidecar: init && declared.restart_policy == "Always",
                ..facts
            }
        })
        .chain(
            statuses
                .iter()
                .filter(|status| !named(&status.name))
                .map(container),
        )
        .collect()
}

fn container(status: &ContainerStatus) -> ContainerFacts {
    let reason = |text: &str| Some(text.to_owned()).filter(|reason| !reason.is_empty());
    let state = &status.state;
    ContainerFacts {
        name: status.name.clone(),
        ready: status.ready,
        restarts: status.restart_count,
        state: match (&state.running, &state.waiting, &state.terminated) {
            (Some(_), _, _) => RunState::Running,
            (_, Some(_), _) => RunState::Waiting,
            (_, _, Some(ended)) => RunState::Terminated {
                exit_code: ended.exit_code,
                reason: reason(&ended.reason),
            },
            _ => RunState::Unknown,
        },
        waiting: state
            .waiting
            .as_ref()
            .and_then(|waiting| reason(&waiting.reason)),
        // From the spec, which `containers` reads.
        sidecar: false,
        last_exit_code: status
            .last_state
            .terminated
            .as_ref()
            .map(|ended| ended.exit_code),
        last_reason: status
            .last_state
            .terminated
            .as_ref()
            .and_then(|ended| reason(&ended.reason)),
    }
}

/// A Kubernetes quantity (`128Mi`, `1.5G`, `250m`, `2e3`) as a number of
/// its base unit, or `None` when it doesn't parse.
pub fn quantity(text: &str) -> Option<f64> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '+' || c == '-'))
        .unwrap_or(text.len());
    let (number, suffix) = text.split_at(split);
    let number: f64 = number.parse().ok()?;
    let scale = match suffix {
        "" => 1.,
        "n" => 1e-9,
        "u" => 1e-6,
        "m" => 1e-3,
        "k" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        "T" => 1e12,
        "P" => 1e15,
        "E" => 1e18,
        "Ki" => 1024.,
        "Mi" => 1024f64.powi(2),
        "Gi" => 1024f64.powi(3),
        "Ti" => 1024f64.powi(4),
        "Pi" => 1024f64.powi(5),
        "Ei" => 1024f64.powi(6),
        exponent if exponent.starts_with(['e', 'E']) => 10f64.powi(exponent[1..].parse().ok()?),
        _ => return None,
    };
    Some(number * scale)
}

/// A CPU quantity in millicores.
pub fn cpu_millis(text: &str) -> Option<f64> {
    quantity(text).map(|cores| cores * 1000.)
}

/// One pod's current use, as metrics-server last sampled it.
#[derive(Clone, Debug, PartialEq)]
pub struct PodUsage {
    pub namespace: String,
    pub name: String,
    pub usage: Amounts,
}

#[derive(Deserialize)]
struct MetricsList {
    #[serde(default, deserialize_with = "nullable")]
    items: Vec<PodMetrics>,
}

#[derive(Deserialize)]
struct PodMetrics {
    metadata: MetricsMeta,
    #[serde(default, deserialize_with = "nullable")]
    containers: Vec<ContainerMetrics>,
}

#[derive(Deserialize)]
struct MetricsMeta {
    #[serde(default, deserialize_with = "nullable")]
    name: String,
    #[serde(default, deserialize_with = "nullable")]
    namespace: String,
}

#[derive(Deserialize)]
struct ContainerMetrics {
    #[serde(default, deserialize_with = "nullable")]
    usage: BTreeMap<String, String>,
}

/// Lists pods' use from `metrics.k8s.io`, in one namespace or all. A
/// cluster without metrics-server answers 404 (`FailureKind::NotFound`).
pub async fn list_pod_usage(
    client: &Client,
    namespace: Option<&str>,
) -> Result<Vec<PodUsage>, Failure> {
    let path = match namespace {
        Some(namespace) => format!("/apis/metrics.k8s.io/v1beta1/namespaces/{namespace}/pods"),
        None => "/apis/metrics.k8s.io/v1beta1/pods".to_owned(),
    };
    let request = json_get(path, JSON)?;
    let list: MetricsList = client.request(request).await.map_err(Failure::from_kube)?;
    Ok(list.items.into_iter().map(usage).collect())
}

fn usage(metrics: PodMetrics) -> PodUsage {
    let total = |key: &str, parse: fn(&str) -> Option<f64>| {
        metrics
            .containers
            .iter()
            .filter_map(|container| container.usage.get(key).and_then(|q| parse(q)))
            .fold(None, |sum: Option<f64>, value| {
                Some(sum.unwrap_or(0.) + value)
            })
    };
    PodUsage {
        usage: Amounts {
            cpu_millis: total("cpu", cpu_millis),
            memory_bytes: total("memory", quantity),
        },
        namespace: metrics.metadata.namespace,
        name: metrics.metadata.name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::table::Table;

    #[test]
    fn quantities_parse_decimal_binary_and_exponent_suffixes() {
        assert_eq!(quantity("128Mi"), Some(134_217_728.));
        assert_eq!(quantity("1Gi"), Some(1_073_741_824.));
        assert_eq!(quantity("1.5G"), Some(1.5e9));
        assert_eq!(quantity("2e3"), Some(2000.));
        assert_eq!(quantity("123456"), Some(123_456.));
        assert_eq!(cpu_millis("250m"), Some(250.));
        assert_eq!(cpu_millis("2"), Some(2000.));
        assert_eq!(cpu_millis("1500000n"), Some(1.5));
        assert_eq!(quantity("12Qi"), None);
        assert_eq!(quantity(""), None);
    }

    fn pod_row(object: &str) -> Table {
        serde_json::from_str(&format!(
            r#"{{"rows":[{{"cells":["api-0"],"object":{object}}}]}}"#
        ))
        .unwrap()
    }

    #[test]
    fn a_full_pod_gives_its_node_states_and_resources() {
        let table = pod_row(
            r#"{"kind":"Pod","metadata":{"name":"api-0","uid":"u",
                "ownerReferences":[{"apiVersion":"apps/v1","kind":"ReplicaSet","name":"api-6c4f",
                  "uid":"r","controller":true}]},
              "spec":{"nodeName":"wk-1","containers":[
                {"name":"app","resources":{"requests":{"cpu":"100m","memory":"64Mi"},
                  "limits":{"cpu":"1","memory":"256Mi"}}},
                {"name":"proxy","restartPolicy":"Always",
                  "resources":{"requests":{"cpu":"50m","memory":"32Mi"}}}],
                "initContainers":[{"name":"migrate"},{"name":"mesh","restartPolicy":"Always"},
                  {"name":"warm"}]},
              "status":{"phase":"Running","qosClass":"Burstable",
                "initContainerStatuses":[
                  {"name":"migrate","ready":false,"restartCount":0,
                    "state":{"terminated":{"exitCode":0,"reason":"Completed"}}},
                  {"name":"mesh","ready":true,"restartCount":1,
                    "state":{"running":{"startedAt":"2026-10-07T10:00:00Z"}}}],
                "containerStatuses":[
                {"name":"app","ready":false,"restartCount":14,
                  "state":{"waiting":{"reason":"CrashLoopBackOff"}},
                  "lastState":{"terminated":{"exitCode":1,"reason":"Error"}}},
                {"name":"proxy","ready":true,"restartCount":0,"state":{"running":{}}}]}}"#,
        );
        let row = &table.rows[0];
        let owner = &row.metadata().unwrap().owner_references[0];
        assert_eq!(
            (owner.kind.as_str(), owner.controller),
            ("ReplicaSet", Some(true))
        );
        let facts = row.pod().unwrap();
        assert_eq!(facts.node, "wk-1");
        assert_eq!(facts.qos, "Burstable");
        assert_eq!(facts.requests.cpu_millis, Some(150.));
        assert_eq!(facts.requests.memory_bytes, Some(96. * 1024. * 1024.));
        // The proxy sets no limit, so the pod as a whole has none.
        assert_eq!(facts.limits.cpu_millis, None);
        let app = &facts.containers[0];
        assert_eq!(app.restarts, 14);
        assert_eq!(app.waiting.as_deref(), Some("CrashLoopBackOff"));
        assert_eq!(app.last_exit_code, Some(1));
        assert_eq!(app.state, RunState::Waiting);
        assert!(facts.containers[1].ready);
        assert_eq!(facts.containers[1].state, RunState::Running);
        // Init containers stay apart from the app containers, in order.
        assert_eq!(facts.containers.len(), 2);
        let init: Vec<_> = facts
            .init_containers
            .iter()
            .map(|c| (c.name.as_str(), &c.state, c.restarts))
            .collect();
        assert_eq!(
            init,
            [
                (
                    "migrate",
                    &RunState::Terminated {
                        exit_code: 0,
                        reason: Some("Completed".into())
                    },
                    0
                ),
                ("mesh", &RunState::Running, 1),
                // Declared, but no status yet.
                ("warm", &RunState::Unknown, 0),
            ]
        );
        // Only an init container restarted Always is a sidecar; the proxy,
        // an app container with the same policy, isn't.
        let sidecars: Vec<_> = (facts.init_containers.iter())
            .chain(&facts.containers)
            .map(|c| c.sidecar)
            .collect();
        assert_eq!(sidecars, [false, true, false, false, false]);
    }

    #[test]
    fn metadata_only_and_odd_shaped_objects_have_no_pod_facts() {
        let table = pod_row(r#"{"kind":"PartialObjectMetadata","metadata":{"name":"a"}}"#);
        assert!(table.rows[0].pod().is_none());
        let table = pod_row(r#"{"metadata":{"name":"a"},"spec":{"containers":"no"}}"#);
        assert!(table.rows[0].pod().is_none());
        assert_eq!(table.rows[0].metadata().unwrap().name, "a");
    }

    #[test]
    fn usage_sums_containers() {
        let list: MetricsList = serde_json::from_str(
            r#"{"items":[{"metadata":{"name":"api-0","namespace":"shop"},"containers":[
                {"name":"app","usage":{"cpu":"12500000n","memory":"10Mi"}},
                {"name":"proxy","usage":{"cpu":"2m","memory":"2Mi"}}]},
              {"metadata":{"name":"idle","namespace":"shop"},"containers":null}]}"#,
        )
        .unwrap();
        let usage: Vec<_> = list.items.into_iter().map(usage).collect();
        assert_eq!(usage[0].usage.cpu_millis, Some(14.5));
        assert_eq!(usage[0].usage.memory_bytes, Some(12. * 1024. * 1024.));
        assert_eq!(usage[1].usage, Amounts::default());
    }
}
