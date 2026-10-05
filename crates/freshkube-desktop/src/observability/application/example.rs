//! Coroot's own view of a fixture application, derived from the signals
//! the Applications list shows, so the page agrees with the row it was
//! opened from. Every name and figure is invented.
use super::super::{example, model::Report};
use freshkube_core::coroot as api;
use std::collections::BTreeMap;

/// Each signal's check: its title, Coroot's condition, threshold and unit.
const CHECKS: [(Report, &str, &str, f32, &str); 12] = [
    (
        Report::Errors,
        "Availability",
        "the successful request percentage < <threshold>",
        99.,
        "percent",
    ),
    (
        Report::Latency,
        "Latency",
        "the percentage of requests served faster than 500ms < <threshold>",
        99.,
        "percent",
    ),
    (
        Report::Upstreams,
        "Upstream services",
        "an upstream service is unavailable or responds slowly",
        0.,
        "",
    ),
    (
        Report::Instances,
        "Instance availability",
        "the number of available instances < <threshold> of the desired",
        75.,
        "percent",
    ),
    (
        Report::Restarts,
        "Restarts",
        "the number of container restarts > <threshold>",
        2.,
        "",
    ),
    (
        Report::Cpu,
        "CPU usage",
        "the CPU usage of a container > <threshold> of its CPU limit",
        80.,
        "percent",
    ),
    (
        Report::Memory,
        "Memory usage",
        "the memory usage of a container > <threshold> of its memory limit",
        80.,
        "percent",
    ),
    (
        Report::Disk,
        "Disk space",
        "the space used on a volume > <threshold>",
        80.,
        "percent",
    ),
    (
        Report::DiskIo,
        "Disk I/O load",
        "the I/O load of a volume > <threshold>",
        5.,
        "seconds/second",
    ),
    (
        Report::Net,
        "Network round-trip time",
        "the round-trip time to a dependency > <threshold>",
        0.01,
        "second",
    ),
    (
        Report::Dns,
        "DNS latency",
        "the 95th percentile of DNS latency > <threshold>",
        0.1,
        "second",
    ),
    (
        Report::Logs,
        "Errors",
        "the number of messages with the ERROR and CRITICAL severity levels > <threshold>",
        0.,
        "",
    ),
];

/// The reports Coroot lists, in its order.
const REPORTS: [&str; 11] = [
    "SLO",
    "Instances",
    "CPU",
    "Memory",
    "Storage",
    "Net",
    "DNS",
    "Logs",
    "Deployments",
    "Profiling",
    "Tracing",
];

pub(in crate::observability) fn app_view(app: &api::AppId) -> api::AppView {
    let apps = example::applications();
    let record = apps.iter().find(|a| a.id == *app);
    let health = example::health(app, false);
    let worker = *app == example::id(example::WORKER);
    let pods = pods(app, record);
    let restarts = record
        .and_then(|r| r.signals.get(Report::Restarts.signal()))
        .map_or("0", |s| s.value.as_str());
    let reports = REPORTS
        .into_iter()
        .map(|name| {
            let mut checks: Vec<_> = CHECKS
                .iter()
                .filter(|(report, ..)| report.server_name() == name && *report != Report::Logs)
                .filter_map(|&(report, title, condition, threshold, unit)| {
                    let signal = record?.signals.get(report.signal())?;
                    let message = issue(&health, report).unwrap_or_default().into();
                    Some(api::Check {
                        id: format!("{}Check", report.label().replace([' ', '/'], "")),
                        title: title.into(),
                        status: signal.status,
                        message,
                        threshold,
                        unit: unit.into(),
                        condition: condition.into(),
                    })
                })
                .collect();
            let logs = (name == "Logs").then(|| logs_check(record, &health));
            let status = checks
                .iter()
                .map(|c| c.status)
                .chain(logs.iter().flatten().map(|c| c.status))
                .max()
                .unwrap_or_default();
            if name == "Logs" {
                checks.clear();
            }
            api::AppReport {
                name: name.into(),
                status,
                checks,
                widgets: widgets(name, worker, &pods, restarts, logs.flatten()),
                custom: false,
                instrumentation: String::new(),
            }
        })
        .collect();
    api::AppView {
        map: map(app, record, &apps, &pods),
        reports,
    }
}

/// The example's verdict on a signal, by the issue named after its check.
fn issue(health: &api::AppHealth, report: Report) -> Option<&str> {
    let id = format!("{}Check", report.label().replace(' ', ""));
    health
        .reports
        .iter()
        .flat_map(|r| &r.issues)
        .find(|issue| issue.id == id)
        .map(|issue| issue.message.as_str())
}

/// Coroot judges logs in the Logs widget rather than the report's checks.
fn logs_check(record: Option<&api::Application>, health: &api::AppHealth) -> Option<api::Check> {
    let (_, title, condition, threshold, unit) = CHECKS[Report::Logs.index()];
    let signal = record?.signals.get(Report::Logs.signal())?;
    Some(api::Check {
        id: format!("{}Check", Report::Logs.label().replace(' ', "")),
        title: title.into(),
        status: signal.status,
        message: issue(health, Report::Logs)
            .map(|_| format!("{} errors occurred", signal.value))
            .unwrap_or_default(),
        threshold,
        unit: unit.into(),
        condition: condition.into(),
    })
}

fn widgets(
    report: &str,
    worker: bool,
    pods: &[(String, bool)],
    restarts: &str,
    logs: Option<api::Check>,
) -> Vec<api::Widget> {
    let full = |kind| api::Widget { kind, width: 1. };
    match report {
        "Instances" => vec![full(api::WidgetKind::Table(instances(pods, restarts)))],
        "Net" if worker => vec![full(api::WidgetKind::Table(dependencies()))],
        "Logs" => vec![full(api::WidgetKind::Logs(logs))],
        "Deployments" => vec![full(api::WidgetKind::Table(deployments()))],
        "Profiling" => vec![full(api::WidgetKind::Profiling)],
        "Tracing" => vec![full(api::WidgetKind::Tracing)],
        _ => vec![],
    }
}

