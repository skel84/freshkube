//! A Service's published endpoints, and a workload's traffic chain down to the
//! Services that select it (#949, N1). One read per document read, started
//! only while the pane is active and dropped with it; the logic is core's
//! `resources::traffic`, this derives the rows once per answer and draws them.
use super::*;
use crate::ui::Tone;
use freshkube_core::resources::traffic::{
    self, EndpointState, Evidence, ServiceReading, StepKind, TrafficChain, Verdict,
};
use gpui_kit::component::{h_flex, v_flex};
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;
mod view;

/// Endpoints drawn per group; the rest are counted.
const GROUP_SHOWN: usize = 50;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Service,
    Workload,
}

struct StepLine {
    id: SharedString,
    kind: &'static str,
    subject: SharedString,
    evidence: &'static str,
    tone: Tone,
    summary: SharedString,
    detail: Option<SharedString>,
}

struct GroupLine {
    id: SharedString,
    title: &'static str,
    tone: Tone,
    /// What the group means, where it could be read as a failure.
    hint: Option<&'static str>,
    count: usize,
    rows: Vec<(SharedString, SharedString)>,
    more: usize,
}

struct PodLine {
    id: SharedString,
    pod: SharedString,
    reason: &'static str,
    detail: SharedString,
}

enum Loaded {
    Service(Box<Result<ServiceReading, Failure>>),
    Workload(Result<traffic::WorkloadServices, Failure>),
}

#[derive(Default)]
pub(super) struct TrafficView {
    scope: Option<Scope>,
    loaded: bool,
    steps: Vec<StepLine>,
    groups: Vec<GroupLine>,
    pods: Vec<PodLine>,
    /// Why the unpublished list is not shown, when it can't be.
    pods_note: Option<SharedString>,
    /// The unpublished list is cut, by the draw cap or by the read's limit.
    pods_more: Option<SharedString>,
    source: Option<SharedString>,
    job: Option<Job>,
    seq: u64,
    shape: crate::example::traffic::Shape,
}

fn tone(verdict: Verdict) -> Tone {
    match verdict {
        Verdict::Good => Tone::Good,
        Verdict::Warn => Tone::Warn,
        Verdict::Broken => Tone::Crit,
        Verdict::Unknown => Tone::Unknown,
        Verdict::Neutral => Tone::Outline,
    }
}

impl TrafficView {
    pub(super) fn set_shape(&mut self, shape: crate::example::traffic::Shape) {
        self.shape = shape;
    }

    pub(super) fn reset(&mut self) {
        self.stop();
        *self = Self {
            seq: self.seq,
            shape: self.shape,
            ..Default::default()
        };
    }

    pub(super) fn stop(&mut self) {
        self.job = None;
        self.seq = self.seq.wrapping_add(1);
    }

    fn has_unknown(&self) -> bool {
        self.steps.iter().any(|step| step.tone == Tone::Unknown) || self.pods_note.is_some()
    }

    fn lines(chain: &TrafficChain) -> Vec<StepLine> {
        chain
            .steps
            .iter()
            .enumerate()
            .map(|(ix, step)| StepLine {
                id: format!("traffic-step-{ix}").into(),
                kind: match step.kind {
                    StepKind::Workload => "Workload",
                    StepKind::Service => "Service",
                    StepKind::LoadBalancer => "Load balancer",
                    StepKind::Endpoints => "Endpoints",
                },
                subject: step.subject.name.clone().into(),
                evidence: match step.evidence {
                    Evidence::Config => "Config",
                    Evidence::Status => "Status",
                },
                tone: tone(step.verdict),
                summary: step.summary.clone().into(),
                detail: step.detail.clone().map(Into::into),
            })
            .collect()
    }

