//! Example Kubernetes objects for the Resources page, made up from the
//! example cluster's name. Nothing here reads a cluster or a kubeconfig.

use chrono::{DateTime, Utc};
use freshkube_core::resources::{
    ApiGroup, Failure, FailureKind, GroupKinds, ObjectDocument, ObjectEvent, PodLogUpdate,
    ResourceKind, SecretValue, Termination, builtin, object_from_yaml,
};

use super::model::{ColumnKind, ResourceColumn, ResourceIdentity, ResourceRow};
use crate::fixture;

/// Kinds the example data includes; the rest say so instead of listing.
pub(crate) const KINDS: [&str; 7] = [
    "pods",
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
    ("payments", "worker", "payments/worker:2.14.3", 4),
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
            let count = match context {
                "staging-eu" => 48,
                "homelab" => 22,
                _ => 140,
            };
            (
                pod_columns(),
                (0..count)
                    .map(|ix| pod(&connection, ix, &nodes, now))
                    .collect(),
            )
        }
        "deployments.apps" => (
            deployment_columns(),
            WORKLOADS
                .iter()
                .enumerate()
                .filter(|(_, (namespace, ..))| *namespace != "batch")
                .map(|(ix, _)| deployment(&connection, ix, now))
                .collect(),
        ),
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
    let name = format!(
        "{app}-{:x}-{}",
        0x6c4f_8d9b + ix % WORKLOADS.len(),
        suffix(ix)
    );
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
    ResourceRow {
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
    }
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
        })
        .collect()
}

fn certificate_yaml(row: &ResourceRow, ix: usize, created: i64) -> String {
    let (namespace, name, issuer, ready) = CERTIFICATES[ix];
    let mut yaml = metadata(
        "Certificate",
        "cert-manager.io/v1",
        row,
        "  generation: 1\n",
    );
    yaml.push_str(&format!(
        "spec:\n  secretName: {name}\n  dnsNames:\n  - {name}.{namespace}.example.internal\n  issuerRef:\n    group: cert-manager.io\n    kind: ClusterIssuer\n    name: {issuer}\n  privateKey:\n    algorithm: ECDSA\n    size: 256\nstatus:\n  conditions:\n  - type: Ready\n    status: '{}'\n    reason: {}\n    message: {}\n    lastTransitionTime: '{}'\n    observedGeneration: 1\n",
        if ready { "True" } else { "False" },
        if ready { "Ready" } else { "DoesNotExist" },
        certificate_status(ready),
        timestamp(created + 60),
    ));
    if ready {
        yaml.push_str(&format!(
            "  notBefore: '{}'\n  notAfter: '{}'\n  renewalTime: '{}'\n  revision: 1\n",
            timestamp(created + 60),
            timestamp(created + 60 + 90 * 86_400),
            timestamp(created + 60 + 60 * 86_400),
        ));
    } else {
        yaml.push_str(&format!(
            "  - type: Issuing\n    status: 'True'\n    reason: DoesNotExist\n    message: {}\n    lastTransitionTime: '{}'\n    observedGeneration: 1\n",
            certificate_status(ready),
            timestamp(created + 60),
        ));
    }
    yaml
}

/// The example row an identity names, with the position its UID encodes.
fn find(identity: &ResourceIdentity, now: i64) -> Option<(ResourceRow, usize)> {
    let context = identity.connection.strip_prefix("example:")?;
    let (_, rows) = read(context, &identity.resource, None, now)?;
    let row = rows.into_iter().find(|row| row.identity == *identity)?;
    let ix = usize::from_str_radix(identity.uid.rsplit('-').next()?, 16).ok()?;
    Some((row, ix))
}

fn time(seconds: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(seconds, 0)
}

