//! The Gateway topology (#949, N2b): boxes for the address, each Gateway, its
//! routes, their backends and the workloads behind them, and the calls
//! between them, from the read and the traces N2a made. No UI types, and no
//! verdict of its own: a box and a call read as the trace says, and a read
//! that was refused is a dashed box that says Unknown, never an empty column.
//!
//! The only read besides N2a's is the workloads: a backend's pods, resolved
//! to the workload that controls them ([`read_workloads`], two bounded lists
//! per namespace).
use super::gateways::{GatewayEntry, GatewayState, GatewaysView};
use super::read::RoutesRead;
use super::trace::{BranchKind, BranchVerdict, RouteVerdict, StepId};
use crate::failure::{Failure, FailureKind};
use crate::resources::object::{JSON, json_get};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Namespaces one read looks for workloads in; the rest read Unknown.
pub const WORKLOAD_NAMESPACES: usize = 25;
/// Pods and ReplicaSets read per namespace.
pub const WORKLOAD_LIMIT: u32 = 500;
/// Boxes one topology draws; Gateways past it are counted in a note.
pub const MAX_BOXES: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Address,
    Gateway,
    Route,
    Backend,
    Workload,
}

impl NodeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Address => "Address",
            Self::Gateway => "Gateway",
            Self::Route => "Route",
            Self::Backend => "Backend",
            Self::Workload => "Workload",
        }
    }
}

/// How a box or a call reads. `Unknown` is never green.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Good,
    Warn,
    Bad,
    Unknown,
    /// Nothing is wrong or right: not judged.
    Neutral,
}

/// What a box opens in Resources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub kind: &'static str,
    pub namespace: String,
    pub name: String,
    /// The object's UID, when the read had one: a link carrying it won't
    /// open an object that replaced this one. Empty when none is known.
    pub uid: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Node {
    pub key: String,
    pub kind: NodeKind,
    pub label: String,
    pub detail: String,
    pub health: Health,
    /// Something that should be there and is not, or was not read.
    pub ghost: bool,
    /// Why it reads as it does, for a tooltip and the inspector.
    pub words: String,
    pub target: Option<Target>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub health: Health,
    /// A listener, a share of traffic or nothing.
    pub label: String,
    pub words: String,
}

#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Topology {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// What was left out or could not be read, in words.
    pub notes: Vec<String>,
}

/// A workload a Service's pods belong to.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WorkloadRef {
    /// `Deployment`, `StatefulSet`, `DaemonSet`, `Job`, `ReplicaSet` or
    /// `Pod` for a pod nothing controls.
    pub kind: String,
    pub namespace: String,
    pub name: String,
    /// The controlling owner's UID, as the pod's owner reference names it.
    pub uid: String,
}

/// The workloads behind each backend Service, by `(namespace, name)`. A
/// Service missing from the map was not looked at.
pub type WorkloadRead = BTreeMap<(String, String), Result<Vec<WorkloadRef>, Failure>>;

fn kind_target(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "Deployment" => "Deployment",
        "StatefulSet" => "StatefulSet",
        "DaemonSet" => "DaemonSet",
        "ReplicaSet" => "ReplicaSet",
        "Job" => "Job",
        "Pod" => "Pod",
        _ => return None,
    })
}

fn health(verdict: RouteVerdict) -> Health {
    match verdict {
        RouteVerdict::NotServing => Health::Bad,
        RouteVerdict::Partly => Health::Warn,
        RouteVerdict::Unknown => Health::Unknown,
        RouteVerdict::Serving => Health::Good,
        RouteVerdict::Mesh => Health::Neutral,
    }
}

fn branch_health(verdict: BranchVerdict) -> Health {
    match verdict {
        BranchVerdict::Serving => Health::Good,
        BranchVerdict::Broken => Health::Bad,
        BranchVerdict::Unknown => Health::Unknown,
        BranchVerdict::Excluded => Health::Neutral,
    }
}

fn gateway_health(state: GatewayState) -> Health {
    match state {
        GatewayState::Failed | GatewayState::Missing => Health::Bad,
        GatewayState::Partly => Health::Warn,
        GatewayState::Pending | GatewayState::Blind | GatewayState::Unknown => Health::Unknown,
        GatewayState::Serving => Health::Good,
    }
}

