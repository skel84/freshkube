//! Example Kubernetes objects for the Resources page, made up from the
//! example cluster's name. Nothing here reads a cluster or a kubeconfig.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use freshkube_core::resources::{
    Amounts, ApiGroup, ContainerFacts, Failure, FailureKind, GroupKinds, ObjectDocument,
    ObjectEvent, PodFacts, PodLogUpdate, PodUsage, ResourceKind, RunState, SecretValue,
    Termination, WorkloadPod, builtin, object_from_yaml,
};

use super::model::{ColumnKind, ResourceColumn, ResourceIdentity, ResourceRow};
use super::rows::{self, PodState};
use crate::fixture;

/// Kinds the example data includes; the rest say so instead of listing.
pub(crate) const KINDS: [&str; 11] = [
    "pods",
    "replicasets.apps",
    "events",
    "persistentvolumeclaims",
    "persistentvolumes",
    "deployments.apps",
    "services",
    "nodes",
    "namespaces",
    "secrets",
    "certificates.cert-manager.io",
];

/// The example cluster's custom API groups and their served versions,
/// preferred first. Some fail or offer nothing listable, as real ones can.
const CUSTOM_GROUPS: [(&str, &[&str]); 7] = [
    ("cert-manager.io", &["v1"]),
    ("cilium.io", &["v2", "v2alpha1"]),
    ("external.metrics.k8s.io", &["v1beta1"]),
    ("metrics.k8s.io", &["v1beta1"]),
    ("monitoring.coreos.com", &["v1", "v1alpha1", "v1beta1"]),
    // Its definition was removed after discovery listed it.
    ("traefik.containo.us", &["v1alpha1"]),
    ("velero.io", &["v1"]),
];

/// (group, version, kind, plural, namespaced) of the custom kinds that
/// discovery finds, each at the version that serves it.
const CUSTOM_KINDS: [(&str, &str, &str, &str, bool); 9] = [
    ("cert-manager.io", "v1", "Certificate", "certificates", true),
    (
        "cert-manager.io",
        "v1",
        "CertificateRequest",
        "certificaterequests",
        true,
    ),
    (
        "cert-manager.io",
        "v1",
        "ClusterIssuer",
        "clusterissuers",
        false,
    ),
    ("cert-manager.io", "v1", "Issuer", "issuers", true),
    (
        "cilium.io",
        "v2alpha1",
        "CiliumLoadBalancerIPPool",
        "ciliumloadbalancerippools",
        false,
    ),
    (
        "cilium.io",
        "v2",
        "CiliumNetworkPolicy",
        "ciliumnetworkpolicies",
        true,
    ),
    ("cilium.io", "v2", "CiliumNode", "ciliumnodes", false),
    (
        "monitoring.coreos.com",
        "v1",
        "PrometheusRule",
        "prometheusrules",
        true,
    ),
    (
        "monitoring.coreos.com",
        "v1",
        "ServiceMonitor",
        "servicemonitors",
        true,
    ),
];

/// (namespace, name, issuer, ready).
const CERTIFICATES: [(&str, &str, &str, bool); 5] = [
    ("kube-system", "webhook-ca", "selfsigned", true),
    ("monitoring", "grafana-tls", "letsencrypt", true),
    ("payments", "api-tls", "internal-ca", false),
    ("payments", "ledger-tls", "internal-ca", true),
    ("web", "gateway-tls", "letsencrypt", true),
];

/// Every example object has this version; none changes by itself.
const EXAMPLE_VERSION: &str = "1";

const NAMESPACES: [&str; 7] = [
    "batch",
    "default",
    "kube-system",
    "monitoring",
    "payments",
    "web",
    "équipe-données",
];

