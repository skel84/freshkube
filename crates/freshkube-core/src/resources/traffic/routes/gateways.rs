//! The Gateway and GatewayClass pages (#949, N2b): what each Gateway and
//! class is, read from the lists and the traces N2a already made, so nothing
//! is read a second time. Read-only, and no UI types.
//!
//! A Gateway is **Failed**, **Partly** serving (a listener the controller
//! rejected), **Pending**, **Blind** (the controller writes no status, or
//! Programmed with no address), **Unknown** (a read was refused) or
//! **Serving**; one the routes name and the cluster doesn't have is **Not
//! found**. Unknown is never green, and a refused list is an error, never an
//! empty page. Configuration (listeners, allowed routes) and what the
//! controller reports are separate lines.
use super::facts::*;
use super::read::RoutesRead;
use super::trace::{
    Cause, Listing, Owner, RouteVerdict, Standing, StepVerdict, blame, class_check, class_step,
    gateway_step, standing,
};
use crate::failure::Failure;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GatewayState {
    Failed,
    /// Routes name it and the cluster has no such Gateway.
    Missing,
    Partly,
    Pending,
    Blind,
    Unknown,
    Serving,
}

impl GatewayState {
    pub const ALL: [Self; 7] = [
        Self::Failed,
        Self::Missing,
        Self::Partly,
        Self::Pending,
        Self::Blind,
        Self::Unknown,
        Self::Serving,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Failed => "Failed",
            Self::Missing => "Not found",
            Self::Partly => "Partly serving",
            Self::Pending => "Pending",
            Self::Blind => "Blind",
            Self::Unknown => "Unknown",
            Self::Serving => "Serving",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListenerState {
    Ready,
    Failed,
    /// The controller reported nothing for this listener.
    Unknown,
}

impl ListenerState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Failed => "Not accepted",
            Self::Unknown => "No status",
        }
    }
}

/// One listener: what the Gateway is set to serve, and what the controller
/// reports for it, apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListenerRow {
    pub name: String,
    pub port: i32,
    pub protocol: String,
    pub hostname: Option<String>,
    /// Which namespaces may attach, in words.
    pub allows: String,
    /// Routes the controller counts as attached; none without a status.
    pub attached: Option<i64>,
    pub state: ListenerState,
    /// The condition that says it is not ready, as the controller wrote it.
    pub says: Option<String>,
    /// Whose problem a rejected listener is.
    pub owner: Option<Owner>,
    /// `tls.certificateRefs` by name; no Secret is read.
    pub certificates: Vec<String>,
}

/// A route attached to a Gateway, with its verdict from N2a's trace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachedRoute {
    pub namespace: String,
    pub name: String,
    pub label: String,
    pub verdict: RouteVerdict,
    /// The trace's own words: where it stops and why.
    pub line: String,
}

/// A hostname that listeners of another Gateway declare too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contested {
    pub host: String,
    pub also: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct GatewayEntry {
    pub facts: GatewayFacts,
    pub state: GatewayState,
    /// Why it reads as it does.
    pub reason: String,
    pub owner: Option<Owner>,
    /// What the controller reports for the Gateway, kept apart.
    pub says: Option<String>,
    /// The Gateway's verdict is older than its generation.
    pub stale: bool,
    pub listeners: Vec<ListenerRow>,
    pub routes: Vec<AttachedRoute>,
    pub contested: Vec<Contested>,
}

impl GatewayEntry {
    pub fn key(&self) -> String {
        format!("{}/{}", self.facts.namespace, self.facts.name)
    }

    pub fn address(&self) -> Option<String> {
        (!self.facts.addresses.is_empty()).then(|| self.facts.addresses.join(", "))
    }

    /// Routes that are not serving (counting partly serving and unknown out).
    pub fn not_serving(&self) -> usize {
        self.routes
            .iter()
            .filter(|route| route.verdict == RouteVerdict::NotServing)
            .count()
    }

    /// The listeners the controller rejected.
    pub fn failing_listeners(&self) -> usize {
        self.listeners
            .iter()
            .filter(|l| l.state == ListenerState::Failed)
            .count()
    }