/// Worse first.
fn rank(health: Health) -> u8 {
    match health {
        Health::Bad => 0,
        Health::Warn => 1,
        Health::Unknown => 2,
        Health::Neutral => 3,
        Health::Good => 4,
    }
}

#[derive(Default)]
struct Builder {
    nodes: Vec<Node>,
    index: BTreeMap<String, usize>,
    edges: Vec<Edge>,
    edge_index: BTreeMap<(String, String), usize>,
}

impl Builder {
    /// A box; one that several routes reach keeps the worse health and is
    /// dashed if any of them says it is.
    fn node(&mut self, node: Node) {
        match self.index.get(&node.key) {
            Some(&ix) => {
                let have = &mut self.nodes[ix];
                have.ghost |= node.ghost;
                if rank(node.health) < rank(have.health) {
                    have.health = node.health;
                    have.words = node.words;
                }
            }
            None => {
                self.index.insert(node.key.clone(), self.nodes.len());
                self.nodes.push(node);
            }
        }
    }

    /// A call; the same pair twice keeps the worse health.
    fn edge(&mut self, edge: Edge) {
        let key = (edge.from.clone(), edge.to.clone());
        match self.edge_index.get(&key) {
            Some(&ix) => {
                if rank(edge.health) < rank(self.edges[ix].health) {
                    self.edges[ix] = edge;
                }
            }
            None => {
                self.edge_index.insert(key, self.edges.len());
                self.edges.push(edge);
            }
        }
    }
}

fn gateway_node(entry: &GatewayEntry) -> Node {
    let facts = &entry.facts;
    Node {
        key: format!("gateway:{}", entry.key()),
        kind: NodeKind::Gateway,
        label: facts.name.clone(),
        detail: format!("{} · {}", facts.namespace, entry.state.label()),
        health: gateway_health(entry.state),
        ghost: entry.state == GatewayState::Missing,
        words: entry.reason.clone(),
        target: (entry.state != GatewayState::Missing).then(|| Target {
            kind: "Gateway",
            namespace: facts.namespace.clone(),
            name: facts.name.clone(),
            uid: facts.uid.clone(),
        }),
    }
}

fn workload_nodes(
    builder: &mut Builder,
    service_key: &str,
    service: &(String, String),
    health: Health,
    workloads: &WorkloadRead,
) {
    let ghost = |label: &str, words: String| Node {
        key: format!("workloads:{}/{}", service.0, service.1),
        kind: NodeKind::Workload,
        label: label.to_owned(),
        detail: service.0.clone(),
        health: Health::Unknown,
        ghost: true,
        words,
        target: None,
    };
    let (node, call) = match workloads.get(service) {
        None => (
            Some(ghost(
                "workloads not read",
                "the workloads behind this Service were not read".into(),
            )),
            "not read".to_owned(),
        ),
        Some(Err(failure)) => (
            Some(ghost(
                "workloads unknown",
                format!(
                    "the pods could not be read ({}: {})",
                    failure.kind.label(),
                    failure.message
                ),
            )),
            "unknown".to_owned(),
        ),
        Some(Ok(list)) if list.is_empty() => (
            Some(ghost(
                "no pods published",
                "the Service publishes no pod, so no workload stands behind it".into(),
            )),
            "no pods".to_owned(),
        ),
        Some(Ok(list)) => {
            for workload in list {
                builder.node(Node {
                    key: format!(
                        "workload:{}/{}/{}",
                        workload.kind, workload.namespace, workload.name
                    ),
                    kind: NodeKind::Workload,
                    label: workload.name.clone(),
                    detail: format!("{} · {}", workload.kind, workload.namespace),
                    health: Health::Neutral,
                    ghost: false,
                    words: format!(
                        "{} {}/{} controls pods this Service publishes",
                        workload.kind, workload.namespace, workload.name
                    ),
                    target: kind_target(&workload.kind).map(|kind| Target {
                        kind,
                        namespace: workload.namespace.clone(),
                        name: workload.name.clone(),
                        uid: workload.uid.clone(),
                    }),
                });
                builder.edge(Edge {
                    from: service_key.to_owned(),
                    to: format!(
                        "workload:{}/{}/{}",
                        workload.kind, workload.namespace, workload.name
                    ),
                    health,
                    label: String::new(),
                    words: "its pods are published by the Service".into(),
                });
            }
            (None, String::new())
        }
    };
    if let Some(node) = node {
        let key = node.key.clone();
        builder.node(node);
        builder.edge(Edge {
            from: service_key.to_owned(),
            to: key,
            health: Health::Unknown,
            label: call,
            words: "what stands behind the Service is not known".into(),
        });
    }
}

