//! NetworkPolicy in a pane (#949, N5): a policy's own Overview (what it
//! selects and what each direction allows) and, on a Pod or a workload, which
//! policies apply and what each direction comes to. One read per document
//! read, started only while the pane is active and dropped with it; the logic
//! is core's `resources::traffic::netpol`, this derives the lines once per
//! answer and draws them. Only Kubernetes NetworkPolicy is read, and the
//! pane says so beside every verdict.
use super::*;
use crate::ui::Tone;
use freshkube_core::resources::traffic::netpol::{
    self, Applying, Direction, DirectionVerdict, PolicyReading, SCOPE_NOTE,
};
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;
mod view;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Policy,
    /// A Pod or a workload: the policies that apply to its pods.
    Pods,
}

enum Loaded {
    Policy(Box<Result<PolicyReading, Failure>>),
    Applying(Result<Applying, Failure>),
}

struct DirectionLine {
    id: SharedString,
    direction: &'static str,
    tone: Tone,
    verdict: &'static str,
    /// One line per rule, in words (AND within an entry, OR across entries).
    rules: Vec<SharedString>,
}

struct AppliedLine {
    id: SharedString,
    name: SharedString,
    text: SharedString,
}

/// What the pane draws: a policy, the policies on a pod, or why neither.
enum Body {
    Policy {
        selector: SharedString,
        /// How many pods it selects, or why that isn't known.
        pods: SharedString,
        pods_tone: Tone,
        directions: Vec<DirectionLine>,
    },
    Applying {
        directions: Vec<DirectionLine>,
        policies: Vec<AppliedLine>,
        /// Policies whose selector could not be judged.
        uncertain: bool,
    },
    Message {
        tone: Tone,
        text: SharedString,
    },
}

#[derive(Default)]
pub(super) struct NetpolView {
    scope: Option<Scope>,
    loaded: bool,
    body: Option<Body>,
    job: Option<Job>,
    seq: u64,
    shape: crate::example::netpol::Shape,
}

fn tone(verdict: DirectionVerdict) -> Tone {
    match verdict {
        DirectionVerdict::NotGoverned | DirectionVerdict::Restricts => Tone::Outline,
        DirectionVerdict::DeniesAll | DirectionVerdict::OpensToAll => Tone::Warn,
        DirectionVerdict::Unknown => Tone::Unknown,
    }
}

fn failure_text(what: &str, failure: &Failure) -> SharedString {
    format!(
        "{what} unknown: {}: {}",
        failure.kind.label(),
        failure.message
    )
    .into()
}

impl NetpolView {
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
        match &self.body {
            Some(Body::Message { tone, .. }) => *tone == Tone::Unknown,
            Some(Body::Policy {
                pods_tone,
                directions,
                ..
            }) => *pods_tone == Tone::Unknown || directions.iter().any(|d| d.tone == Tone::Unknown),
            Some(Body::Applying {
                directions,
                uncertain,
                ..
            }) => *uncertain || directions.iter().any(|d| d.tone == Tone::Unknown),
            None => false,
        }
    }

    fn apply_policy(&mut self, reading: Result<PolicyReading, Failure>) {
        self.body = Some(match reading {
            Err(failure) => Body::Message {
                tone: Tone::Unknown,
                text: failure_text("The policy", &failure),
            },
            Ok(reading) => {
                let policy = &reading.policy;
                let (pods, pods_tone) = match &reading.pods {
                    Err(failure) => (failure_text("Selected pods", failure), Tone::Unknown),
                    Ok(count) => {
                        let mut text = format!(
                            "Selects {}{} of {} pod{} here",
                            if count.cut { "at least " } else { "" },
                            count.selected,
                            count.total,
                            if count.total == 1 { "" } else { "s" },
                        );
                        if count.cut {
                            text.push_str(" (the pod list was cut at its limit)");
                        }
                        if count.unknown > 0 {
                            text.push_str(&format!("; {} could not be judged", count.unknown));
                        }
                        (
                            text.into(),
                            if count.unknown > 0 {
                                Tone::Unknown
                            } else {
                                Tone::Outline
                            },
                        )
                    }
                };
                Body::Policy {
                    selector: format!("Pods: {}", policy.selector_words()).into(),
                    pods,
                    pods_tone,
                    directions: Direction::ALL
                        .into_iter()
                        .map(|direction| {
                            let verdict = policy.verdict(direction);
                            DirectionLine {
                                id: format!("netpol-{}", direction.label().to_lowercase()).into(),
                                direction: direction.label(),
                                tone: tone(verdict),
                                verdict: verdict.label(),
                                rules: if verdict == DirectionVerdict::NotGoverned {
                                    Vec::new()
                                } else {
                                    policy
                                        .rules(direction)
                                        .iter()
                                        .map(|rule| {
                                            rule.describe(match direction {
                                                Direction::Ingress => "from",
                                                Direction::Egress => "to",
                                            })
                                            .into()
                                        })
                                        .collect()
                                },
                            }
                        })
                        .collect(),
                }
            }
        });
    }

    fn apply_applying(&mut self, read: Result<Applying, Failure>) {
        self.body = Some(match read {
            Err(failure) => Body::Message {
                tone: Tone::Unknown,
                text: failure_text("Network policies", &failure),
            },
            Ok(applying) => Body::Applying {
                directions: Direction::ALL
                    .into_iter()
                    .map(|direction| {
                        let verdict = applying.verdict(direction);
                        DirectionLine {
                            id: format!("netpol-{}", direction.label().to_lowercase()).into(),
                            direction: direction.label(),
                            tone: tone(verdict),
                            verdict: verdict.label(),
                            rules: Vec::new(),
                        }
                    })
                    .collect(),
                uncertain: applying.policies.iter().any(|p| p.uncertain),
                policies: applying
                    .policies
                    .iter()
                    .map(|policy| AppliedLine {
                        id: format!("netpol-applied-{}", policy.name).into(),
                        name: policy.name.clone().into(),
                        text: format!(
                            "Ingress {} · Egress {}{}",
                            policy.ingress.label().to_lowercase(),
                            policy.egress.label().to_lowercase(),
                            if policy.uncertain {
                                " · its selector could not be judged"
                            } else {
                                ""
                            }
                        )
                        .into(),
                    })
                    .collect(),
            },
        });
    }
}