/// (namespace, app, image, replicas). Pods cycle through these in order.
const WORKLOADS: [(&str, &str, &str, u32); 11] = [
    ("kube-system", "coredns", "coredns/coredns:1.12.1", 2),
    ("kube-system", "metrics-server", "metrics-server:v0.8.0", 1),
    ("monitoring", "prometheus", "prom/prometheus:v3.5.0", 1),
    ("monitoring", "grafana", "grafana/grafana:12.1.0", 1),
    ("payments", "api", "payments/api:2.14.3", 3),
    ("payments", "worker", "ghcr.io/example/worker:1.8.2", 4),
    ("payments", "ledger", "payments/ledger:1.9.0", 2),
    ("web", "frontend", "web/frontend:5.2.0", 4),
    ("web", "gateway", "envoyproxy/envoy:v1.35.0", 2),
    ("batch", "report", "batch/report:0.4.1", 2),
    ("équipe-données", "ingest", "data/ingest:3.0.0", 2),
];

/// The connection identity of example data for a context.
pub(crate) fn connection(context: &str) -> String {
    format!("example:{context}")
}

/// Namespaces the example cluster has, for the namespace picker.
pub(crate) fn namespaces() -> Vec<String> {
    NAMESPACES.iter().map(|name| (*name).into()).collect()
}

/// A kind's columns and rows as the server would print them, in one
/// namespace or all. `None` when example data doesn't include the kind.
/// The custom API groups discovery lists, by name.
pub(crate) fn custom_groups() -> Vec<ApiGroup> {
    CUSTOM_GROUPS
        .iter()
        .map(|(name, versions)| ApiGroup {
            name: (*name).into(),
            versions: versions.iter().map(|version| (*version).into()).collect(),
        })
        .collect()
}

/// What discovering one example group finds, failures included.
pub(crate) fn group_kinds(group: &str) -> Result<GroupKinds, Failure> {
    let unavailable = "the server is currently unable to handle the request";
    let missing = || {
        Failure::new(
            FailureKind::NotFound,
            "the server could not find the requested resource",
        )
    };
    match group {
        "velero.io" => Err(Failure::new(
            FailureKind::Forbidden,
            "forbidden: User \"example-viewer\" cannot get path \"/apis/velero.io/v1\"",
        )),
        "external.metrics.k8s.io" => Err(Failure::new(FailureKind::Other, unavailable)),
        "metrics.k8s.io" => Ok(GroupKinds {
            unlistable: vec!["NodeMetrics".into(), "PodMetrics".into()],
            ..GroupKinds::default()
        }),
        "traefik.containo.us" => Err(missing()),
        _ if !CUSTOM_GROUPS.iter().any(|(name, _)| *name == group) => Err(missing()),
        _ => Ok(GroupKinds {
            kinds: CUSTOM_KINDS
                .iter()
                .filter(|(name, ..)| *name == group)
                .map(|(group, version, kind, plural, namespaced)| {
                    ResourceKind::new(group, version, kind, plural, *namespaced)
                })
                .collect(),
            unlistable: Vec::new(),
            failures: if group == "monitoring.coreos.com" {
                vec![
                    (
                        "v1alpha1".into(),
                        Failure::new(
                            FailureKind::Timeout,
                            "monitoring.coreos.com/v1alpha1 timed out",
                        ),
                    ),
                    ("v1beta1".into(), missing()),
                ]
            } else {
                Vec::new()
            },
        }),
    }
}

/// An example kind by key: built in, or one discovery finds.
pub(crate) fn kind(key: &str) -> Option<ResourceKind> {
    builtin(key).or_else(|| {
        CUSTOM_KINDS
            .iter()
            .map(|(group, version, kind, plural, namespaced)| {
                ResourceKind::new(group, version, kind, plural, *namespaced)
            })
            .find(|kind| kind.key() == key)
    })
}

