//! Invented NetworkPolicies in the example cluster (`FRESHKUBE_NETPOL`). The
//! policies themselves are one fixed set, so a Resources row opens a real
//! example object; the shape reshapes what the Pod, workload and policy
//! panes read of them: the set cut down to one kind of policy, no policy at
//! all, or the reads refused or cut.
use super::*;
use freshkube_core::resources::traffic::netpol::{
    Applying, PodsOf, PolicyFacts, PolicyReading, SelectedPods, applying, selector_matches,
};
use freshkube_core::resources::{Failure, FailureKind};
use k8s_openapi::api::networking::v1::NetworkPolicy;
use serde_json::{Value, json};

/// The position, in a UID, of the first example policy.
const FIRST: usize = 0xc000;

pub(super) const KEY: &str = "networkpolicies.networking.k8s.io";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    /// Every policy of the set: a default deny, a restriction, an open one.
    #[default]
    Mixed,
    /// Only a default-deny policy.
    DefaultDeny,
    /// Only a policy that opens its pods to everything.
    Open,
    /// Policies that allow listed peers and ports.
    Restricts,
    /// A policy whose peer is an IP range with an exception.
    IpBlock,
    /// A policy whose peers are selected by namespace labels.
    NamespaceSelector,
    /// No policy at all: nothing is governed.
    None,
    /// The pods share the node's network, so the plugin decides for them:
    /// every verdict reads Unknown.
    HostNetwork,
    /// The reads are refused.
    Refused,
    /// The policy list stopped at its limit.
    Cut,
}

impl Shape {
    pub const NAMES: [&'static str; 10] = [
        "mixed",
        "default-deny",
        "open",
        "restricts",
        "ip-block",
        "namespace-selector",
        "none",
        "host-network",
        "refused",
        "cut",
    ];

    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "mixed" => Self::Mixed,
            "default-deny" => Self::DefaultDeny,
            "open" => Self::Open,
            "restricts" => Self::Restricts,
            "ip-block" => Self::IpBlock,
            "namespace-selector" => Self::NamespaceSelector,
            "none" => Self::None,
            "host-network" => Self::HostNetwork,
            "refused" => Self::Refused,
            "cut" => Self::Cut,
            _ => return None,
        })
    }

    /// The policies, by name, this shape lets the panes see.
    fn keeps(self, name: &str) -> bool {
        match self {
            Self::Mixed | Self::HostNetwork | Self::Refused | Self::Cut => true,
            Self::DefaultDeny => name == "default-deny-all",
            Self::Open => name == "open-frontend",
            Self::Restricts => matches!(name, "allow-web-to-api" | "allow-dns"),
            Self::IpBlock => name == "egress-private",
            Self::NamespaceSelector => name == "allow-scrape",
            Self::None => false,
        }
    }
}

fn policy(namespace: &str, name: &str, spec: Value) -> Value {
    json!({
        "apiVersion": "networking.k8s.io/v1",
        "kind": "NetworkPolicy",
        "metadata": {"name": name, "namespace": namespace},
        "spec": spec,
    })
}

fn by_name(name: &str) -> Value {
    json!({"matchLabels": {"kubernetes.io/metadata.name": name}})
}

/// The example cluster's policies.
fn objects() -> Vec<Value> {
    vec![
        policy(
            "payments",
            "default-deny-all",
            json!({"podSelector": {}, "policyTypes": ["Ingress", "Egress"]}),
        ),
        policy(
            "payments",
            "allow-web-to-api",
            json!({
                "podSelector": {"matchLabels": {"app": "api"}},
                "policyTypes": ["Ingress"],
                "ingress": [{
                    "from": [{
                        "namespaceSelector": by_name("web"),
                        "podSelector": {"matchLabels": {"app": "frontend"}},
                    }],
                    "ports": [{"protocol": "TCP", "port": "http"}],
                }],
            }),
        ),
        policy(
            "payments",
            "allow-dns",
            json!({
                "podSelector": {},
                "policyTypes": ["Egress"],
                "egress": [{
                    "to": [{
                        "namespaceSelector": by_name("kube-system"),
                        "podSelector": {"matchLabels": {"k8s-app": "kube-dns"}},
                    }],
                    "ports": [
                        {"protocol": "UDP", "port": 53},
                        {"protocol": "TCP", "port": 53},
                    ],
                }],
            }),
        ),
        policy(
            "web",
            "open-frontend",
            json!({
                "podSelector": {"matchLabels": {"app": "frontend"}},
                "policyTypes": ["Ingress"],
                "ingress": [{}],
            }),
        ),
        policy(
            "web",
            "egress-private",
            json!({
                "podSelector": {"matchLabels": {"app": "gateway"}},
                "policyTypes": ["Egress"],
                "egress": [{
                    "to": [{"ipBlock": {"cidr": "10.0.0.0/8", "except": ["10.1.0.0/16"]}}],
                    "ports": [{"protocol": "TCP", "port": 8000, "endPort": 8100}],
                }],
            }),
        ),
        policy(
            "monitoring",
            "allow-scrape",
            json!({
                "podSelector": {"matchExpressions": [
                    {"key": "app", "operator": "In", "values": ["prometheus", "grafana"]},
                ]},
                "policyTypes": ["Ingress"],
                "ingress": [{
                    "from": [{"namespaceSelector": {}}],
                    "ports": [{"protocol": "TCP", "port": 9090}],
                }],
            }),
        ),
    ]
}