impl DetailPane {
    pub(crate) fn set_netpol_shape(&mut self, shape: crate::example::netpol::Shape) {
        self.netpol.shape = shape;
    }

    /// Reads the policy the open NetworkPolicy is, or the policies that apply
    /// to the open Pod or workload. Nothing else shows network policies.
    pub(super) fn start_netpol(&mut self, cx: &mut Context<Self>) {
        self.netpol.stop();
        let Some(view) = self.detail.as_ref().and_then(|d| d.view.clone()) else {
            self.netpol.reset();
            return;
        };
        let document = &view.document;
        let (scope, labels): (Scope, BTreeMap<String, String>) =
            if document.kind == "NetworkPolicy" && document.api_version == "networking.k8s.io/v1" {
                (Scope::Policy, BTreeMap::new())
            } else if document.kind == "Pod" && document.api_version == "v1" {
                (
                    Scope::Pods,
                    document.overview.labels.iter().cloned().collect(),
                )
            } else if let Some(template) = &document.overview.template_labels {
                (Scope::Pods, template.iter().cloned().collect())
            } else {
                self.netpol.reset();
                return;
            };
        if self.netpol.scope != Some(scope) {
            self.netpol.reset();
        }
        self.netpol.scope = Some(scope);
        if !self.active {
            return;
        }
        let Some(access) = self.access.clone() else {
            return;
        };
        let identity = self.detail.as_ref().unwrap().target.identity.clone();
        let seq = self.netpol.seq;
        let (namespace, name) = (identity.namespace.clone(), identity.name.clone());
        if let ClusterAccess::Example(_) = access {
            let shape = self.netpol.shape;
            let context = identity
                .connection
                .strip_prefix("example:")
                .unwrap_or("prod-fra");
            let loaded = match scope {
                Scope::Policy => Loaded::Policy(Box::new(crate::example::netpol::read_policy(
                    &document.yaml,
                    shape,
                    context,
                    live::now(),
                ))),
                Scope::Pods => Loaded::Applying(crate::example::netpol::read_applying(
                    shape, &namespace, &labels,
                )),
            };
            self.finish_netpol(&identity, seq, loaded, cx);
            return;
        }
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            READ_DEADLINE,
            "Network policy read timed out".to_owned(),
            async move {
                let client = access.client().await.map_err(|failure| failure.message)?;
                Ok(match scope {
                    Scope::Policy => Loaded::Policy(Box::new(
                        netpol::read_policy_by_name(&client, &namespace, &name).await,
                    )),
                    Scope::Pods => {
                        Loaded::Applying(netpol::read_applying(&client, &namespace, &labels).await)
                    }
                })
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("The network policy worker stopped".into()));
            _ = this.update(cx, |pane, cx| {
                let loaded = result.unwrap_or_else(|message| {
                    let failure = Failure::new(FailureKind::Timeout, message);
                    match scope {
                        Scope::Policy => Loaded::Policy(Box::new(Err(failure))),
                        Scope::Pods => Loaded::Applying(Err(failure)),
                    }
                });
                pane.finish_netpol(&identity, seq, loaded, cx);
            });
        });
        self.netpol.job = Some((job, task));
    }

    fn finish_netpol(
        &mut self,
        identity: &ResourceIdentity,
        seq: u64,
        loaded: Loaded,
        cx: &mut Context<Self>,
    ) {
        if !self.active
            || seq != self.netpol.seq
            || self.target_identity() != Some(identity)
            || self
                .detail
                .as_ref()
                .is_some_and(|detail| detail.read == DocumentRead::Deleted)
        {
            return;
        }
        self.netpol.job = None;
        self.netpol.loaded = true;
        match loaded {
            Loaded::Policy(reading) => self.netpol.apply_policy(*reading),
            Loaded::Applying(read) => self.netpol.apply_applying(read),
        }
        cx.notify();
    }

    pub(super) fn retry_netpol(&mut self, cx: &mut Context<Self>) {
        self.netpol.loaded = false;
        self.start_netpol(cx);
        cx.notify();
    }
}
