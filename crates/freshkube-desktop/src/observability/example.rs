//! Sanitized observations and example-only later destinations.
use super::model::*;
use freshkube_core::coroot as api;
use std::collections::BTreeMap;

pub(super) const WORKER: &str = "payments/worker";
pub(super) const POD: &str = "worker-6c4f8da0-bbbbh";
pub(super) fn id(key: &str) -> api::AppId {
    let (ns, name) = key.split_once('/').unwrap_or(("_", key));
    api::AppId::new(format!("fixture:{ns}:Deployment:{name}"))
}
pub(super) fn threshold(_: &str, report: Report) -> &'static str {
    report.default_threshold()
}

pub(super) fn applications() -> Vec<api::Application> {
    let records = [
        ("payments/api", "Go", 0, api::Status::Critical),
        (WORKER, "Go", 0, api::Status::Critical),
        ("payments/ledger", "Java", 0, api::Status::Warning),
        ("payments/ledger-db", "Postgres", 0, api::Status::Warning),
        ("platform/keycloak", "Java", 0, api::Status::Warning),
        ("platform/oauth2-proxy", "Go", 0, api::Status::Warning),
        ("cache/redis-cache", "Redis", 0, api::Status::Warning),
        ("kube-system/kube-apiserver", "Go", 1, api::Status::Warning),
        ("kube-system/etcd", "Talos", 1, api::Status::Warning),
        ("kube-system/coredns", "Go", 1, api::Status::Warning),
        ("kube-system/cilium", "Go", 1, api::Status::Warning),
        (
            "argocd/argocd-application-controller",
            "Go",
            1,
            api::Status::Warning,
        ),
        ("ingress/ingress-nginx", "Nginx", 1, api::Status::Warning),
        ("monitoring/prometheus", "Go", 2, api::Status::Warning),
        ("monitoring/alertmanager", "Go", 2, api::Status::Warning),
        ("logging/loki", "Go", 2, api::Status::Warning),
    ];
    let mut apps: Vec<_> = records
        .into_iter()
        .map(|(key, language, category, status)| api::Application {
            id: id(key),
            cluster: "Fictional cluster".into(),
            category: ["application", "control-plane", "monitoring"][category].into(),
            app_type: language.into(),
            status,
            signals: Report::ALL
                .into_iter()
                .map(|r| {
                    (
                        r.signal().into(),
                        api::Signal {
                            status: api::Status::Ok,
                            value: String::new(),
                        },
                    )
                })
                .collect(),
        })
        .collect();
    let changes = [
        (0, Report::Errors, api::Status::Critical, "2.8%"),
        (0, Report::Upstreams, api::Status::Critical, "worker"),
        (0, Report::Logs, api::Status::Warning, "1.9k"),
        (1, Report::Upstreams, api::Status::Critical, "ledger-db"),
        (1, Report::Instances, api::Status::Critical, "0/1"),
        (1, Report::Restarts, api::Status::Warning, "14"),
        (1, Report::Net, api::Status::Critical, "refused"),
        (1, Report::Logs, api::Status::Warning, "212"),
        (2, Report::Instances, api::Status::Warning, "1/2"),
        (3, Report::Instances, api::Status::Warning, "1/2"),
        (3, Report::Disk, api::Status::Unknown, "required"),
        (4, Report::Instances, api::Status::Warning, "1/2"),
        (4, Report::Memory, api::Status::Warning, "76%"),
        (5, Report::Instances, api::Status::Warning, "1/2"),
        (6, Report::Instances, api::Status::Warning, "2/3"),
        (7, Report::Latency, api::Status::Warning, "294ms"),
        (8, Report::Disk, api::Status::Warning, "83%"),
        (9, Report::Instances, api::Status::Warning, "1/2"),
        (10, Report::Instances, api::Status::Warning, "5/6"),
        (11, Report::Cpu, api::Status::Warning, "86%"),
        (11, Report::Logs, api::Status::Warning, "41"),
        (12, Report::Upstreams, api::Status::Critical, "api"),
        (13, Report::Memory, api::Status::Warning, "88%"),
        (13, Report::Disk, api::Status::Warning, "83%"),
        (14, Report::Instances, api::Status::Warning, "1/2"),
        (15, Report::Instances, api::Status::Warning, "1/2"),
    ];
    for (app, report, status, value) in changes {
        apps[app].signals.insert(
            report.signal().into(),
            api::Signal {
                status,
                value: value.into(),
            },
        );
    }
    apps[1].signals.insert(
        "cpu".into(),
        api::Signal {
            status: api::Status::Ok,
            value: String::new(),
        },
    );
    apps[1].signals.insert(
        "errors".into(),
        api::Signal {
            status: api::Status::Unknown,
            value: String::new(),
        },
    );
    apps[1].signals.remove("disk_io_load");
    let healthy = [
        "checkout",
        "catalog",
        "notifications",
        "billing",
        "scheduler",
        "web",
        "frontend",
        "session",
        "audit",
        "gateway",
        "inventory",
        "search",
        "exporter",
        "queue",
        "processor",
        "cache-warm",
        "sync",
        "reporter",
        "backup",
        "harbor-core",
        "registry",
        "kube-scheduler",
        "kube-controller-manager",
        "metrics-server",
        "kube-proxy",
        "node-local-dns",
        "cert-manager",
        "external-dns",
        "operator",
        "grafana",
        "node-exporter",
    ];
    for (ix, name) in healthy.into_iter().enumerate() {
        let category = if ix < 21 {
            0
        } else if ix < 29 {
            1
        } else {
            2
        };
        let namespace = ["payments", "kube-system", "monitoring"][category];
        apps.push(api::Application {
            id: id(&format!("{namespace}/{name}")),
            cluster: "Fictional cluster".into(),
            category: ["application", "control-plane", "monitoring"][category].into(),
            app_type: "Go".into(),
            status: api::Status::Ok,
            signals: Report::ALL
                .into_iter()
                .map(|r| {
                    (
                        r.signal().into(),
                        api::Signal {
                            status: api::Status::Ok,
                            value: String::new(),
                        },
                    )
                })
                .collect(),
        });
    }
    apps
}

