//! What a forward's port means on its target, and which pod serves it.
//! Pure functions, so they are tested without a server.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::{Pod, PodSpec, ServicePort};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::core::Selector;
use serde_yaml::Value;

use crate::resources::kinds::{ResourceKind, builtin};
use crate::resources::object::{sequence, text};

/// A kind that runs pods from a template and finds them by a selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WorkloadKind {
    Deployment,
    StatefulSet,
    DaemonSet,
    ReplicaSet,
}

impl WorkloadKind {
    pub const ALL: [WorkloadKind; 4] = [
        WorkloadKind::Deployment,
        WorkloadKind::StatefulSet,
        WorkloadKind::DaemonSet,
        WorkloadKind::ReplicaSet,
    ];

    pub fn resource(self) -> ResourceKind {
        let key = match self {
            WorkloadKind::Deployment => "deployments.apps",
            WorkloadKind::StatefulSet => "statefulsets.apps",
            WorkloadKind::DaemonSet => "daemonsets.apps",
            WorkloadKind::ReplicaSet => "replicasets.apps",
        };
        builtin(key).expect("workload kinds are built in")
    }

    /// Its Kubernetes kind, as messages name it.
    pub fn label(self) -> &'static str {
        match self {
            WorkloadKind::Deployment => "Deployment",
            WorkloadKind::StatefulSet => "StatefulSet",
            WorkloadKind::DaemonSet => "DaemonSet",
            WorkloadKind::ReplicaSet => "ReplicaSet",
        }
    }

    pub fn of(kind: &ResourceKind) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|workload| workload.resource() == *kind)
    }
}

/// A port an object declares, as the Ports tab lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclaredPort {
    /// Empty when the port has no name.
    pub name: String,
    /// A container's port, or a Service's own port.
    pub port: u16,
    /// `TCP`, `UDP` or `SCTP`; only TCP can be forwarded.
    pub protocol: String,
    /// The container that declares it; empty for a Service.
    pub container: String,
    /// A Service port's targetPort as written, a number or a name; empty
    /// when unset, which means the Service's port.
    pub target: String,
}

impl DeclaredPort {
    pub fn forwardable(&self) -> bool {
        self.protocol == "TCP"
    }
}

/// The ports an object declares, when its kind can be forwarded: a pod's
/// containers', a workload's pod template's, or a Service's.
pub fn declared_ports(kind: &ResourceKind, object: &Value) -> Option<Vec<DeclaredPort>> {
    let spec = object.get("spec");
    if kind.is_pod() {
        return Some(container_ports(spec));
    }
    if WorkloadKind::of(kind).is_some() {
        let template = spec
            .and_then(|spec| spec.get("template"))
            .and_then(|template| template.get("spec"));
        return Some(container_ports(template));
    }
    if *kind == builtin("services").expect("services are built in") {
        let ports = sequence(spec.and_then(|spec| spec.get("ports")))
            .iter()
            .filter_map(|port| {
                Some(DeclaredPort {
                    name: text(port.get("name")),
                    port: port_number(port.get("port"))?,
                    protocol: protocol(port.get("protocol")),
                    container: String::new(),
                    target: text(port.get("targetPort")),
                })
            })
            .collect();
        return Some(ports);
    }
    None
}

/// The ports of a pod spec's containers, and of its sidecars: init
/// containers that keep running.
fn container_ports(spec: Option<&Value>) -> Vec<DeclaredPort> {
    let sidecars = sequence(spec.and_then(|spec| spec.get("initContainers")))
        .iter()
        .filter(|container| text(container.get("restartPolicy")) == "Always");
    let containers = sequence(spec.and_then(|spec| spec.get("containers")));
    sidecars
        .chain(containers)
        .flat_map(|container| {
            let name = text(container.get("name"));
            sequence(container.get("ports"))
                .iter()
                .filter_map(move |port| {
                    Some(DeclaredPort {
                        name: text(port.get("name")),
                        port: port_number(port.get("containerPort"))?,
                        protocol: protocol(port.get("protocol")),
                        container: name.clone(),
                        target: String::new(),
                    })
                })
        })
        .collect()
}

fn port_number(value: Option<&Value>) -> Option<u16> {
    value
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port > 0)
}

fn protocol(value: Option<&Value>) -> String {
    match text(value) {
        protocol if protocol.is_empty() => "TCP".to_owned(),
        protocol => protocol,
    }
}

