//! Finding an in-cluster Prometheus: list Services, rank them by how
//! Prometheus is usually installed, and confirm candidates in that order
//! through `/api/v1/status/buildinfo`. Only GET and list are used.
use std::time::Duration;

use k8s_openapi::api::core::v1::Service;
use kube::api::{Api, ListParams};

use super::{BuildInfo, ErrorKind, Prometheus, PrometheusService, QueryError};
use crate::resources::{Failure, FailureKind};

/// What discovery looks for, for the page's "no Prometheus" state.
pub const LOOKED_FOR: &str = "a Service named prometheus-operated, labelled \
app.kubernetes.io/name=prometheus or app=prometheus, or named like Prometheus with a \
web or 9090 port";

/// Namespaces read one by one when listing Services cluster-wide is refused.
const FALLBACK_NAMESPACES: [&str; 6] = [
    "monitoring",
    "prometheus",
    "observability",
    "kube-prometheus-stack",
    "default",
    "kube-system",
];

/// Candidates confirmed at most, so a cluster full of look-alikes can't
/// keep discovery going.
const MAX_CONFIRMED: usize = 6;

/// How long one confirmation may take during discovery.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(8);

/// How likely a Service is to be the Prometheus API, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rank {
    /// The operator's governing Service.
    Operated,
    /// Labelled as Prometheus itself.
    Labelled,
    /// Named like Prometheus, not like one of its companions.
    Named,
    /// Only a port that Prometheus usually listens on.
    Port,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub service: PrometheusService,
    pub rank: Rank,
}

/// A candidate that didn't answer as Prometheus, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tried {
    pub service: PrometheusService,
    pub error: QueryError,
}

/// The outcome of discovery that isn't a hard failure.
#[derive(Clone)]
pub enum Discovery {
    Found {
        prometheus: Prometheus,
        build: BuildInfo,
        /// Better-ranked candidates that failed first.
        tried: Vec<Tried>,
    },
    /// Nothing answered as Prometheus. `candidates` holds every ranked
    /// Service, for the page's picker.
    Missing {
        candidates: Vec<Candidate>,
        tried: Vec<Tried>,
    },
}

/// Ranks one Service, or `None` when nothing about it looks like
/// Prometheus's API.
pub fn rank(service: &Service) -> Option<Candidate> {
    let metadata = &service.metadata;
    let name = metadata.name.as_deref()?;
    let namespace = metadata.namespace.as_deref()?;
    let ports = service.spec.as_ref()?.ports.as_ref()?;
    let label = |key: &str| {
        metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get(key))
            .map(String::as_str)
    };
    let rank = if name == "prometheus-operated" {
        Rank::Operated
    } else if label("app.kubernetes.io/name") == Some("prometheus")
        || label("app") == Some("prometheus")
    {
        Rank::Labelled
    } else if named_like_prometheus(name) {
        Rank::Named
    } else {
        Rank::Port
    };
    let named = |wanted: &str| {
        ports
            .iter()
            .find(|port| port.name.as_deref() == Some(wanted))
    };
    let port = named("web")
        .or_else(|| named("http-web"))
        .or_else(|| ports.iter().find(|port| port.port == 9090))
        .or_else(|| {
            // A Service that is clearly Prometheus may name its port plainly.
            (rank <= Rank::Named)
                .then(|| named("http").or_else(|| named("https")))
                .flatten()
        })?;
    let number = u16::try_from(port.port).ok()?;
    let https = port.port == 443
        || port
            .name
            .as_deref()
            .is_some_and(|name| name.contains("https"))
        || port.app_protocol.as_deref() == Some("https");
    Some(Candidate {
        service: PrometheusService {
            https,
            ..PrometheusService::new(namespace, name, number)
        },
        rank,
    })
}

fn named_like_prometheus(name: &str) -> bool {
    const COMPANIONS: [&str; 9] = [
        "alertmanager",
        "operator",
        "node-exporter",
        "kube-state-metrics",
        "pushgateway",
        "blackbox",
        "adapter",
        "grafana",
        "thanos-sidecar",
    ];
    name.contains("prometheus") && !COMPANIONS.iter().any(|other| name.contains(other))
}