impl Topology {
    /// The topology of one read: an error when the Gateways were refused.
    pub fn new(read: &RoutesRead, workloads: &WorkloadRead) -> Result<Self, Failure> {
        let view = GatewaysView::new(read);
        let gateways = view.gateways?;
        let mut builder = Builder::default();
        let mut notes = Vec::new();
        let mut left_out = 0;
        let mut routes_left = 0;
        for entry in &gateways {
            if builder.nodes.len() >= MAX_BOXES {
                left_out += 1;
                continue;
            }
            let gateway = gateway_node(entry);
            let gateway_key = gateway.key.clone();
            // The address, or a ghost that says there is none.
            // A Gateway that does not exist has one too, so it sits in the
            // Gateways' column with the others, not in the address's.
            {
                let (key, label, words, health, ghost) = match entry.address() {
                    Some(address) => (
                        format!("address:{address}"),
                        address.clone(),
                        format!("the address the controller reports: {address}"),
                        Health::Good,
                        false,
                    ),
                    None => (
                        format!("address:none:{}", entry.key()),
                        "no address".to_owned(),
                        if entry.state == GatewayState::Missing {
                            "no Gateway of this name exists, so it has no address".to_owned()
                        } else {
                            "the controller reports no address for this Gateway".to_owned()
                        },
                        Health::Unknown,
                        true,
                    ),
                };
                builder.node(Node {
                    key: key.clone(),
                    kind: NodeKind::Address,
                    label: label.clone(),
                    detail: "address".into(),
                    health,
                    ghost,
                    words,
                    target: None,
                });
                builder.edge(Edge {
                    from: key,
                    to: gateway_key.clone(),
                    health: if ghost { Health::Unknown } else { Health::Good },
                    label: String::new(),
                    words: "the Gateway is reached at this address".into(),
                });
            }
            builder.node(gateway);
            for attached in &entry.routes {
                // The cap holds inside a Gateway too; a route's own boxes are
                // drawn whole.
                if builder.nodes.len() >= MAX_BOXES {
                    routes_left += 1;
                    continue;
                }
                let Some(route) = read.routes.iter().find(|r| {
                    r.facts.namespace == attached.namespace && r.facts.name == attached.name
                }) else {
                    continue;
                };
                let route_key = format!("route:{}/{}", attached.namespace, attached.name);
                builder.node(Node {
                    key: route_key.clone(),
                    kind: NodeKind::Route,
                    label: attached.label.clone(),
                    detail: format!(
                        "{}/{} · {}",
                        attached.namespace,
                        attached.name,
                        route.trace.verdict.label()
                    ),
                    health: health(route.trace.verdict),
                    ghost: false,
                    words: attached.line.clone(),
                    target: Some(Target {
                        kind: "HTTPRoute",
                        namespace: attached.namespace.clone(),
                        name: attached.name.clone(),
                        uid: route.facts.uid.clone(),
                    }),
                });
                let listener = route
                    .facts
                    .parents
                    .iter()
                    .find(|p| {
                        p.is_gateway()
                            && p.name == entry.facts.name
                            && p.namespace_or(&route.facts.namespace) == entry.facts.namespace
                    })
                    .and_then(|p| p.section.clone())
                    .unwrap_or_default();
                builder.edge(Edge {
                    from: gateway_key.clone(),
                    to: route_key.clone(),
                    health: health(attached.verdict),
                    label: listener,
                    words: attached.line.clone(),
                });
                let parents = route.trace.parents.iter().filter(|p| {
                    p.parent.is_gateway()
                        && p.parent.name == entry.facts.name
                        && p.parent.namespace_or(&route.facts.namespace) == entry.facts.namespace
                });
                for (ix, branch) in parents.flat_map(|p| &p.branches).enumerate() {
                    let share = branch.share.map(|s| format!("{s}%")).unwrap_or_default();
                    let (key, label, detail, target) = match (&branch.service, branch.kind) {
                        (Some((namespace, name)), _) => {
                            let ready = branch
                                .steps
                                .iter()
                                .find(|s| s.id == StepId::Endpoints)
                                .map(|s| {
                                    if s.serves.is_empty() {
                                        s.reason.clone()
                                    } else {
                                        s.serves.clone()
                                    }
                                })
                                .unwrap_or_else(|| branch.label.clone());
                            (
                                format!("service:{namespace}/{name}"),
                                name.clone(),
                                format!("{namespace} · {ready}"),
                                Some(Target {
                                    kind: "Service",
                                    namespace: namespace.clone(),
                                    name: name.clone(),
                                    uid: match read.services.get(&(namespace.clone(), name.clone()))
                                    {
                                        Some(Ok(Some(reading))) => reading.facts.uid.clone(),
                                        _ => String::new(),
                                    },
                                }),
                            )
                        }
                        _ => (
                            format!(
                                "branch:{}/{}:{ix}:{}",
                                attached.namespace, attached.name, branch.label
                            ),
                            branch.label.clone(),
                            match branch.kind {
                                BranchKind::Redirect => "answered by the Gateway".into(),
                                BranchKind::Extension => "an extension, not judged".into(),
                                BranchKind::Empty => "nothing to send traffic to".into(),
                                _ => "not a Service, not judged".to_owned(),
                            },
                            None,
                        ),
                    };
                    let health = branch_health(branch.verdict);
                    let words = branch
                        .steps
                        .iter()
                        .find(|s| s.verdict != super::trace::StepVerdict::Pass)
                        .map(|s| format!("{}: {}", s.id.label(), s.reason))
                        .unwrap_or_else(|| "every step passes".into());
                    builder.node(Node {
                        key: key.clone(),
                        kind: NodeKind::Backend,
                        label,
                        detail,
                        health,
                        ghost: matches!(branch.kind, BranchKind::Empty)
                            || (branch.service.is_some()
                                && branch.verdict == BranchVerdict::Broken
                                && branch.steps.iter().any(|s| {
                                    s.id == StepId::Service
                                        && s.verdict == super::trace::StepVerdict::Fail
                                })),
                        words: words.clone(),
                        target,
                    });
                    builder.edge(Edge {
                        from: route_key.clone(),
                        to: key.clone(),
                        health,
                        label: share,
                        words,
                    });
                    if let Some(service) = &branch.service
                        && branch.verdict != BranchVerdict::Excluded
                        && matches!(read.services.get(service), Some(Ok(Some(_))))
                    {
                        workload_nodes(&mut builder, &key, service, health, workloads);
                    }
                }
            }
        }
        let mesh = read
            .routes
            .iter()
            .filter(|r| r.trace.verdict == RouteVerdict::Mesh)
            .count();
        if mesh > 0 {
            notes.push(format!(
                "{mesh} mesh {} (a Service as parent) {} not drawn: no Gateway stands in front of {}",
                if mesh == 1 { "route" } else { "routes" },
                if mesh == 1 { "is" } else { "are" },
                if mesh == 1 { "it" } else { "them" },
            ));
        }
        let unread = read
            .services
            .keys()
            .filter(|key| matches!(workloads.get(*key), Some(Err(_))))
            .count();
        if unread > 0 {
            notes.push(format!(
                "The workloads behind {unread} {} could not be read, so {} read Unknown",
                if unread == 1 { "Service" } else { "Services" },
                if unread == 1 { "it" } else { "they" },
            ));
        }
        if routes_left > 0 {
            notes.push(format!(
                "{routes_left} {} not drawn: a topology draws at most {MAX_BOXES} boxes",
                if routes_left == 1 {
                    "route is"
                } else {
                    "routes are"
                },
            ));
        }
        if left_out > 0 {
            notes.push(format!(
                "{left_out} more {} not drawn: a topology draws at most {MAX_BOXES} boxes",
                if left_out == 1 {
                    "Gateway is"
                } else {
                    "Gateways are"
                },
            ));
        }
        if view.gateways_cut {
            notes.push("Gateways were cut at their limit; more may exist".into());
        }
        if read.routes_cut {
            notes.push("Routes were cut at their limit; more may exist".into());
        }
        if read.backends_cut > 0 {
            notes.push(format!(
                "{} backend Services past the limit were not read",
                read.backends_cut
            ));
        }
        Ok(Self {
            nodes: builder.nodes,
            edges: builder.edges,
            notes,
        })
    }

