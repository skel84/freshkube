//! The HTTPRoute trace: for each parent, the ordered steps from the class
//! down to the endpoints, stopping at the first break. It is a pure function
//! of what was read; every input is a `Result`, so a refused or unserved read
//! is Unknown with its reason and never "not serving".
//!
//! Configuration (what the route asks for and what the gateway is set to
//! serve) and the controller's status are separate lines of one step and are
//! never merged: a step can pass on configuration and fail on what the
//! controller reports.
use super::facts::*;
use super::grant::{GrantProposal, proposal};
use crate::failure::Failure;
use crate::resources::traffic::{EndpointState, ServiceKind, ServiceReading, ServiceShape};
use std::collections::BTreeMap;

/// The ordered steps. The first four are the parent's; the last three are
/// each backend's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepId {
    Class,
    Gateway,
    Listener,
    Namespace,
    Refs,
    Service,
    Endpoints,
}

impl StepId {
    pub fn label(self) -> &'static str {
        match self {
            Self::Class => "Class claimed",
            Self::Gateway => "Gateway programmed",
            Self::Listener => "Listener accepts",
            Self::Namespace => "Namespace allowed",
            Self::Refs => "References resolve",
            Self::Service => "Service has the port",
            Self::Endpoints => "Endpoints ready",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepVerdict {
    Pass,
    /// Passes, with something to know: a stale verdict, some endpoints down.
    Warn,
    Fail,
    /// The read was refused or not served, or the controller says nothing.
    Unknown,
    /// Nothing to judge here: a zero weight, a hand-managed alias.
    Neutral,
    /// An earlier step broke, so this one was not looked at.
    NotReached,
}

/// Whose problem a step is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The platform: the Gateway, its class and what it allows.
    Platform,
    /// Yours: the route, or the Service and the workload behind it.
    Yours,
    /// The controller that implements the Gateway.
    Controller,
    /// This machine's path; only a probe says this (a later slice).
    ThisMachine,
}

impl Owner {
    pub fn label(self) -> &'static str {
        match self {
            Self::Platform => "Platform",
            Self::Yours => "Yours",
            Self::Controller => "Controller",
            Self::ThisMachine => "This machine",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TraceStep {
    pub id: StepId,
    pub verdict: StepVerdict,
    /// Set when the step did not pass.
    pub owner: Option<Owner>,
    /// What the route asks for.
    pub asks: String,
    /// What the gateway, Service or endpoints serve.
    pub serves: String,
    /// Why the step reads as it does.
    pub reason: String,
    /// What the controller reports, kept apart from the two above.
    pub controller: Option<String>,
    /// The verdict was written for an older generation than the object has.
    pub stale: bool,
}

impl TraceStep {
    fn new(id: StepId, verdict: StepVerdict, reason: impl Into<String>) -> Self {
        Self {
            id,
            verdict,
            owner: None,
            asks: String::new(),
            serves: String::new(),
            reason: reason.into(),
            controller: None,
            stale: false,
        }
    }

    fn pass(id: StepId, reason: impl Into<String>) -> Self {
        Self::new(id, StepVerdict::Pass, reason)
    }

    fn fail(id: StepId, owner: Owner, reason: impl Into<String>) -> Self {
        Self {
            owner: Some(owner),
            ..Self::new(id, StepVerdict::Fail, reason)
        }
    }

    fn unknown(id: StepId, owner: Owner, reason: impl Into<String>) -> Self {
        Self {
            owner: Some(owner),
            ..Self::new(id, StepVerdict::Unknown, reason)
        }
    }

    fn quoting(mut self, asks: impl Into<String>, serves: impl Into<String>) -> Self {
        self.asks = asks.into();
        self.serves = serves.into();
        self
    }

    fn reported(mut self, controller: Option<String>) -> Self {
        self.controller = controller;
        self
    }

    fn not_reached(id: StepId) -> Self {
        Self::new(id, StepVerdict::NotReached, "not reached")
    }

    fn breaks(&self) -> bool {
        matches!(self.verdict, StepVerdict::Fail | StepVerdict::Unknown)
    }
}

/// A list read, and whether it stopped at its limit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing<T> {
    pub items: Vec<T>,
    pub cut: bool,
}

impl<T> Listing<T> {
    pub fn new(items: Vec<T>, cut: bool) -> Self {
        Self { items, cut }
    }
}

/// A backend Service as read: not found is an answer (`None`).
pub type BackendRead = Result<Option<ServiceReading>, Failure>;

/// Everything a trace reads besides the route. Each is its own `Result`, so
/// one refused read makes only the steps that need it Unknown.
pub struct Inputs<'a> {
    pub gateways: &'a Result<Listing<GatewayFacts>, Failure>,
    pub classes: &'a Result<Listing<ClassFacts>, Failure>,
    pub grants: &'a Result<Listing<GrantFacts>, Failure>,
    /// Namespace labels, read only for a listener that selects by label.
    pub namespaces: &'a Result<Listing<(String, BTreeMap<String, String>)>, Failure>,
    pub backends: &'a BTreeMap<(String, String), BackendRead>,
    /// The ReferenceGrant version the cluster serves, for a proposal.
    pub grant_version: &'a str,
}

/// How a backend (or rule) leaves the gateway.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchKind {
    /// A Service the trace follows to its endpoints.
    Service,
    /// Answered by the gateway itself; no backend is needed.
    Redirect,
    /// Handed to an extension the trace does not read.
    Extension,
    /// A backend that is not a Service.
    Other,
    /// A rule with nothing to send traffic to.
    Empty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchVerdict {
    Serving,
    Broken,
    Unknown,
    /// Takes no traffic (weight 0).
    Excluded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Branch {
    pub kind: BranchKind,
    /// `namespace/name:port`, or what stands in for a backend.
    pub label: String,
    /// The rule's path, for telling branches of different rules apart.
    pub rule: String,
    /// Share of the rule's traffic, in percent; none for a single backend.
    pub share: Option<u32>,
    pub weight: i64,
    /// Steps Refs, Service and Endpoints; empty when none applies.
    pub steps: Vec<TraceStep>,
    pub verdict: BranchVerdict,
    /// The Service this branch ends at, for a link to its pods.
    pub service: Option<(String, String)>,
    pub proposal: Option<GrantProposal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RouteVerdict {
    NotServing,
    Partly,
    Unknown,
    Serving,
    Mesh,
}

impl RouteVerdict {
    pub const ALL: [Self; 5] = [
        Self::NotServing,
        Self::Partly,
        Self::Unknown,
        Self::Serving,
        Self::Mesh,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::NotServing => "Not serving",
            Self::Partly => "Partly serving",
            Self::Unknown => "Unknown",
            Self::Serving => "Serving",
            Self::Mesh => "Mesh",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ParentTrace {
    pub parent: ParentRef,
    /// `namespace/name` of the parent, and its listener when named.
    pub label: String,
    /// The controller whose verdicts this trace quotes, when it wrote any.
    pub controller: Option<String>,
    /// Steps Class to Namespace.
    pub steps: Vec<TraceStep>,
    pub branches: Vec<Branch>,
    pub verdict: RouteVerdict,
}

/// The one line a Routes row reads: where it stops and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub stops_at: Option<&'static str>,
    pub reason: String,
    pub owner: Option<Owner>,
}

impl Summary {
    /// `stops at X — reason`, or the reason alone when nothing stops.
    pub fn line(&self) -> String {
        match self.stops_at {
            Some(at) => format!("stops at {at} — {}", self.reason),
            None => self.reason.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RouteTrace {
    pub parents: Vec<ParentTrace>,
    pub verdict: RouteVerdict,
    pub summary: Summary,
    /// A verdict of the controller is older than the route or gateway.
    pub stale: bool,
}

impl RouteTrace {
    pub fn proposals(&self) -> Vec<&GrantProposal> {
        let mut found: Vec<&GrantProposal> = Vec::new();
        for proposal in self
            .parents
            .iter()
            .flat_map(|p| &p.branches)
            .filter_map(|b| b.proposal.as_ref())
        {
            if !found.contains(&proposal) {
                found.push(proposal);
            }
        }
        found
    }
}

/// Which side of the platform a controller's reason points at.
fn blame(reason: &str) -> Owner {
    match reason {
        "InvalidCertificateRef" | "InvalidRouteKinds" | "RefNotPermitted" | "BackendNotFound"
        | "InvalidKind" | "UnsupportedValue" | "NoMatchingListenerHostname"
        | "NoMatchingParent" | "UnsupportedProtocol" | "InvalidParameters" => Owner::Yours,
        "NotAllowedByListeners" | "AddressNotAssigned" | "AddressNotUsable" | "NoResources"
        | "ListenersNotValid" | "PortUnavailable" | "HostnameConflict" | "ProtocolConflict"
        | "UnsupportedAddress" => Owner::Platform,
        _ => Owner::Controller,
    }
}

/// Wildcards as the API writes them: `*.example.com` covers any label below.
fn hosts_overlap(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let suffix = |wild: &str, host: &str| {
        wild.strip_prefix('*')
            .is_some_and(|rest| host.len() > rest.len() && host.ends_with(rest))
    };
    suffix(a, b) || suffix(b, a)
}

fn quote_hosts(hosts: &[String]) -> String {
    if hosts.is_empty() {
        "any host".into()
    } else {
        hosts.join(", ")
    }
}

/// Runs steps in order and stops at the first break.
struct Chain {
    steps: Vec<TraceStep>,
    stopped: bool,
}

impl Chain {
    fn new() -> Self {
        Self {
            steps: Vec::new(),
            stopped: false,
        }
    }

    fn run(&mut self, id: StepId, step: impl FnOnce() -> TraceStep) {
        if self.stopped {
            self.steps.push(TraceStep::not_reached(id));
            return;
        }
        let step = step();
        self.stopped = step.breaks();
        self.steps.push(step);
    }
}

fn generation_note(cond: &Cond, now: i64) -> Option<String> {
    cond.generation
        .filter(|seen| *seen < now)
        .map(|seen| format!("stale: written for generation {seen}, now {now}"))
}

fn class_step(gateway: &GatewayFacts, inputs: &Inputs) -> TraceStep {
    let id = StepId::Class;
    let asks = format!("Gateway {} uses class {}", gateway.name, gateway.class);
    let classes = match inputs.classes {
        Ok(classes) => classes,
        Err(failure) => {
            return TraceStep::unknown(
                id,
                Owner::Platform,
                format!("GatewayClasses unreadable ({}: {})", failure.kind.label(), failure.message),
            )
            .quoting(asks, "unknown");
        }
    };
    let Some(class) = classes.items.iter().find(|c| c.name == gateway.class) else {
        if classes.cut {
            return TraceStep::unknown(
                id,
                Owner::Platform,
                "the GatewayClass list was cut; the class may exist",
            )
            .quoting(asks, "unknown");
        }
        return TraceStep::fail(
            id,
            Owner::Yours,
            format!("GatewayClass {} does not exist", gateway.class),
        )
        .quoting(asks, "no such class");
    };
    let serves = format!("controller {}", class.controller);
    match find(&class.conditions, "Accepted") {
        None => TraceStep::unknown(
            id,
            Owner::Controller,
            "blind: the class has no status, so no controller has claimed it",
        )
        .quoting(asks, serves),
        Some(cond) if cond.status == Tri::True => {
            TraceStep::pass(id, format!("claimed by {}", class.controller))
                .quoting(asks, serves)
                .reported(Some(cond.says()))
        }
        Some(cond) if cond.status == Tri::False => TraceStep::fail(
            id,
            Owner::Platform,
            format!("{} did not accept the class", class.controller),
        )
        .quoting(asks, serves)
        .reported(Some(cond.says())),
        Some(cond) => {
            TraceStep::unknown(id, Owner::Controller, "pending: the class is not accepted yet")
                .quoting(asks, serves)
                .reported(Some(cond.says()))
        }
    }
}

fn gateway_step(gateway: &GatewayFacts) -> TraceStep {
    let id = StepId::Gateway;
    let asks = format!("Gateway {}/{}", gateway.namespace, gateway.name);
    if gateway.conditions.is_empty() {
        return TraceStep::unknown(
            id,
            Owner::Controller,
            "blind: the controller writes no status for this Gateway",
        )
        .quoting(asks, "no status");
    }
    let accepted = find(&gateway.conditions, "Accepted");
    let programmed = find(&gateway.conditions, "Programmed");
    let serves = if gateway.addresses.is_empty() {
        "no address".to_owned()
    } else {
        format!("address {}", gateway.addresses.join(", "))
    };
    if let Some(cond) = accepted.filter(|c| c.status == Tri::False) {
        return TraceStep::fail(id, blame(&cond.reason), "the Gateway is not accepted")
            .quoting(asks, serves)
            .reported(Some(cond.says()));
    }
    let Some(programmed) = programmed else {
        return TraceStep::unknown(
            id,
            Owner::Controller,
            "pending: the controller has not reported Programmed",
        )
        .quoting(asks, serves);
    };
    match programmed.status {
        Tri::False => TraceStep::fail(id, blame(&programmed.reason), "the Gateway is not programmed")
            .quoting(asks, serves)
            .reported(Some(programmed.says())),
        Tri::Unknown => TraceStep::unknown(id, Owner::Controller, "pending: not programmed yet")
            .quoting(asks, serves)
            .reported(Some(programmed.says())),
        Tri::True if gateway.addresses.is_empty() => TraceStep::unknown(
            id,
            Owner::Platform,
            "blind: programmed, but no address is reported",
        )
        .quoting(asks, serves)
        .reported(Some(programmed.says())),
        Tri::True => {
            let note = generation_note(programmed, gateway.generation);
            let mut step = TraceStep::pass(id, "programmed with an address")
                .quoting(asks, serves)
                .reported(Some(programmed.says()));
            if let Some(note) = note {
                step.verdict = StepVerdict::Warn;
                step.stale = true;
                step.reason = note;
                step.owner = Some(Owner::Controller);
            }
            step
        }
    }
}

/// Why no listener fits, or the one that does.
enum Pick<'a> {
    Found(&'a Listener),
    Miss(String),
}

fn pick_listener<'a>(gateway: &'a GatewayFacts, parent: &ParentRef, route: &RouteFacts) -> Pick<'a> {
    let mut candidates = gateway
        .listeners
        .iter()
        .filter(|l| parent.section.as_deref().is_none_or(|s| s == l.name))
        .filter(|l| parent.port.is_none_or(|p| p == l.port))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Pick::Miss(match (&parent.section, parent.port) {
            (Some(section), _) => format!("the Gateway has no listener named {section}"),
            (None, Some(port)) => format!("the Gateway has no listener on port {port}"),
            (None, None) => "the Gateway has no listeners".into(),
        });
    }
    candidates.retain(|l| matches!(l.protocol.as_str(), "HTTP" | "HTTPS"));
    if candidates.is_empty() {
        let protocols = gateway
            .listeners
            .iter()
            .map(|l| l.protocol.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Pick::Miss(format!(
            "no HTTP or HTTPS listener to attach an HTTPRoute to (listeners speak {protocols})"
        ));
    }
    candidates.retain(|l| l.kinds.as_ref().is_none_or(|k| k.iter().any(|k| k == "HTTPRoute")));
    if candidates.is_empty() {
        return Pick::Miss("the listener does not allow HTTPRoute".into());
    }
    let matching = candidates
        .iter()
        .copied()
        .filter(|l| {
            route.hostnames.is_empty()
                || l.hostname
                    .as_deref()
                    .is_none_or(|h| route.hostnames.iter().any(|r| hosts_overlap(r, h)))
        })
        .collect::<Vec<_>>();
    if matching.is_empty() {
        let served = candidates
            .iter()
            .map(|l| l.hostname.as_deref().unwrap_or("any host"))
            .collect::<Vec<_>>()
            .join(", ");
        return Pick::Miss(format!(
            "route hostnames {} match no listener hostname ({served})",
            quote_hosts(&route.hostnames)
        ));
    }
    // Of the listeners that fit, prefer one the controller accepted.
    let accepted = |l: &Listener| {
        gateway
            .listener_status
            .iter()
            .find(|s| s.name == l.name)
            .is_some_and(|s| {
                find(&s.conditions, "Accepted").is_some_and(|c| c.status == Tri::True)
            })
    };
    let best = matching
        .iter()
        .copied()
        .find(|l| accepted(l))
        .unwrap_or(matching[0]);
    Pick::Found(best)
}

fn describe_listener(l: &Listener) -> String {
    format!(
        "listener {} {} :{} · {}",
        l.name,
        l.protocol,
        l.port,
        l.hostname.as_deref().unwrap_or("any host")
    )
}

fn route_condition<'a>(status: Option<&'a ParentStatus>, kind: &str) -> Option<&'a Cond> {
    status.and_then(|s| find(&s.conditions, kind))
}

fn listener_step(
    gateway: &GatewayFacts,
    parent: &ParentRef,
    route: &RouteFacts,
    status: Option<&ParentStatus>,
) -> (TraceStep, Option<Listener>) {
    let id = StepId::Listener;
    let asks = format!(
        "{} on {}",
        quote_hosts(&route.hostnames),
        parent.section.as_deref().unwrap_or("any listener")
    );
    let listener = match pick_listener(gateway, parent, route) {
        Pick::Found(listener) => listener,
        Pick::Miss(reason) => {
            let served = gateway
                .listeners
                .iter()
                .map(describe_listener)
                .collect::<Vec<_>>()
                .join("; ");
            let controller = route_condition(status, "Accepted")
                .filter(|c| c.status == Tri::False)
                .map(Cond::says);
            return (
                TraceStep::fail(id, Owner::Yours, reason)
                    .quoting(asks, if served.is_empty() { "no listeners".into() } else { served })
                    .reported(controller),
                None,
            );
        }
    };
    let serves = describe_listener(listener);
    let reported = gateway.listener_status.iter().find(|s| s.name == listener.name);
    let step = match reported {
        None if !gateway.listener_status.is_empty() || !gateway.conditions.is_empty() => {
            TraceStep::unknown(
                id,
                Owner::Controller,
                format!("the controller reports nothing for listener {}", listener.name),
            )
        }
        None => TraceStep::pass(id, format!("listener {} fits by configuration", listener.name)),
        Some(reported) => {
            let bad = ["Accepted", "Programmed", "ResolvedRefs", "Conflicted"]
                .into_iter()
                .filter_map(|kind| find(&reported.conditions, kind))
                .find(|c| match c.kind.as_str() {
                    "Conflicted" => c.status == Tri::True,
                    _ => c.status == Tri::False,
                });
            match bad {
                Some(cond) => TraceStep::fail(
                    id,
                    blame(&cond.reason),
                    format!("listener {} is not accepted", listener.name),
                )
                .reported(Some(cond.says())),
                None => TraceStep::pass(id, format!("listener {} accepts HTTPRoute", listener.name))
                    .reported(find(&reported.conditions, "Accepted").map(Cond::says)),
            }
        }
    };
    (step.quoting(asks, serves), Some(listener.clone()))
}

fn namespace_step(
    gateway: &GatewayFacts,
    listener: &Listener,
    route: &RouteFacts,
    status: Option<&ParentStatus>,
    inputs: &Inputs,
) -> TraceStep {
    let id = StepId::Namespace;
    let asks = format!("route in namespace {}", route.namespace);
    let mut step = match &listener.namespaces {
        AllowedNamespaces::All => {
            TraceStep::pass(id, "the listener accepts routes from every namespace")
                .quoting(asks, "from All")
        }
        AllowedNamespaces::Same => {
            let serves = format!("from Same ({})", gateway.namespace);
            if route.namespace == gateway.namespace {
                TraceStep::pass(id, "same namespace as the Gateway").quoting(asks, serves)
            } else {
                TraceStep::fail(
                    id,
                    Owner::Platform,
                    format!(
                        "the listener accepts only its own namespace, {}",
                        gateway.namespace
                    ),
                )
                .quoting(asks, serves)
            }
        }
        AllowedNamespaces::Selector(selector) => {
            let serves = format!("from Selector ({})", selector.describe());
            match inputs.namespaces {
                Err(failure) => TraceStep::unknown(
                    id,
                    Owner::Platform,
                    format!(
                        "namespaces unreadable ({}: {}), so the selector can't be checked",
                        failure.kind.label(),
                        failure.message
                    ),
                )
                .quoting(asks, serves),
                Ok(namespaces) => {
                    match namespaces.items.iter().find(|(name, _)| *name == route.namespace) {
                        Some((_, labels)) if selector.matches(labels) => {
                            TraceStep::pass(id, "the namespace's labels match the selector")
                                .quoting(asks, serves)
                        }
                        Some(_) => TraceStep::fail(
                            id,
                            Owner::Platform,
                            "the namespace's labels do not match the listener's selector",
                        )
                        .quoting(asks, serves),
                        None if namespaces.cut => TraceStep::unknown(
                            id,
                            Owner::Platform,
                            "the namespace list was cut; the labels are unknown",
                        )
                        .quoting(asks, serves),
                        None => TraceStep::unknown(
                            id,
                            Owner::Platform,
                            "the route's namespace was not in the list",
                        )
                        .quoting(asks, serves),
                    }
                }
            }
        }
    };
    // The controller's own verdict on the route, kept as its own line.
    let accepted = route_condition(status, "Accepted");
    step.controller = accepted.map(Cond::says);
    if step.verdict == StepVerdict::Fail {
        return step;
    }
    match (status, accepted) {
        (None, _) => {
            step.verdict = StepVerdict::Unknown;
            step.owner = Some(Owner::Controller);
            step.reason = "blind: the controller reports nothing for this route on this parent".into();
        }
        (Some(_), None) => {}
        (Some(_), Some(cond)) if cond.status == Tri::False => {
            step.verdict = StepVerdict::Fail;
            step.owner = Some(blame(&cond.reason));
            step.reason = "the controller does not accept the route".into();
        }
        (Some(_), Some(cond)) if cond.status == Tri::Unknown => {
            step.verdict = StepVerdict::Unknown;
            step.owner = Some(Owner::Controller);
            step.reason = "pending: the controller has not decided yet".into();
        }
        (Some(_), Some(cond)) => {
            if let Some(note) = generation_note(cond, route.generation) {
                step.verdict = StepVerdict::Warn;
                step.stale = true;
                step.owner = Some(Owner::Controller);
                step.reason = note;
            }
        }
    }
    step
}

fn dns_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 253
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

fn refs_step(
    route: &RouteFacts,
    backend: &BackendRef,
    namespace: &str,
    status: Option<&ParentStatus>,
    inputs: &Inputs,
) -> (TraceStep, Option<GrantProposal>) {
    let id = StepId::Refs;
    let asks = format!("Service {namespace}/{} from {}", backend.name, route.namespace);
    let refused = route_condition(status, "ResolvedRefs")
        .filter(|c| c.status == Tri::False && c.reason == "RefNotPermitted")
        .map(Cond::says);
    if namespace == route.namespace {
        return (
            TraceStep::pass(id, "same namespace, no grant needed")
                .quoting(asks, "same namespace")
                .reported(refused),
            None,
        );
    }
    let grants = match inputs.grants {
        Ok(grants) => grants,
        Err(failure) => {
            return (
                TraceStep::unknown(
                    id,
                    Owner::Platform,
                    format!(
                        "ReferenceGrants unreadable ({}: {}), so the reference can't be judged",
                        failure.kind.label(),
                        failure.message
                    ),
                )
                .quoting(asks, "unknown")
                .reported(refused),
                None,
            );
        }
    };
    let granted = grants
        .items
        .iter()
        .find(|g| g.namespace == namespace && g.allows(&route.namespace, &backend.name));
    match granted {
        Some(grant) => {
            let step = TraceStep::pass(id, format!("granted by {}/{}", grant.namespace, grant.name))
                .quoting(asks, format!("ReferenceGrant {}/{}", grant.namespace, grant.name));
            if refused.is_some() {
                let mut step = step;
                step.verdict = StepVerdict::Fail;
                step.owner = Some(Owner::Controller);
                step.reason = "a grant exists, but the controller still refuses the reference".into();
                step.controller = refused;
                (step, None)
            } else {
                (step, None)
            }
        }
        None if grants.cut => (
            TraceStep::unknown(
                id,
                Owner::Platform,
                "the ReferenceGrant list was cut; a grant may exist",
            )
            .quoting(asks, "unknown")
            .reported(refused),
            None,
        ),
        None => {
            let grant = (dns_name(&route.namespace) && dns_name(namespace) && dns_name(&backend.name))
                .then(|| {
                    proposal(inputs.grant_version, &route.namespace, namespace, &backend.name)
                });
            (
                TraceStep::fail(
                    id,
                    Owner::Platform,
                    format!(
                        "no ReferenceGrant in {namespace} lets HTTPRoutes in {} refer to Service {}",
                        route.namespace, backend.name
                    ),
                )
                .quoting(asks, "no grant")
                .reported(refused),
                grant,
            )
        }
    }
}

fn service_step(
    backend: &BackendRef,
    namespace: &str,
    read: Option<&BackendRead>,
    status: Option<&ParentStatus>,
) -> TraceStep {
    let id = StepId::Service;
    let asks = match backend.port {
        Some(port) => format!("Service {namespace}/{} port {port}", backend.name),
        None => format!("Service {namespace}/{}, no port", backend.name),
    };
    let missing = route_condition(status, "ResolvedRefs")
        .filter(|c| c.status == Tri::False && matches!(c.reason.as_str(), "BackendNotFound" | "InvalidKind"))
        .map(Cond::says);
    let reading = match read {
        None => {
            return TraceStep::unknown(
                id,
                Owner::Yours,
                "not read: more distinct backends than one read follows",
            )
            .quoting(asks, "unknown");
        }
        Some(Err(failure)) => {
            return TraceStep::unknown(
                id,
                Owner::Yours,
                format!("the Service could not be read ({}: {})", failure.kind.label(), failure.message),
            )
            .quoting(asks, "unknown")
            .reported(missing);
        }
        Some(Ok(None)) => {
            return TraceStep::fail(id, Owner::Yours, format!("Service {namespace}/{} does not exist", backend.name))
                .quoting(asks, "no such Service")
                .reported(missing);
        }
        Some(Ok(Some(reading))) => reading,
    };
    let facts = &reading.facts;
    let ports = facts.ports.iter().map(|p| p.port.to_string()).collect::<Vec<_>>().join(", ");
    let serves = format!("ports {}", if ports.is_empty() { "none".into() } else { ports });
    if let ServiceShape::ExternalName(target) = facts.shape() {
        return TraceStep::new(
            id,
            StepVerdict::Neutral,
            format!("ExternalName alias of {target}: traffic leaves the cluster and is not judged"),
        )
        .quoting(asks, serves);
    }
    match backend.port {
        None => TraceStep::fail(id, Owner::Yours, "a Service backend must name a port")
            .quoting(asks, serves)
            .reported(missing),
        Some(port) if facts.ports.iter().any(|p| p.port == port) => {
            TraceStep::pass(id, format!("the Service exposes port {port}"))
                .quoting(asks, serves)
                .reported(missing)
        }
        Some(port) => TraceStep::fail(
            id,
            Owner::Yours,
            format!("the Service does not expose port {port}"),
        )
        .quoting(asks, serves)
        .reported(missing),
    }
}

fn endpoints_step(backend: &BackendRef, namespace: &str, reading: &ServiceReading) -> TraceStep {
    let id = StepId::Endpoints;
    let asks = format!("ready endpoints for {namespace}/{}", backend.name);
    let published = match &reading.published {
        Ok(published) => published,
        Err(failure) => {
            return TraceStep::unknown(
                id,
                Owner::Yours,
                format!("endpoints unreadable ({}: {})", failure.kind.label(), failure.message),
            )
            .quoting(asks, "unknown");
        }
    };
    let ready = published.count(EndpointState::Ready);
    let total = published.endpoints.len();
    let serves = format!("{ready} of {total} ready via {}", published.source.label());
    if reading.facts.kind == ServiceKind::ExternalName {
        return TraceStep::new(id, StepVerdict::Neutral, "an alias publishes no endpoints")
            .quoting(asks, serves);
    }
    if ready == 0 && total == 0 && reading.facts.selector.is_empty() {
        return TraceStep::new(
            id,
            StepVerdict::Neutral,
            "a Service with no selector publishes what is added to it, and nothing yet",
        )
        .quoting(asks, serves);
    }
    if ready == 0 {
        let reason = if total == 0 {
            "the Service publishes no endpoints".to_owned()
        } else {
            "no endpoint is ready".to_owned()
        };
        return TraceStep::fail(id, Owner::Yours, reason).quoting(asks, serves);
    }
    if ready < total || published.truncated {
        let reason = if published.truncated {
            format!("{ready} ready; the list was cut, so more may exist")
        } else {
            format!("{ready} of {total} endpoints are ready")
        };
        return TraceStep {
            owner: Some(Owner::Yours),
            ..TraceStep::new(id, StepVerdict::Warn, reason).quoting(asks, serves)
        };
    }
    TraceStep::pass(id, format!("{ready} ready endpoints")).quoting(asks, serves)
}

fn branch_verdict(steps: &[TraceStep]) -> BranchVerdict {
    if steps.iter().any(|s| s.verdict == StepVerdict::Fail) {
        BranchVerdict::Broken
    } else if steps.iter().any(|s| s.verdict == StepVerdict::Unknown) {
        BranchVerdict::Unknown
    } else if steps.iter().any(|s| s.verdict == StepVerdict::Neutral) {
        BranchVerdict::Unknown
    } else {
        BranchVerdict::Serving
    }
}

fn branches(
    route: &RouteFacts,
    status: Option<&ParentStatus>,
    inputs: &Inputs,
) -> Vec<Branch> {
    let mut out = Vec::new();
    for rule in &route.rules {
        let rule_label = rule.paths.join(", ");
        let simple = |kind, label: &str, verdict, step: Option<TraceStep>| Branch {
            kind,
            label: label.to_owned(),
            rule: rule_label.clone(),
            share: None,
            weight: 1,
            steps: step.into_iter().collect(),
            verdict,
            service: None,
            proposal: None,
        };
        if rule.backends.is_empty() {
            out.push(if rule.redirect {
                simple(BranchKind::Redirect, "redirect", BranchVerdict::Serving, None)
            } else if rule.extension {
                simple(
                    BranchKind::Extension,
                    "ExtensionRef",
                    BranchVerdict::Unknown,
                    Some(TraceStep::new(
                        StepId::Refs,
                        StepVerdict::Neutral,
                        "handled by an extension the trace does not read",
                    )),
                )
            } else {
                simple(
                    BranchKind::Empty,
                    "no backend",
                    BranchVerdict::Broken,
                    Some(
                        TraceStep::fail(StepId::Refs, Owner::Yours, "the rule has no backendRefs")
                            .quoting("a backend for this rule", "none"),
                    ),
                )
            });
            continue;
        }
        let total: i64 = rule.backends.iter().map(|b| b.weight.max(0)).sum();
        for backend in &rule.backends {
            let namespace = backend.namespace.as_deref().unwrap_or(&route.namespace);
            let label = match backend.port {
                Some(port) => format!("{namespace}/{}:{port}", backend.name),
                None => format!("{namespace}/{}", backend.name),
            };
            let share = (rule.backends.len() > 1 && total > 0)
                .then(|| u32::try_from(backend.weight.max(0) * 100 / total).unwrap_or(0));
            let mut branch = Branch {
                kind: BranchKind::Service,
                label,
                rule: rule_label.clone(),
                share,
                weight: backend.weight,
                steps: Vec::new(),
                verdict: BranchVerdict::Unknown,
                service: None,
                proposal: None,
            };
            if backend.weight <= 0 {
                branch.verdict = BranchVerdict::Excluded;
                out.push(branch);
                continue;
            }
            if !backend.is_service() {
                branch.kind = BranchKind::Other;
                branch.steps.push(TraceStep::new(
                    StepId::Refs,
                    StepVerdict::Neutral,
                    format!("a {} backend is not a Service, so it is not judged", backend.kind),
                ));
                out.push(branch);
                continue;
            }
            let read = inputs.backends.get(&(namespace.to_owned(), backend.name.clone()));
            let mut chain = Chain::new();
            let mut proposal = None;
            chain.run(StepId::Refs, || {
                let (step, p) = refs_step(route, backend, namespace, status, inputs);
                proposal = p;
                step
            });
            chain.run(StepId::Service, || service_step(backend, namespace, read, status));
            chain.run(StepId::Endpoints, || match read {
                Some(Ok(Some(reading))) => endpoints_step(backend, namespace, reading),
                _ => TraceStep::unknown(StepId::Endpoints, Owner::Yours, "the Service was not read"),
            });
            branch.verdict = branch_verdict(&chain.steps);
            branch.steps = chain.steps;
            branch.service = Some((namespace.to_owned(), backend.name.clone()));
            branch.proposal = proposal;
            out.push(branch);
        }
    }
    out
}

/// A parent's verdict from its shared steps and branches.
fn parent_verdict(steps: &[TraceStep], branches: &[Branch]) -> RouteVerdict {
    if steps.iter().any(|s| s.verdict == StepVerdict::Fail) {
        return RouteVerdict::NotServing;
    }
    if steps.iter().any(|s| s.verdict == StepVerdict::Unknown) {
        return RouteVerdict::Unknown;
    }
    let count = |v| branches.iter().filter(|b| b.verdict == v).count();
    let (serving, broken, unknown) = (
        count(BranchVerdict::Serving),
        count(BranchVerdict::Broken),
        count(BranchVerdict::Unknown),
    );
    match (serving, broken, unknown) {
        (s, b, _) if s > 0 && b > 0 => RouteVerdict::Partly,
        (0, b, 0) if b > 0 => RouteVerdict::NotServing,
        (s, 0, 0) if s > 0 => RouteVerdict::Serving,
        _ => RouteVerdict::Unknown,
    }
}

fn trace_parent(
    route: &RouteFacts,
    parent: &ParentRef,
    status: Option<&ParentStatus>,
    inputs: &Inputs,
) -> ParentTrace {
    let label = parent.label(&route.namespace);
    let controller = status.map(|s| s.controller.clone()).filter(|c| !c.is_empty());
    if !parent.is_gateway() {
        return ParentTrace {
            parent: parent.clone(),
            label,
            controller,
            steps: Vec::new(),
            branches: Vec::new(),
            verdict: RouteVerdict::Mesh,
        };
    }
    let gateway_namespace = parent.namespace_or(&route.namespace);
    let mut chain = Chain::new();
    let gateway = match inputs.gateways {
        Err(failure) => Err(TraceStep::unknown(
            StepId::Class,
            Owner::Platform,
            format!("Gateways unreadable ({}: {})", failure.kind.label(), failure.message),
        )),
        Ok(list) => list
            .items
            .iter()
            .find(|g| g.namespace == gateway_namespace && g.name == parent.name)
            .ok_or_else(|| {
                if list.cut {
                    TraceStep::unknown(
                        StepId::Class,
                        Owner::Platform,
                        "the Gateway list was cut; the parent may exist",
                    )
                } else {
                    TraceStep::fail(
                        StepId::Class,
                        Owner::Yours,
                        format!("the parent Gateway {} no longer exists", parent.label(&route.namespace)),
                    )
                }
            }),
    };
    let gateway = match gateway {
        Ok(gateway) => gateway,
        Err(step) => {
            let step = step.quoting(format!("parent Gateway {}", parent.label(&route.namespace)), "not found");
            let mut steps = vec![step];
            steps.extend(
                [StepId::Gateway, StepId::Listener, StepId::Namespace].map(TraceStep::not_reached),
            );
            let verdict = parent_verdict(&steps, &[]);
            return ParentTrace {
                parent: parent.clone(),
                label,
                controller,
                steps,
                branches: Vec::new(),
                verdict,
            };
        }
    };
    chain.run(StepId::Class, || class_step(gateway, inputs));
    chain.run(StepId::Gateway, || gateway_step(gateway));
    let mut listener = None;
    chain.run(StepId::Listener, || {
        let (step, picked) = listener_step(gateway, parent, route, status);
        listener = picked;
        step
    });
    chain.run(StepId::Namespace, || match &listener {
        Some(listener) => namespace_step(gateway, listener, route, status, inputs),
        None => TraceStep::not_reached(StepId::Namespace),
    });
    let reached = !chain.stopped;
    let branches = if reached { branches(route, status, inputs) } else { Vec::new() };
    let verdict = parent_verdict(&chain.steps, &branches);
    ParentTrace {
        parent: parent.clone(),
        label,
        controller,
        steps: chain.steps,
        branches,
        verdict,
    }
}

/// The verdict over every parent: serving only when all are, partly serving
/// when some are, and mesh only when nothing else is judged.
fn route_verdict(parents: &[ParentTrace]) -> RouteVerdict {
    let judged = parents
        .iter()
        .map(|p| p.verdict)
        .filter(|v| *v != RouteVerdict::Mesh)
        .collect::<Vec<_>>();
    if judged.is_empty() {
        return if parents.is_empty() { RouteVerdict::NotServing } else { RouteVerdict::Mesh };
    }
    let has = |v| judged.contains(&v);
    if has(RouteVerdict::Partly) || (has(RouteVerdict::Serving) && has(RouteVerdict::NotServing)) {
        RouteVerdict::Partly
    } else if has(RouteVerdict::Unknown) {
        RouteVerdict::Unknown
    } else if has(RouteVerdict::NotServing) {
        RouteVerdict::NotServing
    } else {
        RouteVerdict::Serving
    }
}

fn summarize(route: &RouteFacts, parents: &[ParentTrace], verdict: RouteVerdict) -> Summary {
    if route.parents.is_empty() {
        return Summary {
            stops_at: Some("Parent"),
            reason: "spec.parentRefs is empty, so no Gateway is asked to serve it".into(),
            owner: Some(Owner::Yours),
        };
    }
    let step = |p: &ParentTrace| {
        p.steps
            .iter()
            .find(|s| s.breaks())
            .map(|s| (s.id.label(), s.reason.clone(), s.owner))
            .or_else(|| {
                p.branches.iter().flat_map(|b| &b.steps).find_map(|s| {
                    (s.verdict == StepVerdict::Fail || s.verdict == StepVerdict::Unknown)
                        .then(|| (s.id.label(), s.reason.clone(), s.owner))
                })
            })
    };
    match verdict {
        RouteVerdict::Mesh => Summary {
            stops_at: None,
            reason: "a mesh route: the parent is a Service, and mesh routes are not judged".into(),
            owner: None,
        },
        RouteVerdict::Serving => Summary {
            stops_at: None,
            reason: format!(
                "served through {}",
                parents
                    .iter()
                    .filter(|p| p.verdict == RouteVerdict::Serving)
                    .map(|p| p.label.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            owner: None,
        },
        wanted => {
            let first = parents
                .iter()
                .filter(|p| p.verdict == wanted || wanted == RouteVerdict::Partly)
                .find_map(step);
            match first {
                Some((at, reason, owner)) => Summary { stops_at: Some(at), reason, owner },
                None => Summary {
                    stops_at: None,
                    reason: "no step explains the verdict".into(),
                    owner: None,
                },
            }
        }
    }
}

/// Traces one route against what was read. One trace per parent, and per
/// controller when several wrote a verdict for the same parent.
pub fn trace_route(route: &RouteFacts, inputs: &Inputs) -> RouteTrace {
    let mut parents = Vec::new();
    for parent in &route.parents {
        let statuses = route
            .status
            .iter()
            .filter(|s| s.parent.same_as(parent, &route.namespace))
            .collect::<Vec<_>>();
        if statuses.is_empty() {
            parents.push(trace_parent(route, parent, None, inputs));
        }
        for status in statuses {
            parents.push(trace_parent(route, parent, Some(status), inputs));
        }
    }
    let verdict = route_verdict(&parents);
    let stale = parents
        .iter()
        .flat_map(|p| p.steps.iter().chain(p.branches.iter().flat_map(|b| &b.steps)))
        .any(|s| s.stale);
    let summary = summarize(route, &parents, verdict);
    RouteTrace { parents, verdict, summary, stale }
}