/// Every Service that might be Prometheus, best first. Lists cluster-wide,
/// or the usual monitoring namespaces when that is refused.
pub async fn list_candidates(client: &kube::Client) -> Result<Vec<Candidate>, QueryError> {
    let services = match Api::<Service>::all(client.clone())
        .list(&ListParams::default())
        .await
    {
        Ok(list) => list.items,
        Err(error) => {
            let failure = Failure::from_kube(error);
            if failure.kind != FailureKind::Forbidden {
                return Err(listing_error(failure));
            }
            let mut services = Vec::new();
            let mut allowed = false;
            for namespace in FALLBACK_NAMESPACES {
                match Api::<Service>::namespaced(client.clone(), namespace)
                    .list(&ListParams::default())
                    .await
                {
                    Ok(list) => {
                        allowed = true;
                        services.extend(list.items);
                    }
                    Err(error) => match Failure::from_kube(error) {
                        Failure {
                            kind: FailureKind::Forbidden | FailureKind::NotFound,
                            ..
                        } => {}
                        failure => return Err(listing_error(failure)),
                    },
                }
            }
            if !allowed {
                return Err(QueryError::new(
                    ErrorKind::Refused,
                    "Not allowed to list Services to find Prometheus",
                ));
            }
            services
        }
    };
    let mut candidates: Vec<Candidate> = services.iter().filter_map(rank).collect();
    candidates.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
            .then_with(|| a.service.namespace.cmp(&b.service.namespace))
            .then_with(|| a.service.name.cmp(&b.service.name))
    });
    Ok(candidates)
}

fn listing_error(failure: Failure) -> QueryError {
    let kind = match failure.kind {
        FailureKind::Unauthorized | FailureKind::Forbidden => ErrorKind::Refused,
        FailureKind::NotFound => ErrorKind::NotFound,
        FailureKind::Timeout => ErrorKind::TimedOut,
        FailureKind::Config | FailureKind::Unreachable | FailureKind::Other => {
            ErrorKind::Unavailable
        }
    };
    QueryError::new(kind, format!("Couldn't list Services: {}", failure.message))
}

/// Confirms one Service answers as Prometheus, and reads its scrape
/// interval. A failed interval read leaves the default.
pub async fn confirm(
    client: &kube::Client,
    service: PrometheusService,
) -> Result<(Prometheus, BuildInfo), QueryError> {
    confirm_within(client, service, super::REQUEST_TIMEOUT).await
}

async fn confirm_within(
    client: &kube::Client,
    service: PrometheusService,
    timeout: Duration,
) -> Result<(Prometheus, BuildInfo), QueryError> {
    let prometheus = Prometheus::new(client.clone(), service).with_timeout(timeout);
    let build = prometheus.build_info().await?;
    let interval = prometheus.read_scrape_interval().await.ok().flatten();
    let prometheus = match interval {
        Some(seconds) => prometheus.with_scrape_interval(seconds),
        None => prometheus,
    }
    .with_timeout(super::REQUEST_TIMEOUT);
    Ok((prometheus, build))
}

/// Finds Prometheus: the remembered Service first, then the ranked
/// candidates in order. Errors only when the cluster refuses outright: the
/// Services can't be listed, or every candidate refused the proxy.
pub async fn discover(
    client: &kube::Client,
    remembered: Option<PrometheusService>,
) -> Result<Discovery, QueryError> {
    let mut tried = Vec::new();
    if let Some(service) = remembered {
        match confirm(client, service.clone()).await {
            Ok((prometheus, build)) => {
                return Ok(Discovery::Found {
                    prometheus,
                    build,
                    tried,
                });
            }
            // RBAC applies to every Service alike; don't look further.
            Err(error) if error.kind == ErrorKind::Refused => return Err(error),
            Err(error) => tried.push(Tried { service, error }),
        }
    }
    let candidates = list_candidates(client).await?;
    for candidate in candidates.iter().take(MAX_CONFIRMED) {
        if tried
            .iter()
            .any(|tried: &Tried| tried.service == candidate.service)
        {
            continue;
        }
        match confirm_within(client, candidate.service.clone(), CONFIRM_TIMEOUT).await {
            Ok((prometheus, build)) => {
                return Ok(Discovery::Found {
                    prometheus,
                    build,
                    tried,
                });
            }
            Err(error) if error.kind == ErrorKind::Refused => return Err(error),
            Err(error) => tried.push(Tried {
                service: candidate.service.clone(),
                error,
            }),
        }
    }
    Ok(Discovery::Missing { candidates, tried })
}