pub(super) fn map() -> api::ServiceMap {
    let names = [
        "ingress/ingress-nginx",
        "platform/oauth2-proxy",
        "payments/api",
        "payments/ledger",
        WORKER,
        "payments/ledger-db",
        "cache/redis-cache",
    ];
    let apps = applications();
    let nodes = names
        .iter()
        .map(|key| {
            let app = apps.iter().find(|a| a.id == id(key)).unwrap();
            api::MapNode {
                id: app.id.clone(),
                cluster: app.cluster.clone(),
                category: app.category.clone(),
                status: app.status,
                custom: false,
                labels: BTreeMap::new(),
                indicators: BTreeMap::new(),
                distance: None,
            }
        })
        .collect();
    let edges = [
        (0, 1, false),
        (0, 2, false),
        (1, 2, false),
        (2, 3, true),
        (2, 4, true),
        (4, 5, true),
        (4, 6, false),
        (3, 5, true),
    ]
    .into_iter()
    .map(|(from, to, problem)| api::MapEdge {
        from: id(names[from]),
        to: id(names[to]),
        status: if problem {
            api::Status::Critical
        } else {
            api::Status::Ok
        },
        rps: Some(12.),
        latency_seconds: Some(0.003),
        sent_bytes_per_second: Some(2400.),
        received_bytes_per_second: None,
        issue: if problem {
            "Connection errors reported by Coroot".into()
        } else {
            String::new()
        },
    })
    .collect();
    api::ServiceMap { nodes, edges }
}