    fn apply_service(&mut self, reading: Result<ServiceReading, Failure>) {
        self.groups.clear();
        self.pods.clear();
        self.pods_note = None;
        self.pods_more = None;
        self.source = None;
        let reading = match reading {
            Ok(reading) => reading,
            Err(failure) => {
                self.steps = vec![StepLine {
                    id: "traffic-step-0".into(),
                    kind: "Service",
                    subject: "".into(),
                    evidence: "Status",
                    tone: Tone::Unknown,
                    summary: "Unknown: the Service could not be read".into(),
                    detail: Some(format!("{}: {}", failure.kind.label(), failure.message).into()),
                }];
                return;
            }
        };
        self.steps = Self::lines(&traffic::service_chain(&reading));
        // An alias publishes nothing, so there is no source or group to draw.
        let alias = matches!(
            reading.facts.shape(),
            freshkube_core::resources::traffic::ServiceShape::ExternalName(_)
        );
        if alias {
            return;
        }
        let endpoints_failed = reading.published.is_err();
        if let Ok(published) = &reading.published {
            self.source = Some(published.source.label().into());
            for state in EndpointState::ALL {
                let count = published.count(state);
                if count == 0 {
                    continue;
                }
                self.groups.push(GroupLine {
                    id: format!(
                        "traffic-group-{}",
                        state.label().to_lowercase().replace(' ', "-")
                    )
                    .into(),
                    title: state.label(),
                    tone: match state {
                        EndpointState::Ready => Tone::Good,
                        _ => Tone::Warn,
                    },
                    hint: (state == EndpointState::Unrouted)
                        .then_some("No port on this slice matches a Service port"),
                    count,
                    rows: published
                        .group(state)
                        .take(GROUP_SHOWN)
                        .map(|endpoint| {
                            (
                                format!("traffic-endpoint-{}", endpoint.address).into(),
                                [
                                    Some(endpoint.address.as_str()),
                                    endpoint.pod.as_deref(),
                                    endpoint.node.as_deref(),
                                ]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(" · ")
                                .into(),
                            )
                        })
                        .collect(),
                    more: count.saturating_sub(GROUP_SHOWN),
                });
            }
        }
        match reading.unpublished {
            Some(Ok(pods)) => {
                let mut more = Vec::new();
                if pods.len() > GROUP_SHOWN {
                    more.push(format!("{} more not shown", pods.len() - GROUP_SHOWN));
                }
                if reading.pods_cut {
                    more.push("The pod list was cut at its limit; more may exist".to_owned());
                }
                self.pods_more = (!more.is_empty()).then(|| more.join(". ").into());
                self.pods = pods
                    .iter()
                    .take(GROUP_SHOWN)
                    .map(|pod| PodLine {
                        id: format!("traffic-pod-{}", pod.name).into(),
                        pod: pod.name.clone().into(),
                        reason: pod.reason.label(),
                        detail: pod.detail.clone().into(),
                    })
                    .collect();
            }
            Some(Err(failure)) => {
                // The endpoints step already quotes a failed endpoint read.
                self.pods_note = Some(if endpoints_failed {
                    "Unpublished pods unknown: the endpoints could not be read".into()
                } else {
                    format!(
                        "Unpublished pods unknown: {}: {}",
                        failure.kind.label(),
                        failure.message
                    )
                    .into()
                });
            }
            None => {}
        }
    }

    fn apply_workload(
        &mut self,
        kind: &str,
        namespace: &str,
        name: &str,
        labels: &BTreeMap<String, String>,
        services: Result<traffic::WorkloadServices, Failure>,
    ) {
        self.groups.clear();
        self.pods.clear();
        self.pods_note = None;
        self.pods_more = None;
        self.source = None;
        self.steps = Self::lines(&traffic::workload_chain(
            kind, namespace, name, labels, &services,
        ));
    }
}

impl DetailPane {
    pub(crate) fn set_services_shape(&mut self, shape: crate::example::traffic::Shape) {
        self.traffic.set_shape(shape);
    }

