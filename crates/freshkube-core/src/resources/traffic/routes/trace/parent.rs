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

/// Why no listener fits, or the one that does.
enum Pick<'a> {
    Found(&'a Listener),
    Miss(String),
}

fn pick_listener<'a>(
    gateway: &'a GatewayFacts,
    parent: &ParentRef,
    route: &RouteFacts,
) -> Pick<'a> {
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
    candidates.retain(|l| {
        l.kinds
            .as_ref()
            .is_none_or(|k| k.iter().any(|k| k == "HTTPRoute"))
    });
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
            .is_some_and(|s| find(&s.conditions, "Accepted").is_some_and(|c| c.status == Tri::True))
    };
    let best = matching
        .iter()
        .copied()
        .find(|l| accepted(l))
        .unwrap_or(matching[0]);
    Pick::Found(best)
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

pub(super) fn listener_step(
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
                    .quoting(
                        asks,
                        if served.is_empty() {
                            "no listeners".into()
                        } else {
                            served
                        },
                    )
                    .reported(controller),
                None,
            );
        }
    };
    let serves = describe_listener(listener);
    let reported = gateway
        .listener_status
        .iter()
        .find(|s| s.name == listener.name);
    let step = match reported {
        None if !gateway.listener_status.is_empty() || !gateway.conditions.is_empty() => {
            TraceStep::unknown(
                id,
                Owner::Controller,
                format!(
                    "the controller reports nothing for listener {}",
                    listener.name
                ),
            )
        }
        None => TraceStep::pass(
            id,
            format!("listener {} fits by configuration", listener.name),
        ),
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
                None => {
                    TraceStep::pass(id, format!("listener {} accepts HTTPRoute", listener.name))
                        .reported(find(&reported.conditions, "Accepted").map(Cond::says))
                }
            }
        }
    };
    (step.quoting(asks, serves), Some(listener.clone()))
}

pub(super) fn namespace_step(
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
                // A cut list is no basis for a label match either way.
                Ok(namespaces) if namespaces.cut => TraceStep::unknown(
                    id,
                    Owner::Platform,
                    "the namespace list was cut, so the selector can't be checked",
                )
                .quoting(asks, serves),
                Ok(namespaces) => {
                    match namespaces
                        .items
                        .iter()
                        .find(|(name, _)| *name == route.namespace)
                    {
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