pub(crate) fn read(
    context: &str,
    key: &str,
    namespace: Option<&str>,
    now: i64,
) -> Option<(Vec<ResourceColumn>, Vec<ResourceRow>)> {
    let connection = connection(context);
    let (columns, rows) = match key {
        "pods" => {
            let nodes: Vec<&str> = fixture::kubernetes_nodes(context)
                .iter()
                .map(|node| node.name)
                .collect();
            (
                pod_columns(),
                (0..pod_count(context))
                    .map(|ix| pod(&connection, ix, &nodes, now))
                    .collect(),
            )
        }
        "deployments.apps" => (
            deployment_columns(),
            WORKLOADS
                .iter()
                .enumerate()
                .map(|(ix, _)| deployment(&connection, ix, now))
                .collect(),
        ),
        "replicasets.apps" | "events" | "persistentvolumeclaims" | "persistentvolumes" => {
            summary::extra_rows(&connection, context, key, now)
        }
        "services" => (service_columns(), services(&connection, now)),
        "nodes" => (node_columns(), nodes(&connection, context, now)),
        "secrets" => (secret_columns(), secrets(&connection, now)),
        "certificates.cert-manager.io" => (certificate_columns(), certificates(&connection, now)),
        "namespaces" => (
            namespace_columns(),
            NAMESPACES
                .iter()
                .enumerate()
                .map(|(ix, name)| ResourceRow {
                    identity: identity(&connection, "namespaces", "", name, ix),
                    cells: vec![(*name).into(), "Active".into(), String::new()],
                    created: Some(now - 400 * 86_400 + ix as i64 * 3_600),
                    terminating: false,
                    resource_version: EXAMPLE_VERSION.into(),
                    owner: None,
                    generated: None,
                    pod: None,
                })
                .collect(),
        ),
        _ => return None,
    };
    // Like the server, a namespace narrows only namespaced kinds.
    let namespaced = kind(key).is_some_and(|kind| kind.namespaced);
    let rows = match namespace.filter(|_| namespaced) {
        Some(namespace) => rows
            .into_iter()
            .filter(|row| row.identity.namespace == namespace)
            .collect(),
        None => rows,
    };
    Some((columns, rows))
}

fn identity(
    connection: &str,
    resource: &str,
    namespace: &str,
    name: &str,
    ix: usize,
) -> ResourceIdentity {
    // Distinct per cluster, resource and position, and stable across reads,
    // as a real UID is for one incarnation.
    let salt = connection
        .bytes()
        .chain(*b"/")
        .chain(resource.bytes())
        .fold(0u64, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(u64::from(byte))
        });
    ResourceIdentity {
        connection: connection.into(),
        resource: resource.into(),
        namespace: namespace.into(),
        name: name.into(),
        uid: format!(
            "{:08x}-0000-4000-8000-{:012x}",
            salt & 0xffff_ffff,
            ix as u64
        ),
    }
}

/// Five letters unique to `ix`, like the suffix of a generated pod name.
fn suffix(ix: usize) -> String {
    const LETTERS: &[u8; 10] = b"bcdfghjkmn";
    format!("{ix:05}")
        .bytes()
        .map(|digit| LETTERS[usize::from(digit - b'0')] as char)
        .collect()
}

pub(crate) fn pod_columns() -> Vec<ResourceColumn> {
    vec![
        ResourceColumn::new("Name", ColumnKind::Text, false),
        ResourceColumn::new("Ready", ColumnKind::Text, false),
        ResourceColumn::new("Status", ColumnKind::Text, false),
        ResourceColumn::new("Restarts", ColumnKind::Text, false),
        ResourceColumn::new("Age", ColumnKind::Age, false),
        ResourceColumn::new("IP", ColumnKind::Text, true),
        ResourceColumn::new("Node", ColumnKind::Text, true),
    ]
}