fn timestamp(seconds: i64) -> String {
    time(seconds)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Standard base64, for Secret data.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = chunk.iter().enumerate().fold(0u32, |bits, (ix, byte)| {
            bits | u32::from(*byte) << (16 - 8 * ix)
        });
        for ix in 0..4 {
            encoded.push(if ix <= chunk.len() {
                ALPHABET[(bits >> (18 - 6 * ix) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    encoded
}

/// The metadata block every example object starts with.
fn metadata(kind: &str, api_version: &str, row: &ResourceRow, extra: &str) -> String {
    let identity = &row.identity;
    let namespace = if identity.namespace.is_empty() {
        String::new()
    } else {
        format!("  namespace: {}\n", identity.namespace)
    };
    format!(
        "apiVersion: {api_version}\nkind: {kind}\nmetadata:\n  name: {}\n{namespace}  uid: {}\n  resourceVersion: '{}'\n  creationTimestamp: '{}'\n{extra}",
        identity.name,
        identity.uid,
        row.resource_version,
        timestamp(row.created.unwrap_or_default()),
    )
}

/// The full object behind an example row, written as the server would
/// return it. `None` for anything the example cluster doesn't have.
pub(crate) fn document(identity: &ResourceIdentity, now: i64) -> Option<ObjectDocument> {
    let (row, ix) = find(identity, now)?;
    let created = row.created.unwrap_or_default();
    let yaml = match identity.resource.as_str() {
        "pods" => pod_yaml(&row, ix, created, now),
        "deployments.apps" => deployment_yaml(&row, ix, created),
        "services" => service_yaml(&row, ix),
        "nodes" => node_yaml(&row, created),
        "namespaces" => format!(
            "{}spec:\n  finalizers:\n  - kubernetes\nstatus:\n  phase: Active\n",
            metadata(
                "Namespace",
                "v1",
                &row,
                &format!(
                    "  labels:\n    kubernetes.io/metadata.name: {}\n",
                    row.identity.name
                ),
            )
        ),
        "secrets" => {
            let (_, _, secret_type, data) = SECRETS.get(ix)?;
            let mut yaml = metadata("Secret", "v1", &row, "");
            yaml.push_str(&format!("type: {secret_type}\ndata:\n"));
            for (key, value) in *data {
                yaml.push_str(&format!("  {key}: {}\n", base64(value)));
            }
            yaml
        }
        "certificates.cert-manager.io" => certificate_yaml(&row, ix, created),
        _ => return None,
    };
    object_from_yaml(&kind(&identity.resource)?, &yaml).ok()
}

fn pod_yaml(row: &ResourceRow, ix: usize, created: i64, now: i64) -> String {
    let (_, app, image, _) = WORKLOADS[ix % WORKLOADS.len()];
    let hash = format!("{:x}", 0x6c4f_8d9b + ix % WORKLOADS.len());
    let status = row.cells[2].as_str();
    let ready = row.cells[1].starts_with("1/");
    let restarts: u32 = row.cells[3]
        .split_whitespace()
        .next()
        .and_then(|count| count.parse().ok())
        .unwrap_or(0);
    let labels = format!(
        "  generateName: {app}-{hash}-\n  labels:\n    app: {app}\n    pod-template-hash: '{hash}'\n  ownerReferences:\n  - apiVersion: apps/v1\n    kind: ReplicaSet\n    name: {app}-{hash}\n    uid: {}-rs\n    controller: true\n    blockOwnerDeletion: true\n",
        row.identity.uid
    );
    let scheduled = status != "Pending";
    let init = has_init_container(ix);
    let mut yaml = metadata("Pod", "v1", row, &labels);
    yaml.push_str(&format!(
        "spec:\n{}  containers:\n  - name: {app}\n    image: {image}\n    ports:\n    - containerPort: 8080\n      protocol: TCP\n    resources:\n      requests:\n        cpu: 100m\n        memory: 128Mi\n      limits:\n        memory: 256Mi\n",
        if init {
            format!("  initContainers:\n  - name: {INIT_CONTAINER}\n    image: busybox:1.37\n")
        } else {
            String::new()
        }
    ));
    if scheduled {
        yaml.push_str(&format!("  nodeName: {}\n", row.cells[6]));
    }
    yaml.push_str("  restartPolicy: Always\n  serviceAccountName: default\nstatus:\n");
    let phase = match status {
        "Completed" => "Succeeded",
        "Pending" | "ContainerCreating" => "Pending",
        _ => "Running",
    };
    yaml.push_str(&format!("  phase: {phase}\n  conditions:\n"));
    if scheduled {
        yaml.push_str(&format!(
            "  - type: Ready\n    status: '{}'\n    lastTransitionTime: '{}'\n{}  - type: PodScheduled\n    status: 'True'\n    lastTransitionTime: '{}'\n  podIP: {}\n  startTime: '{}'\n{}  containerStatuses:\n  - name: {app}\n    image: {image}\n    ready: {ready}\n    restartCount: {restarts}\n{}    state:\n",
            if ready { "True" } else { "False" },
            timestamp(created + 40),
            if ready {
                String::new()
            } else {
                format!("    reason: ContainersNotReady\n    message: 'containers with unready status: [{app}]'\n")
            },
            timestamp(created),
            row.cells[5],
            timestamp(created),
            if init {
                format!(
                    "  initContainerStatuses:\n  - name: {INIT_CONTAINER}\n    image: busybox:1.37\n    ready: true\n    restartCount: 0\n    state:\n      terminated:\n        exitCode: 0\n        reason: Completed\n        finishedAt: '{}'\n",
                    timestamp(created + 25)
                )
            } else {
                String::new()
            },
            match last_termination(status, restarts, ix, now) {
                Some(last) => format!(
                    "    lastState:\n      terminated:\n        exitCode: {}\n        reason: {}\n        finishedAt: '{}'\n",
                    last.exit_code,
                    last.reason,
                    last.finished.unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                ),
                None => String::new(),
            },
        ));
        yaml.push_str(&match status {
            "Running" => format!("      running:\n        startedAt: '{}'\n", timestamp(created + 30)),
            "Completed" => format!(
                "      terminated:\n        exitCode: 0\n        reason: Completed\n        finishedAt: '{}'\n",
                timestamp(created + 300)
            ),
            "CrashLoopBackOff" => "      waiting:\n        reason: CrashLoopBackOff\n        message: back-off 5m0s restarting failed container\n".into(),
            other => format!("      waiting:\n        reason: {other}\n"),
        });
    } else {
        yaml.push_str(&format!(
            "  - type: PodScheduled\n    status: 'False'\n    reason: Unschedulable\n    message: '0/3 nodes are available: 3 Insufficient memory.'\n    lastTransitionTime: '{}'\n",
            timestamp(created)
        ));
    }
    yaml
}

/// The init container some example pods run first.
const INIT_CONTAINER: &str = "init-config";

fn has_init_container(ix: usize) -> bool {
    ix.is_multiple_of(3)
}

/// How an example pod's app container ended before its current instance.
fn last_termination(status: &str, restarts: u32, ix: usize, now: i64) -> Option<Termination> {
    let (exit_code, reason, finished) = match status {
        "CrashLoopBackOff" => (137, "OOMKilled", now - 180),
        "Running" if restarts > 0 => (1, "Error", now - (ix as i64 % 9 + 1) * 86_400),
        _ => return None,
    };
    Some(Termination {
        exit_code,
        reason: reason.into(),
        finished: time(finished),
    })
}

/// Lines an example container writes, cycling through these.
const LOG_MESSAGES: [&str; 8] = [
    "level=info msg=\"GET /healthz 200\" duration=1.2ms",
    "level=info msg=\"GET /api/v1/items 200\" duration=14ms",
    "level=debug msg=\"cache hit\" key=items:page=1",
    "level=info msg=\"POST /api/v1/orders 201\" duration=38ms",
    "level=warn msg=\"slow query\" duration=812ms table=orders",
    "level=info msg=\"GET /api/v1/items 200\" duration=11ms",
    "level=error msg=\"upstream timeout\" upstream=ledger:8080 attempt=1",
    "level=info msg=\"GET /healthz 200\" duration=0.9ms",
];

/// One example log line as Kubernetes writes it with timestamps.
pub(crate) fn pod_log_line(sequence: u64, at: DateTime<Utc>) -> String {
    format!(
        "{} {}",
        at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        LOG_MESSAGES[sequence as usize % LOG_MESSAGES.len()]
    )
}

/// `count` example lines, one every `step` seconds, the last at `end`.
fn log_lines(count: i64, step: i64, end: i64) -> impl Iterator<Item = PodLogUpdate> {
    (0..count).map(move |ix| {
        let at = time(end - (count - 1 - ix) * step).unwrap_or_default();
        PodLogUpdate::Line(pod_log_line(ix as u64, at))
    })
}

/// What following an example container's log reports at once, and whether
/// it goes on writing. The metrics server refuses its logs, as a viewer
/// without `pods/log` would see.
pub(crate) fn pod_log(
    identity: &ResourceIdentity,
    container: &str,
    previous: bool,
    now: i64,
) -> (Vec<PodLogUpdate>, bool) {
    let failed = |kind, message: String| {
        (
            vec![PodLogUpdate::Failed(Failure::new(kind, message))],
            false,
        )
    };
    let Some((row, ix)) = find(identity, now) else {
        return failed(FailureKind::NotFound, "Example data has no such pod".into());
    };
    let (namespace, app, ..) = WORKLOADS[ix % WORKLOADS.len()];
    let name = &identity.name;
    if app == "metrics-server" {
        return failed(
            FailureKind::Forbidden,
            format!(
                "pods \"{name}\" is forbidden: User \"viewer\" cannot get resource \"pods/log\" in API group \"\" in the namespace \"{namespace}\""
            ),
        );
    }
    let status = row.cells[2].as_str();
    let created = row.created.unwrap_or(now);
    let streaming = PodLogUpdate::Streaming;
    if container == INIT_CONTAINER && has_init_container(ix) {
        if status == "Pending" {
            return (vec![PodLogUpdate::Waiting("Not started".into())], false);
        }
        let mut updates = vec![streaming];
        updates.extend(log_lines(3, 2, created + 24));
        updates.push(PodLogUpdate::Ended(Some(Termination {
            exit_code: 0,
            reason: "Completed".into(),
            finished: time(created + 25),
        })));
        return (updates, false);
    }
    if container != app {
        return failed(
            FailureKind::NotFound,
            format!("The pod has no container named {container}"),
        );
    }
    let restarts = row.cells[3]
        .split_whitespace()
        .next()
        .and_then(|count| count.parse().ok())
        .unwrap_or(0);
    let last = last_termination(status, restarts, ix, now);
    if previous {
        let Some(last) = last else {
            return failed(
                FailureKind::Other,
                format!(
                    "previous terminated container \"{container}\" in pod \"{name}\" not found"
                ),
            );
        };
        let end = last.finished.map_or(now, |finished| finished.timestamp()) - 1;
        let mut updates = vec![streaming];
        updates.extend(log_lines(24, 5, end));
        updates.push(PodLogUpdate::Ended(None));
        return (updates, false);
    }
    match status {
        "Running" => {
            let mut updates = vec![streaming];
            updates.extend(log_lines(40, 7, now - 1));
            (updates, true)
        }
        "CrashLoopBackOff" => {
            // The instance that just ended, then the wait to start again.
            let mut updates = vec![streaming];
            updates.extend(log_lines(12, 5, now - 181));
            updates.push(PodLogUpdate::Restarting(last));
            updates.push(PodLogUpdate::Waiting("CrashLoopBackOff".into()));
            (updates, false)
        }
        "Completed" => {
            let mut updates = vec![streaming];
            updates.extend(log_lines(15, 18, created + 299));
            updates.push(PodLogUpdate::Ended(Some(Termination {
                exit_code: 0,
                reason: "Completed".into(),
                finished: time(created + 300),
            })));
            (updates, false)
        }
        "Pending" => (vec![PodLogUpdate::Waiting("Not started".into())], false),
        other => (vec![PodLogUpdate::Waiting(other.into())], false),
    }
}

fn deployment_yaml(row: &ResourceRow, ix: usize, created: i64) -> String {
    let (_, app, image, _) = WORKLOADS[ix % WORKLOADS.len()];
    let (replicas, available) = (&row.cells[2], &row.cells[3]);
    let progressing = if replicas == available {
        "NewReplicaSetAvailable"
    } else {
        "ReplicaSetUpdated"
    };
    format!(
        "{}spec:\n  replicas: {replicas}\n  selector:\n    matchLabels:\n      app: {app}\n  strategy:\n    type: RollingUpdate\n    rollingUpdate:\n      maxSurge: 25%\n      maxUnavailable: 25%\n  template:\n    metadata:\n      labels:\n        app: {app}\n    spec:\n      containers:\n      - name: {app}\n        image: {image}\n        ports:\n        - containerPort: 8080\n          protocol: TCP\nstatus:\n  observedGeneration: 3\n  replicas: {replicas}\n  updatedReplicas: {replicas}\n  readyReplicas: {available}\n  availableReplicas: {available}\n  conditions:\n  - type: Available\n    status: '{}'\n    reason: {}\n    message: Deployment {}.\n    lastUpdateTime: '{}'\n    lastTransitionTime: '{}'\n  - type: Progressing\n    status: 'True'\n    reason: {progressing}\n    message: ReplicaSet \"{app}-{:x}\" has successfully progressed.\n    lastUpdateTime: '{}'\n    lastTransitionTime: '{}'\n",
        metadata(
            "Deployment",
            "apps/v1",
            row,
            &format!(
                "  generation: 3\n  labels:\n    app: {app}\n  annotations:\n    deployment.kubernetes.io/revision: '3'\n"
            ),
        ),
        if replicas == available {
            "True"
        } else {
            "False"
        },
        if replicas == available {
            "MinimumReplicasAvailable"
        } else {
            "MinimumReplicasUnavailable"
        },
        if replicas == available {
            "has minimum availability"
        } else {
            "does not have minimum availability"
        },
        timestamp(created + 120),
        timestamp(created + 120),
        0x6c4f_8d9b + ix % WORKLOADS.len(),
        timestamp(created + 60),
        timestamp(created),
    )
}

fn service_yaml(row: &ResourceRow, ix: usize) -> String {
    let (kind, cluster_ip) = (&row.cells[1], &row.cells[2]);
    let selector = match ix.checked_sub(1).map(|ix| WORKLOADS[ix].1) {
        Some(app) => format!("  selector:\n    app: {app}\n"),
        None => String::new(),
    };
    let ports = if kind == "LoadBalancer" {
        "  - name: http\n    port: 80\n    targetPort: 8080\n    nodePort: 31080\n    protocol: TCP\n  - name: https\n    port: 443\n    targetPort: 8443\n    nodePort: 31443\n    protocol: TCP\n"
    } else if ix == 0 {
        "  - name: https\n    port: 443\n    targetPort: 6443\n    protocol: TCP\n"
    } else {
        "  - name: http\n    port: 8080\n    targetPort: 8080\n    protocol: TCP\n"
    };
    let status = if kind == "LoadBalancer" {
        format!(
            "status:\n  loadBalancer:\n    ingress:\n    - ip: {}\n",
            row.cells[3]
        )
    } else {
        "status:\n  loadBalancer: {}\n".into()
    };
    format!(
        "{}spec:\n  type: {kind}\n  clusterIP: {cluster_ip}\n  clusterIPs:\n  - {cluster_ip}\n  ports:\n{ports}{selector}  sessionAffinity: None\n{status}",
        metadata("Service", "v1", row, ""),
    )
}

fn node_yaml(row: &ResourceRow, created: i64) -> String {
    let name = &row.identity.name;
    let ready = row.cells[1] == "Ready";
    let role = if row.cells[2] == "control-plane" {
        "    node-role.kubernetes.io/control-plane: ''\n"
    } else {
        ""
    };
    let mut yaml = metadata(
        "Node",
        "v1",
        row,
        &format!(
            "  labels:\n    beta.kubernetes.io/arch: amd64\n    beta.kubernetes.io/os: linux\n    kubernetes.io/arch: amd64\n    kubernetes.io/hostname: {name}\n    kubernetes.io/os: linux\n{role}  annotations:\n    node.alpha.kubernetes.io/ttl: '0'\n    volumes.kubernetes.io/controller-managed-attach-detach: 'true'\n"
        ),
    );
    yaml.push_str(&format!(
        "spec:\n  podCIDR: 10.244.0.0/24\nstatus:\n  addresses:\n  - type: InternalIP\n    address: {}\n  - type: Hostname\n    address: {name}\n  capacity:\n    cpu: '8'\n    memory: 32856156Ki\n    pods: '110'\n  conditions:\n",
        row.cells[5]
    ));
    for (condition, healthy, reason) in [
        ("MemoryPressure", "False", "KubeletHasSufficientMemory"),
        ("DiskPressure", "False", "KubeletHasNoDiskPressure"),
        ("PIDPressure", "False", "KubeletHasSufficientPID"),
    ] {
        yaml.push_str(&format!(
            "  - type: {condition}\n    status: '{healthy}'\n    reason: {reason}\n    lastTransitionTime: '{}'\n",
            timestamp(created + 30)
        ));
    }
    yaml.push_str(&if ready {
        format!(
            "  - type: Ready\n    status: 'True'\n    reason: KubeletReady\n    message: kubelet is posting ready status\n    lastTransitionTime: '{}'\n",
            timestamp(created + 60)
        )
    } else {
        format!(
            "  - type: Ready\n    status: Unknown\n    reason: NodeStatusUnknown\n    message: Kubelet stopped posting node status.\n    lastTransitionTime: '{}'\n",
            timestamp(created + 86_400)
        )
    });
    yaml.push_str(&format!(
        "  nodeInfo:\n    architecture: amd64\n    containerRuntimeVersion: containerd://2.1.4\n    kernelVersion: 6.12.48-talos\n    kubeletVersion: {}\n    operatingSystem: linux\n    osImage: Talos (v1.11.2)\n",
        row.cells[4]
    ));
    yaml
}

#[allow(clippy::too_many_arguments)]
fn event(
    identity: &ResourceIdentity,
    n: usize,
    warning: bool,
    reason: &str,
    message: String,
    count: u32,
    (first, last): (i64, i64),
    source: String,
) -> ObjectEvent {
    ObjectEvent {
        uid: format!("{}-event-{n}", identity.uid),
        event_type: if warning { "Warning" } else { "Normal" }.into(),
        reason: reason.into(),
        message,
        count,
        first_seen: time(first),
        last_seen: time(last),
        source,
        field_path: String::new(),
    }
}

/// Events the example cluster recorded about an object, oldest first.
pub(crate) fn events(identity: &ResourceIdentity, now: i64) -> Vec<ObjectEvent> {
    let Some((row, ix)) = find(identity, now) else {
        return Vec::new();
    };
    let created = row.created.unwrap_or_default();
    let address = identity.address();
    match identity.resource.as_str() {
        "pods" => {
            let (_, app, image, _) = WORKLOADS[ix % WORKLOADS.len()];
            let status = row.cells[2].as_str();
            let node = &row.cells[6];
            let kubelet = format!("kubelet on {node}");
            if status == "Pending" {
                return vec![event(
                    identity,
                    0,
                    true,
                    "FailedScheduling",
                    "0/3 nodes are available: 3 Insufficient memory. preemption: 0/3 nodes are available: 3 No preemption victims found for incoming pod.".into(),
                    6,
                    (created, now - 240),
                    "default-scheduler".into(),
                )];
            }
            let mut events = vec![event(
                identity,
                0,
                false,
                "Scheduled",
                format!("Successfully assigned {address} to {node}"),
                1,
                (created, created),
                "default-scheduler".into(),
            )];
            if status == "ContainerCreating" {
                events.push(event(
                    identity,
                    1,
                    false,
                    "Pulling",
                    format!("Pulling image \"{image}\""),
                    1,
                    (created + 2, created + 2),
                    kubelet,
                ));
                return events;
            }
            for (n, (reason, message)) in [
                (
                    "Pulled",
                    format!("Container image \"{image}\" already present on machine"),
                ),
                ("Created", format!("Created container: {app}")),
                ("Started", format!("Started container {app}")),
            ]
            .into_iter()
            .enumerate()
            {
                let at = created + 5 + n as i64;
                let mut event = event(
                    identity,
                    n + 1,
                    false,
                    reason,
                    message,
                    1,
                    (at, at),
                    kubelet.clone(),
                );
                event.field_path = format!("spec.containers{{{app}}}");
                events.push(event);
            }
            if status == "CrashLoopBackOff" {
                let mut event = event(
                    identity,
                    4,
                    true,
                    "BackOff",
                    format!(
                        "Back-off restarting failed container {app} in pod {}_{}({})",
                        identity.name, identity.namespace, identity.uid
                    ),
                    14,
                    (created + 600, now - 180),
                    kubelet,
                );
                event.field_path = format!("spec.containers{{{app}}}");
                events.push(event);
            }
            events
        }
        "deployments.apps" => {
            let (_, app, ..) = WORKLOADS[ix % WORKLOADS.len()];
            vec![event(
                identity,
                0,
                false,
                "ScalingReplicaSet",
                format!(
                    "Scaled up replica set {app}-{:x} from 0 to {}",
                    0x6c4f_8d9b + ix % WORKLOADS.len(),
                    row.cells[2]
                ),
                1,
                (created + 60, created + 60),
                "deployment-controller".into(),
            )]
        }
        "nodes" if row.cells[1] != "Ready" => vec![event(
            identity,
            0,
            true,
            "NodeNotReady",
            format!("Node {} status is now: NodeNotReady", identity.name),
            1,
            (now - 3_000, now - 3_000),
            "node-controller".into(),
        )],
        "certificates.cert-manager.io" => {
            let (_, name, ..) = CERTIFICATES[ix];
            let issued = if row.cells[1] == "True" {
                (
                    "Issuing",
                    "The certificate has been successfully issued".to_owned(),
                )
            } else {
                (
                    "Requested",
                    format!("Created new CertificateRequest resource \"{name}-1\""),
                )
            };
            [
                ("Issuing", certificate_status(false).to_owned()),
                (
                    "Generated",
                    "Stored new private key in temporary Secret resource".to_owned(),
                ),
                issued,
            ]
            .into_iter()
            .enumerate()
            .map(|(n, (reason, message))| {
                let at = created + 60 + n as i64;
                event(
                    identity,
                    n,
                    false,
                    reason,
                    message,
                    1,
                    (at, at),
                    "cert-manager-certificates-issuing".into(),
                )
            })
            .collect()
        }
        _ => Vec::new(),
    }
}

/// One value of an example Secret, as revealing it would read it.
pub(crate) fn secret_value(
    identity: &ResourceIdentity,
    key: &str,
    now: i64,
) -> Option<SecretValue> {
    let (_, ix) = find(identity, now)?;
    let (_, _, _, data) = SECRETS.get(ix)?;
    let (_, value) = data.iter().find(|(name, _)| *name == key)?;
    Some(match String::from_utf8(value.to_vec()) {
        Ok(text) => SecretValue::Text(text),
        Err(error) => SecretValue::Binary(error.into_bytes().len()),
    })
}

/// Test rows: a fixed clock and connection, so values are reproducible.
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
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn example_kinds_read_with_unique_identities_and_matching_cells() {
        for context in fixture::CONTEXTS {
            for key in KINDS {
                let (columns, rows) = read(context, key, None, TEST_NOW).unwrap();
                assert!(!rows.is_empty(), "{context} {key}");
                let unique: HashSet<_> = rows.iter().map(|row| &row.identity).collect();
                assert_eq!(unique.len(), rows.len(), "{context} {key}");
                for row in &rows {
                    assert_eq!(row.cells.len(), columns.len(), "{key}");
                    assert_eq!(row.cells[0], row.identity.name);
                    assert_eq!(row.identity.connection, connection(context));
                }
            }
        }
        assert!(read("prod-fra", "configmaps", None, TEST_NOW).is_none());
    }

    #[test]
    fn every_example_object_reads_in_full_with_its_events() {
        for key in KINDS {
            let (_, rows) = read("homelab", key, None, TEST_NOW).unwrap();
            for row in &rows {
                let identity = &row.identity;
                let document = document(identity, TEST_NOW)
                    .unwrap_or_else(|| panic!("{key} {}", identity.address()));
                assert_eq!(document.uid, identity.uid);
                assert_eq!(document.name, identity.name);
                assert_eq!(document.namespace.unwrap_or_default(), identity.namespace);
                assert_eq!(document.resource_version, row.resource_version);
                assert!(document.overview.created.is_some(), "{key}");
                for event in events(identity, TEST_NOW) {
                    assert!(event.uid.starts_with(&identity.uid));
                    assert!(event.last_seen >= event.first_seen);
                }
            }
        }
        // Not an example object, so nothing to read.
        let mut stranger = pod_rows(1).remove(0).identity;
        stranger.connection = connection("homelab");
        assert!(document(&stranger, TEST_NOW).is_none());
        assert!(events(&stranger, TEST_NOW).is_empty());

        let (_, pods) = read("prod-fra", "pods", None, TEST_NOW).unwrap();
        let crashing = pods
            .iter()
            .find(|row| row.cells[2] == "CrashLoopBackOff")
            .unwrap();
        assert!(
            events(&crashing.identity, TEST_NOW)
                .iter()
                .any(ObjectEvent::is_warning)
        );
    }

    #[test]
    fn example_secrets_hide_values_until_one_is_revealed() {
        let (_, secrets) = read("homelab", "secrets", Some("payments"), TEST_NOW).unwrap();
        let identity = &secrets[0].identity;
        let document = document(identity, TEST_NOW).unwrap();
        assert!(!document.yaml.contains("example-only-password"));
        assert!(!document.yaml.contains(&base64(b"example-only-password")));
        let keys = &document.overview.secret.unwrap().keys;
        assert_eq!(keys.len(), 3);
        assert_eq!(
            secret_value(identity, "password", TEST_NOW),
            Some(SecretValue::Text("example-only-password".into()))
        );
        assert_eq!(
            secret_value(identity, "keystore.p12", TEST_NOW),
            Some(SecretValue::Binary(6))
        );
        assert_eq!(secret_value(identity, "missing", TEST_NOW), None);
    }

    #[test]
    fn example_discovery_covers_every_group_state() {
        let groups = custom_groups();
        assert_eq!(groups.len(), CUSTOM_GROUPS.len());
        assert_eq!(groups[1].versions, ["v2", "v2alpha1"]);
        let certs = group_kinds("cert-manager.io").unwrap();
        assert_eq!(certs.kinds.len(), 4);
        assert!(certs.failures.is_empty());
        assert_eq!(
            kind("certificates.cert-manager.io"),
            Some(certs.kinds[0].clone())
        );
        assert_eq!(kind("pods"), builtin("pods"));
        assert_eq!(kind("widgets.example.com"), None);
        assert_eq!(
            group_kinds("monitoring.coreos.com").unwrap().failures.len(),
            2
        );
        assert_eq!(
            group_kinds("traefik.containo.us").unwrap_err().kind,
            FailureKind::NotFound
        );
        assert!(group_kinds("metrics.k8s.io").unwrap().kinds.is_empty());
        assert_eq!(
            group_kinds("velero.io").unwrap_err().kind,
            FailureKind::Forbidden
        );
        assert_eq!(
            group_kinds("external.metrics.k8s.io").unwrap_err().kind,
            FailureKind::Other
        );
        assert_eq!(
            group_kinds("gone.example.com").unwrap_err().kind,
            FailureKind::NotFound
        );
        // A namespace narrows a namespaced custom kind too.
        let (_, payments) = read(
            "homelab",
            "certificates.cert-manager.io",
            Some("payments"),
            TEST_NOW,
        )
        .unwrap();
        assert_eq!(payments.len(), 2);
    }

    #[test]
    fn base64_matches_the_standard_alphabet_and_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(base64(b"hunter2"), "aHVudGVyMg==");
        assert_eq!(base64(&[0xfb, 0xff]), "+/8=");
    }

    #[test]
    fn a_namespace_reads_only_its_objects() {
        let (_, all) = read("prod-fra", "pods", None, TEST_NOW).unwrap();
        let (_, payments) = read("prod-fra", "pods", Some("payments"), TEST_NOW).unwrap();
        assert!(!payments.is_empty() && payments.len() < all.len());
        assert!(
            payments
                .iter()
                .all(|row| row.identity.namespace == "payments")
        );
        let statuses: HashSet<_> = all.iter().map(|row| row.cells[2].as_str()).collect();
        for status in ["Running", "CrashLoopBackOff", "Pending", "Completed"] {
            assert!(statuses.contains(status), "{status}");
        }
        // A namespace doesn't narrow cluster-scoped kinds.
        let (_, nodes) = read("prod-fra", "nodes", Some("payments"), TEST_NOW).unwrap();
        assert_eq!(nodes.len(), fixture::kubernetes_nodes("prod-fra").len());
    }
}