    /// `3 routes, 1 not serving`, or `no routes`.
    pub fn routes_line(&self) -> String {
        match (self.routes.len(), self.not_serving()) {
            (0, _) => "no routes".into(),
            (n, 0) => format!("{n} {}", if n == 1 { "route" } else { "routes" }),
            (n, bad) => format!(
                "{n} {}, {bad} not serving",
                if n == 1 { "route" } else { "routes" }
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClassState {
    Refused,
    /// Gateways name it and the cluster has no such class.
    Missing,
    Pending,
    Blind,
    Claimed,
}

impl ClassState {
    pub const ALL: [Self; 5] = [
        Self::Refused,
        Self::Missing,
        Self::Pending,
        Self::Blind,
        Self::Claimed,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Refused => "Not accepted",
            Self::Missing => "Not found",
            Self::Pending => "Pending",
            Self::Blind => "Blind",
            Self::Claimed => "Claimed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClassEntry {
    pub name: String,
    /// The class's UID, for a link; empty for a class that does not exist.
    pub uid: String,
    /// None for a class the Gateways name and the cluster lacks.
    pub controller: Option<String>,
    pub state: ClassState,
    pub reason: String,
    /// What the controller reports, kept apart from the reason.
    pub says: Option<String>,
    /// The verdict was written for an older generation than the class has.
    pub stale: bool,
    /// The Gateways of this class, with how each reads.
    pub gateways: Vec<(String, GatewayState)>,
}

/// What the Gateway pages draw from one read.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GatewaysView {
    pub gateways: Result<Vec<GatewayEntry>, Failure>,
    pub classes: Result<Vec<ClassEntry>, Failure>,
    /// The Gateway list stopped at its limit, so a parent not in it may exist.
    pub gateways_cut: bool,
    pub classes_cut: bool,
}

struct Assessed {
    state: GatewayState,
    reason: String,
    owner: Option<Owner>,
    says: Option<String>,
    stale: bool,
}

/// How the controller's own reading of a Gateway and its class reads: the
/// trace's first two steps, so the Gateway pages and the route trace judge
/// them as one.
fn assess(gateway: &GatewayFacts, classes: &Result<Listing<ClassFacts>, Failure>) -> Assessed {
    let steps = [class_step(gateway, classes), gateway_step(gateway)];
    let deciding = steps
        .iter()
        .find(|step| !matches!(step.verdict, StepVerdict::Pass))
        .unwrap_or(&steps[1]);
    let state = match (deciding.verdict, deciding.cause) {
        (StepVerdict::Fail, _) => GatewayState::Failed,
        (StepVerdict::Unknown, Some(Cause::Blind)) => GatewayState::Blind,
        (StepVerdict::Unknown, Some(Cause::Pending)) => GatewayState::Pending,
        (StepVerdict::Unknown, _) => GatewayState::Unknown,
        // A warning is a pass with something to know: a stale verdict.
        (StepVerdict::Warn | StepVerdict::Pass, _) => GatewayState::Serving,
        (_, _) => GatewayState::Unknown,
    };
    Assessed {
        state,
        reason: deciding.reason.clone(),
        owner: deciding.owner,
        says: deciding.controller.clone(),
        stale: deciding.stale,
    }
}

fn listener_rows(gateway: &GatewayFacts) -> Vec<ListenerRow> {
    gateway
        .listeners
        .iter()
        .map(|l| {
            let status = gateway.listener_status.iter().find(|s| s.name == l.name);
            // The route trace's own judgement of a listener, so the two pages
            // cannot disagree: only a condition that says the listener is not
            // working rejects it.
            let (state, bad) = match standing(gateway, l) {
                Standing::Bad(cond) => (ListenerState::Failed, Some(cond)),
                Standing::Silent => (ListenerState::Unknown, None),
                Standing::Good(_) if status.is_some_and(|s| !s.conditions.is_empty()) => {
                    (ListenerState::Ready, None)
                }
                Standing::Good(_) => (ListenerState::Unknown, None),
            };
            ListenerRow {
                name: l.name.clone(),
                port: l.port,
                protocol: l.protocol.clone(),
                hostname: l.hostname.clone(),
                allows: match &l.namespaces {
                    AllowedNamespaces::Same => "routes from its own namespace".into(),
                    AllowedNamespaces::All => "routes from every namespace".into(),
                    AllowedNamespaces::Selector(selector) => {
                        format!("routes from namespaces matching {}", selector.describe())
                    }
                },
                attached: status.map(|s| s.attached),
                state,
                says: bad.map(Cond::says),
                owner: bad.map(|cond| blame(&cond.reason)),
                certificates: l.certificates.clone(),
            }
        })
        .collect()
}

/// Routes by the Gateway they name, with the worst verdict among the parent
/// traces that name it.
fn routes_by_gateway(read: &RoutesRead) -> BTreeMap<(String, String), Vec<AttachedRoute>> {
    let mut found: BTreeMap<(String, String), BTreeMap<String, AttachedRoute>> = BTreeMap::new();
    for entry in &read.routes {
        for parent in entry.trace.parents.iter().filter(|p| p.parent.is_gateway()) {
            let key = (
                parent
                    .parent
                    .namespace_or(&entry.facts.namespace)
                    .to_owned(),
                parent.parent.name.clone(),
            );
            let route = AttachedRoute {
                namespace: entry.facts.namespace.clone(),
                name: entry.facts.name.clone(),
                label: entry.facts.label(),
                verdict: parent.verdict,
                line: entry.trace.summary.line(),
            };
            let slot = found.entry(key).or_default();
            let id = format!("{}/{}", route.namespace, route.name);
            match slot.get(&id) {
                Some(have) if have.verdict <= route.verdict => {}
                _ => {
                    slot.insert(id, route);
                }
            }
        }
    }
    found
        .into_iter()
        .map(|(key, routes)| {
            let mut routes = routes.into_values().collect::<Vec<_>>();
            routes.sort_by(|a, b| {
                (a.verdict, &a.namespace, &a.name).cmp(&(b.verdict, &b.namespace, &b.name))
            });
            (key, routes)
        })
        .collect()
}

/// A hostname a listener names exactly; wildcards overlap too widely to call
/// contested.
fn contested_hosts(gateways: &[GatewayFacts]) -> BTreeMap<String, Vec<Contested>> {
    let mut owners: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for gateway in gateways {
        for host in gateway
            .listeners
            .iter()
            .filter_map(|l| l.hostname.as_deref())
            .filter(|host| !host.starts_with('*'))
        {
            owners
                .entry(host)
                .or_default()
                .insert(format!("{}/{}", gateway.namespace, gateway.name));
        }
    }
    let mut by_gateway: BTreeMap<String, Vec<Contested>> = BTreeMap::new();
    for (host, who) in owners.into_iter().filter(|(_, who)| who.len() > 1) {
        for gateway in &who {
            by_gateway
                .entry(gateway.clone())
                .or_default()
                .push(Contested {
                    host: host.to_owned(),
                    also: who
                        .iter()
                        .filter(|other| *other != gateway)
                        .cloned()
                        .collect(),
                });
        }
    }
    by_gateway
}

fn class_entries(
    list: &Listing<ClassFacts>,
    classes: &Result<Listing<ClassFacts>, Failure>,
    gateways: &Result<Vec<GatewayEntry>, Failure>,
) -> Vec<ClassEntry> {
    let uses = |name: &str| -> Vec<(String, GatewayState)> {
        gateways
            .as_ref()
            .map(|list| {
                list.iter()
                    .filter(|g| g.state != GatewayState::Missing && g.facts.class == name)
                    .map(|g| (g.key(), g.state))
                    .collect()
            })
            .unwrap_or_default()
    };
    let classes_read = classes;
    let classes = list;
    let mut entries = classes
        .items
        .iter()
        .map(|class| {
            // The trace's own judgement of the class, so the pages agree.
            let step = class_check(
                &class.name,
                format!("GatewayClass {}", class.name),
                classes_read,
            );
            let state = match (step.verdict, step.cause) {
                (StepVerdict::Fail, _) => ClassState::Refused,
                (StepVerdict::Unknown, Some(Cause::Pending)) => ClassState::Pending,
                (StepVerdict::Unknown, _) => ClassState::Blind,
                (StepVerdict::Pass | StepVerdict::Warn, _) => ClassState::Claimed,
                (_, _) => ClassState::Blind,
            };
            ClassEntry {
                name: class.name.clone(),
                uid: class.uid.clone(),
                controller: Some(class.controller.clone()),
                state,
                reason: step.reason,
                says: step.controller,
                stale: step.stale,
                gateways: uses(&class.name),
            }
        })
        .collect::<Vec<_>>();
    // Classes the Gateways name and the cluster does not have; a cut list
    // can't say they are missing.
    if !classes.cut
        && let Ok(list) = gateways
    {
        let known = classes
            .items
            .iter()
            .map(|c| c.name.as_str())
            .collect::<BTreeSet<_>>();
        let wanted = list
            .iter()
            .filter(|g| g.state != GatewayState::Missing)
            .map(|g| g.facts.class.as_str())
            .filter(|name| !name.is_empty() && !known.contains(name))
            .collect::<BTreeSet<_>>();
        entries.extend(wanted.into_iter().map(|name| ClassEntry {
            name: name.to_owned(),
            uid: String::new(),
            controller: None,
            state: ClassState::Missing,
            reason: format!("no GatewayClass {name} exists, though Gateways name it"),
            says: None,
            stale: false,
            gateways: uses(name),
        }));
    }
    entries.sort_by(|a, b| (a.state, &a.name).cmp(&(b.state, &b.name)));
    entries
}

impl GatewaysView {
    pub fn new(read: &RoutesRead) -> Self {
        let mut attached = routes_by_gateway(read);
        let gateways = read.gateways.as_ref().map_err(Clone::clone).map(|list| {
            let contested = contested_hosts(&list.items);
            let mut entries = list
                .items
                .iter()
                .map(|facts| {
                    let assessed = assess(facts, &read.classes);
                    let key = (facts.namespace.clone(), facts.name.clone());
                    let listeners = listener_rows(facts);
                    let mut state = assessed.state;
                    let mut reason = assessed.reason;
                    let failing = listeners
                        .iter()
                        .filter(|l| l.state == ListenerState::Failed)
                        .count();
                    let mut owner = assessed.owner;
                    if state == GatewayState::Serving && failing > 0 {
                        let first = listeners
                            .iter()
                            .find(|l| l.state == ListenerState::Failed)
                            .expect("a failing listener");
                        let says = first.says.as_deref().unwrap_or("not accepted");
                        owner = first.owner;
                        if failing == listeners.len() {
                            // Nothing is served: that is a failure, not part of one.
                            state = GatewayState::Failed;
                            reason = if failing == 1 {
                                format!(
                                    "its only listener, {}, is not accepted: {says}",
                                    first.name
                                )
                            } else {
                                format!(
                                    "none of its {failing} listeners is accepted; {} says: {says}",
                                    first.name
                                )
                            };
                        } else {
                            state = GatewayState::Partly;
                            reason = format!(
                                "{failing} of {} listeners not accepted; {} says: {says}",
                                listeners.len(),
                                first.name
                            );
                        }
                    }
                    GatewayEntry {
                        state,
                        reason,
                        owner,
                        says: assessed.says,
                        stale: assessed.stale,
                        contested: contested
                            .get(&format!("{}/{}", facts.namespace, facts.name))
                            .cloned()
                            .unwrap_or_default(),
                        routes: attached.remove(&key).unwrap_or_default(),
                        listeners,
                        facts: facts.clone(),
                    }
                })
                .collect::<Vec<_>>();
            // A parent the routes name that no Gateway answers to. A cut list
            // can't say so.
            if !list.cut {
                for ((namespace, name), routes) in std::mem::take(&mut attached) {
                    entries.push(GatewayEntry {
                        facts: GatewayFacts {
                            namespace: namespace.clone(),
                            name: name.clone(),
                            uid: String::new(),
                            generation: 0,
                            class: String::new(),
                            listeners: Vec::new(),
                            conditions: Vec::new(),
                            addresses: Vec::new(),
                            listener_status: Vec::new(),
                        },
                        state: GatewayState::Missing,
                        reason: format!(
                            "{} {} name it as a parent, and no such Gateway exists",
                            routes.len(),
                            if routes.len() == 1 { "route" } else { "routes" }
                        ),
                        owner: Some(Owner::Yours),
                        says: None,
                        stale: false,
                        listeners: Vec::new(),
                        routes,
                        contested: Vec::new(),
                    });
                }
            }
            entries.sort_by_key(|a| (a.state, a.key()));
            entries
        });
        let classes = match &read.classes {
            Ok(list) => Ok(class_entries(list, &read.classes, &gateways)),
            Err(failure) => Err(failure.clone()),
        };
        Self {
            gateways,
            classes,
            gateways_cut: read.gateways.as_ref().is_ok_and(|l| l.cut),
            classes_cut: read.classes.as_ref().is_ok_and(|l| l.cut),
        }
    }
}