fn name_of(object: &Value) -> (&str, &str) {
    (
        object["metadata"]["namespace"].as_str().unwrap_or_default(),
        object["metadata"]["name"].as_str().unwrap_or_default(),
    )
}

/// Whether a row of this kind, at this position in its UID, is one of these.
pub(super) fn holds(resource: &str, ix: usize) -> bool {
    ix >= FIRST && resource == KEY
}

pub(super) fn columns() -> Vec<ResourceColumn> {
    vec![
        ResourceColumn::new("Name", ColumnKind::Text, false),
        ResourceColumn::new("Pod-Selector", ColumnKind::Text, false),
        ResourceColumn::new("Age", ColumnKind::Age, false),
    ]
}

pub(super) fn rows(connection: &str, now: i64) -> Vec<ResourceRow> {
    objects()
        .iter()
        .enumerate()
        .map(|(ix, object)| {
            let (namespace, name) = name_of(object);
            let facts = facts(object);
            ResourceRow {
                identity: identity(connection, KEY, namespace, name, FIRST + ix),
                cells: vec![name.into(), facts.selector_words(), String::new()],
                created: Some(now - 20 * 86_400 + ix as i64 * 3_600),
                terminating: false,
                resource_version: EXAMPLE_VERSION.into(),
                owner: None,
                generated: None,
                pod: None,
            }
        })
        .collect()
}

/// The object behind a row, with the row's identity.
pub(super) fn document(row: &ResourceRow, ix: usize) -> Option<String> {
    let mut object = objects().into_iter().nth(ix.checked_sub(FIRST)?)?;
    let metadata = object.get_mut("metadata")?.as_object_mut()?;
    metadata.insert("uid".into(), row.identity.uid.clone().into());
    metadata.insert(
        "resourceVersion".into(),
        row.resource_version.clone().into(),
    );
    metadata.insert(
        "creationTimestamp".into(),
        objects::timestamp(row.created.unwrap_or_default()).into(),
    );
    serde_yaml::to_string(&object).ok()
}

fn facts(object: &Value) -> PolicyFacts {
    let policy: NetworkPolicy = serde_json::from_value(object.clone()).unwrap_or_default();
    PolicyFacts::from_policy(&policy)
}

fn refused() -> Failure {
    Failure::new(
        FailureKind::Forbidden,
        "networkpolicies is forbidden for the invented viewer",
    )
}

/// What the Pod and workload panes read: the policies of the pods' namespace
/// that apply to them.
pub fn read_applying(shape: Shape, pods: &PodsOf) -> Result<Applying, Failure> {
    match shape {
        Shape::Refused => Err(refused()),
        Shape::Cut => Err(Failure::new(
            FailureKind::Other,
            "NetworkPolicy list cut at 500",
        )),
        shape => {
            let kept = objects()
                .iter()
                .filter(|object| shape.keeps(name_of(object).1))
                .map(facts)
                .collect::<Vec<_>>();
            // This shape makes the example pods host-network, as a node
            // exporter's are.
            Ok(applying(
                &kept,
                &PodsOf {
                    host_network: pods.host_network || shape == Shape::HostNetwork,
                    ..*pods
                },
            ))
        }
    }
}

/// What a policy's own pane reads: the policy as shown, and its pods, the
/// example pods of its namespace counted by its selector.
pub fn read_policy(
    facts: PolicyFacts,
    shape: Shape,
    context: &str,
    now: i64,
) -> Result<PolicyReading, Failure> {
    let pods = match shape {
        Shape::Refused => Err(Failure::new(
            FailureKind::Forbidden,
            "pods is forbidden for the invented viewer",
        )),
        shape => {
            let rows = super::read(context, "pods", Some(&facts.namespace), now)
                .map(|(_, rows)| rows)
                .unwrap_or_default();
            let mut counted = SelectedPods {
                selected: 0,
                unknown: 0,
                total: rows.len(),
                host_network: 0,
                cut: shape == Shape::Cut,
            };
            for row in &rows {
                let labels = objects::document(&row.identity, now)
                    .map(|document| document.overview.labels.into_iter().collect())
                    .unwrap_or_default();
                match selector_matches(&facts.pod_selector, &labels) {
                    freshkube_core::resources::traffic::netpol::Match::Yes => counted.selected += 1,
                    freshkube_core::resources::traffic::netpol::Match::Unknown => {
                        counted.unknown += 1
                    }
                    freshkube_core::resources::traffic::netpol::Match::No => {}
                }
            }
            Ok(counted)
        }
    };
    Ok(PolicyReading::new(facts, pods))
}