pub(super) fn health(app: &api::AppId, extended: bool) -> api::AppHealth {
    let apps = applications();
    let value = apps.iter().find(|a| a.id == *app);
    let worker = *app == id(WORKER);
    // Each report takes the worst of the checks the matrix shows for it, so
    // the report agrees with the row the user opened it from.
    let reports = [
        "SLO",
        "Instances",
        "CPU",
        "Memory",
        "Storage",
        "Net",
        "DNS",
        "Logs",
    ]
    .into_iter()
    .map(|name| {
        let checks: Vec<_> = Report::ALL
            .into_iter()
            .filter(|r| r.server_name() == name)
            .filter_map(|r| Some((r, value?.signals.get(r.signal())?)))
            .collect();
        let status = checks
            .iter()
            .map(|(_, signal)| signal.status)
            .max()
            .unwrap_or_default();
        let issues = checks
            .iter()
            .filter(|(_, signal)| signal.status > api::Status::Info)
            .map(|(report, signal)| api::Issue {
                id: format!("{}Check", report.label().replace(' ', "")),
                title: issue_title(*report).into(),
                status: signal.status,
                message: if worker && *report == Report::Net {
                    "The worker cannot connect to ledger-db:5432. The Service exposes port 6432."
                        .into()
                } else {
                    format!("{} reported {}.", report.label(), signal.value)
                },
            })
            .collect();
        api::Report {
            name: name.into(),
            status,
            issues,
            charts: if extended && name == "CPU" {
                vec![api::Chart {
                    title: "CPU usage (cores)".into(),
                    series: vec![api::SeriesSummary {
                        name: "worker".into(),
                        last: Some(0.1),
                        min: Some(0.0),
                        max: Some(0.2),
                        avg: Some(0.1),
                        sparkline: vec![Some(0.1), None, Some(0.2)],
                        ..Default::default()
                    }],
                    series_omitted: 0,
                }]
            } else {
                vec![]
            },
            log_patterns: if extended && worker && name == "Logs" {
                vec![api::LogPatternSummary {
                    hash: "example-connect".into(),
                    severity: "error".into(),
                    sample: "Connection refused (sanitized example)".into(),
                    messages: 212,
                }]
            } else {
                vec![]
            },
        }
    })
    .collect();
    api::AppHealth {
        id: app.clone(),
        namespace: app.namespace().unwrap_or_default().into(),
        vitals: vec![],
        status: value.map_or(api::Status::Unknown, |a| a.status),
        reports,
        dependencies: if worker {
            vec![api::Dependency {
                id: id("payments/ledger-db"),
                status: api::Status::Warning,
                connectivity: api::Status::Critical,
                connectivity_message: "Connection refused".into(),
                protocols: vec!["postgres".into()],
                rtt_seconds: None,
                rps: None,
                errors_per_sec: None,
                latency_seconds: None,
            }]
        } else {
            vec![]
        },
        clients: if worker {
            vec![api::ClientLink {
                id: id("payments/api"),
                status: api::Status::Critical,
                rps: Some(12.),
                latency_seconds: Some(0.003),
            }]
        } else {
            vec![]
        },
    }
}

fn issue_title(report: Report) -> &'static str {
    match report {
        Report::Errors => "Requests failing",
        Report::Latency => "Requests slow",
        Report::Upstreams => "Upstream failing",
        Report::Instances => "Instances unavailable",
        Report::Restarts => "Containers restarting",
        Report::Cpu => "CPU throttled",
        Report::Memory => "Memory near its limit",
        Report::Disk => "Disk filling up",
        Report::DiskIo => "Disk saturated",
        Report::Net => "Connection errors",
        Report::Dns => "DNS errors",
        Report::Logs => "Errors in logs",
    }
}