fn pod(connection: &str, ix: usize, nodes: &[&str], now: i64) -> ResourceRow {
    let (namespace, app, ..) = WORKLOADS[ix % WORKLOADS.len()];
    let hash = format!("{:x}", 0x6c4f_8d9b + ix % WORKLOADS.len());
    let name = format!("{app}-{hash}-{}", suffix(ix));
    let (ready, status, restarts) = if namespace == "batch" {
        ("0/1", "Completed", "0".to_owned())
    } else if ix % 23 == 5 {
        ("0/1", "CrashLoopBackOff", "14 (3m ago)".to_owned())
    } else if ix % 31 == 9 {
        ("0/1", "Pending", "0".to_owned())
    } else if ix % 37 == 11 {
        ("0/1", "ContainerCreating", "0".to_owned())
    } else if ix.is_multiple_of(5) {
        (
            "1/1",
            "Running",
            format!("{} ({}d ago)", ix % 4 + 1, ix % 9 + 1),
        )
    } else {
        ("1/1", "Running", "0".to_owned())
    };
    let mut row = ResourceRow {
        identity: identity(connection, "pods", namespace, &name, ix),
        cells: vec![
            name,
            ready.into(),
            status.into(),
            restarts,
            String::new(),
            format!("10.244.{}.{}", ix / 200, ix % 200 + 10),
            nodes
                .get(ix % nodes.len().max(1))
                .copied()
                .unwrap_or("<none>")
                .into(),
        ],
        created: Some(now - (ix as i64 * 7_919 % 2_000_000) - 120),
        terminating: false,
        resource_version: EXAMPLE_VERSION.into(),
        owner: None,
        generated: None,
        pod: None,
    };
    // Batch pods belong to a Job; the rest to a Deployment's ReplicaSet,
    // labelled with its template hash as the Deployment labels them.
    let (controller, labels) = if namespace == "batch" {
        (("Job", format!("{app}-{hash}")), BTreeMap::new())
    } else {
        (
            ("ReplicaSet", format!("{app}-{hash}")),
            BTreeMap::from([("pod-template-hash".to_owned(), hash.clone())]),
        )
    };
    let restarts = row.cells[3]
        .split_whitespace()
        .next()
        .and_then(|count| count.parse().ok())
        .unwrap_or(0);
    let facts = pod_facts(ix, ready == "1/1", status, restarts);
    rows::derive(
        &mut row,
        &pod_columns(),
        Some((controller.0, &controller.1)),
        &labels,
        Some(facts),
    );
    row
}

/// The example pod's app container, an init container for the workloads
/// that migrate or prepare data first, and its resources: requests and
/// limits vary by workload, and some pods set no limits.
fn pod_facts(ix: usize, ready: bool, status: &str, restarts: u32) -> PodFacts {
    let workload = ix % WORKLOADS.len();
    let mebi = 1024. * 1024.;
    let cpu = 50. * (workload % 4 + 1) as f64;
    let memory = 64. * mebi * (workload % 3 + 1) as f64;
    let limited = workload % 5 != 3;
    PodFacts {
        qos: if limited { "Burstable" } else { "BestEffort" }.into(),
        requests: Amounts {
            cpu_millis: Some(cpu),
            memory_bytes: Some(memory),
        },
        limits: Amounts {
            cpu_millis: limited.then_some(cpu * 4.),
            memory_bytes: limited.then_some(memory * 2.),
        },
        containers: vec![app_container(ready, status, restarts)],
        init_containers: init_container(WORKLOADS[workload].1, status),
        ..PodFacts::default()
    }
}

fn app_container(ready: bool, status: &str, restarts: u32) -> ContainerFacts {
    let crashing = status == "CrashLoopBackOff";
    let state = match status {
        "Running" => RunState::Running,
        "Completed" => RunState::Terminated {
            exit_code: 0,
            reason: Some("Completed".into()),
        },
        "Pending" => RunState::Unknown,
        _ => RunState::Waiting,
    };
    ContainerFacts {
        name: "app".into(),
        ready,
        restarts,
        waiting: (state == RunState::Waiting).then(|| status.to_owned()),
        state,
        last_exit_code: crashing.then_some(1),
        last_reason: crashing.then(|| "Error".to_owned()),
    }
}

