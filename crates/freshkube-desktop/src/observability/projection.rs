//! Both fixture and provider observations enter here. Render reads these values.
use super::*;
use freshkube_core::coroot as api;

impl Report {
    pub(super) fn signal(self) -> &'static str {
        match self {
            Self::Errors => "errors",
            Self::Latency => "latency",
            Self::Upstreams => "upstreams",
            Self::Instances => "instances",
            Self::Restarts => "restarts",
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Disk => "disk_usage",
            Self::DiskIo => "disk_io_load",
            Self::Net => "network",
            Self::Dns => "dns",
            Self::Logs => "logs",
        }
    }
    pub(super) fn server_name(self) -> &'static str {
        match self {
            Self::Errors | Self::Latency => "SLO",
            Self::Upstreams | Self::Net => "Net",
            Self::Restarts | Self::Instances => "Instances",
            Self::Disk | Self::DiskIo => "Storage",
            other => other.label(),
        }
    }
}
impl From<api::Status> for Status {
    fn from(value: api::Status) -> Self {
        match value {
            api::Status::Ok => Self::Ok,
            api::Status::Unknown => Self::Unknown,
            api::Status::Info => Self::Info,
            api::Status::Warning => Self::Warning,
            api::Status::Critical => Self::Critical,
        }
    }
}
impl Status {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Ok => "Healthy",
            Self::Unknown => "Unknown",
            Self::Absent => "Not reported",
            Self::Info => "Info",
            Self::Warning => "Warning",
            Self::Critical => "Critical",
            Self::Integration => "Integration required",
            Self::LogError => "Log errors",
        }
    }
}

pub(super) fn applications(raw: &[api::Application]) -> Vec<Application> {
    let mut values: Vec<_> = raw
        .iter()
        .map(|app| {
            let checks = Report::ALL.map(|report| {
                let signal = app.signals.get(report.signal());
                let state = signal.map_or(Status::Absent, |signal| signal.status.into());
                let value = signal
                    .filter(|s| !s.value.is_empty())
                    .map_or_else(|| state.label().to_string(), |s| s.value.clone());
                Check {
                    status: state,
                    tooltip: format!(
                        "{} · {}: {} · {}",
                        app.id,
                        report.label(),
                        state.label(),
                        value
                    ),
                    value,
                    element_id: format!("obs-check-{}-{}", app.id, report.slug()).into(),
                }
            });
            Application {
                id: app.id.clone(),
                key: app.id.short(),
                namespace: app.id.namespace().unwrap_or("Outside Kubernetes").into(),
                name: app.id.name().into(),
                namespace_prefix: format!(
                    "{}/",
                    app.id.namespace().unwrap_or("Outside Kubernetes")
                ),
                language: app.app_type.clone(),
                category: app.category.clone(),
                status: app.status.into(),
                checks,
                search: format!("{} {} {}", app.id, app.app_type, app.category).to_lowercase(),
                row_id: format!("obs-app-{}", app.id).into(),
                name_id: format!("obs-name-{}", app.id).into(),
            }
        })
        .collect();
    values.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| {
                let rank = |s| match s {
                    Status::Critical => 0,
                    Status::Warning => 1,
                    Status::Unknown => 2,
                    Status::Info => 3,
                    _ => 4,
                };
                rank(a.status).cmp(&rank(b.status))
            })
            .then(a.id.cmp(&b.id))
    });
    values
}

pub(super) fn map(
    raw: &api::ServiceMap,
) -> (std::rc::Rc<Vec<MapNode>>, std::rc::Rc<Vec<Connection>>) {
    // Stable, bounded grid. No iterative force simulation on the UI thread.
    let mut ordered: Vec<_> = raw.nodes.iter().collect();
    ordered.sort_by(|a, b| a.id.cmp(&b.id));
    let rows = ordered.len().div_ceil(4).max(1);
    let nodes: Vec<_> = ordered
        .iter()
        .enumerate()
        .map(|(ix, node)| MapNode {
            app: node.id.clone(),
            label: node.id.name().into(),
            tooltip: format!("{} · Open application", node.id),
            namespace: node.id.namespace().unwrap_or("External / unmapped").into(),
            status: node.status.into(),
            x: (ix % 4) as f32 * 0.25,
            y: (ix / 4) as f32 / rows as f32,
            element_id: format!("obs-map-node-{}", node.id).into(),
        })
        .collect();
    let positions: BTreeMap<_, _> = nodes
        .iter()
        .enumerate()
        .map(|(ix, n)| (&n.app, ix))
        .collect();
    let connections = raw.edges.iter().filter_map(|edge| {
        let (from,to) = (*positions.get(&edge.from)?, *positions.get(&edge.to)?);
        let number = |v: Option<f64>, unit: &str| v.filter(|v| v.is_finite()).map_or_else(
            || "Not reported".into(), |v| format!("{v:.3} {unit}"));
        let label = format!("{} → {}",edge.from.short(),edge.to.short());
        let detail = format!("{}\nRequests: {}\nLatency: {}\nSent: {}\nReceived: {}\nStatistics may be rounded by Coroot.",
            if edge.issue.is_empty() { "No connection issue reported" } else { &edge.issue },
            number(edge.rps,"rps"),number(edge.latency_seconds,"s"),
            number(edge.sent_bytes_per_second,"B/s"),number(edge.received_bytes_per_second,"B/s"));
        Some(Connection {
            id: LinkId(edge.from.clone(),edge.to.clone()), from,to, status: edge.status.into(),
            element_id: format!("obs-map-edge-{}--{}", edge.from,edge.to).into(),
            button_id: format!("obs-map-link-{}--{}", edge.from,edge.to).into(),
            traffic: edge.sent_bytes_per_second.filter(|v| v.is_finite() && *v > 0.).map_or(1., |v| (v.log10() as f32).clamp(1.,4.)),
            tooltip: format!("{label} · {detail}"), label, detail,
        })
    }).collect();
    (std::rc::Rc::new(nodes), std::rc::Rc::new(connections))
}

impl ObservabilityPage {
    pub(super) fn apply_applications(&mut self, raw: &[api::Application]) {
        self.applications = applications(raw);
        self.categories = self
            .applications
            .iter()
            .map(|a| a.category.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .into();
        self.namespaces = self
            .applications
            .iter()
            .filter_map(|a| a.id.namespace().map(str::to_owned))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .into();
        self.cluster_ids = raw
            .iter()
            .filter(|a| !a.id.is_external())
            .map(|a| a.id.cluster_id().to_string())
            .filter(|id| !id.is_empty())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        if self
            .selected_app
            .as_ref()
            .is_some_and(|id| !self.applications.iter().any(|a| &a.id == id))
        {
            self.selected_app = None;
            self.report_snapshot = None;
        }
        self.project();
    }
    pub(super) fn selected_application(&self) -> Option<&Application> {
        let selected = self.selected_app.as_ref()?;
        self.applications.iter().find(|a| &a.id == selected)
    }
}