pub(super) fn flame() -> Vec<FlameFrame> {
    let rows = [
        ("total", 0., 1., 0, 0, "860m", None),
        ("runtime.goexit", 0., 0.78, 1, 0, "670m", Some(0)),
        ("runtime.gcBgMarkWorker", 0.78, 0.22, 1, 9, "190m", Some(0)),
        (
            "controller.(*ApplicationController).processAppRefreshQueueItem",
            0.,
            0.64,
            2,
            18,
            "550m",
            Some(1),
        ),
        (
            "cache.(*liveStateCache).get",
            0.64,
            0.14,
            2,
            -9,
            "120m",
            Some(1),
        ),
        ("runtime.gcDrain", 0.78, 0.22, 2, 7, "190m", Some(2)),
        (
            "controller.(*appStateManager).CompareAppState",
            0.,
            0.45,
            3,
            18,
            "387m",
            Some(3),
        ),
        (
            "controller.(*ApplicationController).sync",
            0.45,
            0.19,
            3,
            0,
            "163m",
            Some(3),
        ),
        (
            "cache.(*clusterCache).get",
            0.64,
            0.14,
            3,
            -11,
            "120m",
            Some(4),
        ),
        ("runtime.scanobject", 0.78, 0.14, 3, 7, "120m", Some(5)),
        ("runtime.mallocgc", 0.92, 0.08, 3, 4, "69m", Some(5)),
        ("diff.(*Diff).Diff", 0., 0.25, 4, 18, "215m", Some(6)),
        (
            "normalizers.(*knownTypeFields).Normalize",
            0.25,
            0.20,
            4,
            11,
            "172m",
            Some(6),
        ),
        ("kube.(*client).Patch", 0.45, 0.19, 4, 0, "163m", Some(7)),
        (
            "structured-merge-diff.(*TypedValue).Compare",
            0.,
            0.15,
            5,
            18,
            "129m",
            Some(11),
        ),
        (
            "json.(*decodeState).object",
            0.15,
            0.10,
            5,
            11,
            "86m",
            Some(11),
        ),
        ("filepath.(*Set).Insert", 0.25, 0.09, 5, 18, "77m", Some(12)),
        ("json.Marshal", 0.45, 0.12, 5, 0, "103m", Some(13)),
    ];
    rows.into_iter()
        .map(|(name, x, width, depth, delta, cpu, parent)| FlameFrame {
            name,
            x,
            width,
            depth,
            delta,
            cpu,
            parent,
        })
        .collect()
}

pub(super) fn chart(
    title: &'static str,
    unit: &'static str,
    hours: u32,
    failure: bool,
    two: bool,
) -> Chart {
    let maximum = if unit == "% of limit" {
        100.
    } else if failure {
        4.
    } else {
        60.
    };
    // The example snapshot ends at 15:00. Deploy 12:52; node event 14:48.
    let deploy = 1. - 128. / (hours as f32 * 60.);
    let event = (1. - 12. / (hours as f32 * 60.)).clamp(0., 1.);
    let values = (0..64)
        .map(|ix| {
            let minutes_ago = (1. - ix as f32 / 63.) * hours as f32 * 60.;
            let after_deploy = minutes_ago <= 128.;
            if failure {
                if after_deploy {
                    2.8 + (ix % 5) as f32 * 0.1
                } else {
                    0.
                }
            } else if unit == "% of limit" {
                (if after_deploy { 72. } else { 48. }) + (ix % 4) as f32 * 1.5
            } else if after_deploy {
                0.
            } else {
                40. + (ix % 4) as f32
            }
        })
        .collect();
    let mut series = vec![Series {
        label: if failure {
            "failed requests"
        } else if unit == "% of limit" {
            "CPU"
        } else {
            "ledger-db"
        },
        values,
    }];
    if two {
        series.push(Series {
            label: if unit == "% of limit" {
                "previous period"
            } else {
                "redis-cache"
            },
            values: (0..64)
                .map(|ix| {
                    if unit == "% of limit" {
                        48. + (ix % 5) as f32
                    } else {
                        16. + (ix % 4) as f32
                    }
                })
                .collect(),
        });
    }
    Chart {
        title,
        unit,
        series,
        maximum,
        y_ticks: std::array::from_fn(|i| format!("{:.0}", maximum * (1. - i as f32 / 3.))),
        ticks: [
            format!("−{hours}h"),
            format!("−{}m", hours * 40),
            format!("−{}m", hours * 20),
            "now".into(),
        ],
        deploy,
        event,
    }
}

/// Example incidents, newest problem first, as Coroot's incident list
/// answers them; dated back from the observation window's end.
pub(super) fn incidents(to: chrono::DateTime<chrono::Utc>) -> Vec<api::Incident> {
    ["INC-12", "INC-13", "INC-11"]
        .into_iter()
        .filter_map(|key| incident_view(key, to))
        .map(|view| view.incident().clone())
        .collect()
}