/// Ledger migrates its schema and Prometheus prepares its data folder
/// before the app starts; a pod not yet scheduled has run neither.
fn init_container(app: &str, status: &str) -> Vec<ContainerFacts> {
    let name = match app {
        "ledger" => "migrate",
        "prometheus" => "init-data",
        _ => return Vec::new(),
    };
    let state = match status {
        "Pending" => RunState::Unknown,
        "ContainerCreating" => RunState::Running,
        _ => RunState::Terminated {
            exit_code: 0,
            reason: Some("Completed".into()),
        },
    };
    vec![ContainerFacts {
        name: name.into(),
        state,
        ..ContainerFacts::default()
    }]
}

/// What metrics-server would report for the example pods of `context`
/// that run: a share of each pod's request that drifts with `tick`.
pub(crate) fn pod_usage(context: &str, tick: u64) -> Vec<PodUsage> {
    (0..pod_count(context))
        .map(|ix| pod("", ix, &[], 0))
        .filter_map(|row| {
            let pod = row.pod.as_ref()?;
            if !matches!(
                pod.state,
                PodState::Running | PodState::NotReady | PodState::Failing
            ) {
                return None;
            }
            let ix = row.identity.uid.len() + row.identity.name.len();
            let share = 0.25 + 0.6 * ((ix as f64 * 0.37 + tick as f64 * 0.3).sin().abs());
            let scale = |amount: Option<f64>| amount.map(|amount| (amount * share).round());
            Some(PodUsage {
                namespace: row.identity.namespace.clone(),
                name: row.identity.name.clone(),
                usage: Amounts {
                    cpu_millis: scale(pod.requests.cpu_millis.map(|cpu| cpu * 1.6)),
                    memory_bytes: scale(pod.requests.memory_bytes),
                },
            })
        })
        .collect()
}

/// Whether `app` runs in `namespace`: its Service and Deployment select
/// example pods, Ready or not.
pub(crate) fn runs_app(namespace: &str, app: &str) -> bool {
    WORKLOADS
        .iter()
        .any(|(ns, name, ..)| *ns == namespace && *name == app)
}

/// How many example pods `context` has.
fn pod_count(context: &str) -> usize {
    match context {
        "staging-eu" => 48,
        "homelab" | "talos-production-frankfurt-equinix-fr5-baremetal-b7" => 22,
        _ => 140,
    }
}

/// The phase of the example pod `name` in `context`, as its status reads
/// it, or `None` when there is no such pod.
pub(crate) fn pod_phase(context: &str, name: &str) -> Option<&'static str> {
    (0..pod_count(context))
        .map(|ix| pod("", ix, &[], 0))
        .find(|row| row.identity.name == name)
        .map(|row| match row.cells[2].as_str() {
            "Completed" => "Succeeded",
            "Pending" | "ContainerCreating" => "Pending",
            _ => "Running",
        })
}

/// The first Running, Ready example pod of `app` in `namespace`, as a
/// Service or Deployment of that name would choose it for a forward.
pub(crate) fn ready_pod(context: &str, namespace: &str, app: &str) -> Option<String> {
    (0..pod_count(context))
        .map(|ix| pod("", ix, &[], 0))
        .find(|row| {
            row.identity.namespace == namespace
                && row.cells[0].starts_with(&format!("{app}-"))
                && row.cells[1] == "1/1"
                && row.cells[2] == "Running"
        })
        .map(|row| row.identity.name)
}

/// The example pods `selector` picks in `workload`'s namespace, as a
/// workload's Logs tab watches them. Only `key=value` terms match, which is
/// all the example workloads use.
pub(crate) fn workload_pods(
    workload: &ResourceIdentity,
    selector: &str,
    now: i64,
) -> Vec<WorkloadPod> {
    let Some(context) = workload.connection.strip_prefix("example:") else {
        return Vec::new();
    };
    let Some(terms) = selector
        .split(',')
        .map(|term| term.split_once('='))
        .collect::<Option<Vec<_>>>()
    else {
        return Vec::new();
    };
    let Some((_, rows)) = read(context, "pods", None, now) else {
        return Vec::new();
    };
    rows.into_iter()
        .enumerate()
        .filter(|(_, row)| row.identity.namespace == workload.namespace)
        .filter_map(|(ix, row)| {
            let yaml = objects::pod_yaml(&row, ix, row.created.unwrap_or(now), now);
            let object: serde_yaml::Value = serde_yaml::from_str(&yaml).ok()?;
            let labels = object.get("metadata")?.get("labels")?;
            terms
                .iter()
                .all(|(key, value)| {
                    labels.get(*key).and_then(serde_yaml::Value::as_str) == Some(*value)
                })
                .then(|| WorkloadPod::of_object(&object))
        })
        .collect()
}

