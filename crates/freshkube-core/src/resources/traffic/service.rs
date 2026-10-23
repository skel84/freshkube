//! A Service's configuration: what its spec asks for, kept apart from what
//! the slices say it publishes.
use k8s_openapi::api::core::v1::Service;
use std::collections::BTreeMap;

/// The Service's `spec.type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceKind {
    ClusterIp,
    NodePort,
    LoadBalancer,
    ExternalName,
}

impl ServiceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::ClusterIp => "ClusterIP",
            Self::NodePort => "NodePort",
            Self::LoadBalancer => "LoadBalancer",
            Self::ExternalName => "ExternalName",
        }
    }
}

/// What the Service's `status.loadBalancer` says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadBalancerState {
    /// The Service is not a LoadBalancer.
    NotApplicable,
    /// A LoadBalancer with no ingress address yet.
    Pending,
    Addresses(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServicePort {
    pub name: String,
    pub port: i32,
    pub protocol: String,
    /// `targetPort` as written: a number or a container port's name. Empty
    /// means the Service's own port.
    pub target: String,
}

/// How a Service finds its endpoints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceShape {
    /// A selector: the slice controller manages the slices.
    Selector,
    /// No selector: its slices or Endpoints are managed by hand or by another
    /// controller. Read as what it is, never as broken.
    NoSelector,
    /// A DNS alias; nothing is published.
    ExternalName(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServiceFacts {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub kind: ServiceKind,
    pub headless: bool,
    pub cluster_ip: String,
    pub selector: BTreeMap<String, String>,
    pub ports: Vec<ServicePort>,
    pub external_name: Option<String>,
    pub load_balancer: LoadBalancerState,
}

impl ServiceFacts {
    pub fn from_service(service: &Service) -> Self {
        let spec = service.spec.clone().unwrap_or_default();
        let kind = match spec.type_.as_deref() {
            Some("NodePort") => ServiceKind::NodePort,
            Some("LoadBalancer") => ServiceKind::LoadBalancer,
            Some("ExternalName") => ServiceKind::ExternalName,
            _ => ServiceKind::ClusterIp,
        };
        let cluster_ip = spec.cluster_ip.unwrap_or_default();
        let load_balancer = if kind == ServiceKind::LoadBalancer {
            let ingress = service
                .status
                .as_ref()
                .and_then(|status| status.load_balancer.as_ref())
                .and_then(|balancer| balancer.ingress.clone())
                .unwrap_or_default();
            let addresses = ingress
                .into_iter()
                .filter_map(|entry| entry.ip.or(entry.hostname))
                .collect::<Vec<_>>();
            if addresses.is_empty() {
                LoadBalancerState::Pending
            } else {
                LoadBalancerState::Addresses(addresses)
            }
        } else {
            LoadBalancerState::NotApplicable
        };
        Self {
            namespace: service.metadata.namespace.clone().unwrap_or_default(),
            name: service.metadata.name.clone().unwrap_or_default(),
            uid: service.metadata.uid.clone().unwrap_or_default(),
            kind,
            headless: cluster_ip == "None",
            cluster_ip,
            selector: spec.selector.unwrap_or_default(),
            ports: spec
                .ports
                .unwrap_or_default()
                .into_iter()
                .map(|port| ServicePort {
                    name: port.name.unwrap_or_default(),
                    port: port.port,
                    protocol: port.protocol.unwrap_or_else(|| "TCP".into()),
                    target: match port.target_port {
                        Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(
                            number,
                        )) => number.to_string(),
                        Some(
                            k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::String(name),
                        ) => name,
                        None => String::new(),
                    },
                })
                .collect(),
            external_name: spec
                .external_name
                .filter(|_| kind == ServiceKind::ExternalName),
            load_balancer,
        }
    }

    pub fn shape(&self) -> ServiceShape {
        match (&self.external_name, self.selector.is_empty()) {
            (Some(target), _) => ServiceShape::ExternalName(target.clone()),
            (None, true) => ServiceShape::NoSelector,
            (None, false) => ServiceShape::Selector,
        }
    }

    /// Whether the selector picks `labels`: nonempty, and every entry matches.
    pub fn selects(&self, labels: &BTreeMap<String, String>) -> bool {
        !self.selector.is_empty()
            && self
                .selector
                .iter()
                .all(|(key, value)| labels.get(key) == Some(value))
    }

    /// The selector as the API's `labelSelector` query.
    pub fn selector_query(&self) -> String {
        self.selector
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}
