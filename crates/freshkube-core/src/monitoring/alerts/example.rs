//! Made-up alerts, rules and silences for `--fixture` and the tests, in the
//! shapes `FRESHKUBE_ALERTS` names. The names are invented.
use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};

use super::discover::LOOKED_FOR;
use super::model::{
    Alert, AlertState, Kind, Rule, RuleGroup, RuleHealth, RuleKind, Severity, Silence,
    SilenceMatcher, SilenceState,
};
use super::read::{Active, AlertsData, ReadFrom, RulesData, SilencesData};
use super::state::{State, Unknown};

/// What the example sources look like.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    /// Alerts of every state, rules with one broken, silences of every state.
    #[default]
    Firing,
    /// A current read with nothing active.
    Quiet,
    /// No Alertmanager or vmalert.
    NotFound,
    /// The proxy refused the read.
    Refused,
    /// Another Alertmanager answered beside the one read.
    Several,
    /// A central store whose alerts have no cluster label.
    NoLabel,
    /// The last read failed; the one before it is kept as evidence.
    Stale,
    /// Only PrometheusRule objects.
    RulesOnly,
}

impl Shape {
    /// The values `FRESHKUBE_ALERTS` takes.
    pub const NAMES: [&'static str; 8] = [
        "firing",
        "quiet",
        "not-found",
        "refused",
        "several",
        "no-label",
        "stale",
        "rules-only",
    ];

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "firing" => Self::Firing,
            "quiet" => Self::Quiet,
            "not-found" => Self::NotFound,
            "refused" => Self::Refused,
            "several" => Self::Several,
            "no-label" => Self::NoLabel,
            "stale" => Self::Stale,
            "rules-only" => Self::RulesOnly,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
}

fn read() -> Vec<ReadFrom> {
    vec![
        ReadFrom {
            kind: Kind::Alertmanager,
            label: "monitoring/alertmanager-operated:9093".into(),
            version: Some("0.27.0".into()),
            replicas: Some(3),
        },
        ReadFrom {
            kind: Kind::Vmalert,
            label: "monitoring/vmalert-shop:8880".into(),
            version: None,
            replicas: None,
        },
    ]
}

fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn alert(
    now: DateTime<Utc>,
    name: &str,
    state: AlertState,
    severity: &str,
    minutes: i64,
    pairs: &[(&str, &str)],
    summary: &str,
    runbook: bool,
) -> Alert {
    let mut all = vec![("alertname", name), ("severity", severity)];
    all.extend_from_slice(pairs);
    let labels = labels(&all);
    Alert {
        name: name.to_owned(),
        state,
        severity: Severity::parse(Some(severity)),
        fingerprint: format!("example-{name}-{}", pairs.len()),
        summary: Some(summary.to_owned()),
        description: Some(format!("{summary}. This is example data.")),
        runbook: runbook.then(|| format!("https://runbooks.example.com/{name}")),
        since: Some(now - Duration::minutes(minutes)),
        labels,
        silenced_by: if state == AlertState::Silenced {
            vec!["silence-worker-2".into()]
        } else {
            Vec::new()
        },
        inhibited_by: Vec::new(),
        source: if state == AlertState::Pending {
            Kind::Vmalert
        } else {
            Kind::Alertmanager
        },
    }
}

/// Every example alert: firing, pending, silenced and inhibited.
pub fn alerts(now: DateTime<Utc>) -> Vec<Alert> {
    use AlertState::*;
    vec![
        alert(
            now,
            "KubePodCrashLooping",
            Firing,
            "critical",
            42,
            &[
                ("namespace", "payments"),
                ("pod", "payments-api-6d8f9c7b5-x2k4q"),
                ("container", "api"),
            ],
            "Pod payments/payments-api-6d8f9c7b5-x2k4q is crash looping",
            true,
        ),
        alert(
            now,
            "NodeFilesystemSpaceFillingUp",
            Firing,
            "critical",
            130,
            &[("node", "worker-1"), ("mountpoint", "/var")],
            "Filesystem on worker-1 is filling up",
            true,
        ),
        alert(
            now,
            "KubeDeploymentReplicasMismatch",
            Firing,
            "warning",
            18,
            &[("namespace", "shop"), ("deployment", "shop-api")],
            "Deployment shop/shop-api has not matched the expected replicas",
            true,
        ),
        alert(
            now,
            "TargetDown",
            Firing,
            "warning",
            9,
            &[("namespace", "monitoring"), ("job", "node-exporter")],
            "1 of 4 node-exporter targets is down",
            false,
        ),
        alert(
            now,
            "CPUThrottlingHigh",
            Pending,
            "warning",
            3,
            &[("namespace", "shop"), ("pod", "shop-web-5c8d6b7f94-k9m2p")],
            "Pod shop/shop-web-5c8d6b7f94-k9m2p is throttled",
            false,
        ),
        alert(
            now,
            "KubeNodeNotReady",
            Silenced,
            "warning",
            75,
            &[("node", "worker-2")],
            "Node worker-2 is not ready",
            true,
        ),
        alert(
            now,
            "InfoInhibitor",
            Inhibited,
            "none",
            600,
            &[("namespace", "shop")],
            "Inhibits info alerts while a warning fires",
            false,
        ),
    ]
}

