//! Cached pod relationships and their requests. Rendering reads only these rows.
use super::*;
use crate::desktop::nodes::{NodeRow, NodeTab};
use crate::resources::{ResourceLink, model::ObjectRef};
use freshkube_core::resources::{ContainerState, Owner, PodLinks};
use std::sync::Arc;

mod cause;
mod view;

#[derive(Clone)]
pub(in crate::resources::pane) struct OwnerLink {
    pub(super) id: SharedString,
    pub(super) label: SharedString,
    pub(super) intent: ResourceLink,
}
impl OwnerLink {
    pub(in crate::resources::pane) fn new(owner: &Owner, namespace: &str) -> Self {
        Self {
            id: format!("owner-{}-{}-{}", owner.kind, namespace, owner.name).into(),
            label: format!(
                "{} {}{}",
                owner.kind,
                owner.name,
                if owner.controller {
                    " (controller)"
                } else {
                    ""
                }
            )
            .into(),
            intent: ResourceLink::Owner {
                api_version: owner.api_version.clone(),
                kind: owner.kind.clone(),
                object: ObjectRef {
                    namespace: namespace.into(),
                    name: owner.name.clone(),
                    uid: owner.uid.clone(),
                },
            },
        }
    }
}
struct NodeLink {
    name: String,
    ready: SharedString,
    tone: crate::ui::Tone,
    problems: Vec<SharedString>,
    services: bool,
    /// Talos's kubelet service on the node, when Talos reports the node.
    kubelet: Option<(crate::ui::Tone, SharedString)>,
}
struct ServiceLink {
    id: SharedString,
    label: SharedString,
    object: ObjectRef,
}
struct ContainerLine {
    id: SharedString,
    name: String,
    image: SharedString,
    state: SharedString,
    restarts: SharedString,
    previous: bool,
    started: bool,
}
struct RecentEvent {
    id: SharedString,
    reason: SharedString,
    message: SharedString,
}
#[derive(Default)]
pub(super) struct CrossLinks {
    pub(in crate::resources::pane) cause: Option<cause::CauseCard>,
    timeline: Option<cause::Timeline>,
    /// ServiceAccount, IP and QoS class.
    pub(in crate::resources::pane) facts: Vec<(SharedString, SharedString)>,
    /// The container Logs opens first: the one at fault, else the default.
    logs: Option<String>,
    node: Option<NodeLink>,
    owners: Vec<OwnerLink>,
    services: Vec<ServiceLink>,
    services_note: SharedString,
    containers: Vec<ContainerLine>,
    containers_note: SharedString,
    errors: Vec<SharedString>,
    recent: Vec<RecentEvent>,
    pod: bool,
}
impl DetailPane {
    pub(crate) fn set_node_rows(&mut self, rows: Arc<Vec<NodeRow>>, cx: &mut Context<Self>) {
        self.node_rows = rows;
        self.rebuild_links();
        if self.active && self.cross_links.pod {
            cx.notify();
        }
    }
    pub(super) fn rebuild_links(&mut self) {
        let Some(view) = self.detail.as_ref().and_then(|detail| detail.view.as_ref()) else {
            self.cross_links = Default::default();
            return;
        };
        let document = &view.document;
        let Some(pod) = &document.overview.pod else {
            self.cross_links = Default::default();
            return;
        };
        let node = pod.node.as_ref().map(|name| {
            let row = self
                .node_rows
                .iter()
                .find(|row| row.key.kubernetes.as_ref() == Some(name));
            NodeLink {
                name: name.clone(),
                ready: row
                    .map(|row| row.ready)
                    .unwrap_or("Node unavailable")
                    .into(),
                tone: row.map(|row| row.tone).unwrap_or_default(),
                problems: row.map(|row| row.problems.clone()).unwrap_or_default(),
                services: row.is_some_and(|row| row.service_problem || row.talos.is_some()),
                kubelet: row.and_then(|row| row.talos.as_ref()).map(|talos| {
                    match talos
                        .services
                        .iter()
                        .find(|service| service.id == "kubelet")
                    {
                        Some(service) => {
                            let health = crate::presentation::service_health(service);
                            (
                                crate::ui::health_tone(health),
                                format!(
                                    "kubelet {} · {}",
                                    service.state.to_lowercase(),
                                    crate::presentation::health_text(&health).to_lowercase()
                                )
                                .into(),
                            )
                        }
                        None => (crate::ui::Tone::Unknown, "kubelet not reported".into()),
                    }
                }),
            }
        });
        let namespace = document.namespace.as_deref().unwrap_or_default();
        let mut owners = document
            .overview
            .owners
            .iter()
            .map(|owner| OwnerLink::new(owner, namespace))
            .collect::<Vec<_>>();
        let mut services = Vec::new();
        let mut errors = Vec::new();
        let services_note = if let Some(links) = &self.pod_links {
            if let Some(chain) = &links.controller
                && document
                    .overview
                    .owners
                    .iter()
                    .any(|owner| owner.uid == chain.via_uid)
            {
                owners.extend(
                    chain
                        .owners
                        .iter()
                        .map(|owner| OwnerLink::new(owner, namespace)),
                );
            }
            if let Some(error) = &links.controller_error {
                errors.push(format!("Can't read controller: {error}").into());
            }
            match &links.services {
                Err(error) => {
                    errors.push(format!("Can't list services: {error}").into());
                    "Services unavailable".into()
                }
                Ok(_) => {
                    let matches = links
                        .selected_by(&document.overview.labels)
                        .collect::<Vec<_>>();
                    services = matches
                        .iter()
                        .take(200)
                        .map(|service| ServiceLink {
                            id: format!("selected-service-{}-{}", service.namespace, service.name)
                                .into(),
                            label: service.name.clone().into(),
                            object: ObjectRef {
                                namespace: service.namespace.clone(),
                                name: service.name.clone(),
                                uid: service.uid.clone(),
                            },
                        })
                        .collect();
                    if matches.len() > 200 {
                        format!("First 200 of {} matching Services", matches.len()).into()
                    } else if matches.is_empty() {
                        "No matching Services".into()
                    } else {
                        SharedString::default()
                    }
                }
            }
        } else {
            "Looking up Services and controller…".into()
        };
        let containers_note = if pod.containers.len() > 200 {
            format!("First 200 of {} containers", pod.containers.len()).into()
        } else {
            SharedString::default()
        };
        let containers = pod
            .containers
            .iter()
            .take(200)
            .map(|container| ContainerLine {
                id: format!("pod-container-{}", container.name).into(),
                name: container.name.clone(),
                image: container.image.clone().into(),
                state: match &container.state {
                    ContainerState::Waiting(reason) if reason.is_empty() => "Waiting".into(),
                    ContainerState::Waiting(reason) => reason.clone().into(),
                    ContainerState::Running(_) => "Running".into(),
                    ContainerState::Terminated(end) => {
                        format!("{} · exit {}", end.reason, end.exit_code).into()
                    }
                },
                restarts: format!("{} restarts", container.restarts).into(),
                previous: container.has_previous(),
                started: container.started(),
            })
            .collect();
        let recent = self
            .detail
            .as_ref()
            .unwrap()
            .events
            .shown()
            .iter()
            .filter(|event| event.is_warning())
            .take(3)
            .map(|event| RecentEvent {
                id: format!("recent-event-{}", event.uid).into(),
                reason: event.reason.clone().into(),
                message: event.message.clone().into(),
            })
            .collect();
        let now = chrono::Utc::now();
        let status = document.overview.pod_status.as_ref();
        let diagnosis = status.and_then(|status| status.diagnose());
        let cause = diagnosis
            .as_ref()
            .map(|diagnosis| cause::CauseCard::new(diagnosis, Some(pod), now));
        let timeline =
            status.and_then(|status| cause::Timeline::new(status, diagnosis.as_ref(), now));
        let facts = status
            .map(|status| {
                [
                    ("ServiceAccount", &status.service_account),
                    ("Pod IP", &status.ip),
                    ("QoS class", &status.qos),
                ]
                .into_iter()
                .filter(|(_, value)| !value.is_empty())
                .map(|(label, value)| (label.into(), value.clone().into()))
                .collect()
            })
            .unwrap_or_default();
        let logs = diagnosis
            .and_then(|diagnosis| diagnosis.container)
            .or_else(|| pod.default.clone());
        self.cross_links = CrossLinks {
            cause,
            timeline,
            facts,
            logs,
            node,
            owners,
            services,
            services_note,
            containers,
            containers_note,
            errors,
            recent,
            pod: true,
        };
    }
    pub(super) fn start_links(&mut self, cx: &mut Context<Self>) {
        if !self.active || self.pod_links.is_some() || self.links_job.is_some() {
            return;
        }
        let (Some(view), Some(access)) = (
            self.detail.as_ref().and_then(|detail| detail.view.clone()),
            self.access.clone(),
        ) else {
            return;
        };
        if view.document.overview.pod.is_none() {
            return;
        }
        let identity = self.detail.as_ref().unwrap().target.identity.clone();
        self.links_seq = self.links_seq.wrapping_add(1);
        let sequence = self.links_seq;
        if matches!(access, KubeAccess::Example) {
            let links = example_links(&identity, &view.document);
            self.finish_links(&identity, sequence, links, cx);
            return;
        }
        let document = view.document.clone();
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            Duration::from_secs(15),
            "Reading pod relationships timed out".into(),
            async move {
                let client = access.client().await?;
                Ok(freshkube_core::resources::collect_pod_links(&client, &document).await)
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("The relationship worker stopped".into()));
            _ = this.update(cx, |pane, cx| {
                let links = result.unwrap_or_else(|error| {
                    PodLinks::from_sources(
                        &view.document,
                        Err(Failure::new(FailureKind::Other, error)),
                        None,
                    )
                });
                pane.finish_links(&identity, sequence, links, cx);
            });
        });
        self.links_job = Some((job, task));
    }
    fn finish_links(
        &mut self,
        identity: &ResourceIdentity,
        sequence: u64,
        links: PodLinks,
        cx: &mut Context<Self>,
    ) {
        if self.links_seq != sequence || !self.active || self.target_identity() != Some(identity) {
            return;
        }
        self.links_job = None;
        self.pod_links = Some(links);
        self.rebuild_links();
        cx.notify();
    }
    fn retry_links(&mut self, cx: &mut Context<Self>) {
        self.pod_links = None;
        self.start_links(cx);
        self.rebuild_links();
        cx.notify();
    }
    fn container_logs(&mut self, name: String, previous: bool, cx: &mut Context<Self>) {
        self.logs
            .update(cx, |logs, cx| logs.open_container(name, previous, cx));
        self.set_tab(Tab::Logs, cx);
    }
}
fn example_links(
    identity: &ResourceIdentity,
    document: &freshkube_core::resources::ObjectDocument,
) -> PodLinks {
    let context = identity
        .connection
        .strip_prefix("example:")
        .unwrap_or("prod-fra");
    let services = example::read(context, "services", Some(&identity.namespace), live::now())
        .map(|(_, rows)| {
            rows.into_iter()
                .filter_map(|row| example::document(&row.identity, live::now()))
                .filter_map(|document| serde_yaml::from_str(&document.yaml).ok())
                .collect()
        })
        .unwrap_or_default();
    let replica_set = document
        .overview
        .owners
        .iter()
        .find(|owner| owner.kind == "ReplicaSet" && owner.controller)
        .map(|owner| {
            let identity = ResourceIdentity {
                resource: "replicasets.apps".into(),
                name: owner.name.clone(),
                uid: owner.uid.clone(),
                ..identity.clone()
            };
            example::document(&identity, live::now()).ok_or_else(|| {
                Failure::new(FailureKind::NotFound, "Example data has no such ReplicaSet")
            })
        });
    PodLinks::from_sources(document, Ok(services), replica_set)
}

#[cfg(test)]
mod tests;
