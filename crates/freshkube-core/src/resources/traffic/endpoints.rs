//! What a Service publishes: its EndpointSlices, or the legacy Endpoints when
//! the slices are not served. This is status; the Service spec is read apart.
use super::service::ServiceFacts;
use k8s_openapi::api::core::v1::{Endpoints, ObjectReference, Pod};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use std::collections::BTreeSet;

/// Which API answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointSource {
    Slices,
    /// The legacy `Endpoints`, read because the slice API was not served or its list was refused.
    Endpoints,
}

impl EndpointSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Slices => "EndpointSlices",
            Self::Endpoints => "Endpoints (legacy)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EndpointState {
    Ready,
    /// Terminating but still serving: finishing its connections.
    Draining,
    NotReady,
    /// Published, but its slice carries no port a Service port maps to, so
    /// nothing reaches it through this Service.
    Unrouted,
}

impl EndpointState {
    pub const ALL: [Self; 4] = [Self::Ready, Self::Draining, Self::NotReady, Self::Unrouted];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Draining => "Draining",
            Self::NotReady => "Not ready",
            Self::Unrouted => "Unrouted",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Endpoint {
    pub address: String,
    pub state: EndpointState,
    pub pod: Option<String>,
    pub pod_uid: Option<String>,
    pub node: Option<String>,
    pub zone: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Published {
    pub source: EndpointSource,
    pub endpoints: Vec<Endpoint>,
    /// How many slices answered (1 for Endpoints).
    pub slices: usize,
    /// The list was cut at its limit, so endpoints may be missing.
    pub truncated: bool,
}

impl Published {
    pub fn count(&self, state: EndpointState) -> usize {
        self.endpoints.iter().filter(|e| e.state == state).count()
    }

    pub fn group(&self, state: EndpointState) -> impl Iterator<Item = &Endpoint> {
        self.endpoints.iter().filter(move |e| e.state == state)
    }

    /// Slices that belong to `service`, matched by the
    /// `kubernetes.io/service-name` label, in the Service's namespace.
    pub fn from_slices(
        service: &ServiceFacts,
        slices: Vec<EndpointSlice>,
        truncated: bool,
    ) -> Self {
        let wanted = service_port_names(service);
        let slices = slices
            .into_iter()
            .filter(|slice| {
                slice
                    .metadata
                    .labels
                    .as_ref()
                    .and_then(|labels| labels.get(SERVICE_LABEL))
                    .is_some_and(|name| *name == service.name)
                    && slice
                        .metadata
                        .namespace
                        .as_deref()
                        .is_none_or(|ns| ns == service.namespace)
            })
            .collect::<Vec<_>>();
        let mut endpoints = Vec::new();
        for slice in &slices {
            let routed = routed(
                slice
                    .ports
                    .iter()
                    .flatten()
                    .map(|port| port_key(port.name.clone(), port.protocol.clone())),
                &wanted,
            );
            for endpoint in &slice.endpoints {
                let conditions = endpoint.conditions.clone().unwrap_or_default();
                // Nil `ready` and `serving` mean true; nil `terminating` false.
                let ready = conditions.ready.unwrap_or(true);
                let terminating = conditions.terminating.unwrap_or(false);
                let serving = conditions.serving.unwrap_or(ready);
                let state = if terminating && serving {
                    EndpointState::Draining
                } else if !ready || terminating {
                    EndpointState::NotReady
                } else if !routed {
                    EndpointState::Unrouted
                } else {
                    EndpointState::Ready
                };
                for address in &endpoint.addresses {
                    endpoints.push(Endpoint {
                        address: address.clone(),
                        state,
                        pod: target_name(&endpoint.target_ref),
                        pod_uid: target_uid(&endpoint.target_ref),
                        node: endpoint.node_name.clone(),
                        zone: endpoint.zone.clone(),
                    });
                }
            }
        }
        Self {
            source: EndpointSource::Slices,
            endpoints: sorted(endpoints),
            slices: slices.len(),
            truncated,
        }
    }

    pub fn from_endpoints(service: &ServiceFacts, endpoints: &Endpoints) -> Self {
        let wanted = service_port_names(service);
        let mut out = Vec::new();
        for subset in endpoints.subsets.iter().flatten() {
            let routed = routed(
                subset
                    .ports
                    .iter()
                    .flatten()
                    .map(|port| port_key(port.name.clone(), port.protocol.clone())),
                &wanted,
            );
            let groups = [
                (&subset.addresses, true),
                (&subset.not_ready_addresses, false),
            ];
            for (addresses, ready) in groups {
                for address in addresses.iter().flatten() {
                    out.push(Endpoint {
                        address: address.ip.clone(),
                        state: match (ready, routed) {
                            (false, _) => EndpointState::NotReady,
                            (true, false) => EndpointState::Unrouted,
                            (true, true) => EndpointState::Ready,
                        },
                        pod: target_name(&address.target_ref),
                        pod_uid: target_uid(&address.target_ref),
                        node: address.node_name.clone(),
                        zone: None,
                    });
                }
            }
        }
        Self {
            source: EndpointSource::Endpoints,
            endpoints: sorted(out),
            slices: 1,
            truncated: false,
        }
    }
}

pub const SERVICE_LABEL: &str = "kubernetes.io/service-name";

fn sorted(mut endpoints: Vec<Endpoint>) -> Vec<Endpoint> {
    endpoints.sort_by(|a, b| a.address.cmp(&b.address).then(a.pod.cmp(&b.pod)));
    endpoints.dedup();
    endpoints
}

fn target_name(target: &Option<ObjectReference>) -> Option<String> {
    target
        .as_ref()
        .filter(|target| target.kind.as_deref().is_none_or(|kind| kind == "Pod"))
        .and_then(|target| target.name.clone())
}

fn target_uid(target: &Option<ObjectReference>) -> Option<String> {
    target.as_ref().and_then(|target| target.uid.clone())
}

/// The Service's port names; empty when it declares no ports.
fn service_port_names(service: &ServiceFacts) -> BTreeSet<(String, String)> {
    service
        .ports
        .iter()
        .map(|port| (port.name.clone(), port.protocol.to_uppercase()))
        .collect()
}

/// A slice or Endpoints port as name and protocol; no protocol means TCP.
fn port_key(name: Option<String>, protocol: Option<String>) -> (String, String) {
    (
        name.unwrap_or_default(),
        protocol.unwrap_or_else(|| "TCP".into()).to_uppercase(),
    )
}

/// Whether a slice's ports reach any Service port. A Service with no ports
/// (a headless one) has nothing to be unrouted from.
fn routed(
    slice_ports: impl Iterator<Item = (String, String)>,
    wanted: &BTreeSet<(String, String)>,
) -> bool {
    wanted.is_empty() || slice_ports.into_iter().any(|name| wanted.contains(&name))
}

/// Why a pod the selector matches is in no endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnpublishedReason {
    Terminating,
    NotReady,
    NoMatchingPort,
    /// Ready and matching, but in none of the slices read: the slice has not
    /// caught up, or the read was cut short.
    OutsideSlice,
}

impl UnpublishedReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Terminating => "Terminating",
            Self::NotReady => "Not ready",
            Self::NoMatchingPort => "No matching port",
            Self::OutsideSlice => "Outside the slices read",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnpublishedPod {
    pub name: String,
    pub reason: UnpublishedReason,
    pub detail: String,
}

/// The pods `service` selects that no endpoint names, with the reason. Pods
/// published are left out; a Service with no selector selects none.
pub fn unpublished_pods(
    service: &ServiceFacts,
    published: &Published,
    pods: &[Pod],
) -> Vec<UnpublishedPod> {
    let mut out = pods
        .iter()
        .filter(|pod| {
            pod.metadata
                .namespace
                .as_deref()
                .is_none_or(|ns| ns == service.namespace)
                && service.selects(pod.metadata.labels.as_ref().unwrap_or(&Default::default()))
        })
        .filter(|pod| !is_published(pod, published))
        .map(|pod| {
            let (reason, detail) = reason_for(service, pod);
            UnpublishedPod {
                name: pod.metadata.name.clone().unwrap_or_default(),
                reason,
                detail,
            }
        })
        .collect::<Vec<_>>();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn is_published(pod: &Pod, published: &Published) -> bool {
    let name = pod.metadata.name.as_deref();
    let uid = pod.metadata.uid.as_deref();
    let ips = pod
        .status
        .iter()
        .flat_map(|status| status.pod_ips.iter().flatten())
        .map(|ip| ip.ip.as_str())
        .chain(pod.status.iter().filter_map(|s| s.pod_ip.as_deref()))
        .collect::<Vec<_>>();
    published
        .endpoints
        .iter()
        .any(|endpoint| match (&endpoint.pod, &endpoint.pod_uid) {
            (Some(target), Some(target_uid)) if Some(target.as_str()) == name => {
                uid.is_none_or(|uid| uid == target_uid)
            }
            (Some(target), None) if Some(target.as_str()) == name => true,
            _ => ips.contains(&endpoint.address.as_str()),
        })
}

fn reason_for(service: &ServiceFacts, pod: &Pod) -> (UnpublishedReason, String) {
    if pod.metadata.deletion_timestamp.is_some() {
        return (
            UnpublishedReason::Terminating,
            "Deletion was requested".into(),
        );
    }
    let status = pod.status.clone().unwrap_or_default();
    let phase = status.phase.clone().unwrap_or_default();
    let ready = status
        .conditions
        .iter()
        .flatten()
        .any(|condition| condition.type_ == "Ready" && condition.status == "True");
    if !ready {
        let detail = if status.pod_ip.is_none() && status.pod_ips.is_none() {
            format!("{} with no IP yet", phase_or_unknown(&phase))
        } else {
            format!("Phase {}, Ready is not True", phase_or_unknown(&phase))
        };
        return (UnpublishedReason::NotReady, detail);
    }
    let declared = pod
        .spec
        .iter()
        .flat_map(|spec| spec.containers.iter())
        .flat_map(|container| container.ports.iter().flatten())
        .filter_map(|port| port.name.as_deref())
        .collect::<BTreeSet<_>>();
    // A numeric targetPort needs no declaration; a named one must exist.
    let missing = service
        .ports
        .iter()
        .find(|port| port.target.parse::<u16>().is_err() && !port.target.is_empty())
        .filter(|port| !declared.contains(port.target.as_str()));
    if let Some(port) = missing {
        return (
            UnpublishedReason::NoMatchingPort,
            format!(
                "No container declares port \"{}\" (Service port {})",
                port.target, port.port
            ),
        );
    }
    (
        UnpublishedReason::OutsideSlice,
        "Ready and matching, but in none of the slices read".into(),
    )
}

fn phase_or_unknown(phase: &str) -> &str {
    if phase.is_empty() { "Unknown" } else { phase }
}
