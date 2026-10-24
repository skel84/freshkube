//! The steps of a route's parent: the class, the Gateway, a listener and the
//! namespace.
use super::*;

fn generation_note(cond: &Cond, now: i64) -> Option<String> {
    cond.generation
        .filter(|seen| *seen < now)
        .map(|seen| format!("stale: written for generation {seen}, now {now}"))
}

pub(super) fn class_step(gateway: &GatewayFacts, inputs: &Inputs) -> TraceStep {
    let id = StepId::Class;
    let asks = format!("Gateway {} uses class {}", gateway.name, gateway.class);
    let classes = match inputs.classes {
        Ok(classes) => classes,
        Err(failure) => {
            return TraceStep::unknown(
                id,
                Owner::Platform,
                format!(
                    "GatewayClasses unreadable ({}: {})",
                    failure.kind.label(),
                    failure.message
                ),
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
            Owner::Platform,
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
        Some(cond) => TraceStep::unknown(
            id,
            Owner::Controller,
            "pending: the class is not accepted yet",
        )
        .quoting(asks, serves)
        .reported(Some(cond.says())),
    }
}

pub(super) fn gateway_step(gateway: &GatewayFacts) -> TraceStep {
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
        Tri::False => TraceStep::fail(
            id,
            blame(&programmed.reason),
            "the Gateway is not programmed",
        )
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

/// Why no listener fits by configuration, and whose problem that is.
struct Miss {
    reason: String,
    owner: Owner,
}

/// The listeners that fit the route by name, port, protocol, kind and
/// hostname, in the Gateway's order. The namespace is the next step's.
fn fitting<'a>(
    gateway: &'a GatewayFacts,
    parent: &ParentRef,
    route: &RouteFacts,
) -> Result<Vec<&'a Listener>, Miss> {
    let miss = |reason: String, owner| Miss { reason, owner };
    let mut candidates = gateway
        .listeners
        .iter()
        .filter(|l| parent.section.as_deref().is_none_or(|s| s == l.name))
        .filter(|l| parent.port.is_none_or(|p| p == l.port))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Err(match (&parent.section, parent.port) {
            (Some(section), _) => miss(
                format!("the Gateway has no listener named {section}"),
                Owner::Yours,
            ),
            (None, Some(port)) => miss(
                format!("the Gateway has no listener on port {port}"),
                Owner::Yours,
            ),
            (None, None) => miss("the Gateway has no listeners".into(), Owner::Platform),
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
        return Err(miss(
            format!(
                "no HTTP or HTTPS listener to attach an HTTPRoute to (listeners speak {protocols})"
            ),
            Owner::Platform,
        ));
    }
    candidates.retain(|l| l.allows_http_route());
    if candidates.is_empty() {
        return Err(miss(
            "the listener does not allow HTTPRoute".into(),
            Owner::Platform,
        ));
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
        return Err(miss(
            format!(
                "route hostnames {} match no listener hostname ({served})",
                quote_hosts(&route.hostnames)
            ),
            Owner::Yours,
        ));
    }
    Ok(matching)
}

fn describe_listener(l: &Listener) -> String {
    let certificates = if l.certificates.is_empty() {
        String::new()
    } else {
        format!(" · certificate {}", l.certificates.join(", "))
    };
    format!(
        "listener {} {} :{} · {}{certificates}",
        l.name,
        l.protocol,
        l.port,
        l.hostname.as_deref().unwrap_or("any host")
    )
}

/// What the controller reports of one listener.
enum Standing<'a> {
    /// Nothing wrong reported, with its `Accepted` line when it wrote one.
    Good(Option<&'a Cond>),
    /// The Gateway has a status, but nothing for this listener.
    Silent,
    /// A condition that says the listener is not working.
    Bad(&'a Cond),
}

fn standing<'a>(gateway: &'a GatewayFacts, listener: &Listener) -> Standing<'a> {
    let reported = gateway
        .listener_status
        .iter()
        .find(|s| s.name == listener.name);
    match reported {
        None if !gateway.listener_status.is_empty() || !gateway.conditions.is_empty() => {
            Standing::Silent
        }
        None => Standing::Good(None),
        Some(reported) => {
            let bad = ["Accepted", "Programmed", "ResolvedRefs", "Conflicted"]
                .into_iter()
                .filter_map(|kind| find(&reported.conditions, kind))
                .find(|c| match c.kind.as_str() {
                    "Conflicted" => c.status == Tri::True,
                    _ => c.status == Tri::False,
                });
            match bad {
                Some(cond) => Standing::Bad(cond),
                None => Standing::Good(find(&reported.conditions, "Accepted")),
            }
        }
    }
}

/// A Fail worked out from configuration never overrules the route's own
/// controller saying, for the current generation, that it accepted the route
/// (or resolved its references): that reads Unknown, with both sides quoted.
pub(super) fn overruled(
    mut step: TraceStep,
    controller: Option<&Cond>,
    generation: i64,
) -> TraceStep {
    let current = controller
        .filter(|c| c.status == Tri::True && c.generation.is_none_or(|seen| seen >= generation));
    let Some(cond) = current else {
        return step;
    };
    if step.verdict != StepVerdict::Fail {
        return step;
    }
    step.verdict = StepVerdict::Unknown;
    step.owner = Some(Owner::Controller);
    step.reason = format!(
        "configuration says: {}; but the controller reports {}",
        step.reason,
        cond.says()
    );
    step.controller = Some(cond.says());
    step
}

/// Step 3: some listener fits the route and is not rejected. Returns the
/// listeners the next step looks at, the best first.
pub(super) fn listener_step(
    gateway: &GatewayFacts,
    parent: &ParentRef,
    route: &RouteFacts,
    status: Option<&ParentStatus>,
) -> (TraceStep, Vec<Listener>) {
    let id = StepId::Listener;
    let asks = format!(
        "{} on {}",
        quote_hosts(&route.hostnames),
        parent.section.as_deref().unwrap_or("any listener")
    );
    let accepted = route_condition(status, "Accepted");
    let fit = match fitting(gateway, parent, route) {
        Ok(fit) => fit,
        Err(miss) => {
            let served = gateway
                .listeners
                .iter()
                .map(describe_listener)
                .collect::<Vec<_>>()
                .join("; ");
            let controller = accepted.filter(|c| c.status == Tri::False).map(Cond::says);
            let step = TraceStep::fail(id, miss.owner, miss.reason)
                .quoting(
                    asks,
                    if served.is_empty() {
                        "no listeners".into()
                    } else {
                        served
                    },
                )
                .reported(controller);
            return (overruled(step, accepted, route.generation), Vec::new());
        }
    };
    // Of the listeners that fit, the one the controller accepted comes first.
    let rank = |l: &Listener| match standing(gateway, l) {
        Standing::Good(Some(c)) if c.status == Tri::True => 0,
        Standing::Good(_) => 1,
        Standing::Silent => 2,
        Standing::Bad(_) => 3,
    };
    let mut ordered = fit.into_iter().collect::<Vec<_>>();
    ordered.sort_by_key(|l| rank(l));
    let best = ordered[0];
    let serves = describe_listener(best);
    let step = match standing(gateway, best) {
        Standing::Good(Some(cond)) => {
            TraceStep::pass(id, format!("listener {} accepts HTTPRoute", best.name))
                .reported(Some(cond.says()))
        }
        Standing::Good(None) => {
            TraceStep::pass(id, format!("listener {} fits by configuration", best.name))
        }
        Standing::Silent => TraceStep::unknown(
            id,
            Owner::Controller,
            format!("the controller reports nothing for listener {}", best.name),
        ),
        Standing::Bad(cond) => TraceStep::fail(
            id,
            blame(&cond.reason),
            if ordered.len() == 1 {
                format!("listener {} is not accepted", best.name)
            } else {
                format!(
                    "none of the {} listeners that fit is accepted; {} is not",
                    ordered.len(),
                    best.name
                )
            },
        )
        .reported(Some(cond.says())),
    };
    let usable = ordered
        .into_iter()
        .filter(|l| !matches!(standing(gateway, l), Standing::Bad(_)))
        .cloned()
        .collect();
    (step.quoting(asks, serves), usable)
}

/// Whether one listener lets the route's namespace attach.
enum Allow {
    Yes(String),
    No(String),
    Unknown(String),
}

fn allows(
    gateway: &GatewayFacts,
    listener: &Listener,
    route: &RouteFacts,
    inputs: &Inputs,
) -> (Allow, String) {
    match &listener.namespaces {
        AllowedNamespaces::All => (
            Allow::Yes("the listener accepts routes from every namespace".into()),
            format!("listener {}: from All", listener.name),
        ),
        AllowedNamespaces::Same => {
            let text = format!(
                "listener {}: from Same ({})",
                listener.name, gateway.namespace
            );
            if route.namespace == gateway.namespace {
                (Allow::Yes("same namespace as the Gateway".into()), text)
            } else {
                (
                    Allow::No(format!(
                        "the listener accepts only its own namespace, {}",
                        gateway.namespace
                    )),
                    text,
                )
            }
        }
        AllowedNamespaces::Selector(selector) => {
            let text = format!(
                "listener {}: from Selector ({})",
                listener.name,
                selector.describe()
            );
            let verdict = match inputs.namespaces {
                Err(failure) => Allow::Unknown(format!(
                    "namespaces unreadable ({}: {}), so the selector can't be checked",
                    failure.kind.label(),
                    failure.message
                )),
                // A cut list is no basis for a label match either way.
                Ok(namespaces) if namespaces.cut => Allow::Unknown(
                    "the namespace list was cut, so the selector can't be checked".into(),
                ),
                Ok(namespaces) => {
                    match namespaces
                        .items
                        .iter()
                        .find(|(name, _)| *name == route.namespace)
                    {
                        Some((_, labels)) if selector.matches(labels) => {
                            Allow::Yes("the namespace's labels match the selector".into())
                        }
                        Some(_) => Allow::No(
                            "the namespace's labels do not match the listener's selector".into(),
                        ),
                        None => Allow::Unknown("the route's namespace was not in the list".into()),
                    }
                }
            };
            (verdict, text)
        }
    }
}

/// Step 4: one of the listeners that fit lets the route's namespace attach,
/// and the controller's own `Accepted` for the route agrees.
pub(super) fn namespace_step(
    gateway: &GatewayFacts,
    listeners: &[Listener],
    route: &RouteFacts,
    status: Option<&ParentStatus>,
    inputs: &Inputs,
) -> TraceStep {
    let id = StepId::Namespace;
    let asks = format!("route in namespace {}", route.namespace);
    let results = listeners
        .iter()
        .map(|l| allows(gateway, l, route, inputs))
        .collect::<Vec<_>>();
    let every = results
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    let mut step = if let Some((Allow::Yes(reason), text)) =
        results.iter().find(|(a, _)| matches!(a, Allow::Yes(_)))
    {
        TraceStep::pass(id, reason.clone()).quoting(asks, text.clone())
    } else if let Some(Allow::Unknown(reason)) = results
        .iter()
        .map(|(a, _)| a)
        .find(|a| matches!(a, Allow::Unknown(_)))
    {
        TraceStep::unknown(id, Owner::Platform, reason.clone()).quoting(asks, every)
    } else {
        let reason = match results.as_slice() {
            [(Allow::No(reason), _)] => reason.clone(),
            _ => format!(
                "none of the listeners that fit accepts routes from namespace {}",
                route.namespace
            ),
        };
        TraceStep::fail(id, Owner::Platform, reason).quoting(asks, every)
    };
    // The controller's own verdict on the route, kept as its own line.
    let accepted = route_condition(status, "Accepted");
    step.controller = accepted.map(Cond::says);
    if step.verdict == StepVerdict::Fail {
        return overruled(step, accepted, route.generation);
    }
    match (status, accepted) {
        (None, _) => {
            step.verdict = StepVerdict::Unknown;
            step.owner = Some(Owner::Controller);
            step.reason =
                "blind: the controller reports nothing for this route on this parent".into();
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
