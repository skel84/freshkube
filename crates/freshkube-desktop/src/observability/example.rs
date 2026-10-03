//! One bounded, deterministic fictional cluster shared by all seven screens.
use super::model::*;

pub(super) const CATEGORIES: [&str; 3] = ["Applications", "Control plane", "Monitoring"];
pub(super) const WORKER: &str = "payments/worker";
pub(super) const POD: &str = "worker-6c4f8da0-bbbbh";

pub(super) fn threshold(app: &str, report: Report) -> &'static str {
    match (app, report) {
        ("platform/keycloak", Report::Memory) => "70",
        ("kube-system/etcd" | "monitoring/prometheus", Report::Disk) => "80",
        ("kube-system/kube-apiserver", Report::Latency) => "250",
        _ => report.default_threshold(),
    }
}

pub(super) fn applications() -> Vec<Application> {
    let records = [
        ("payments/api", "Go", 0, Status::Critical),
        (WORKER, "Go", 0, Status::Critical),
        ("payments/ledger", "Java", 0, Status::Warning),
        ("payments/ledger-db", "Postgres", 0, Status::Warning),
        ("platform/keycloak", "Java", 0, Status::Warning),
        ("platform/oauth2-proxy", "Go", 0, Status::Warning),
        ("cache/redis-cache", "Redis", 0, Status::Warning),
        ("kube-system/kube-apiserver", "Go", 1, Status::Warning),
        ("kube-system/etcd", "Talos", 1, Status::Warning),
        ("kube-system/coredns", "Go", 1, Status::Warning),
        ("kube-system/cilium", "Go", 1, Status::Warning),
        (
            "argocd/argocd-application-controller",
            "Go",
            1,
            Status::Warning,
        ),
        ("ingress/ingress-nginx", "Nginx", 1, Status::Warning),
        ("monitoring/prometheus", "Go", 2, Status::Warning),
        ("monitoring/alertmanager", "Go", 2, Status::Warning),
        ("logging/loki", "Go", 2, Status::Warning),
    ];
    let mut apps: Vec<_> = records
        .into_iter()
        .map(|(key, language, category, status)| {
            let (namespace, name) = key.split_once('/').unwrap();
            let defaults = [
                "0%", "12ms", "ok", "1/1", "0", "9%", "27%", "—", "0.3ms", "0.5ms", "0",
            ];
            Application {
                key: key.into(),
                namespace: namespace.into(),
                name: name.into(),
                language,
                category,
                status,
                checks: defaults.map(|value| Check {
                    status: if value == "—" {
                        Status::Unknown
                    } else {
                        Status::Ok
                    },
                    value: value.into(),
                }),
                search: format!("{key} {language}").to_lowercase(),
            }
        })
        .collect();
    let changes = [
        (0, Report::Errors, Status::Critical, "2.8%"),
        (0, Report::Upstreams, Status::Critical, "worker"),
        (0, Report::Logs, Status::LogError, "1.9k"),
        (1, Report::Upstreams, Status::Critical, "ledger-db"),
        (1, Report::Instances, Status::Critical, "0/1"),
        (1, Report::Restarts, Status::Warning, "14"),
        (1, Report::Net, Status::Critical, "refused"),
        (1, Report::Logs, Status::LogError, "212"),
        (2, Report::Instances, Status::Warning, "1/2"),
        (3, Report::Instances, Status::Warning, "1/2"),
        (3, Report::Disk, Status::Integration, "required"),
        (4, Report::Instances, Status::Warning, "1/2"),
        (4, Report::Memory, Status::Warning, "76%"),
        (5, Report::Instances, Status::Warning, "1/2"),
        (6, Report::Instances, Status::Warning, "2/3"),
        (7, Report::Latency, Status::Warning, "294ms"),
        (8, Report::Disk, Status::Warning, "83%"),
        (9, Report::Instances, Status::Warning, "1/2"),
        (10, Report::Instances, Status::Warning, "5/6"),
        (11, Report::Cpu, Status::Warning, "86%"),
        (11, Report::Logs, Status::LogError, "41"),
        (12, Report::Upstreams, Status::Critical, "api"),
        (13, Report::Memory, Status::Warning, "88%"),
        (13, Report::Disk, Status::Warning, "83%"),
        (14, Report::Instances, Status::Warning, "1/2"),
        (15, Report::Instances, Status::Warning, "1/2"),
    ];
    for (app, report, status, value) in changes {
        apps[app].checks[report.index()] = Check {
            status,
            value: value.into(),
        };
    }
    apps[1].checks[Report::Cpu.index()].value = "0%".into();
    apps[1].checks[Report::Errors.index()] = Check {
        status: Status::Unknown,
        value: "—".into(),
    };
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
        let key = format!("{namespace}/{name}");
        apps.push(Application {
            namespace: namespace.into(),
            name: name.into(),
            search: key.clone(),
            key,
            language: "Go",
            category,
            status: Status::Ok,
            checks: [
                "0%", "8ms", "ok", "2/2", "0", "12%", "24%", "—", "0.2ms", "0.4ms", "0",
            ]
            .map(|value| Check {
                status: if value == "—" {
                    Status::Unknown
                } else {
                    Status::Ok
                },
                value: value.into(),
            }),
        });
    }
    apps
}

