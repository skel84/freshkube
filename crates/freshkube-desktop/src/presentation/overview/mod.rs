//! Overview text and destinations prepared from the shared snapshots.
use super::{NodeSummary, Roster, cluster_summary};
use crate::{
    desktop::{
        Page,
        nodes::{NodeRow, NodeTab},
    },
    presentation::attention::Destination,
    ui::Tone,
};
use freshkube_core::{
    cluster_overview::ClusterOverview,
    kubernetes_summary::{KubernetesSummary, Part},
    workloads::HealthState,
};
use gpui_kit::SharedString;

#[derive(Clone)]
pub(crate) enum CardTarget {
    Page(Page),
    Kind(&'static str, String),
    Destination(Destination),
    Services,
}
pub(crate) struct Card {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) figure: SharedString,
    pub(crate) detail: SharedString,
    pub(crate) tone: Tone,
    pub(crate) segments: Vec<Tone>,
    pub(crate) meter: Option<(f64, super::MemoryLevel)>,
    pub(crate) target: CardTarget,
}
#[derive(Default)]
pub(crate) struct Overview {
    pub(crate) cards: Vec<Card>,
    pub(crate) subtitle: SharedString,
    pub(crate) roster: SharedString,
    pub(crate) roster_tip: SharedString,
    pub(crate) drift: Option<SharedString>,
    pub(crate) drift_tip: SharedString,
    pub(crate) warnings: Vec<SharedString>,
}
fn part<T>(
    value: Option<&Part<T>>,
    kind: &str,
    loaded: impl FnOnce(&T) -> (String, String, Tone),
) -> (String, String, Tone) {
    match value {
        Some(Part::Loaded(value)) => loaded(value),
        Some(Part::Refused(error) | Part::Failed(error)) => {
            let figure = error
                .last_good
                .as_ref()
                .map(|value| loaded(value).0)
                .unwrap_or_else(|| "Unavailable".into());
            let detail = if error.last_good.is_some() {
                format!("Last known · {kind}: {error}")
            } else {
                format!("Can't read {kind}: {error}")
            };
            (figure, detail, Tone::Unknown)
        }
        None => ("—".into(), format!("Waiting for {kind}"), Tone::Unknown),
    }
}
impl Overview {
    pub(crate) fn build(
        rows: &[NodeRow],
        nodes: &[NodeSummary],
        kube: Option<&KubernetesSummary>,
        talos: Option<&ClusterOverview>,
        fixture: bool,
        kube_only: bool,
    ) -> Self {
        let mut result = Self::default();
        let summary = talos.map(|cluster| cluster_summary(cluster, nodes));
        let mut subtitles = Vec::new();
        if let Some(summary) = &summary {
            subtitles.push(format!("Talos {}", summary.versions.join(" / ")));
            if !summary.platforms.is_empty() || !summary.arches.is_empty() {
                subtitles.push(format!(
                    "{} {}",
                    summary.platforms.join(" / "),
                    summary.arches.join(" / ")
                ));
            }
            if kube.is_none() {
                subtitles.push(format!(
                    "{} of {} Talos nodes answering · {} control planes · {} workers",
                    summary.responding, summary.total, summary.control_planes, summary.workers
                ));
            }
            if summary.versions.len() > 1 {
                result.drift = Some(format!("{} Talos versions", summary.versions.len()).into());
                result.drift_tip = format!("Nodes run {}", summary.versions.join(" and ")).into();
            }
        }
        if let Some(kube) = kube {
            if let Some(version) = kube.version.loaded() {
                subtitles.push(format!("Kubernetes {version}"));
            }
            if kube.nodes.loaded().is_some() || talos.is_some() {
                let suffix = if kube.nodes.is_current() {
                    ""
                } else {
                    " (last known)"
                };
                subtitles.push(format!("{} nodes{suffix}", rows.len()));
            } else {
                subtitles.push("Nodes unavailable".into());
            }
            if let Some(pods) = kube.pods.loaded() {
                let suffix = if kube.pods.is_current() {
                    ""
                } else {
                    " (last known)"
                };
                subtitles.push(format!("{} pods{suffix}", pods.total));
            }
            if let Some(namespaces) = kube.namespaces.loaded() {
                let suffix = if kube.namespaces.is_current() {
                    ""
                } else {
                    " (last known)"
                };
                subtitles.push(format!("{namespaces} namespaces{suffix}"));
            }
        }
        result.subtitle = subtitles.join(" · ").into();
        if let Some(talos) = talos {
            let roster = if fixture {
                Roster::Fixture
            } else if !talos.discovery_members.is_empty() {
                Roster::Discovery
            } else {
                Roster::FallbackOrEndpoints
            };
            result.roster = format!("Roster: {}", roster.label()).into();
            result.roster_tip = format!(
                "Node list from {}. Kubernetes access: {}.",
                roster.label(),
                talos
                    .kubeconfig_source
                    .as_deref()
                    .unwrap_or("not available")
            )
            .into();
            result.warnings = [&talos.discovery_warning, &talos.kubeconfig_warning]
                .into_iter()
                .flatten()
                .map(|warning| warning.clone().into())
                .collect();
        } else {
            result.roster = "Roster: Kubernetes API".into();
            result.roster_tip = "Node list from the Kubernetes API.".into();
        }
        let ready = rows
            .iter()
            .filter(|row| row.kubernetes.as_ref().is_some_and(|node| node.is_ready()))
            .count();
        let planes = rows
            .iter()
            .filter(|row| row.role == super::Role::ControlPlane)
            .count();
        let (figure, detail, tone) = part(kube.map(|kube| &kube.nodes), "nodes", |_| {
            (
                format!("{ready} / {} Ready", rows.len()),
                format!("{planes} control planes · {} workers", rows.len() - planes),
                if ready == rows.len() {
                    Tone::Good
                } else {
                    Tone::Warn
                },
            )
        });
        result.cards.push(Card {
            id: "tile-nodes",
            label: "Nodes",
            figure: figure.into(),
            detail: detail.into(),
            tone,
            segments: rows.iter().map(|row| row.tone).collect(),
            meter: None,
            target: CardTarget::Page(Page::Nodes),
        });
        if !kube_only {
            let (figure, detail, tone) = summary
                .as_ref()
                .and_then(|summary| summary.etcd.as_ref())
                .map(|etcd| {
                    let quorum = freshkube_core::indicators::quorum(etcd.healthy, etcd.total);
                    let tolerance = quorum.remaining_tolerance;
                    (
                        if quorum.state.has_quorum() {
                            "Quorum".to_owned()
                        } else {
                            "Quorum unconfirmed".to_owned()
                        },
                        format!(
                            "{} of {} answered · tolerates {tolerance} additional member {}",
                            etcd.healthy,
                            etcd.total,
                            if tolerance == 1 {
                                "failure"
                            } else {
                                "failures"
                            }
                        ),
                        if quorum.state.has_quorum() && tolerance > 0 {
                            Tone::Good
                        } else {
                            Tone::Warn
                        },
                    )
                })
                .unwrap_or(("—".into(), "No etcd status reported".into(), Tone::Unknown));
            let api = kube
                .map(|kube| match &kube.version {
                    Part::Loaded(_) => "Kubernetes version read".to_owned(),
                    Part::Refused(error) | Part::Failed(error) => {
                        format!("Kubernetes API: {error}")
                    }
                })
                .unwrap_or("Waiting for Kubernetes API".into());
            result.cards.push(Card {
                id: "tile-etcd",
                label: "Control plane",
                figure: figure.into(),
                detail: format!("{detail} · {api}").into(),
                tone,
                segments: vec![],
                meter: None,
                target: CardTarget::Page(Page::Etcd),
            });
        }
        let (figure, detail, tone) = kube
            .and_then(|kube| kube.workloads.snapshot())
            .map(|snapshot| {
                let workloads: Vec<_> = snapshot
                    .namespaces
                    .iter()
                    .flat_map(|ns| &ns.workloads)
                    .collect();
                let unhealthy = workloads
                    .iter()
                    .filter(|workload| {
                        matches!(
                            workload.health,
                            HealthState::Failing | HealthState::Degraded
                        )
                    })
                    .count();
                let healthy = workloads
                    .iter()
                    .filter(|workload| workload.health == HealthState::Healthy)
                    .count();
                let progressing = workloads.len() - healthy - unhealthy;
                let first = workloads
                    .iter()
                    .find(|workload| workload.health != HealthState::Healthy)
                    .map(|workload| {
                        format!(
                            "{}/{} · {} of {} available · ",
                            workload.namespace, workload.name, workload.ready, workload.desired
                        )
                    })
                    .unwrap_or_default();
                (
                    format!("{unhealthy} unhealthy"),
                    format!("{first}{healthy} healthy · {progressing} progressing"),
                    if unhealthy > 0 {
                        Tone::Warn
                    } else {
                        Tone::Good
                    },
                )
            })
            .unwrap_or((
                "Unavailable".into(),
                kube.map(|kube| {
                    kube.workloads
                        .unavailable()
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" · ")
                })
                .unwrap_or("Waiting for workloads".into()),
                Tone::Unknown,
            ));
        let (detail, tone) =
            if let Some(kube) = kube.filter(|kube| !kube.workloads.unavailable().is_empty()) {
                (
                    format!(
                        "{detail} · {}",
                        kube.workloads
                            .unavailable()
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(" · ")
                    ),
                    Tone::Unknown,
                )
            } else {
                (detail, tone)
            };
        result.cards.push(Card {
            id: "tile-workloads",
            label: "Workloads",
            figure: figure.into(),
            detail: detail.into(),
            tone,
            segments: vec![],
            meter: None,
            target: CardTarget::Page(Page::Health),
        });
        let (figure, detail, tone) = part(kube.map(|kube| &kube.pods), "pods", |pods| {
            let mut parts = Vec::new();
            let crash = pods
                .issues_by_status
                .get("CrashLoopBackOff")
                .copied()
                .unwrap_or(0);
            if crash > 0 {
                parts.push(format!("{crash} CrashLoopBackOff"));
            }
            let pending = pods.phases.get("Pending").copied().unwrap_or(0);
            if pending > 0 {
                parts.push(format!("{pending} Pending"));
            }
            if pods.on_not_ready > 0 && kube.is_some_and(|kube| kube.nodes.is_current()) {
                parts.push(format!("{} on a NotReady node", pods.on_not_ready));
            }
            if parts.is_empty() {
                parts.push("No pod issues reported".into());
            }
            (
                pods.total.to_string(),
                parts.join(" · "),
                if pods.issues.is_empty() {
                    Tone::Good
                } else {
                    Tone::Warn
                },
            )
        });
        let filter = kube
            .and_then(|kube| kube.pods.loaded())
            .and_then(|pods| pods.issues.first())
            .map(|pod| pod.issue.label().to_owned())
            .unwrap_or_default();
        result.cards.push(Card {
            id: "tile-pods",
            label: "Pods",
            figure: figure.into(),
            detail: detail.into(),
            tone,
            segments: vec![],
            meter: None,
            target: CardTarget::Kind("pods", filter),
        });
        if !kube_only {
            let counts = summary
                .as_ref()
                .map(|summary| summary.services)
                .unwrap_or_default();
            result.cards.push(Card {
                id: "tile-services",
                label: "System services",
                figure: format!("{} unhealthy", counts.unhealthy).into(),
                detail: format!(
                    "{}{} healthy · {} not reported",
                    summary
                        .as_ref()
                        .and_then(|summary| summary.first_unhealthy.as_ref())
                        .map(|(node, service)| format!("{node}/{service} · "))
                        .unwrap_or_default(),
                    counts.healthy,
                    counts.unknown
                )
                .into(),
                tone: if counts.unhealthy > 0 {
                    Tone::Warn
                } else {
                    Tone::Good
                },
                segments: vec![],
                meter: None,
                target: CardTarget::Services,
            });
            let peak = summary
                .as_ref()
                .and_then(|summary| summary.peak_memory.as_ref());
            let target = peak
                .and_then(|(name, _)| rows.iter().find(|row| row.key.talos.as_ref() == Some(name)))
                .map(|row| {
                    CardTarget::Destination(Destination::Node(row.key.clone(), NodeTab::Processes))
                })
                .unwrap_or(CardTarget::Page(Page::Nodes));
            result.cards.push(Card {
                id: "tile-memory",
                label: "Peak memory",
                figure: peak
                    .map(|(_, percent)| format!("{percent:.0} %"))
                    .unwrap_or("—".into())
                    .into(),
                detail: peak
                    .map(|(name, percent)| {
                        format!(
                            "{name}{}",
                            crate::ui::memory_tone(super::memory_level(*percent))
                                .map(|(_, text)| format!(" · {text}"))
                                .unwrap_or_default()
                        )
                    })
                    .unwrap_or("No memory data reported".into())
                    .into(),
                tone: if peak.is_some_and(|(_, percent)| *percent >= 90.) {
                    Tone::Crit
                } else {
                    Tone::Good
                },
                segments: vec![],
                meter: peak.map(|(_, percent)| (*percent, super::memory_level(*percent))),
                target,
            });
        }
        let (figure, detail, tone) = part(kube.map(|kube| &kube.events), "events", |events| {
            (
                events.total.to_string(),
                format!(
                    "Warnings in the last hour · {}",
                    events
                        .reasons
                        .iter()
                        .take(3)
                        .map(|(reason, count)| format!("{reason} {count}"))
                        .collect::<Vec<_>>()
                        .join(" · ")
                ),
                if events.total > 0 {
                    Tone::Warn
                } else {
                    Tone::Good
                },
            )
        });
        result.cards.push(Card {
            id: "tile-events",
            label: "Events",
            figure: figure.into(),
            detail: detail.into(),
            tone,
            segments: vec![],
            meter: None,
            target: CardTarget::Kind("events", "Warning".into()),
        });
        if !kube_only {
            let (figure, detail, tone) = part(kube.map(|kube| &kube.claims), "claims", |claims| {
                (
                    format!("{} Pending", claims.pending_count),
                    format!(
                        "{}{} bound · {} PVs available",
                        claims
                            .pending
                            .first()
                            .map(|claim| format!(
                                "{}/{} · {} · ",
                                claim.namespace, claim.name, claim.reason
                            ))
                            .unwrap_or_default(),
                        claims.bound,
                        kube.and_then(|kube| kube.available_volumes.loaded())
                            .map(ToString::to_string)
                            .unwrap_or("unavailable".into())
                    ),
                    if claims.pending_count > 0 {
                        Tone::Warn
                    } else {
                        Tone::Good
                    },
                )
            });
            result.cards.push(Card {
                id: "tile-storage",
                label: "Storage",
                figure: figure.into(),
                detail: detail.into(),
                tone,
                segments: vec![],
                meter: None,
                target: CardTarget::Kind("persistentvolumeclaims", "Pending".into()),
            });
        }
        if let Some(kube) = kube {
            result.warnings.extend(
                kube.observations
                    .iter()
                    .filter_map(|(source, observation)| {
                        observation.message().map(|message| {
                            let when = observation
                                .last_success()
                                .map(|at| format!(" · last observed {} UTC", at.format("%H:%M:%S")))
                                .unwrap_or_default();
                            format!("{}: {message}{when}", source.resource()).into()
                        })
                    }),
            );
        }
        result
    }
}