    /// Reads what the open Service publishes, or the Services a workload's
    /// pods are behind. Nothing else shows traffic.
    pub(super) fn start_traffic(&mut self, cx: &mut Context<Self>) {
        self.traffic.stop();
        let Some(view) = self.detail.as_ref().and_then(|d| d.view.clone()) else {
            self.traffic.reset();
            return;
        };
        let document = &view.document;
        let scope = if document.kind == "Service" && document.api_version == "v1" {
            Scope::Service
        } else if document.overview.template_labels.is_some() {
            Scope::Workload
        } else {
            self.traffic.reset();
            return;
        };
        let changed = self.traffic.scope != Some(scope);
        if changed {
            self.traffic.reset();
        }
        self.traffic.scope = Some(scope);
        if !self.active {
            return;
        }
        let Some(access) = self.access.clone() else {
            return;
        };
        let identity = self.detail.as_ref().unwrap().target.identity.clone();
        let seq = self.traffic.seq;
        let namespace = identity.namespace.clone();
        let name = identity.name.clone();
        let labels: BTreeMap<String, String> = document
            .overview
            .template_labels
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect();
        let kind = document.kind.clone();
        let meta = (kind, labels.clone());
        if let ClusterAccess::Example(_) = access {
            let loaded = self.example_traffic(&identity, scope, &view.document.yaml, &labels);
            self.finish_traffic(&identity, seq, loaded, meta, cx);
            return;
        }
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            READ_DEADLINE,
            "Traffic read timed out".to_owned(),
            async move {
                let client = access.client().await.map_err(|failure| failure.message)?;
                Ok(match scope {
                    Scope::Service => Loaded::Service(Box::new(
                        traffic::read_service_by_name(&client, &namespace, &name).await,
                    )),
                    Scope::Workload => Loaded::Workload(
                        traffic::read_workload_services(&client, &namespace, &labels).await,
                    ),
                })
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("The traffic worker stopped".into()));
            _ = this.update(cx, |pane, cx| {
                let loaded = result.unwrap_or_else(|message| {
                    let failure = Failure::new(FailureKind::Timeout, message);
                    match scope {
                        Scope::Service => Loaded::Service(Box::new(Err(failure))),
                        Scope::Workload => Loaded::Workload(Err(failure)),
                    }
                });
                pane.finish_traffic(&identity, seq, loaded, meta, cx);
            });
        });
        self.traffic.job = Some((job, task));
    }

    fn example_traffic(
        &self,
        identity: &ResourceIdentity,
        scope: Scope,
        yaml: &str,
        labels: &BTreeMap<String, String>,
    ) -> Loaded {
        use crate::example::traffic::{Shape, facts, reading};
        let shape = self.traffic.shape;
        let context = identity
            .connection
            .strip_prefix("example:")
            .unwrap_or("prod-fra");
        match scope {
            Scope::Service => Loaded::Service(Box::new(
                facts(yaml, shape)
                    .map(|facts| reading(facts, shape, true))
                    .ok_or_else(|| Failure::new(FailureKind::Other, "Example Service unreadable")),
            )),
            Scope::Workload if shape == Shape::ServicesRefused => {
                Loaded::Workload(Err(Failure::new(
                    FailureKind::Forbidden,
                    "services is forbidden for the invented viewer",
                )))
            }
            Scope::Workload => Loaded::Workload(Ok(example::read(
                context,
                "services",
                Some(&identity.namespace),
                live::now(),
            )
            .map(|(_, rows)| {
                rows.into_iter()
                    .filter_map(|row| example::document(&row.identity, live::now()))
                    .filter_map(|document| facts(&document.yaml, shape))
                    .filter(|facts| facts.selects(labels))
                    .map(|facts| reading(facts, shape, false))
                    .collect::<Vec<_>>()
            })
            .map(|readings| {
                let total = readings.len();
                traffic::WorkloadServices::new(readings, total)
            })
            .unwrap_or_else(|| traffic::WorkloadServices::new(Vec::new(), 0)))),
        }
    }

    fn finish_traffic(
        &mut self,
        identity: &ResourceIdentity,
        seq: u64,
        loaded: Loaded,
        meta: (String, BTreeMap<String, String>),
        cx: &mut Context<Self>,
    ) {
        if !self.active
            || seq != self.traffic.seq
            || self.target_identity() != Some(identity)
            || self
                .detail
                .as_ref()
                .is_some_and(|detail| detail.read == DocumentRead::Deleted)
        {
            return;
        }
        self.traffic.job = None;
        self.traffic.loaded = true;
        match loaded {
            Loaded::Service(reading) => self.traffic.apply_service(*reading),
            Loaded::Workload(services) => self.traffic.apply_workload(
                &meta.0,
                &identity.namespace,
                &identity.name,
                &meta.1,
                services,
            ),
        }
        cx.notify();
    }

    pub(super) fn retry_traffic(&mut self, cx: &mut Context<Self>) {
        self.traffic.loaded = false;
        self.start_traffic(cx);
        cx.notify();
    }
}
