//! Finding the cluster's Alertmanager and vmalert: list Services, rank them
//! by how they are usually installed, and confirm the best of each kind with
//! a real read. Only GET and list are used.
use k8s_openapi::api::core::v1::{Service, ServicePort};
use kube::api::{Api, ListParams};

use super::super::discovery::list_services;
use super::super::{ErrorKind, PrometheusService, QueryError};
use super::model::Kind;
use super::read::{Client, Sources};
use crate::monitoring::Prometheus;
use crate::resources::{Failure, FailureKind};

/// What discovery looks for, for the page's "not found" state.
pub const LOOKED_FOR: &str = "a Service named alertmanager-operated, labelled \
app.kubernetes.io/name=alertmanager, or named like Alertmanager with a web or 9093 port, \
and a vmalert Service on port 8880";

/// What discovery looks for in an Alertmanager, for the Silences page.
pub const LOOKED_FOR_ALERTMANAGER: &str = "a Service named alertmanager-operated, labelled \
app.kubernetes.io/name=alertmanager, or named like Alertmanager with a web or 9093 port";

/// What the Rules page looks for.
pub const LOOKED_FOR_RULES: &str = "a vmalert Service on port 8880 and PrometheusRule objects \
in the cluster";

/// Candidates confirmed at most per kind.
const MAX_CONFIRMED: usize = 3;

/// How likely a Service is to be what it looks like, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rank {
    /// The operator's governing Service.
    Operated,
    Labelled,
    Named,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub kind: Kind,
    pub service: PrometheusService,
    pub rank: Rank,
}

/// The Services a session found before and read from, tried first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Remembered {
    pub alertmanager: Option<PrometheusService>,
    pub vmalert: Option<PrometheusService>,
}

fn label<'a>(service: &'a Service, key: &str) -> Option<&'a str> {
    service
        .metadata
        .labels
        .as_ref()
        .and_then(|labels| labels.get(key))
        .map(String::as_str)
}

fn port<'a>(ports: &'a [ServicePort], names: &[&str], numbers: &[i32]) -> Option<&'a ServicePort> {
    names
        .iter()
        .find_map(|wanted| {
            ports
                .iter()
                .find(|port| port.name.as_deref() == Some(*wanted))
        })
        .or_else(|| ports.iter().find(|port| numbers.contains(&port.port)))
}

fn speaks_tls(port: &ServicePort) -> bool {
    port.port == 443
        || port
            .name
            .as_deref()
            .is_some_and(|name| name.contains("https"))
        || port.app_protocol.as_deref() == Some("https")
}

/// Ranks one Service as an Alertmanager or a vmalert, or `None` when
/// nothing about it looks like either. The mesh port Alertmanager's replicas
/// gossip on is never chosen.
pub fn rank(service: &Service) -> Option<Candidate> {
    let name = service.metadata.name.as_deref()?;
    let namespace = service.metadata.namespace.as_deref()?;
    let ports = service.spec.as_ref()?.ports.as_ref()?;
    let app = label(service, "app.kubernetes.io/name").or_else(|| label(service, "app"));
    let lower = name.to_ascii_lowercase();
    let (kind, rank, chosen) = if lower.contains("alertmanager") {
        let rank = if name == "alertmanager-operated" {
            Rank::Operated
        } else if app == Some("alertmanager") || app == Some("vmalertmanager") {
            Rank::Labelled
        } else if ["operator", "webhook", "exporter", "discord", "bot"]
            .iter()
            .any(|other| lower.contains(other))
        {
            return None;
        } else {
            Rank::Named
        };
        let chosen = port(ports, &["web", "http-web", "http"], &[9093])?;
        (Kind::Alertmanager, rank, chosen)
    } else if lower.starts_with("vmalert")
        || app == Some("vmalert")
        || lower.contains("victoria-metrics-alert")
    {
        let rank = if app == Some("vmalert") {
            Rank::Labelled
        } else {
            Rank::Named
        };
        let chosen = port(ports, &["http", "web"], &[8880])?;
        (Kind::Vmalert, rank, chosen)
    } else {
        return None;
    };
    let number = u16::try_from(chosen.port).ok()?;
    Some(Candidate {
        kind,
        service: PrometheusService {
            https: speaks_tls(chosen),
            ..PrometheusService::new(namespace, name, number)
        },
        rank,
    })
}