/// A Service's selector as a label selector, or `None` when it selects no
/// pods (an ExternalName Service, or one with hand-made endpoints).
pub(crate) fn service_selector(selector: Option<&BTreeMap<String, String>>) -> Option<String> {
    let selector: Selector = selector?.clone().into_iter().collect();
    (!selector.selects_all()).then(|| selector.to_string())
}

/// A workload's selector as a label selector, or `None` when it has none.
///
/// Built by hand rather than with `kube::core::Selector`, which sorts an
/// `In` set's values and so would change the selector text sent.
pub(crate) fn label_selector(selector: Option<&LabelSelector>) -> Option<String> {
    let selector = selector?;
    let mut terms: Vec<String> = selector
        .match_labels
        .iter()
        .flatten()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    for expression in selector.match_expressions.iter().flatten() {
        let values = expression.values.clone().unwrap_or_default().join(",");
        let key = &expression.key;
        terms.push(match expression.operator.as_str() {
            "In" => format!("{key} in ({values})"),
            "NotIn" => format!("{key} notin ({values})"),
            "Exists" => key.clone(),
            "DoesNotExist" => format!("!{key}"),
            // The API server refuses any other operator.
            _ => continue,
        });
    }
    (!terms.is_empty()).then(|| terms.join(","))
}

/// The port to forward to on a pod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RemotePort {
    Number(u16),
    /// A Service's targetPort by name, looked up in each pod's containers.
    Named(String),
}

impl RemotePort {
    /// A Service port's targetPort: a number, a name, or the Service's own
    /// port when unset. A number written as text counts as a number.
    pub(crate) fn of_service(port: &ServicePort) -> Self {
        let own = u16::try_from(port.port).unwrap_or_default();
        match &port.target_port {
            Some(IntOrString::Int(number)) => {
                RemotePort::Number(u16::try_from(*number).unwrap_or(own))
            }
            Some(IntOrString::String(name)) if name.is_empty() => RemotePort::Number(own),
            Some(IntOrString::String(name)) => match name.parse() {
                Ok(number) => RemotePort::Number(number),
                Err(_) => RemotePort::Named(name.clone()),
            },
            None => RemotePort::Number(own),
        }
    }

    /// The port on `pod`, or `None` when it has no container port by that
    /// name.
    pub(crate) fn on(&self, pod: &PodInfo) -> Option<u16> {
        match self {
            RemotePort::Number(port) => Some(*port),
            RemotePort::Named(name) => pod
                .ports
                .iter()
                .find(|(port_name, _)| port_name == name)
                .map(|(_, port)| *port),
        }
    }
}

/// What choosing and tracking a pod needs to know about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PodInfo {
    pub(crate) name: String,
    pub(crate) uid: String,
    pub(crate) phase: String,
    pub(crate) ready: bool,
    /// When it last turned Ready.
    pub(crate) ready_since: Option<DateTime<Utc>>,
    pub(crate) terminating: bool,
    /// Named container ports, for a Service's named targetPort.
    pub(crate) ports: Vec<(String, u16)>,
}

impl PodInfo {
    pub(crate) fn of(pod: &Pod) -> Self {
        let status = pod.status.as_ref();
        let ready = status
            .and_then(|status| status.conditions.as_ref())
            .into_iter()
            .flatten()
            .find(|condition| condition.type_ == "Ready");
        Self {
            name: pod.metadata.name.clone().unwrap_or_default(),
            uid: pod.metadata.uid.clone().unwrap_or_default(),
            phase: status
                .and_then(|status| status.phase.clone())
                .unwrap_or_default(),
            ready: ready.is_some_and(|condition| condition.status == "True"),
            ready_since: ready
                .and_then(|condition| condition.last_transition_time.as_ref())
                .map(|time| time.0),
            terminating: pod.metadata.deletion_timestamp.is_some(),
            ports: pod.spec.as_ref().map(named_ports).unwrap_or_default(),
        }
    }

    /// Ready to take a Service's or workload's connections.
    pub(crate) fn usable(&self) -> bool {
        self.ready && !self.terminating && self.phase == "Running"
    }

    /// The pod finished and runs nothing any more.
    pub(crate) fn stopped(&self) -> bool {
        matches!(self.phase.as_str(), "Succeeded" | "Failed")
    }
}

