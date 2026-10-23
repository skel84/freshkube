//! The fields of HTTPRoute, Gateway, GatewayClass and ReferenceGrant that the
//! trace reads, parsed from the objects as the API returned them. Nothing here
//! needs the gateway-api crate or a particular bundle version, so a cluster
//! serving `v1beta1` parses like one serving `v1`. A missing field reads as
//! the API's default, and an unreadable one as absent, never as a failure.
use serde_json::Value;
use std::collections::BTreeMap;

pub const GATEWAY_GROUP: &str = "gateway.networking.k8s.io";

/// A condition's `status`, with anything unexpected read as Unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tri {
    True,
    False,
    Unknown,
}

/// One status condition and the generation it was written for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cond {
    pub kind: String,
    pub status: Tri,
    pub reason: String,
    pub message: String,
    pub generation: Option<i64>,
}

impl Cond {
    /// What the controller said, in one line.
    pub fn says(&self) -> String {
        match (self.reason.is_empty(), self.message.is_empty()) {
            (true, true) => self.kind.clone(),
            (false, true) => format!("{}: {}", self.kind, self.reason),
            (true, false) => format!("{}: {}", self.kind, self.message),
            (false, false) => format!("{}: {} — {}", self.kind, self.reason, self.message),
        }
    }
}

pub(super) fn conditions(value: &Value) -> Vec<Cond> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some(Cond {
                kind: c["type"].as_str()?.to_owned(),
                status: match c["status"].as_str() {
                    Some("True") => Tri::True,
                    Some("False") => Tri::False,
                    _ => Tri::Unknown,
                },
                reason: text(&c["reason"]),
                message: text(&c["message"]),
                generation: c["observedGeneration"].as_i64(),
            })
        })
        .collect()
}

pub(super) fn find<'a>(conditions: &'a [Cond], kind: &str) -> Option<&'a Cond> {
    conditions.iter().find(|c| c.kind == kind)
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

fn opt(value: &Value) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

fn port(value: &Value) -> Option<i32> {
    value.as_i64().and_then(|p| i32::try_from(p).ok())
}

/// A route's `parentRefs` entry, or the same reference as a controller wrote
/// it into `status.parents`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParentRef {
    pub group: String,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
    pub section: Option<String>,
    pub port: Option<i32>,
}

impl ParentRef {
    fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            group: value["group"]
                .as_str()
                .unwrap_or(GATEWAY_GROUP)
                .to_owned(),
            kind: value["kind"].as_str().unwrap_or("Gateway").to_owned(),
            namespace: opt(&value["namespace"]),
            name: opt(&value["name"])?,
            section: opt(&value["sectionName"]),
            port: port(&value["port"]),
        })
    }

    /// A Gateway; anything else (a Service, for a mesh) is not judged.
    pub fn is_gateway(&self) -> bool {
        self.group == GATEWAY_GROUP && self.kind == "Gateway"
    }

    /// The parent's namespace, which a route's own namespace stands in for.
    pub fn namespace_or<'a>(&'a self, route: &'a str) -> &'a str {
        self.namespace.as_deref().unwrap_or(route)
    }

    /// `namespace/name`, with the listener when the route names one.
    pub fn label(&self, route: &str) -> String {
        let base = format!("{}/{}", self.namespace_or(route), self.name);
        match (&self.section, self.port) {
            (Some(section), _) => format!("{base} · {section}"),
            (None, Some(port)) => format!("{base} · :{port}"),
            (None, None) => base,
        }
    }

    pub(super) fn same_as(&self, other: &Self, route: &str) -> bool {
        self.group == other.group
            && self.kind == other.kind
            && self.name == other.name
            && self.namespace_or(route) == other.namespace_or(route)
            && self.section == other.section
            && self.port == other.port
    }
}

/// One controller's verdict on one parent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParentStatus {
    pub parent: ParentRef,
    pub controller: String,
    pub conditions: Vec<Cond>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendRef {
    pub group: String,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
    pub port: Option<i32>,
    /// `weight`, defaulting to 1 as the API does.
    pub weight: i64,
}

impl BackendRef {
    pub fn is_service(&self) -> bool {
        self.group.is_empty() && self.kind == "Service"
    }
}

