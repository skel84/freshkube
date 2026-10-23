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

/// Each namespace with its labels, for a listener that selects by label.
pub type NamespaceLabels = Listing<(String, BTreeMap<String, String>)>;

/// A backend Service as read: not found is an answer (`None`).
pub type BackendRead = Result<Option<ServiceReading>, Failure>;

/// Everything a trace reads besides the route. Each is its own `Result`, so
/// one refused read makes only the steps that need it Unknown.
pub struct Inputs<'a> {
    pub gateways: &'a Result<Listing<GatewayFacts>, Failure>,
    pub classes: &'a Result<Listing<ClassFacts>, Failure>,
    pub grants: &'a Result<Listing<GrantFacts>, Failure>,
    /// Namespace labels, read only for a listener that selects by label.
    pub namespaces: &'a Result<NamespaceLabels, Failure>,
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
        "InvalidCertificateRef"
        | "InvalidRouteKinds"
        | "RefNotPermitted"
        | "BackendNotFound"
        | "InvalidKind"
        | "UnsupportedValue"
        | "NoMatchingListenerHostname"
        | "NoMatchingParent"
        | "UnsupportedProtocol"
        | "InvalidParameters" => Owner::Yours,
        "NotAllowedByListeners"
        | "AddressNotAssigned"
        | "AddressNotUsable"
        | "NoResources"
        | "ListenersNotValid"
        | "PortUnavailable"
        | "HostnameConflict"
        | "ProtocolConflict"
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

fn route_condition<'a>(status: Option<&'a ParentStatus>, kind: &str) -> Option<&'a Cond> {
    status.and_then(|s| find(&s.conditions, kind))
}

mod backend;
mod parent;

use backend::branches;
use parent::{class_step, gateway_step, listener_step, namespace_step};

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
    let controller = status
        .map(|s| s.controller.clone())
        .filter(|c| !c.is_empty());
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
            format!(
                "Gateways unreadable ({}: {})",
                failure.kind.label(),
                failure.message
            ),
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
                        format!(
                            "the parent Gateway {} no longer exists",
                            parent.label(&route.namespace)
                        ),
                    )
                }
            }),
    };
    let gateway = match gateway {
        Ok(gateway) => gateway,
        Err(step) => {
            let step = step.quoting(
                format!("parent Gateway {}", parent.label(&route.namespace)),
                "not found",
            );
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
    let branches = if reached {
        branches(route, status, inputs)
    } else {
        Vec::new()
    };
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
        return if parents.is_empty() {
            RouteVerdict::NotServing
        } else {
            RouteVerdict::Mesh
        };
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
                Some((at, reason, owner)) => Summary {
                    stops_at: Some(at),
                    reason,
                    owner,
                },
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
        .flat_map(|p| {
            p.steps
                .iter()
                .chain(p.branches.iter().flat_map(|b| &b.steps))
        })
        .any(|s| s.stale);
    let summary = summarize(route, &parents, verdict);
    RouteTrace {
        parents,
        verdict,
        summary,
        stale,
    }
}