/// One example incident with its objectives, as Coroot's incident view
/// answers it.
pub(super) fn incident_view(
    key: &str,
    to: chrono::DateTime<chrono::Utc>,
) -> Option<api::IncidentView> {
    use serde_json::json;
    let at = |minutes: i64| (to - chrono::Duration::minutes(minutes)).to_rfc3339();
    let rate = |severity: &str, long: f64, short: f64| {
        json!({
            "severity": severity,
            "long_window_seconds": 3600,
            "short_window_seconds": 300,
            "long_window_burn_rate": long,
            "short_window_burn_rate": short,
            "threshold": 14.4,
        })
    };
    let availability = |compliance: &str, violated: bool| {
        json!({
            "objective": "99% of requests should not fail",
            "compliance": compliance,
            "violated": violated,
        })
    };
    let latency = |compliance: &str, violated: bool| {
        json!({
            "objective": "99% of requests should be served faster than 500ms",
            "compliance": compliance,
            "violated": violated,
            "latency_threshold_seconds": 0.5,
        })
    };
    let value = match key {
        "INC-12" => json!({
            "incident": {
                "key": key,
                "app": id("payments/api").as_str(),
                "cluster": "Fictional cluster",
                "severity": "critical",
                "state": "open",
                "opened_at": at(122),
                "duration_seconds": 122 * 60,
                "impact_percent": 2.8,
                "description": "Serving errors",
                "rca": {
                    "status": "OK",
                    "summary": "Settlement calls from payments/api fail while payments/worker restarts.",
                    "root_cause": "Release 1.8.2 of payments/worker changed DATABASE_URL to port 5432. The ledger-db Service exposes only 6432 (pgbouncer). The worker exits on connection refused and the API's settlement calls fail.",
                    "immediate_fixes": "Roll back payments/worker to 1.8.1, or set DATABASE_URL to ledger-db:6432.",
                    "propagation": [
                        {
                            "app_id": id(WORKER).as_str(),
                            "status": "critical",
                            "issues": ["Exits on start · connection refused by ledger-db:5432"],
                        },
                        {
                            "app_id": id("payments/ledger-db").as_str(),
                            "status": "warning",
                            "issues": ["Refused connections on 5432"],
                        },
                    ],
                },
                "slo": {
                    "availability_impact_percent": 2.8,
                    "latency_impact_percent": 0.4,
                    "availability_burn_rates": [rate("critical", 18.4, 21.0)],
                    "latency_burn_rates": [rate("ok", 0.4, 0.6)],
                },
            },
            "availability": availability("97.2%", true),
            "latency": latency("99.6%", false),
        }),
        "INC-13" => json!({
            "incident": {
                "key": key,
                "app": id("platform/keycloak").as_str(),
                "cluster": "Fictional cluster",
                "severity": "warning",
                "state": "open",
                "opened_at": at(10),
                "duration_seconds": 10 * 60,
                "impact_percent": 0.3,
                "description": "Lost an instance",
                "rca": {
                    "status": "OK",
                    "root_cause": "talos-wk-fra1-03 stopped reporting. One keycloak instance is no longer ready; the remaining instance continues serving requests. The worker release happened earlier and is unrelated.",
                    "propagation": [
                        {
                            "app_id": id("platform/oauth2-proxy").as_str(),
                            "status": "warning",
                            "issues": ["Requests reach the remaining instance"],
                        },
                    ],
                },
                "slo": {
                    "availability_impact_percent": 0.3,
                    "latency_impact_percent": null,
                    "availability_burn_rates": [rate("warning", 6.2, 15.1)],
                    "latency_burn_rates": [],
                },
            },
            "availability": availability("99.7%", true),
            "latency": latency("99.4%", false),
        }),
        "INC-11" => json!({
            "incident": {
                "key": key,
                "app": id("payments/checkout").as_str(),
                "cluster": "Fictional cluster",
                "severity": "warning",
                "state": "resolved",
                "opened_at": at(170),
                "resolved_at": at(140),
                "duration_seconds": 30 * 60,
                "impact_percent": 0.6,
                "description": "Latency above objective",
                "rca": {
                    "status": "OK",
                    "summary": "Checkout slowed while ledger-db ran its nightly vacuum; latency recovered when it finished.",
                },
                "slo": {
                    "availability_impact_percent": null,
                    "latency_impact_percent": 0.6,
                    "availability_burn_rates": [],
                    "latency_burn_rates": [rate("warning", 2.1, 0.8)],
                },
            },
            "availability": availability("99.9%", false),
            "latency": latency("98.4%", true),
        }),
        _ => return None,
    };
    Some(serde_json::from_value(value).expect("example incident"))
}
