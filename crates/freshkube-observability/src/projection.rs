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
    pub(super) fn tone(self) -> Tone {
        match self {
            Self::Ok => Tone::Good,
            Self::Critical => Tone::Crit,
            Self::Warning | Self::LogError => Tone::Warn,
            _ => Tone::Unknown,
        }
    }
    pub(super) fn report_tone(self) -> Option<Tone> {
        match self {
            Self::Critical => Some(Tone::Crit),
            Self::Warning | Self::LogError => Some(Tone::Warn),
            Self::Unknown => Some(Tone::Unknown),
            _ => None,
        }
    }
    pub(super) fn rank(self) -> u8 {
        match self {
            Self::Critical => 0,
            Self::Warning | Self::LogError => 1,
            Self::Unknown => 2,
            Self::Info => 3,
            _ => 4,
        }
    }
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Ok => "Healthy",
            Self::Unknown => "Unknown",
            Self::Absent => "Not reported",
            Self::Info => "Info",
            Self::Warning => "Warning",
            Self::Critical => "Critical",
            Self::LogError => "Log errors",
        }
    }
}

pub(super) fn applications(raw: &[api::Application]) -> Vec<Application> {
    let mut values: Vec<_> = raw
        .iter()
        .map(|app| {
            let label = format!("{} · {}", view::app_label(&app.id), app.id.kind());
            let checks = Report::ALL.map(|report| {
                let signal = app.signals.get(report.signal());
                let state = signal.map_or(Status::Absent, |signal| signal.status.into());
                let raw_value = signal.map_or("", |s| s.value.as_str());
                let value = compact_signal(report, state, raw_value);
                Check {
                    status: state,
                    tooltip: format!(
                        "{label} · {}: {}{}",
                        report.label(),
                        state.label(),
                        if raw_value.is_empty() {
                            String::new()
                        } else {
                            format!(" · {raw_value}")
                        }
                    )
                    .into(),
                    label: format!(
                        "{}: {}",
                        report.label(),
                        if raw_value.is_empty() {
                            state.label().to_lowercase()
                        } else {
                            format!("{} · {raw_value}", state.label().to_lowercase())
                        }
                    )
                    .into(),
                    value: value.into(),
                    element_id: format!("obs-check-{}-{}", app.id, report.slug()).into(),
                }
            });
            Application {
                id: app.id.clone(),
                label: label.into(),
                key: app.id.short(),
                namespace: app.id.namespace().unwrap_or("Outside Kubernetes").into(),
                name: app.id.name().into(),
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
        a.namespace
            .cmp(&b.namespace)
            .then_with(|| a.status.rank().cmp(&b.status.rank()))
            .then(a.id.cmp(&b.id))
    });
    values
}

/// Coroot's original text remains in the tooltip and accessibility label.
/// Only the table's display value is compacted, when observations arrive.
fn compact_signal(report: Report, state: Status, raw: &str) -> String {
    match state {
        Status::Absent => return "—".into(),
        Status::Ok if raw.is_empty() => return "ok".into(),
        Status::Info if raw.is_empty() => return "info".into(),
        _ => {}
    }
    let raw = raw.trim();
    if report == Report::Upstreams && matches!(state, Status::Critical | Status::Warning) {
        // A numeric upstream value is already a count. Otherwise Coroot's
        // value names the failing upstreams; retain those names in the detail.
        if let Ok(count) = raw.parse::<u64>() {
            return count.to_string();
        }
        if let Some((count, label)) = raw.split_once(char::is_whitespace)
            && matches!(
                label.trim(),
                "upstreams" | "failed upstreams" | "failing upstreams"
            )
            && let Ok(count) = count.parse::<u64>()
        {
            return count.to_string();
        }
        if !raw.is_empty() {
            return raw
                .split([',', ';', '\n'])
                .filter(|name| !name.trim().is_empty())
                .count()
                .to_string();
        }
    }
    // Coroot supplies human-readable values. Remove whitespace around known
    // units and count labels without discarding an unrecognised qualifier.
    if let Some((number, unit)) = raw.split_once(char::is_whitespace)
        && number.parse::<f64>().is_ok_and(f64::is_finite)
    {
        let unit = match unit.trim() {
            "percent" | "%" => Some("%"),
            "ms" | "s" | "µs" | "ns" | "B" | "KB" | "MB" | "GB" | "KiB" | "MiB" | "GiB" | "B/s" => {
                Some(unit.trim())
            }
            "restarts" | "restart" | "errors" | "error" | "logs" | "instances" => Some(""),
            _ => None,
        };
        if let Some(unit) = unit {
            return format!("{number}{unit}");
        }
    }
    if raw.chars().count() > 24 || raw.contains(['\n', '\r']) {
        state.label().into()
    } else {
        raw.into()
    }
}

pub(super) fn map(
    raw: &api::ServiceMap,
) -> (std::rc::Rc<Vec<MapNode>>, std::rc::Rc<Vec<Connection>>) {
    // Sorted by id, so the graph's pages are stable; the graph places them.
    let mut ordered: Vec<_> = raw.nodes.iter().collect();
    ordered.sort_by(|a, b| a.id.cmp(&b.id));
    let nodes: Vec<_> = ordered
        .iter()
        .map(|node| MapNode {
            app: node.id.clone(),
            label: node.id.name().into(),
            tooltip: format!(
                "{} · {} · Open application",
                view::app_label(&node.id),
                node.id.kind()
            ),
            namespace: node.id.namespace().unwrap_or("External / unmapped").into(),
            status: node.status.into(),
            element_id: format!("obs-map-node-{}", node.id).into(),
        })
        .collect();
    // A connection is kept only when both its ends are on the map.
    let known: std::collections::BTreeSet<_> = nodes.iter().map(|n| &n.app).collect();
    let connections = raw.edges.iter().filter_map(|edge| {
        if !known.contains(&edge.from) || !known.contains(&edge.to) {
            return None;
        }
        let label = format!("{} → {}",edge.from.short(),edge.to.short());
        let detail = format!("{}\nRequests: {}\nLatency: {}\nSent: {}\nReceived: {}\nStatistics may be rounded by Coroot.",
            if edge.issue.is_empty() { "No connection issue reported" } else { &edge.issue },
            format::requests(edge.rps),format::seconds(edge.latency_seconds),
            format::bytes_per_second(edge.sent_bytes_per_second),format::bytes_per_second(edge.received_bytes_per_second));
        Some(Connection {
            id: LinkId(edge.from.clone(),edge.to.clone()), status: edge.status.into(),
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
        let first_observation = self.category_defaults_pending && !raw.is_empty();
        self.applications = applications(raw);
        self.prepare_application_columns();
        self.categories = self
            .applications
            .iter()
            .map(|a| a.category.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|name| CategoryChoice {
                id: format!("obs-category-{name}").into(),
                label: format!("Category: {name}").into(),
                name,
            })
            .collect::<Vec<_>>()
            .into();
        if first_observation
            && !self.all_categories
            && !self
                .categories
                .iter()
                .any(|choice| self.active_categories.contains(&choice.name))
        {
            self.all_categories = true;
        }
        if first_observation {
            self.category_defaults_pending = false;
        }
        self.namespaces = self
            .applications
            .iter()
            .filter_map(|a| a.id.namespace().map(str::to_owned))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .into();
        let mut choices: Vec<_> = self
            .applications
            .iter()
            .map(|a| (a.id.clone(), SharedString::from(view::app_label(&a.id))))
            .collect();
        choices.sort_by(|a, b| a.1.cmp(&b.1));
        self.app_choices = choices.into();
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

impl ObservabilityPage {
    /// User filters start at the first row; observations preserve the current position.
    pub(super) fn project_filters(&mut self) {
        self.project();
        self.application_table
            .scroll
            .scroll_to_item_strict(0, ScrollStrategy::Top);
    }

    pub(super) fn choose_categories(&mut self) {
        self.category_defaults_pending = false;
        if self.all_categories {
            self.active_categories = Rc::new(
                self.categories
                    .iter()
                    .map(|choice| choice.name.clone())
                    .collect(),
            );
            self.all_categories = false;
        }
    }

    pub(super) fn project(&mut self) {
        self.matrix.clear();
        self.counts = [0; 7];
        self.shown_apps = 0;
        // The prepared application order is namespace, severity, then identity.
        // Group only rows the current filters show; counts share the same scope.
        let mut offset = 0;
        for apps in self
            .applications
            .chunk_by(|a, b| a.namespace == b.namespace)
        {
            let base = offset;
            offset += apps.len();
            let mut shown = vec![];
            let mut worst = Status::Ok;
            for (index, app) in apps.iter().enumerate() {
                if (!self.all_categories && !self.active_categories.contains(&app.category))
                    || !app.search.contains(&self.query_text)
                    || self
                        .namespace
                        .as_ref()
                        .is_some_and(|ns| *ns != app.namespace)
                {
                    continue;
                }
                for (ix, filter) in Filter::ALL.iter().enumerate() {
                    if filter.matches(app) {
                        self.counts[ix] += 1;
                    }
                }
                if self.filter.matches(app) {
                    if app.status.rank() < worst.rank() {
                        worst = app.status;
                    }
                    shown.push(MatrixRow::App(base + index));
                }
            }
            if !shown.is_empty() {
                self.shown_apps += shown.len();
                let label = apps[0].namespace.clone();
                self.matrix.push(MatrixRow::Group {
                    id: format!("obs-group-{label}").into(),
                    label,
                    status: worst,
                    summary: app_count(shown.len()),
                });
                self.matrix.extend(shown);
            }
        }
        self.app_count = app_count(self.shown_apps);
    }
}

fn app_count(count: usize) -> String {
    format!("{count} {}", if count == 1 { "app" } else { "apps" })
}

#[cfg(test)]
mod compact_tests {
    use super::{Report, Status, compact_signal};

    #[test]
    fn compact_values_keep_units_and_count_failing_upstreams() {
        for (report, raw, expected) in [
            (Report::Errors, "2.1 percent", "2.1%"),
            (Report::Latency, "340 ms", "340ms"),
            (Report::Restarts, "14 restarts", "14"),
            (Report::Instances, "0/1", "0/1"),
            (Report::Logs, "1.9k", "1.9k"),
            (Report::Upstreams, "worker, ledger-db", "2"),
            (Report::Upstreams, "ledger-db", "1"),
            (Report::Upstreams, "14", "14"),
            (Report::Upstreams, "14 failing upstreams", "14"),
        ] {
            assert_eq!(compact_signal(report, Status::Critical, raw), expected);
        }
        assert_eq!(
            compact_signal(Report::Upstreams, Status::Unknown, "required"),
            "required"
        );
        assert_eq!(compact_signal(Report::Errors, Status::Ok, ""), "ok");
        assert_eq!(compact_signal(Report::Errors, Status::Absent, ""), "—");
    }
}