pub(crate) fn deployment_columns() -> Vec<ResourceColumn> {
    vec![
        ResourceColumn::new("Name", ColumnKind::Text, false),
        ResourceColumn::new("Ready", ColumnKind::Text, false),
        ResourceColumn::new("Up-to-date", ColumnKind::Number, false),
        ResourceColumn::new("Available", ColumnKind::Number, false),
        ResourceColumn::new("Age", ColumnKind::Age, false),
        ResourceColumn::new("Containers", ColumnKind::Text, true),
        ResourceColumn::new("Images", ColumnKind::Text, true),
        ResourceColumn::new("Selector", ColumnKind::Text, true),
    ]
}

fn deployment(connection: &str, ix: usize, now: i64) -> ResourceRow {
    let (namespace, app, image, replicas) = WORKLOADS[ix % WORKLOADS.len()];
    // Past the catalogue, test data keeps going with larger replica counts.
    let replicas = replicas as usize + ix / WORKLOADS.len() * 3;
    let available = if app == "worker" {
        replicas - 1
    } else {
        replicas
    };
    let name = if ix < WORKLOADS.len() {
        app.to_owned()
    } else {
        format!("{app}-{}", ix / WORKLOADS.len())
    };
    ResourceRow {
        identity: identity(connection, "deployments.apps", namespace, &name, ix),
        cells: vec![
            name,
            format!("{available}/{replicas}"),
            replicas.to_string(),
            available.to_string(),
            String::new(),
            app.into(),
            image.into(),
            format!("app={app}"),
        ],
        created: Some(now - 90 * 86_400 + ix as i64 * 86_400),
        terminating: false,
        resource_version: EXAMPLE_VERSION.into(),
        owner: None,
        generated: None,
        pod: None,
    }
}

fn service_columns() -> Vec<ResourceColumn> {
    vec![
        ResourceColumn::new("Name", ColumnKind::Text, false),
        ResourceColumn::new("Type", ColumnKind::Text, false),
        ResourceColumn::new("Cluster-IP", ColumnKind::Text, false),
        ResourceColumn::new("External-IP", ColumnKind::Text, false),
        ResourceColumn::new("Port(s)", ColumnKind::Text, false),
        ResourceColumn::new("Age", ColumnKind::Age, false),
        ResourceColumn::new("Selector", ColumnKind::Text, true),
    ]
}

fn services(connection: &str, now: i64) -> Vec<ResourceRow> {
    let mut rows = vec![ResourceRow {
        identity: identity(connection, "services", "default", "kubernetes", 0),
        cells: vec![
            "kubernetes".into(),
            "ClusterIP".into(),
            "10.96.0.1".into(),
            "<none>".into(),
            "443/TCP".into(),
            String::new(),
            "<none>".into(),
        ],
        created: Some(now - 400 * 86_400),
        terminating: false,
        resource_version: EXAMPLE_VERSION.into(),
        owner: None,
        generated: None,
        pod: None,
    }];
    for (ix, (namespace, app, ..)) in WORKLOADS.iter().enumerate() {
        if *namespace == "batch" {
            continue;
        }
        let gateway = *app == "gateway";
        rows.push(ResourceRow {
            identity: identity(connection, "services", namespace, app, ix + 1),
            cells: vec![
                (*app).into(),
                if gateway { "LoadBalancer" } else { "ClusterIP" }.into(),
                format!("10.96.{}.{}", ix / 4 + 1, ix * 7 % 250 + 2),
                if gateway { "203.0.113.40" } else { "<none>" }.into(),
                if gateway {
                    "80:31080/TCP,443:31443/TCP".into()
                } else {
                    "8080/TCP".to_owned()
                },
                String::new(),
                format!("app={app}"),
            ],
            created: Some(now - 80 * 86_400 + ix as i64 * 86_400),
            terminating: false,
            resource_version: EXAMPLE_VERSION.into(),
            owner: None,
            generated: None,
            pod: None,
        });
    }
    rows
}