fn text(value: &str) -> api::Cell {
    api::Cell {
        value: value.into(),
        ..Default::default()
    }
}
fn with_status(value: &str, status: api::Status) -> api::Cell {
    api::Cell {
        status: Some(status),
        ..text(value)
    }
}

/// The application's pods, as the map lists them: the worker's two, or one
/// named after any other application, up while its instances check passes.
fn pods(app: &api::AppId, record: Option<&api::Application>) -> Vec<(String, bool)> {
    if *app == example::id(example::WORKER) {
        return vec![
            (example::POD.into(), true),
            ("worker-6c4f8da0-x2k9q".into(), false),
        ];
    }
    let failing = record
        .and_then(|r| r.signals.get(Report::Instances.signal()))
        .is_some_and(|s| s.status >= api::Status::Warning);
    vec![(format!("{}-0", app.name()), failing)]
}

fn instances(pods: &[(String, bool)], restarts: &str) -> api::Table {
    api::Table {
        header: ["Instance", "Status", "Restarts", "IP", "Node"]
            .map(String::from)
            .into(),
        rows: pods
            .iter()
            .enumerate()
            .map(|(ix, (name, failing))| {
                let node = ["node-a", "node-b"][ix % 2];
                vec![
                    with_status(
                        name,
                        if *failing {
                            api::Status::Critical
                        } else {
                            api::Status::Ok
                        },
                    ),
                    text(if *failing { "CrashLoopBackOff" } else { "up" }),
                    text(if *failing { restarts } else { "0" }),
                    text(&format!("10.0.{}.{}", ix + 1, 17 + 14 * ix)),
                    api::Cell {
                        link: Some(api::CellLink {
                            title: node.into(),
                            view: "nodes".into(),
                            id: node.into(),
                        }),
                        ..text(node)
                    },
                ]
            })
            .collect(),
    }
}

fn dependencies() -> api::Table {
    let link = |key: &str| {
        let id = example::id(key);
        api::CellLink {
            title: id.name().into(),
            view: "applications".into(),
            id: id.as_str().into(),
        }
    };
    let row = |key: &str, status, connectivity: &str, rtt: &str| {
        vec![
            api::Cell {
                link: Some(link(key)),
                ..with_status(example::id(key).name(), status)
            },
            text(connectivity),
            api::Cell {
                unit: "ms".into(),
                ..text(rtt)
            },
            text("0"),
        ]
    };
    api::Table {
        header: ["Dependency", "Connectivity", "RTT", "Retransmissions"]
            .map(String::from)
            .into(),
        rows: vec![
            row(
                "payments/ledger-db",
                api::Status::Critical,
                "connection refused",
                "—",
            ),
            row("cache/redis-cache", api::Status::Ok, "ok", "0.4"),
        ],
    }
}

fn deployments() -> api::Table {
    let summary = |report: &str, ok, message: &str| api::DeploymentSummary {
        report: report.into(),
        ok,
        message: message.into(),
        time_ms: None,
    };
    let row = |version: &str, age: &str, summaries: Vec<api::DeploymentSummary>| {
        vec![
            text(version),
            text(age),
            api::Cell {
                deployments: summaries,
                ..Default::default()
            },
        ]
    };
    api::Table {
        header: ["Version", "Deployed", "Summary"].map(String::from).into(),
        rows: vec![
            row(
                "worker:1.8.2",
                "2 hours ago",
                vec![
                    summary("Instances", false, "restarts increased"),
                    summary("CPU", true, "CPU usage unchanged"),
                ],
            ),
            row(
                "worker:1.8.1",
                "3 days ago",
                vec![summary("SLO", true, "no change")],
            ),
        ],
    }
}

fn map(
    app: &api::AppId,
    record: Option<&api::Application>,
    apps: &[api::Application],
    pods: &[(String, bool)],
) -> api::AppMap {
    let worker = *app == example::id(example::WORKER);
    // A box's status is the application's own, as the Applications list shows it.
    let node = |key: &str, link: Option<(api::Status, &str)>| {
        let id = example::id(key);
        api::MapApp {
            status: apps
                .iter()
                .find(|a| a.id == id)
                .map_or(api::Status::Unknown, |a| a.status),
            id,
            cluster: "Fictional cluster".into(),
            category: "application".into(),
            link: link.map(|(status, reason)| api::MapLink {
                status,
                reason: reason.into(),
                both_ways: false,
                stats: vec!["12 rps".into(), "3 ms".into()],
                weight: None,
            }),
            ..Default::default()
        }
    };
    api::AppMap {
        app: api::MapApp {
            id: app.clone(),
            status: record.map_or(api::Status::Unknown, |a| a.status),
            labels: BTreeMap::from([("ns".into(), app.namespace().unwrap_or_default().into())]),
            ..Default::default()
        },
        instances: pods
            .iter()
            .map(|(id, _)| api::MapInstance {
                id: id.clone(),
                labels: BTreeMap::new(),
            })
            .collect(),
        clients: if worker {
            vec![node("payments/api", Some((api::Status::Ok, "")))]
        } else {
            vec![]
        },
        dependencies: if worker {
            vec![
                node(
                    "payments/ledger-db",
                    Some((api::Status::Critical, "connection refused")),
                ),
                node("cache/redis-cache", Some((api::Status::Ok, ""))),
            ]
        } else {
            vec![]
        },
    }
}
