//! Prepared presentation snapshots for Coroot observations and fixture previews. These contain no
//! credentials, clients or commands capable of changing a cluster.
use freshkube_core::coroot::AppId;
use gpui_kit::assets::IconName;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Applications,
    ServiceMap,
    Application,
    Incidents,
    Deployments,
    Profiling,
    Traces,
}
impl Destination {
    pub const NAVIGATION: [Self; 6] = [
        Self::Applications,
        Self::ServiceMap,
        Self::Incidents,
        Self::Traces,
        Self::Profiling,
        Self::Deployments,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Applications => "Applications",
            Self::ServiceMap => "Service map",
            Self::Application => "Application",
            Self::Incidents => "Incidents",
            Self::Deployments => "Deployments",
            Self::Profiling => "Profiling",
            Self::Traces => "Traces",
        }
    }
    pub fn slug(self) -> &'static str {
        match self {
            Self::Applications => "applications",
            Self::ServiceMap => "service-map",
            Self::Application => "application",
            Self::Incidents => "incidents",
            Self::Deployments => "deployments",
            Self::Profiling => "profiling",
            Self::Traces => "traces",
        }
    }
    pub fn icon(self) -> IconName {
        match self {
            Self::Applications | Self::Application => IconName::LayoutGrid,
            Self::ServiceMap => IconName::Waypoints,
            Self::Incidents => IconName::TriangleAlert,
            Self::Deployments => IconName::ArrowUp,
            Self::Profiling => IconName::Flame,
            Self::Traces => IconName::ListTree,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Report {
    Errors,
    Latency,
    Upstreams,
    Instances,
    Restarts,
    Cpu,
    Memory,
    Disk,
    DiskIo,
    Net,
    Dns,
    Logs,
}
impl Report {
    pub(super) const ALL: [Self; 12] = [
        Self::Errors,
        Self::Latency,
        Self::Upstreams,
        Self::Instances,
        Self::Restarts,
        Self::Cpu,
        Self::Memory,
        Self::Disk,
        Self::DiskIo,
        Self::Net,
        Self::Dns,
        Self::Logs,
    ];
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Errors => "Errors",
            Self::Latency => "Latency",
            Self::Upstreams => "Upstreams",
            Self::Instances => "Instances",
            Self::Restarts => "Restarts",
            Self::Cpu => "CPU",
            Self::Memory => "Memory",
            Self::Disk => "Disk",
            Self::DiskIo => "Disk I/O",
            Self::Net => "Net",
            Self::Dns => "DNS",
            Self::Logs => "Logs",
        }
    }
    pub(super) fn slug(self) -> &'static str {
        match self {
            Self::Errors => "errors",
            Self::Latency => "latency",
            Self::Upstreams => "upstreams",
            Self::Instances => "instances",
            Self::Restarts => "restarts",
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Disk => "disk",
            Self::DiskIo => "disk-io",
            Self::Net => "net",
            Self::Dns => "dns",
            Self::Logs => "logs",
        }
    }
    pub(super) fn default_threshold(self) -> &'static str {
        match self {
            Self::Cpu | Self::Memory | Self::Disk => "85",
            Self::Latency => "500",
            _ => "0",
        }
    }
    pub(super) fn index(self) -> usize {
        self as usize
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    Ok,
    Warning,
    Critical,
    Unknown,
    LogError,
    Info,
    Absent,
}
#[derive(Clone)]
pub(super) struct Check {
    pub status: Status,
    pub value: gpui_kit::SharedString,
    pub label: gpui_kit::SharedString,
    pub tooltip: gpui_kit::SharedString,
    pub element_id: gpui_kit::SharedString,
}
#[derive(Clone)]
pub(super) struct Application {
    pub id: AppId,
    pub label: gpui_kit::SharedString,
    pub key: String,
    pub namespace: String,
    pub name: String,
    pub language: String,
    pub category: String,
    pub status: Status,
    pub checks: [Check; 12],
    pub search: String,
    pub row_id: gpui_kit::SharedString,
    pub name_id: gpui_kit::SharedString,
}

#[derive(Clone)]
pub(super) struct CategoryChoice {
    pub name: String,
    pub id: gpui_kit::SharedString,
    pub label: gpui_kit::SharedString,
}
impl Application {
    pub fn check(&self, report: Report) -> &Check {
        &self.checks[report.index()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Filter {
    Problems,
    All,
    Critical,
    Warning,
    Logs,
    Unknown,
    Ok,
}
impl Filter {
    pub(super) const ALL: [Self; 7] = [
        Self::Problems,
        Self::All,
        Self::Critical,
        Self::Warning,
        Self::Logs,
        Self::Unknown,
        Self::Ok,
    ];
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Problems => "Problems",
            Self::All => "All",
            Self::Critical => "Critical",
            Self::Warning => "Warning",
            Self::Logs => "Errors in logs",
            Self::Unknown => "Unknown",
            Self::Ok => "OK",
        }
    }
    pub(super) fn slug(self) -> &'static str {
        match self {
            Self::Problems => "problems",
            Self::All => "all",
            Self::Critical => "critical",
            Self::Warning => "warning",
            Self::Logs => "logs",
            Self::Unknown => "unknown",
            Self::Ok => "ok",
        }
    }
    pub(super) fn matches(self, app: &Application) -> bool {
        match self {
            Self::All => true,
            Self::Problems => matches!(
                app.status,
                Status::Critical | Status::Warning | Status::Unknown
            ),
            Self::Critical => app.status == Status::Critical,
            Self::Warning => app.status == Status::Warning,
            Self::Logs => matches!(
                app.check(Report::Logs).status,
                Status::Warning | Status::Critical | Status::LogError
            ),
            Self::Unknown => app.status == Status::Unknown,
            Self::Ok => app.status == Status::Ok,
        }
    }
}
#[derive(Clone)]
pub(super) enum MatrixRow {
    Group {
        id: gpui_kit::SharedString,
        label: String,
        status: Status,
        summary: String,
    },
    App(usize),
}
#[derive(Clone)]
pub(super) struct MapNode {
    pub app: AppId,
    pub label: String,
    pub tooltip: String,
    pub namespace: String,
    pub element_id: gpui_kit::SharedString,
    pub status: Status,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LinkId(pub AppId, pub AppId);
#[derive(Clone)]
pub(super) struct Connection {
    pub id: LinkId,
    pub element_id: gpui_kit::SharedString,
    pub button_id: gpui_kit::SharedString,
    pub status: Status,
    pub traffic: f32,
    pub label: String,
    pub detail: String,
    pub tooltip: String,
}
#[derive(Clone)]
pub(super) struct FlameFrame {
    pub name: &'static str,
    pub x: f32,
    pub width: f32,
    pub depth: usize,
    pub delta: i8,
    pub cpu: &'static str,
    pub parent: Option<usize>,
}
#[derive(Clone)]
pub(super) struct Series {
    pub label: &'static str,
    pub values: Vec<f32>,
}
#[derive(Clone)]
pub(super) struct Chart {
    pub title: &'static str,
    pub unit: &'static str,
    pub series: Vec<Series>,
    pub maximum: f32,
    pub y_ticks: [String; 4],
    pub ticks: [String; 4],
    pub deploy: f32,
    pub event: f32,
}

#[cfg(test)]
mod tests {
    use super::Report;

    #[test]
    fn a_report_index_is_its_place_in_all() {
        for (place, report) in Report::ALL.into_iter().enumerate() {
            assert_eq!(report.index(), place);
        }
    }
}