    pub fn node(&self, key: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.key == key)
    }
}

/// An object's controlling owner as `(kind, name, uid)`.
fn controller(object: &Value) -> Option<(String, String, String)> {
    object["metadata"]["ownerReferences"]
        .as_array()?
        .iter()
        .find(|owner| owner["controller"].as_bool() == Some(true))
        .and_then(|owner| {
            Some((
                owner["kind"].as_str()?.to_owned(),
                owner["name"].as_str()?.to_owned(),
                owner["uid"].as_str().unwrap_or_default().to_owned(),
            ))
        })
}

/// Whether the pods or the ReplicaSets of a namespace were cut at their limit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListCuts {
    pub pods: bool,
    pub replica_sets: bool,
}

/// Resolves pod names to workloads from the pods and ReplicaSets of one
/// namespace. A pod nothing controls is a `Pod` of its own and a ReplicaSet
/// with no owner stays a ReplicaSet. A pod, or a ReplicaSet, that a list cut
/// at its limit doesn't have is not known: the answer is an error, never a
/// bare `Pod` box that would pass for the truth.
pub fn resolve_workloads(
    namespace: &str,
    pod_names: &[String],
    pods: &[Value],
    replica_sets: &[Value],
    cut: ListCuts,
) -> Result<Vec<WorkloadRef>, Failure> {
    type Owner = Option<(String, String, String)>;
    let owners: BTreeMap<&str, (Owner, String)> = pods
        .iter()
        .filter_map(|pod| {
            Some((
                pod["metadata"]["name"].as_str()?,
                (
                    controller(pod),
                    pod["metadata"]["uid"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                ),
            ))
        })
        .collect();
    let sets: BTreeMap<&str, Owner> = replica_sets
        .iter()
        .filter_map(|set| Some((set["metadata"]["name"].as_str()?, controller(set))))
        .collect();
    let unknown = |what: &str, name: &str, limit: &str| {
        Failure::new(
            FailureKind::Other,
            format!(
                "the {limit} list was cut at {WORKLOAD_LIMIT} and {what} {name} is not in it, so its workload is not known"
            ),
        )
    };
    let mut found = BTreeSet::new();
    for pod in pod_names {
        let (kind, name, uid) = match owners.get(pod.as_str()) {
            None if cut.pods => return Err(unknown("pod", pod, "pod")),
            None => ("Pod".to_owned(), pod.clone(), String::new()),
            Some((Some(owner), _)) => owner.clone(),
            Some((None, uid)) => ("Pod".to_owned(), pod.clone(), uid.clone()),
        };
        let (kind, name, uid) = match (kind.as_str(), sets.get(name.as_str())) {
            ("ReplicaSet", None) if cut.replica_sets => {
                return Err(unknown("ReplicaSet", &name, "ReplicaSet"));
            }
            ("ReplicaSet", Some(Some(owner))) => owner.clone(),
            _ => (kind, name, uid),
        };
        found.insert(WorkloadRef {
            kind,
            namespace: namespace.to_owned(),
            name,
            uid,
        });
    }
    Ok(found.into_iter().collect())
}

async fn list_items(client: &kube::Client, path: String) -> Result<(Vec<Value>, bool), Failure> {
    let list: Value = client
        .request(json_get(format!("{path}?limit={WORKLOAD_LIMIT}"), JSON)?)
        .await
        .map_err(Failure::from_kube)?;
    let cut = list["metadata"]["continue"]
        .as_str()
        .is_some_and(|token| !token.is_empty());
    Ok((list["items"].as_array().cloned().unwrap_or_default(), cut))
}

/// The workloads behind every backend Service the read found: its published
/// pods, resolved through the pods' and ReplicaSets' controlling owners. Two
/// lists per namespace, bounded; a namespace past the cap, or a refused list,
/// is an error for its Services (Unknown), never an empty answer.
pub async fn read_workloads(client: &kube::Client, read: &RoutesRead) -> WorkloadRead {
    let mut by_namespace: BTreeMap<String, Vec<(String, Vec<String>)>> = BTreeMap::new();
    let mut result = WorkloadRead::new();
    for ((namespace, name), reading) in &read.services {
        let Ok(Some(reading)) = reading else {
            continue;
        };
        match &reading.published {
            Ok(published) => {
                let pods = published
                    .endpoints
                    .iter()
                    .filter_map(|e| e.pod.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                by_namespace
                    .entry(namespace.clone())
                    .or_default()
                    .push((name.clone(), pods));
            }
            Err(failure) => {
                result.insert((namespace.clone(), name.clone()), Err(failure.clone()));
            }
        }
    }
    let past = by_namespace.len().saturating_sub(WORKLOAD_NAMESPACES);
    let names = by_namespace.keys().cloned().collect::<Vec<_>>();
    for namespace in names.iter().skip(WORKLOAD_NAMESPACES) {
        for (name, _) in &by_namespace[namespace] {
            result.insert(
                (namespace.clone(), name.clone()),
                Err(Failure::new(
                    FailureKind::Other,
                    format!("{past} namespaces past {WORKLOAD_NAMESPACES} were not read"),
                )),
            );
        }
    }
    let reads = names
        .iter()
        .take(WORKLOAD_NAMESPACES)
        .map(|namespace| async move {
            let pods = list_items(client, format!("/api/v1/namespaces/{namespace}/pods"));
            let sets = list_items(
                client,
                format!("/apis/apps/v1/namespaces/{namespace}/replicasets"),
            );
            let (pods, sets) = futures::join!(pods, sets);
            (namespace.clone(), pods, sets)
        });
    for (namespace, pods, sets) in futures::future::join_all(reads).await {
        for (name, pod_names) in &by_namespace[&namespace] {
            let answer = match (&pods, &sets) {
                (Ok((pods, pods_cut)), Ok((sets, sets_cut))) => resolve_workloads(
                    &namespace,
                    pod_names,
                    pods,
                    sets,
                    ListCuts {
                        pods: *pods_cut,
                        replica_sets: *sets_cut,
                    },
                ),
                (Err(failure), _) | (_, Err(failure)) => Err(failure.clone()),
            };
            result.insert((namespace.clone(), name.clone()), answer);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(health: Health, ghost: bool, words: &str) -> Node {
        Node {
            key: "service:shop/cart".into(),
            kind: NodeKind::Backend,
            label: "cart".into(),
            detail: String::new(),
            health,
            ghost,
            words: words.into(),
            target: None,
        }
    }

    #[test]
    fn a_box_several_routes_reach_keeps_the_worse_health_and_any_dashed_flag() {
        let mut builder = Builder::default();
        builder.node(node(Health::Good, false, "serving"));
        builder.node(node(Health::Bad, true, "broken for the second route"));
        builder.node(node(Health::Warn, false, "warning"));
        assert_eq!(builder.nodes.len(), 1);
        let kept = &builder.nodes[0];
        assert_eq!(kept.health, Health::Bad);
        assert!(kept.ghost);
        assert_eq!(kept.words, "broken for the second route");
        // The other way round, the first being the worse.
        let mut builder = Builder::default();
        builder.node(node(Health::Unknown, false, "unknown"));
        builder.node(node(Health::Good, false, "serving"));
        assert_eq!(builder.nodes[0].health, Health::Unknown);
    }
}
