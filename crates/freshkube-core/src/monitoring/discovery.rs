//! Finding an in-cluster Prometheus API: list Services, rank them by how
//! Prometheus, VictoriaMetrics, Thanos and Mimir are usually installed, and
//! confirm candidates in that order with a query for `1`. Only GET and list
//! are used.
use std::time::Duration;

use k8s_openapi::api::core::v1::{Service, ServicePort};
use kube::api::{Api, ListParams};

use super::{BuildInfo, ErrorKind, Prometheus, PrometheusService, QueryError};
use crate::resources::{Failure, FailureKind};

/// What discovery looks for, for the page's "no Prometheus" state.
pub const LOOKED_FOR: &str = "a Service named prometheus-operated, labelled \
app.kubernetes.io/name=prometheus or app=prometheus, a VictoriaMetrics vmsingle or \
vmselect, a Thanos query or a Mimir query frontend, or one named like Prometheus with a \
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
    /// Another server that answers the Prometheus API, by its usual name
    /// or label: VictoriaMetrics, Thanos or Mimir.
    Compatible,
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
    let named = |wanted: &str| {
        ports
            .iter()
            .find(|port| port.name.as_deref() == Some(wanted))
    };
    if let Some((port, path)) = compatible(name, label("app.kubernetes.io/name"), ports) {
        let number = u16::try_from(port.port).ok()?;
        return Some(Candidate {
            service: PrometheusService {
                https: speaks_tls(port),
                ..PrometheusService::new(namespace, name, number).with_path(path)
            },
            rank: Rank::Compatible,
        });
    }
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
    Some(Candidate {
        service: PrometheusService {
            https: speaks_tls(port),
            ..PrometheusService::new(namespace, name, number)
        },
        rank,
    })
}

fn speaks_tls(port: &ServicePort) -> bool {
    port.port == 443
        || port
            .name
            .as_deref()
            .is_some_and(|name| name.contains("https"))
        || port.app_protocol.as_deref() == Some("https")
}

/// A VictoriaMetrics, Thanos or Mimir Service that serves queries, with
/// the port and path prefix its API uses. Their writers, agents and
/// alerting companions don't match.
fn compatible<'a>(
    name: &str,
    app: Option<&str>,
    ports: &'a [ServicePort],
) -> Option<(&'a ServicePort, &'static str)> {
    let port = |numbers: &[i32]| {
        ports
            .iter()
            .find(|port| numbers.contains(&port.port))
            .or_else(|| {
                ports.iter().find(|port| {
                    port.name
                        .as_deref()
                        .is_some_and(|name| name.starts_with("http"))
                })
            })
    };
    let is = |prefix: &str, labels: &[&str]| {
        name.starts_with(prefix) || app.is_some_and(|app| labels.contains(&app))
    };
    if is("vmsingle", &["vmsingle", "victoria-metrics-single"])
        || name.contains("victoria-metrics-single")
    {
        return Some((port(&[8428, 8429])?, ""));
    }
    if is("vmselect", &["vmselect"]) || name.contains("-vmselect") {
        return Some((port(&[8481])?, "select/0/prometheus"));
    }
    if name.contains("thanos-query")
        || name.contains("thanos-querier")
        || app.is_some_and(|app| app == "thanos-query" || app == "thanos-querier")
    {
        return Some((port(&[9090, 10902])?, ""));
    }
    if (name.contains("mimir") || name.contains("cortex"))
        && (name.contains("query-frontend")
            || name.ends_with("-gateway")
            || name.ends_with("-nginx"))
    {
        return Some((port(&[8080, 80])?, "prometheus"));
    }
    None
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

/// Confirms one Service answers the Prometheus API, and reads its version
/// and scrape interval. Either may be missing on other servers.
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
    finish(Prometheus::new(client.clone(), service).with_timeout(timeout)).await
}

/// Confirms a URL answers the Prometheus API, with the token if any.
pub async fn confirm_url(
    url: String,
    token: Option<String>,
) -> Result<(Prometheus, BuildInfo), QueryError> {
    finish(Prometheus::direct(url, token)?).await
}

async fn finish(prometheus: Prometheus) -> Result<(Prometheus, BuildInfo), QueryError> {
    prometheus.probe().await?;
    let build = prometheus
        .build_info()
        .await
        .unwrap_or(BuildInfo { version: None });
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