pub(super) fn map() -> (Vec<MapNode>, Vec<Connection>) {
    use Status::*;
    let nodes = vec![
        MapNode {
            app: "ingress/ingress-nginx",
            label: "ingress-nginx",
            namespace: "ingress · from internet",
            x: 0.02,
            y: 0.40,
            status: Warning,
        },
        MapNode {
            app: "platform/oauth2-proxy",
            label: "oauth2-proxy",
            namespace: "platform",
            x: 0.27,
            y: 0.12,
            status: Warning,
        },
        MapNode {
            app: "payments/api",
            label: "api",
            namespace: "payments",
            x: 0.27,
            y: 0.40,
            status: Critical,
        },
        MapNode {
            app: "payments/harbor-core",
            label: "harbor-core",
            namespace: "payments",
            x: 0.27,
            y: 0.76,
            status: Ok,
        },
        MapNode {
            app: "platform/keycloak",
            label: "keycloak",
            namespace: "platform",
            x: 0.52,
            y: 0.12,
            status: Warning,
        },
        MapNode {
            app: WORKER,
            label: "worker",
            namespace: "payments",
            x: 0.52,
            y: 0.40,
            status: Critical,
        },
        MapNode {
            app: "payments/ledger",
            label: "ledger",
            namespace: "payments",
            x: 0.52,
            y: 0.76,
            status: Warning,
        },
        MapNode {
            app: "payments/ledger-db",
            label: "ledger-db",
            namespace: "payments · :6432",
            x: 0.77,
            y: 0.40,
            status: Warning,
        },
        MapNode {
            app: "cache/redis-cache",
            label: "redis-cache",
            namespace: "cache",
            x: 0.77,
            y: 0.76,
            status: Warning,
        },
    ];
    let links = vec![
        Connection {
            from: 0,
            to: 1,
            status: Ok,
            traffic: 2.,
            label: "ingress → auth",
            detail: "42 requests/s · 0.2% errors · RTT 0.4ms",
        },
        Connection {
            from: 0,
            to: 2,
            status: Critical,
            traffic: 3.,
            label: "ingress → api",
            detail: "120 requests/s · 2.8% errors · 4,212 requests impacted",
        },
        Connection {
            from: 0,
            to: 3,
            status: Ok,
            traffic: 1.,
            label: "ingress → registry",
            detail: "8 requests/s · 0% errors · RTT 0.2ms",
        },
        Connection {
            from: 1,
            to: 4,
            status: Warning,
            traffic: 2.,
            label: "auth → keycloak",
            detail: "1 of 2 instances available · RTT 120ms",
        },
        Connection {
            from: 2,
            to: 5,
            status: Critical,
            traffic: 3.,
            label: "api → worker",
            detail: "POST /v1/settlements returns 503 · worker has no ready instances",
        },
        Connection {
            from: 5,
            to: 7,
            status: Critical,
            traffic: 2.,
            label: "worker → ledger-db",
            detail: "0 established · 14 failed / 5 min · connection refused :5432",
        },
        Connection {
            from: 5,
            to: 8,
            status: Ok,
            traffic: 1.,
            label: "worker → redis",
            detail: "18 connections/s before restart · RTT 0.2ms",
        },
        Connection {
            from: 6,
            to: 7,
            status: Warning,
            traffic: 2.,
            label: "ledger → database",
            detail: "Primary is serving on :6432 · replica's node is NotReady",
        },
    ];
    (nodes, links)
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