fn named_ports(spec: &PodSpec) -> Vec<(String, u16)> {
    spec.init_containers
        .iter()
        .flatten()
        .chain(&spec.containers)
        .flat_map(|container| container.ports.iter().flatten())
        .filter_map(|port| {
            Some((
                port.name.clone().filter(|name| !name.is_empty())?,
                u16::try_from(port.container_port).ok()?,
            ))
        })
        .collect()
}

/// The pod a Service or workload forward uses: Ready, not terminating, with
/// the port, Ready the longest, then first by name, so the choice holds
/// while nothing changes. Returns it with the port on it.
pub(crate) fn choose_pod<'a>(
    pods: &'a [PodInfo],
    remote: &RemotePort,
) -> Option<(&'a PodInfo, u16)> {
    pods.iter()
        .filter(|pod| pod.usable())
        .filter_map(|pod| Some((pod, remote.on(pod)?)))
        .min_by(|(a, _), (b, _)| {
            (a.ready_since.is_none(), a.ready_since, &a.name).cmp(&(
                b.ready_since.is_none(),
                b.ready_since,
                &b.name,
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelectorRequirement;
    use serde_json::json;

    fn yaml(value: serde_json::Value) -> Value {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_pod_declares_its_containers_and_sidecars_ports() {
        let pod = yaml(json!({"spec": {
            "initContainers": [
                {"name": "setup", "ports": [{"containerPort": 1}]},
                {"name": "proxy", "restartPolicy": "Always",
                 "ports": [{"name": "admin", "containerPort": 15000}]}],
            "containers": [
                {"name": "db", "ports": [
                    {"name": "mysql", "containerPort": 3306},
                    {"containerPort": 53, "protocol": "UDP"}]},
                {"name": "plain"}]}}));
        let ports = declared_ports(&builtin("pods").unwrap(), &pod).unwrap();
        let summary: Vec<_> = ports
            .iter()
            .map(|port| {
                (
                    port.container.as_str(),
                    port.name.as_str(),
                    port.port,
                    port.forwardable(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("proxy", "admin", 15000, true),
                ("db", "mysql", 3306, true),
                ("db", "", 53, false),
            ],
            "a finished init container serves nothing; UDP is listed but can't forward"
        );
    }

    #[test]
    fn a_workload_declares_its_templates_ports() {
        let deployment = yaml(json!({"spec": {"template": {"spec": {"containers": [
            {"name": "web", "ports": [{"name": "http", "containerPort": 8080}]}]}}}}));
        for workload in WorkloadKind::ALL {
            let ports = declared_ports(&workload.resource(), &deployment).unwrap();
            assert_eq!(ports.len(), 1, "{workload:?}");
            assert_eq!(ports[0].port, 8080);
        }
        assert_eq!(
            declared_ports(&builtin("configmaps").unwrap(), &deployment),
            None,
            "a ConfigMap has no ports"
        );
    }

    #[test]
    fn a_service_declares_its_own_ports_with_their_targets() {
        let service = yaml(json!({"spec": {"ports": [
            {"name": "http", "port": 80, "targetPort": "web"},
            {"port": 443, "targetPort": 8443},
            {"port": 9000}]}}));
        let ports = declared_ports(&builtin("services").unwrap(), &service).unwrap();
        let summary: Vec<_> = ports
            .iter()
            .map(|port| (port.port, port.target.as_str(), port.protocol.as_str()))
            .collect();
        assert_eq!(
            summary,
            vec![(80, "web", "TCP"), (443, "8443", "TCP"), (9000, "", "TCP")]
        );
    }

    #[test]
    fn selectors_become_label_selectors() {
        let labels = BTreeMap::from([
            ("app".to_owned(), "web".to_owned()),
            ("tier".to_owned(), "front".to_owned()),
        ]);
        assert_eq!(
            service_selector(Some(&labels)).as_deref(),
            Some("app=web,tier=front")
        );
        assert_eq!(service_selector(Some(&BTreeMap::new())), None);
        assert_eq!(service_selector(None), None);
        let requirement = |key: &str, operator: &str, values: &[&str]| LabelSelectorRequirement {
            key: key.into(),
            operator: operator.into(),
            values: (!values.is_empty())
                .then(|| values.iter().map(|value| (*value).to_owned()).collect()),
        };
        let selector = LabelSelector {
            match_labels: Some(BTreeMap::from([("app".to_owned(), "db".to_owned())])),
            match_expressions: Some(vec![
                requirement("role", "In", &["primary", "replica"]),
                requirement("zone", "NotIn", &["a"]),
                requirement("ready", "Exists", &[]),
                requirement("legacy", "DoesNotExist", &[]),
            ]),
        };
        assert_eq!(
            label_selector(Some(&selector)).as_deref(),
            Some("app=db,role in (primary,replica),zone notin (a),ready,!legacy")
        );
        assert_eq!(label_selector(Some(&LabelSelector::default())), None);
    }

    fn pod(name: &str, ready_since: Option<&str>, ports: &[(&str, u16)]) -> PodInfo {
        PodInfo {
            name: name.into(),
            uid: format!("uid-{name}"),
            phase: "Running".into(),
            ready: ready_since.is_some(),
            ready_since: ready_since.map(|time| time.parse().unwrap()),
            terminating: false,
            ports: ports
                .iter()
                .map(|(name, port)| ((*name).to_owned(), *port))
                .collect(),
        }
    }

    #[test]
    fn a_service_target_port_is_a_number_a_name_or_its_own_port() {
        let service_port = |target: Option<IntOrString>| ServicePort {
            port: 80,
            target_port: target,
            ..ServicePort::default()
        };
        assert_eq!(
            RemotePort::of_service(&service_port(Some(IntOrString::Int(8080)))),
            RemotePort::Number(8080)
        );
        assert_eq!(
            RemotePort::of_service(&service_port(Some(IntOrString::String("3000".into())))),
            RemotePort::Number(3000)
        );
        assert_eq!(
            RemotePort::of_service(&service_port(None)),
            RemotePort::Number(80)
        );
        let named = RemotePort::of_service(&service_port(Some(IntOrString::String("web".into()))));
        assert_eq!(named, RemotePort::Named("web".into()));
        let with = pod("a", Some("2026-10-02T10:00:00Z"), &[("web", 8081)]);
        let without = pod("b", Some("2026-10-02T10:00:00Z"), &[("admin", 9000)]);
        assert_eq!(named.on(&with), Some(8081));
        assert_eq!(named.on(&without), None);
    }

    #[test]
    fn the_pod_ready_longest_is_chosen() {
        let pods = vec![
            pod("web-c", Some("2026-10-02T09:00:00Z"), &[]),
            pod("web-a", Some("2026-10-02T11:00:00Z"), &[]),
            pod("web-b", Some("2026-10-02T09:00:00Z"), &[]),
            pod("web-d", None, &[]),
        ];
        let (chosen, port) = choose_pod(&pods, &RemotePort::Number(8080)).unwrap();
        assert_eq!(
            (chosen.name.as_str(), port),
            ("web-b", 8080),
            "the earliest Ready, then by name"
        );
        let mut terminating = pods.clone();
        terminating[2].terminating = true;
        terminating[0].phase = "Succeeded".into();
        assert_eq!(
            choose_pod(&terminating, &RemotePort::Number(8080))
                .unwrap()
                .0
                .name,
            "web-a",
            "terminating and finished pods are passed over"
        );
        assert_eq!(
            choose_pod(&pods[3..], &RemotePort::Number(8080)),
            None,
            "none Ready"
        );
        assert_eq!(
            choose_pod(&pods, &RemotePort::Named("web".into())),
            None,
            "none has the named port"
        );
    }

    #[test]
    fn a_pod_reads_its_readiness_and_named_ports() {
        let pod: Pod = serde_json::from_value(json!({
            "metadata": {"name": "web-0", "uid": "u-1",
                "deletionTimestamp": "2026-10-02T12:00:00Z"},
            "spec": {"containers": [{"name": "app", "ports": [
                {"name": "http", "containerPort": 8080}, {"containerPort": 9090}]}]},
            "status": {"phase": "Running", "conditions": [
                {"type": "Ready", "status": "True",
                 "lastTransitionTime": "2026-10-02T10:00:00Z"}]}}))
        .unwrap();
        let info = PodInfo::of(&pod);
        assert_eq!(info.name, "web-0");
        assert!(info.ready);
        assert!(info.terminating);
        assert!(!info.usable(), "terminating");
        assert_eq!(info.ports, vec![("http".to_owned(), 8080)]);
        assert_eq!(
            info.ready_since,
            Some("2026-10-02T10:00:00Z".parse().unwrap())
        );
    }
}