fn node_columns() -> Vec<ResourceColumn> {
    vec![
        ResourceColumn::new("Name", ColumnKind::Text, false),
        ResourceColumn::new("Status", ColumnKind::Text, false),
        ResourceColumn::new("Roles", ColumnKind::Text, false),
        ResourceColumn::new("Age", ColumnKind::Age, false),
        ResourceColumn::new("Version", ColumnKind::Text, false),
        ResourceColumn::new("Internal-IP", ColumnKind::Text, true),
    ]
}

fn nodes(connection: &str, context: &str, now: i64) -> Vec<ResourceRow> {
    fixture::kubernetes_nodes(context)
        .iter()
        .enumerate()
        .map(|(ix, node)| ResourceRow {
            identity: identity(connection, "nodes", "", node.name, ix),
            cells: vec![
                node.name.into(),
                if node.ready { "Ready" } else { "NotReady" }.into(),
                if node.control_plane {
                    "control-plane"
                } else {
                    "<none>"
                }
                .into(),
                String::new(),
                "v1.34.1".into(),
                node.address.into(),
            ],
            created: Some(now - 200 * 86_400 + ix as i64 * 600),
            terminating: false,
            resource_version: EXAMPLE_VERSION.into(),
            owner: None,
            generated: None,
            pod: None,
        })
        .collect()
}

fn namespace_columns() -> Vec<ResourceColumn> {
    vec![
        ResourceColumn::new("Name", ColumnKind::Text, false),
        ResourceColumn::new("Status", ColumnKind::Text, false),
        ResourceColumn::new("Age", ColumnKind::Age, false),
    ]
}

/// A Secret's keys with their values.
type SecretData = &'static [(&'static str, &'static [u8])];

/// (namespace, name, type, keys with values). Made up for the example
/// cluster; none of them is a real credential.
const SECRETS: [(&str, &str, &str, SecretData); 4] = [
    (
        "kube-system",
        "bootstrap-token-k3x9a1",
        "bootstrap.kubernetes.io/token",
        &[
            ("token-id", b"k3x9a1"),
            ("token-secret", b"example0secret00"),
            ("usage-bootstrap-authentication", b"true"),
        ],
    ),
    (
        "monitoring",
        "grafana-admin",
        "Opaque",
        &[
            ("admin-user", b"admin"),
            ("admin-password", b"example-only-password"),
        ],
    ),
    (
        "payments",
        "ledger-database",
        "Opaque",
        &[
            ("username", b"ledger"),
            ("password", b"example-only-password"),
            ("keystore.p12", &[0x30, 0x82, 0xff, 0xfe, 0x00, 0x01]),
        ],
    ),
    (
        "web",
        "gateway-tls",
        "kubernetes.io/tls",
        &[
            ("tls.crt", b"example certificate, not a real one\n"),
            ("tls.key", b"example key, not a real one\n"),
        ],
    ),
];

fn secret_columns() -> Vec<ResourceColumn> {
    vec![
        ResourceColumn::new("Name", ColumnKind::Text, false),
        ResourceColumn::new("Type", ColumnKind::Text, false),
        ResourceColumn::new("Data", ColumnKind::Number, false),
        ResourceColumn::new("Age", ColumnKind::Age, false),
    ]
}

