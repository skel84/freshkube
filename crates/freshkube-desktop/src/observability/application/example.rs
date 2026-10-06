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
    let health = example::health(app);
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
    let half = |chart| api::Widget {
        kind: api::WidgetKind::Chart(chart),
        width: 0.5,
    };
    match report {
        "SLO" if worker => {
            let [requests, errors] = slo_charts();
            vec![
                half(requests),
                half(errors),
                full(api::WidgetKind::Heatmap(latency_heatmap())),
            ]
        }
        "Instances" if worker => {
            let [up, restarted] = instance_charts(pods);
            vec![
                half(up),
                half(restarted),
                full(api::WidgetKind::Table(instances(pods, restarts))),
            ]
        }
        "Instances" => vec![full(api::WidgetKind::Table(instances(pods, restarts)))],
        "CPU" if worker => {
            let [delay, throttled] = cpu_charts();
            vec![
                api::Widget {
                    kind: cpu_usage(pods),
                    width: 1.,
                },
                half(delay),
                half(throttled),
            ]
        }
        "Net" if worker => {
            let [rtt, failed] = net_charts();
            vec![
                half(rtt),
                half(failed),
                full(api::WidgetKind::Table(dependencies())),
            ]
        }
        "Logs" => vec![full(api::WidgetKind::Logs(logs))],
        "Deployments" => vec![full(api::WidgetKind::Table(deployments()))],
        "Profiling" => vec![full(api::WidgetKind::Profiling)],
        "Tracing" => vec![full(api::WidgetKind::Tracing)],
        _ => vec![],
    }
}

/// Minutes since ledger-db began refusing the worker's connections.
const FAILING_MINUTES: usize = 20;
/// The example charts' points, one a minute.
const POINTS: usize = 60;

/// The example's last hour, ending on the current minute.
fn hour() -> (i64, i64) {
    let to = chrono::Utc::now().timestamp() / 60 * 60_000;
    (to - POINTS as i64 * 60_000, to)
}

fn chart(title: &str, series: Vec<api::Series>) -> api::AppChart {
    let (from_ms, to_ms) = hour();
    api::AppChart {
        title: title.into(),
        from_ms,
        to_ms,
        step_ms: 60_000,
        series,
        ..Default::default()
    }
}

fn series(name: &str, points: impl Fn(usize, bool) -> Option<f32>) -> api::Series {
    api::Series {
        name: name.into(),
        points: (0..POINTS)
            .map(|p| points(p, p >= POINTS - FAILING_MINUTES))
            .collect(),
        ..Default::default()
    }
}

/// A gentle repeating swing around 1, so lines don't sit flat.
fn wave(point: usize, phase: usize) -> f32 {
    1. + 0.15 * ((point + phase) % 7) as f32 / 6. - 0.075
}

/// A rollout of the worker a little before ledger-db began refusing it.
fn rollout() -> api::Annotation {
    let (_, to_ms) = hour();
    let at = to_ms - (FAILING_MINUTES as i64 + 6) * 60_000;
    api::Annotation {
        name: "worker:1.8.2".into(),
        from_ms: at,
        to_ms: at,
        icon: "mdi-swap-horizontal-circle-outline".into(),
    }
}

fn slo_charts() -> [api::AppChart; 2] {
    let fast = series("0-100ms", |p, failing| {
        Some(if failing { 21. } else { 38. } * wave(p, 0))
    });
    let slow = series("100-500ms", |p, failing| {
        Some(if failing { 9. } else { 4. } * wave(p, 3))
    });
    let failed = |name| {
        series(name, |p, failing| {
            Some(if failing { 12. * wave(p, 5) } else { 0. })
        })
    };
    let mut total = series("total", |p, failing| {
        Some(if failing { 42. } else { 42.5 } * wave(p, 1))
    });
    total.fill = true;
    [
        api::AppChart {
            stacked: true,
            threshold: Some(total),
            annotations: vec![rollout()],
            ..chart(
                "Requests to the worker app, per second",
                vec![fast, slow, failed("errors")],
            )
        },
        api::AppChart {
            column: true,
            ..chart("Errors, per second", vec![failed("ledger-db refused")])
        },
    ]
}

fn latency_heatmap() -> api::AppHeatmap {
    let (from_ms, to_ms) = hour();
    let row = |name: &str, title: &str, rate: f32, failing_rate: f32| api::Series {
        name: name.into(),
        title: title.into(),
        points: (0..POINTS)
            .map(|p| {
                let rate = if p >= POINTS - FAILING_MINUTES {
                    failing_rate
                } else {
                    rate
                };
                (rate > 0.).then(|| rate * wave(p, 2))
            })
            .collect(),
        ..Default::default()
    };
    api::AppHeatmap {
        title: "Latency & Errors heatmap, requests per second".into(),
        from_ms,
        to_ms,
        step_ms: 60_000,
        rows: vec![
            row("0.1", "100ms", 38., 21.),
            row("0.5", "500ms", 4., 9.),
            row("1", "1s", 0.4, 0.),
            row("errors", "errors", 0., 12.),
        ],
    }
}

fn instance_charts(pods: &[(String, bool)]) -> [api::AppChart; 2] {
    let up = series("up", |_, failing| Some(if failing { 1. } else { 2. }));
    let desired = series("desired", |_, _| Some(2.));
    let restarts = pods
        .iter()
        .filter(|(_, failing)| *failing)
        .map(|(pod, _)| {
            series(pod, |p, failing| {
                Some(if failing && p % 3 == 0 { 2. } else { 0. })
            })
        })
        .collect();
    [
        api::AppChart {
            threshold: Some(desired),
            ..chart("Instances", vec![up])
        },
        api::AppChart {
            column: true,
            ..chart("Restarts", restarts)
        },
    ]
}

/// CPU usage by container, one chart per instance, the picker opening on
/// the busier one.
fn cpu_usage(pods: &[(String, bool)]) -> api::WidgetKind {
    let charts = pods
        .iter()
        .enumerate()
        .map(|(ix, (pod, failing))| {
            let usage = if *failing { 0.04 } else { 0.21 };
            api::AppChart {
                featured: !failing,
                threshold: Some(series("limit", |_, _| Some(0.5))),
                annotations: vec![rollout()],
                ..chart(
                    pod,
                    vec![series("worker", |p, _| Some(usage * wave(p, ix * 2)))],
                )
            }
        })
        .collect();
    api::WidgetKind::ChartGroup {
        title: "CPU usage of container <selector>, cores".into(),
        charts,
    }
}

fn cpu_charts() -> [api::AppChart; 2] {
    [
        chart(
            "CPU delay, seconds/second",
            vec![series("worker", |p, _| Some(0.002 * wave(p, 1)))],
        ),
        chart(
            "Throttled time, seconds/second",
            vec![series("worker", |_, _| Some(0.))],
        ),
    ]
}

fn net_charts() -> [api::AppChart; 2] {
    // A refused connection measures no round trip: the line stops.
    let ledger = series("ledger-db:5432", |p, failing| {
        (!failing).then(|| 0.0011 * wave(p, 0))
    });
    let api = series("api:8080", |p, _| Some(0.0018 * wave(p, 4)));
    let failed = series("ledger-db:5432", |p, failing| {
        Some(if failing { 6. * wave(p, 2) } else { 0. })
    });
    [
        chart(
            "Network round-trip time to dependencies, seconds",
            vec![ledger, api],
        ),
        api::AppChart {
            column: true,
            ..chart("Failed TCP connections, per second", vec![failed])
        },
    ]
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
