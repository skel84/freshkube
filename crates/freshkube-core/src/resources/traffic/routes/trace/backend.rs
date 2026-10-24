//! The steps of one backend: its reference, its Service and its endpoints,
//! and the branches a route's rules make of them.
use super::parent::overruled;
use super::*;

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
    let asks = format!(
        "Service {namespace}/{} from {}",
        backend.name, route.namespace
    );
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
            let step =
                TraceStep::pass(id, format!("granted by {}/{}", grant.namespace, grant.name))
                    .quoting(
                        asks,
                        format!("ReferenceGrant {}/{}", grant.namespace, grant.name),
                    );
            if refused.is_some() {
                let mut step = step;
                step.verdict = StepVerdict::Fail;
                step.owner = Some(Owner::Controller);
                step.reason =
                    "a grant exists, but the controller still refuses the reference".into();
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
            let step = TraceStep::fail(
                id,
                Owner::Platform,
                format!(
                    "no ReferenceGrant in {namespace} lets HTTPRoutes in {} refer to Service {}",
                    route.namespace, backend.name
                ),
            )
            .quoting(asks, "no grant")
            .reported(refused);
            // No grant found never overrules the controller saying it
            // resolved the reference: that is Unknown, and proposes nothing.
            let step = overruled(
                step,
                route_condition(status, "ResolvedRefs"),
                route.generation,
            );
            let grant = (step.verdict == StepVerdict::Fail
                && dns_name(&route.namespace)
                && dns_name(namespace)
                && dns_name(&backend.name))
            .then(|| {
                proposal(
                    inputs.grant_version,
                    &route.namespace,
                    namespace,
                    &backend.name,
                )
            });
            (step, grant)
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
        .filter(|c| {
            c.status == Tri::False && matches!(c.reason.as_str(), "BackendNotFound" | "InvalidKind")
        })
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
                format!(
                    "the Service could not be read ({}: {})",
                    failure.kind.label(),
                    failure.message
                ),
            )
            .quoting(asks, "unknown")
            .reported(missing);
        }
        Some(Ok(None)) => {
            return TraceStep::fail(
                id,
                Owner::Yours,
                format!("Service {namespace}/{} does not exist", backend.name),
            )
            .quoting(asks, "no such Service")
            .reported(missing);
        }
        Some(Ok(Some(reading))) => reading,
    };
    let facts = &reading.facts;
    let ports = facts
        .ports
        .iter()
        .map(|p| p.port.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let serves = format!(
        "ports {}",
        if ports.is_empty() {
            "none".into()
        } else {
            ports
        }
    );
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
                format!(
                    "endpoints unreadable ({}: {})",
                    failure.kind.label(),
                    failure.message
                ),
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
    if ready == 0 && published.truncated {
        return TraceStep::unknown(
            id,
            Owner::Yours,
            "none of the endpoints read is ready, but the list was cut, so a ready one may exist",
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
    } else if steps
        .iter()
        .any(|s| matches!(s.verdict, StepVerdict::Unknown | StepVerdict::Neutral))
    {
        BranchVerdict::Unknown
    } else {
        BranchVerdict::Serving
    }
}

pub(super) fn branches(
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
                simple(
                    BranchKind::Redirect,
                    "redirect",
                    BranchVerdict::Serving,
                    None,
                )
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
                    format!(
                        "a {} backend is not a Service, so it is not judged",
                        backend.kind
                    ),
                ));
                out.push(branch);
                continue;
            }
            let read = inputs
                .backends
                .get(&(namespace.to_owned(), backend.name.clone()));
            let mut chain = Chain::new();
            let mut proposal = None;
            chain.run(StepId::Refs, || {
                let (step, p) = refs_step(route, backend, namespace, status, inputs);
                proposal = p;
                step
            });
            chain.run(StepId::Service, || {
                service_step(backend, namespace, read, status)
            });
            chain.run(StepId::Endpoints, || match read {
                Some(Ok(Some(reading))) => endpoints_step(backend, namespace, reading),
                _ => {
                    TraceStep::unknown(StepId::Endpoints, Owner::Yours, "the Service was not read")
                }
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
