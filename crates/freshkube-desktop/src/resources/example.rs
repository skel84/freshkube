//! Example Kubernetes objects for the Resources page, made up from the
//! example cluster's name. Nothing here reads a cluster or a kubeconfig.

use freshkube_core::resources::builtin;

use super::model::{ColumnKind, ResourceColumn, ResourceIdentity, ResourceRow};
use crate::fixture;

/// Kinds the example data includes; the rest say so instead of listing.
pub(crate) const KINDS: [&str; 5] = [
    "pods",
    "deployments.apps",
    "services",
    "nodes",
    "namespaces",
];

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
                })
                .collect(),
        ),
        _ => return None,
    };
    // Like the server, a namespace narrows only namespaced kinds.
    let namespaced = builtin(key).is_some_and(|kind| kind.namespaced);
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
        assert!(read("prod-fra", "secrets", None, TEST_NOW).is_none());
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