/// Every candidate in `services`, best first of each kind.
pub fn rank_all(services: &[Service]) -> Vec<Candidate> {
    let mut candidates: Vec<Candidate> = services.iter().filter_map(rank).collect();
    candidates.sort_by(|a, b| {
        (a.kind, a.rank, &a.service.namespace, &a.service.name).cmp(&(
            b.kind,
            b.rank,
            &b.service.namespace,
            &b.service.name,
        ))
    });
    candidates
}

/// The Services to look at: the one namespace beside a central store, or
/// every Service the identity may list.
async fn services(client: &kube::Client, beside: Option<&str>) -> Result<Vec<Service>, QueryError> {
    let Some(namespace) = beside else {
        return list_services(client, "Alertmanager or vmalert").await;
    };
    Api::<Service>::namespaced(client.clone(), namespace)
        .list(&ListParams::default())
        .await
        .map(|list| list.items)
        .map_err(|error| {
            let failure = Failure::from_kube(error);
            let kind = match failure.kind {
                FailureKind::Forbidden => ErrorKind::Refused,
                FailureKind::Unauthorized => ErrorKind::Unauthorized,
                FailureKind::NotFound => ErrorKind::NotFound,
                FailureKind::Timeout => ErrorKind::TimedOut,
                FailureKind::Config | FailureKind::Unreachable | FailureKind::Other => {
                    ErrorKind::Unavailable
                }
            };
            QueryError::new(
                kind,
                format!("Couldn't list Services in {namespace}: {}", failure.message),
            )
        })
}

/// Confirms one Service answers as `kind`, with a read of its own.
async fn confirm(
    client: &kube::Client,
    kind: Kind,
    service: PrometheusService,
) -> Result<Client, QueryError> {
    let client = Client::new(kind, Prometheus::new(client.clone(), service));
    client.confirm().await
}

/// What discovery found: the sources, and the other Services of the same
/// kind that also answered.
#[derive(Clone)]
pub struct Found {
    pub sources: Sources,
    /// Confirmed Services beyond the one read, for the page to name: several
    /// Alertmanagers, not replicas of one.
    pub others: Vec<String>,
    /// Candidates that didn't answer, with why, for the not-found state.
    pub tried: Vec<(String, QueryError)>,
}

/// Finds the Alertmanager and the vmalert: the remembered Services first,
/// then the ranked candidates. Errors only when the cluster refuses
/// outright: the Services can't be listed or every candidate refused the
/// proxy. Finding neither is `Ok` with empty sources.
pub async fn find(
    client: &kube::Client,
    remembered: &Remembered,
    beside: Option<&str>,
) -> Result<Found, QueryError> {
    let mut found = Found {
        sources: Sources::default(),
        others: Vec::new(),
        tried: Vec::new(),
    };
    for (kind, service) in [
        (Kind::Alertmanager, remembered.alertmanager.clone()),
        (Kind::Vmalert, remembered.vmalert.clone()),
    ] {
        let Some(service) = service else { continue };
        match confirm(client, kind, service.clone()).await {
            Ok(confirmed) => found.sources.set(confirmed),
            Err(error) if error.is_refused() => return Err(error),
            Err(error) => found.tried.push((service.label(), error)),
        }
    }
    if found.sources.alertmanager.is_some() && found.sources.vmalert.is_some() {
        return Ok(found);
    }
    let candidates = rank_all(&services(client, beside).await?);
    let mut confirmed = [0usize; 2];
    for candidate in candidates {
        let slot = usize::from(candidate.kind == Kind::Vmalert);
        let have = found.sources.has(candidate.kind);
        if confirmed[slot] >= MAX_CONFIRMED
            || found
                .tried
                .iter()
                .any(|(label, _)| *label == candidate.service.label())
            || found.sources.service(candidate.kind) == Some(&candidate.service)
        {
            continue;
        }
        confirmed[slot] += 1;
        match confirm(client, candidate.kind, candidate.service.clone()).await {
            Ok(client) if have => found.others.push(client.label().to_owned()),
            Ok(client) => found.sources.set(client),
            Err(error) if error.is_refused() => return Err(error),
            Err(error) => found.tried.push((candidate.service.label(), error)),
        }
    }
    Ok(found)
}