/// One rule: where it matches, what it sends traffic to, and the filters that
/// stand in for a backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Each path match as `Type value`; `PathPrefix /` when none is written.
    pub paths: Vec<String>,
    pub backends: Vec<BackendRef>,
    /// A `RequestRedirect` filter answers without a backend.
    pub redirect: bool,
    /// An `ExtensionRef` filter hands the request to something the trace
    /// does not read.
    pub extension: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteFacts {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub generation: i64,
    pub hostnames: Vec<String>,
    pub parents: Vec<ParentRef>,
    pub rules: Vec<Rule>,
    pub status: Vec<ParentStatus>,
}

impl RouteFacts {
    pub fn parse(object: &Value) -> Option<Self> {
        let meta = &object["metadata"];
        let spec = &object["spec"];
        let rules = spec["rules"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|rule| {
                let filters = |filters: &Value, kind: &str| {
                    filters
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|f| f["type"].as_str() == Some(kind))
                };
                let mut paths = rule["matches"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|m| {
                        let path = &m["path"];
                        Some(format!(
                            "{} {}",
                            path["type"].as_str().unwrap_or("PathPrefix"),
                            path["value"].as_str()?
                        ))
                    })
                    .collect::<Vec<_>>();
                if paths.is_empty() {
                    paths.push("PathPrefix /".into());
                }
                Rule {
                    paths,
                    backends: rule["backendRefs"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|b| {
                            Some(BackendRef {
                                group: text(&b["group"]),
                                kind: b["kind"].as_str().unwrap_or("Service").to_owned(),
                                namespace: opt(&b["namespace"]),
                                name: opt(&b["name"])?,
                                port: port(&b["port"]),
                                weight: b["weight"].as_i64().unwrap_or(1),
                            })
                        })
                        .collect(),
                    redirect: filters(&rule["filters"], "RequestRedirect"),
                    extension: filters(&rule["filters"], "ExtensionRef"),
                }
            })
            .collect();
        Some(Self {
            namespace: text(&meta["namespace"]),
            name: opt(&meta["name"])?,
            uid: text(&meta["uid"]),
            generation: meta["generation"].as_i64().unwrap_or(0),
            hostnames: strings(&spec["hostnames"]),
            parents: spec["parentRefs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(ParentRef::parse)
                .collect(),
            rules,
            status: object["status"]["parents"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|entry| {
                    Some(ParentStatus {
                        parent: ParentRef::parse(&entry["parentRef"])?,
                        controller: text(&entry["controllerName"]),
                        conditions: conditions(&entry["conditions"]),
                    })
                })
                .collect(),
        })
    }

    /// `host/path` for a title: the first hostname (`*` when none) and the
    /// first path match.
    pub fn label(&self) -> String {
        let host = self.hostnames.first().map_or("*", String::as_str);
        let path = self
            .rules
            .first()
            .and_then(|rule| rule.paths.first())
            .and_then(|path| path.split_once(' ').map(|(_, value)| value))
            .unwrap_or("/");
        format!("{host}{path}")
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

/// Which namespaces may attach routes to a listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AllowedNamespaces {
    Same,
    All,
    Selector(Selector),
}

/// A label selector, as `allowedRoutes.namespaces.selector` writes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selector {
    pub labels: BTreeMap<String, String>,
    /// `(key, operator, values)`.
    pub expressions: Vec<(String, String, Vec<String>)>,
}