fn secrets(connection: &str, now: i64) -> Vec<ResourceRow> {
    SECRETS
        .iter()
        .enumerate()
        .map(|(ix, (namespace, name, secret_type, data))| ResourceRow {
            identity: identity(connection, "secrets", namespace, name, ix),
            cells: vec![
                (*name).into(),
                (*secret_type).into(),
                data.len().to_string(),
                String::new(),
            ],
            created: Some(now - 120 * 86_400 + ix as i64 * 86_400),
            terminating: false,
            resource_version: EXAMPLE_VERSION.into(),
            owner: None,
            generated: None,
            pod: None,
        })
        .collect()
}

fn certificate_columns() -> Vec<ResourceColumn> {
    vec![
        ResourceColumn::new("Name", ColumnKind::Text, false),
        ResourceColumn::new("Ready", ColumnKind::Text, false),
        ResourceColumn::new("Secret", ColumnKind::Text, false),
        ResourceColumn::new("Issuer", ColumnKind::Text, true),
        ResourceColumn::new("Status", ColumnKind::Text, true),
        ResourceColumn::new("Age", ColumnKind::Age, false),
    ]
}

fn certificate_status(ready: bool) -> &'static str {
    if ready {
        "Certificate is up to date and has not expired"
    } else {
        "Issuing certificate as Secret does not exist"
    }
}

fn certificates(connection: &str, now: i64) -> Vec<ResourceRow> {
    CERTIFICATES
        .iter()
        .enumerate()
        .map(|(ix, (namespace, name, issuer, ready))| ResourceRow {
            identity: identity(
                connection,
                "certificates.cert-manager.io",
                namespace,
                name,
                ix,
            ),
            cells: vec![
                (*name).into(),
                if *ready { "True" } else { "False" }.into(),
                (*name).into(),
                (*issuer).into(),
                certificate_status(*ready).into(),
                String::new(),
            ],
            created: Some(now - 90 * 86_400 + ix as i64 * 86_400),
            terminating: false,
            resource_version: EXAMPLE_VERSION.into(),
            owner: None,
            generated: None,
            pod: None,
        })
        .collect()
}

mod objects;
mod summary;
pub(crate) use objects::{document, events, pod_log, pod_log_line, secret_value};
pub(crate) use summary::seed_summary;
#[cfg(test)]
pub(crate) use summary::summary;
#[cfg(test)]
const TEST_NOW: i64 = 1_790_000_000;

#[cfg(test)]
pub(crate) fn pod_rows(count: usize) -> Vec<ResourceRow> {
    let nodes = ["node-a", "node-b", "node-c"];
    (0..count)
        .map(|ix| pod("test", ix, &nodes, TEST_NOW))
        .collect()
}

/// A pod no `pod_rows` call returns.
#[cfg(test)]
pub(crate) fn inserted_pod(n: usize) -> ResourceRow {
    let mut row = pod("test", 90_000 + n, &["node-a"], TEST_NOW);
    row.identity.name = format!("inserted-{n}");
    row.cells[0] = row.identity.name.clone();
    row
}

/// The same name created again: a new incarnation with a new UID.
#[cfg(test)]
pub(crate) fn recreated(row: &ResourceRow) -> ResourceRow {
    let mut row = row.clone();
    row.identity.uid.push_str("-recreated");
    row.created = Some(TEST_NOW);
    row
}

#[cfg(test)]
pub(crate) fn deployment_rows(count: usize) -> Vec<ResourceRow> {
    (0..count)
        .map(|ix| deployment("test", ix, TEST_NOW))
        .collect()
}

#[cfg(test)]
mod tests;

/// Node lists use the same objects as the all-namespace resource list.
pub(crate) fn read_filtered(
    context: &str,
    key: &str,
    namespace: Option<&str>,
    selector: Option<&str>,
    now: i64,
) -> Option<(Vec<ResourceColumn>, Vec<ResourceRow>)> {
    let (columns, mut rows) = read(context, key, namespace, now)?;
    if key == "pods"
        && let Some(node) = selector.and_then(|selector| selector.strip_prefix("spec.nodeName="))
    {
        let column = columns.iter().position(|column| column.name == "Node")?;
        rows.retain(|row| row.cells.get(column).is_some_and(|value| value == node));
    }
    Some((columns, rows))
}
