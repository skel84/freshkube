//! The traffic chain as a list of steps. Each step is one piece of evidence,
//! either configuration (what a spec asks for) or status (what a controller
//! reports); the two are never merged into one line. Later slices put
//! routes and gateways in front of the first step with [`TrafficChain::prepend`].
use super::endpoints::{EndpointState, Published, UnpublishedPod};
use super::service::{LoadBalancerState, ServiceFacts, ServiceKind, ServiceShape};
use crate::failure::Failure;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    Workload,
    Service,
    LoadBalancer,
    Endpoints,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evidence {
    /// A spec: what was asked for.
    Config,
    /// A status or a slice: what a controller reports.
    Status,
}

/// How a step reads. `Unknown` is its own verdict and is never drawn green.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Good,
    Warn,
    Broken,
    Unknown,
    /// Nothing is wrong or right: an alias, a hand-managed Service.
    Neutral,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Subject {
    pub kind: String,
    pub namespace: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TrafficStep {
    pub kind: StepKind,
    pub subject: Subject,
    pub evidence: Evidence,
    pub verdict: Verdict,
    pub summary: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrafficChain {
    pub steps: Vec<TrafficStep>,
}

impl TrafficChain {
    /// Puts `steps` in front, as a route or gateway step would be.
    pub fn prepend(&mut self, steps: Vec<TrafficStep>) {
        self.steps.splice(0..0, steps);
    }
}

/// One Service as read: its spec, what it publishes and the pods it selects
/// that nothing publishes. Each read stands alone, so one refused read makes
/// only its own part unknown.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ServiceReading {
    pub facts: ServiceFacts,
    pub published: Result<Published, Failure>,
    /// `None` when pods were not asked for (the workload chain).
    pub unpublished: Option<Result<Vec<UnpublishedPod>, Failure>>,
}

impl ServiceReading {
    pub fn new(
        facts: ServiceFacts,
        published: Result<Published, Failure>,
        unpublished: Option<Result<Vec<UnpublishedPod>, Failure>>,
    ) -> Self {
        Self {
            facts,
            published,
            unpublished,
        }
    }

    /// The Service's steps: its spec, a LoadBalancer's address, then what it
    /// publishes.
    pub fn steps(&self) -> Vec<TrafficStep> {
        let facts = &self.facts;
        let subject = Subject {
            kind: "Service".into(),
            namespace: facts.namespace.clone(),
            name: facts.name.clone(),
        };
        let ports = facts
            .ports
            .iter()
            .map(|port| format!("{}/{}", port.port, port.protocol))
            .collect::<Vec<_>>()
            .join(", ");
        let spec = match facts.shape() {
            ServiceShape::ExternalName(target) => (
                Verdict::Neutral,
                format!("ExternalName alias of {target}"),
                Some("A DNS alias: nothing is published, and nothing here is broken".to_owned()),
            ),
            ServiceShape::NoSelector => (
                Verdict::Neutral,
                format!("{} · no selector", facts.kind.label()),
                Some(
                    "Its slices or Endpoints are managed by hand or by another controller"
                        .to_owned(),
                ),
            ),
            ServiceShape::Selector => (
                Verdict::Neutral,
                format!(
                    "{} · selects {}",
                    facts.kind.label(),
                    facts.selector_query()
                ),
                None,
            ),
        };
        let mut steps = vec![TrafficStep {
            kind: StepKind::Service,
            subject: subject.clone(),
            evidence: Evidence::Config,
            verdict: spec.0,
            summary: format!(
                "{}{}",
                spec.1,
                if ports.is_empty() {
                    String::new()
                } else {
                    format!(" · ports {ports}")
                }
            ),
            detail: spec.2,
        }];
        if facts.kind == ServiceKind::LoadBalancer {
            let (verdict, summary) = match &facts.load_balancer {
                LoadBalancerState::Pending => (
                    Verdict::Warn,
                    "LoadBalancer pending: no ingress address yet".to_owned(),
                ),
                LoadBalancerState::Addresses(addresses) => {
                    (Verdict::Good, format!("Address {}", addresses.join(", ")))
                }
                LoadBalancerState::NotApplicable => (Verdict::Unknown, String::new()),
            };
            steps.push(TrafficStep {
                kind: StepKind::LoadBalancer,
                subject: subject.clone(),
                evidence: Evidence::Status,
                verdict,
                summary,
                detail: None,
            });
        }
        if let ServiceShape::ExternalName(_) = facts.shape() {
            return steps;
        }
        steps.push(self.published_step(subject));
        steps
    }

    fn published_step(&self, subject: Subject) -> TrafficStep {
        let step = |verdict, summary: String, detail| TrafficStep {
            kind: StepKind::Endpoints,
            subject: subject.clone(),
            evidence: Evidence::Status,
            verdict,
            summary,
            detail,
        };
        let published = match &self.published {
            Ok(published) => published,
            Err(failure) => {
                return step(
                    Verdict::Unknown,
                    "Endpoints unknown: the read failed".into(),
                    Some(format!("{}: {}", failure.kind.label(), failure.message)),
                );
            }
        };
        let counts = EndpointState::ALL
            .iter()
            .filter_map(|state| {
                let count = published.count(*state);
                (count > 0).then(|| format!("{count} {}", state.label().to_lowercase()))
            })
            .collect::<Vec<_>>();
        let via = format!("via {}", published.source.label());
        let cut = published
            .truncated
            .then_some("The list was cut; more may exist".to_owned());
        if published.endpoints.is_empty() {
            return match self.facts.shape() {
                ServiceShape::NoSelector => step(
                    Verdict::Neutral,
                    format!("No endpoints yet {via}"),
                    Some("A Service with no selector publishes what is added to it".into()),
                ),
                _ => step(
                    Verdict::Broken,
                    format!("Publishes no endpoints {via}"),
                    cut,
                ),
            };
        }
        let ready = published.count(EndpointState::Ready);
        let verdict = if ready == 0 {
            Verdict::Broken
        } else if ready == published.endpoints.len() && !published.truncated {
            Verdict::Good
        } else {
            Verdict::Warn
        };
        step(verdict, format!("{} {via}", counts.join(" · ")), cut)
    }
}

/// The chain for a Service: its own steps.
pub fn service_chain(reading: &ServiceReading) -> TrafficChain {
    TrafficChain {
        steps: reading.steps(),
    }
}

/// The chain for a workload: the workload, then, for each Service that
/// selects its pods, that Service's steps. A refused Services list is one
/// Unknown step, never "no Services".
pub fn workload_chain(
    kind: &str,
    namespace: &str,
    name: &str,
    template_labels: &BTreeMap<String, String>,
    services: &Result<Vec<ServiceReading>, Failure>,
) -> TrafficChain {
    let subject = Subject {
        kind: kind.into(),
        namespace: namespace.into(),
        name: name.into(),
    };
    let step = |verdict, summary: String, detail| TrafficStep {
        kind: StepKind::Workload,
        subject: subject.clone(),
        evidence: Evidence::Config,
        verdict,
        summary,
        detail,
    };
    let mut chain = TrafficChain::default();
    match services {
        Err(failure) => chain.steps.push(step(
            Verdict::Unknown,
            "Services unknown: the list failed".into(),
            Some(format!("{}: {}", failure.kind.label(), failure.message)),
        )),
        Ok(readings) => {
            let selecting = readings
                .iter()
                .filter(|reading| reading.facts.selects(template_labels))
                .collect::<Vec<_>>();
            chain.steps.push(if selecting.is_empty() {
                step(
                    Verdict::Neutral,
                    "No Service selects its pods".into(),
                    Some("Nothing in this namespace sends traffic here through a Service".into()),
                )
            } else {
                step(
                    Verdict::Neutral,
                    format!("Selected by {} Service(s)", selecting.len()),
                    None,
                )
            });
            for reading in selecting {
                chain.steps.extend(reading.steps());
            }
        }
    }
    chain
}