/// The active ones, as the session reads them.
pub fn active(now: DateTime<Utc>) -> Active {
    let alerts: Vec<Alert> = alerts(now)
        .into_iter()
        .filter(|alert| alert.state.is_active())
        .collect();
    Active {
        alerts,
        more: 0,
        read: read(),
        others: Vec::new(),
    }
}

/// What the session knows under `shape`, at `now`.
pub fn state(shape: Shape, now: DateTime<Utc>) -> State {
    match shape {
        Shape::Firing => State::Known(active(now)),
        Shape::Quiet => State::Known(Active {
            alerts: Vec::new(),
            more: 0,
            read: read(),
            others: Vec::new(),
        }),
        Shape::NotFound => State::Unknown(Unknown::NotFound(LOOKED_FOR.into())),
        Shape::Refused => State::Unknown(Unknown::Refused(
            crate::monitoring::QueryError::refused().message,
        )),
        Shape::Several => State::Known(Active {
            others: vec!["monitoring/alertmanager-staging:9093".into()],
            ..active(now)
        }),
        Shape::NoLabel => State::Unknown(Unknown::NoClusterLabel("cluster".into())),
        Shape::Stale => State::Stale {
            last: active(now),
            why: "Alertmanager didn't answer within 20 s".into(),
        },
        Shape::RulesOnly => State::Unknown(Unknown::RulesOnly),
    }
}

/// The Alerts page under `shape`.
pub fn alerts_page(shape: Shape, now: DateTime<Utc>) -> Result<AlertsData, Unknown> {
    match state(shape, now) {
        State::Known(active) | State::Stale { last: active, .. } => {
            let alerts = match shape {
                Shape::Quiet => Vec::new(),
                _ => alerts(now),
            };
            Ok(AlertsData {
                alerts,
                more: 0,
                read: active.read,
                others: active.others,
            })
        }
        State::Unknown(unknown) => Err(unknown),
        State::Reading => Err(Unknown::Unread("Not read yet".into())),
    }
}

fn rule(name: &str, kind: RuleKind, health: RuleHealth, evaluated: bool) -> Rule {
    Rule {
        name: name.to_owned(),
        kind,
        health,
        last_error: (health == RuleHealth::Error).then(|| {
            "vector contains metrics with the same labelset after applying alert labels".into()
        }),
        last_evaluation: None,
        query: match kind {
            RuleKind::Alerting => "up == 0".into(),
            RuleKind::Recording => {
                "sum by (namespace) (rate(container_cpu_usage_seconds_total[5m]))".into()
            }
        },
        alerts: 0,
        firing: false,
        source: if evaluated {
            Kind::Vmalert
        } else {
            Kind::PrometheusRule
        },
    }
}