impl Selector {
    fn parse(value: &Value) -> Self {
        Self {
            labels: value["matchLabels"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                .collect(),
            expressions: value["matchExpressions"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|e| (text(&e["key"]), text(&e["operator"]), strings(&e["values"])))
                .collect(),
        }
    }

    pub fn matches(&self, labels: &BTreeMap<String, String>) -> bool {
        self.labels.iter().all(|(k, v)| labels.get(k) == Some(v))
            && self
                .expressions
                .iter()
                .all(|(key, op, values)| match op.as_str() {
                    "In" => labels.get(key).is_some_and(|v| values.contains(v)),
                    "NotIn" => labels.get(key).is_none_or(|v| !values.contains(v)),
                    "Exists" => labels.contains_key(key),
                    "DoesNotExist" => !labels.contains_key(key),
                    _ => false,
                })
    }

    pub fn describe(&self) -> String {
        let mut parts = self
            .labels
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>();
        parts.extend(
            self.expressions
                .iter()
                .map(|(k, op, values)| format!("{k} {op} ({})", values.join(", "))),
        );
        if parts.is_empty() {
            "every namespace".into()
        } else {
            parts.join(", ")
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listener {
    pub name: String,
    pub port: i32,
    pub protocol: String,
    pub hostname: Option<String>,
    pub namespaces: AllowedNamespaces,
    /// `allowedRoutes.kinds`; none means the protocol's default kinds.
    pub kinds: Option<Vec<String>>,
    /// `tls.certificateRefs`, by name alone: no Secret is read.
    pub certificates: Vec<String>,
}

/// What the controller reported for one listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListenerStatus {
    pub name: String,
    pub attached: i64,
    pub conditions: Vec<Cond>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayFacts {
    pub namespace: String,
    pub name: String,
    pub generation: i64,
    pub class: String,
    pub listeners: Vec<Listener>,
    pub conditions: Vec<Cond>,
    pub addresses: Vec<String>,
    pub listener_status: Vec<ListenerStatus>,
}

impl GatewayFacts {
    pub fn parse(object: &Value) -> Option<Self> {
        let meta = &object["metadata"];
        let status = &object["status"];
        Some(Self {
            namespace: text(&meta["namespace"]),
            name: opt(&meta["name"])?,
            generation: meta["generation"].as_i64().unwrap_or(0),
            class: text(&object["spec"]["gatewayClassName"]),
            listeners: object["spec"]["listeners"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|l| {
                    let allowed = &l["allowedRoutes"];
                    let from = &allowed["namespaces"];
                    Some(Listener {
                        name: opt(&l["name"])?,
                        port: port(&l["port"]).unwrap_or(0),
                        protocol: text(&l["protocol"]),
                        hostname: opt(&l["hostname"]),
                        namespaces: match from["from"].as_str() {
                            Some("All") => AllowedNamespaces::All,
                            Some("Selector") => {
                                AllowedNamespaces::Selector(Selector::parse(&from["selector"]))
                            }
                            _ => AllowedNamespaces::Same,
                        },
                        kinds: allowed["kinds"].as_array().map(|kinds| {
                            kinds.iter().map(|k| text(&k["kind"])).collect::<Vec<_>>()
                        }),
                        certificates: l["tls"]["certificateRefs"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|c| opt(&c["name"]))
                            .collect(),
                    })
                })
                .collect(),
            conditions: conditions(&status["conditions"]),
            addresses: status["addresses"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| opt(&a["value"]))
                .collect(),
            listener_status: status["listeners"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|l| {
                    Some(ListenerStatus {
                        name: opt(&l["name"])?,
                        attached: l["attachedRoutes"].as_i64().unwrap_or(0),
                        conditions: conditions(&l["conditions"]),
                    })
                })
                .collect(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassFacts {
    pub name: String,
    pub controller: String,
    pub conditions: Vec<Cond>,
}

impl ClassFacts {
    pub fn parse(object: &Value) -> Option<Self> {
        Some(Self {
            name: opt(&object["metadata"]["name"])?,
            controller: text(&object["spec"]["controllerName"]),
            conditions: conditions(&object["status"]["conditions"]),
        })
    }
}

/// A ReferenceGrant: which kinds in which namespaces may refer to which
/// kinds in its own namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantFacts {
    pub namespace: String,
    pub name: String,
    /// `(group, kind, namespace)`.
    pub from: Vec<(String, String, String)>,
    /// `(group, kind, name)`; no name grants every object of the kind.
    pub to: Vec<(String, String, Option<String>)>,
}

impl GrantFacts {
    pub fn parse(object: &Value) -> Option<Self> {
        let spec = &object["spec"];
        Some(Self {
            namespace: text(&object["metadata"]["namespace"]),
            name: opt(&object["metadata"]["name"])?,
            from: spec["from"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|f| (text(&f["group"]), text(&f["kind"]), text(&f["namespace"])))
                .collect(),
            to: spec["to"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|t| (text(&t["group"]), text(&t["kind"]), opt(&t["name"])))
                .collect(),
        })
    }

    /// Whether this grant lets an HTTPRoute in `from_namespace` refer to the
    /// Service `service` in the grant's own namespace.
    pub fn allows(&self, from_namespace: &str, service: &str) -> bool {
        self.from
            .iter()
            .any(|(g, k, n)| g == GATEWAY_GROUP && k == "HTTPRoute" && n == from_namespace)
            && self
                .to
                .iter()
                .any(|(g, k, n)| g.is_empty() && k == "Service" && n.as_deref().is_none_or(|n| n == service))
    }
}