/// The example rule groups; `evaluated` ones carry health and times.
pub fn rule_groups(now: DateTime<Utc>, evaluated: bool) -> Vec<RuleGroup> {
    let health = |health| {
        if evaluated {
            health
        } else {
            RuleHealth::Unknown
        }
    };
    let mut groups = vec![
        RuleGroup {
            name: "kubernetes-apps".into(),
            origin: Some("monitoring/kube-prometheus-rules".into()),
            interval_seconds: Some(30.),
            last_evaluation: evaluated.then(|| now - Duration::seconds(12)),
            rules: vec![
                rule(
                    "KubePodCrashLooping",
                    RuleKind::Alerting,
                    health(RuleHealth::Ok),
                    evaluated,
                ),
                rule(
                    "KubeDeploymentReplicasMismatch",
                    RuleKind::Alerting,
                    health(RuleHealth::Ok),
                    evaluated,
                ),
                rule(
                    "TargetDown",
                    RuleKind::Alerting,
                    health(RuleHealth::Error),
                    evaluated,
                ),
            ],
        },
        RuleGroup {
            name: "node.rules".into(),
            origin: Some("monitoring/node-rules".into()),
            interval_seconds: Some(60.),
            last_evaluation: evaluated.then(|| now - Duration::seconds(41)),
            rules: vec![
                rule(
                    "NodeFilesystemSpaceFillingUp",
                    RuleKind::Alerting,
                    health(RuleHealth::Ok),
                    evaluated,
                ),
                rule(
                    "namespace:cpu_usage:rate5m",
                    RuleKind::Recording,
                    health(RuleHealth::Ok),
                    evaluated,
                ),
            ],
        },
    ];
    if evaluated {
        for group in &mut groups {
            for rule in &mut group.rules {
                rule.last_evaluation = group.last_evaluation;
            }
        }
        groups[0].rules[2].last_error = Some(
            "vector contains metrics with the same labelset after applying alert labels".into(),
        );
        groups[0].rules[0].alerts = 1;
        groups[0].rules[0].firing = true;
    }
    groups
}

/// The Rules page under `shape`.
pub fn rules_page(shape: Shape, now: DateTime<Utc>) -> Result<RulesData, Unknown> {
    match shape {
        Shape::NotFound => Err(Unknown::NotFound(
            "No vmalert was found and the cluster holds no PrometheusRule".into(),
        )),
        Shape::Refused => Err(Unknown::Refused(
            crate::monitoring::QueryError::refused().message,
        )),
        Shape::NoLabel => Ok(RulesData {
            groups: rule_groups(now, false),
            evaluated: false,
            read: Vec::new(),
            note: Some(
                "The central vmalert's rules aren't scoped to one cluster, so only this \
                 cluster's PrometheusRule objects are shown"
                    .into(),
            ),
        }),
        Shape::RulesOnly => Ok(RulesData {
            groups: rule_groups(now, false),
            evaluated: false,
            read: Vec::new(),
            note: Some("No vmalert evaluates these rules here, so their health is Unknown".into()),
        }),
        _ => Ok(RulesData {
            groups: rule_groups(now, true),
            evaluated: true,
            read: read(),
            note: None,
        }),
    }
}

/// The example silences, one of each state.
pub fn silences(now: DateTime<Utc>) -> Vec<Silence> {
    let matcher = |name: &str, value: &str, regex: bool| SilenceMatcher {
        name: name.into(),
        value: value.into(),
        regex,
        equal: true,
    };
    vec![
        Silence {
            id: "silence-worker-2".into(),
            state: SilenceState::Active,
            matchers: vec![matcher("node", "worker-2", false)],
            created_by: "alex@example.com".into(),
            comment: "Planned maintenance on worker-2".into(),
            starts_at: Some(now - Duration::hours(1)),
            ends_at: Some(now + Duration::hours(1)),
        },
        Silence {
            id: "silence-shop-nightly".into(),
            state: SilenceState::Pending,
            matchers: vec![
                matcher("alertname", "KubeJobFailed", false),
                matcher("namespace", "shop|payments", true),
            ],
            created_by: "sam@example.com".into(),
            comment: "Nightly import window".into(),
            starts_at: Some(now + Duration::hours(5)),
            ends_at: Some(now + Duration::hours(7)),
        },
        Silence {
            id: "silence-old".into(),
            state: SilenceState::Expired,
            matchers: vec![matcher("alertname", "Watchdog", false)],
            created_by: "alex@example.com".into(),
            comment: "Upgrade of the monitoring stack".into(),
            starts_at: Some(now - Duration::days(2)),
            ends_at: Some(now - Duration::days(2) + Duration::hours(2)),
        },
    ]
}

/// The Silences page under `shape`.
pub fn silences_page(shape: Shape, now: DateTime<Utc>) -> Result<SilencesData, Unknown> {
    match state(shape, now) {
        State::Known(active) | State::Stale { last: active, .. } => Ok(SilencesData {
            silences: silences(now),
            alerts: if shape == Shape::Quiet {
                Vec::new()
            } else {
                alerts(now)
            },
            read: active.read,
        }),
        State::Unknown(Unknown::RulesOnly) => Err(Unknown::NotFound(
            "No Alertmanager was found, and only it holds silences".into(),
        )),
        State::Unknown(unknown) => Err(unknown),
        State::Reading => Err(Unknown::Unread("Not read yet".into())),
    }
}
