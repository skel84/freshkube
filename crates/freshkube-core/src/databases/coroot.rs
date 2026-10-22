//! What Coroot says of a database, projected from its application page.
//! Read through core's own decoding (`crate::coroot`), with the project and
//! cluster a workspace entry's link names; never a Coroot the setup file
//! does not name. A check whose data is missing is Unknown, never a failure.
use super::{Engine, Lag, LagSource};
use crate::HealthIndicator;
use crate::coroot::{
    AppId, AppReport, AppView, Check, Link, Provider, ReadError, Status, TimeRange, WidgetKind,
};
use chrono::{DateTime, Duration, Utc};
use std::collections::BTreeMap;

/// Application pages read at once; databases past it are not read.
pub const MAX_VIEWS: usize = 10;
/// The longest a whole read may take: the list, then the pages.
const DEADLINE: std::time::Duration = std::time::Duration::from_secs(45);
/// How far back the list and the pages are read.
const WINDOW: i64 = 3600;
/// Coroot's check of continuous WAL archiving, by its id (`model/check.go`).
const ARCHIVING_CHECK: &str = "PostgresWalArchiving";

/// The application id Coroot gives a database of `engine`.
pub fn app_id(cluster: &str, engine: Engine, namespace: &str, name: &str) -> AppId {
    let kind = match engine {
        Engine::CloudNativePg => "DatabaseCluster",
        Engine::MariaDb => "StatefulSet",
    };
    AppId::new(format!("{cluster}:{namespace}:{kind}:{name}"))
}

/// The report of Coroot's page that judges the database.
pub fn report_name(engine: Engine) -> &'static str {
    match engine {
        Engine::CloudNativePg => "Postgres",
        Engine::MariaDb => "Mysql",
    }
}

/// A check that fired, in Coroot's words.
#[derive(Clone, Debug, PartialEq)]
pub struct Fired {
    pub title: String,
    pub message: String,
    pub status: Status,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DatabaseReport {
    pub app: AppId,
    pub status: Status,
    /// Checks that fired, worst first.
    pub fired: Vec<Fired>,
    /// Titles of the checks that passed.
    pub ok: Vec<String>,
    /// Titles of the checks with no data to judge by.
    pub unused: Vec<String>,
    /// The SLO report's checks.
    pub slo: Vec<Fired>,
    /// Coroot's WAL archiving check, when it has one.
    pub archiving: Option<Status>,
    /// Replication lag per instance, from the last sample of Coroot's chart.
    pub lag: BTreeMap<String, Lag>,
    pub read_at: DateTime<Utc>,
}

impl DatabaseReport {
    pub fn health(&self) -> HealthIndicator {
        let worst = self.fired.iter().map(|f| f.status).max();
        match worst {
            Some(Status::Critical) => HealthIndicator::Error,
            Some(Status::Warning) => HealthIndicator::Warning,
            _ if !self.ok.is_empty() || !self.fired.is_empty() => HealthIndicator::Healthy,
            _ => HealthIndicator::Unknown,
        }
    }
}

/// Why no Coroot answer was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Absence {
    /// Observability holds no Coroot connection.
    NotConnected,
    /// The workspace entry names no Coroot cluster.
    NotLinked,
    /// Neither the entry nor the connection names a project.
    NoProject,
}
impl Absence {
    pub fn label(self) -> &'static str {
        match self {
            Self::NotConnected => "Coroot isn't connected",
            Self::NotLinked => "This cluster names no Coroot cluster",
            Self::NoProject => "No Coroot project is named for this cluster",
        }
    }
}

/// The three states where the operator's data still stands, and the answer.
#[derive(Clone, Debug, PartialEq)]
pub enum CorootState {
    /// Nothing was asked of Coroot.
    NoCoroot(Absence),
    /// Coroot lists no such application.
    AppNotFound,
    /// Coroot lists the application, but its page holds no report for the
    /// engine (Coroot did not recognise it as that database).
    NoReport,
    /// Coroot could not be read. The last answer is dimmed and not used.
    Unreachable {
        why: ReadError,
        last: Option<Box<DatabaseReport>>,
    },
    /// Past the cap, or not asked for yet.
    NotRead,
    Read(Box<DatabaseReport>),
}

impl CorootState {
    /// What Coroot adds to the row's state: Unknown unless it answered.
    pub fn health(&self) -> HealthIndicator {
        match self {
            Self::Read(report) => report.health(),
            _ => HealthIndicator::Unknown,
        }
    }
    /// The answer, only when it is current.
    pub fn report(&self) -> Option<&DatabaseReport> {
        match self {
            Self::Read(report) => Some(report),
            _ => None,
        }
    }
}

/// The answers of one read, by the database's UID.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Evidence {
    pub states: BTreeMap<String, CorootState>,
}
impl Evidence {
    pub fn state(&self, uid: &str) -> Option<&CorootState> {
        self.states.get(uid)
    }
    /// Keeps the last good answer beside an unreachable one.
    pub fn retain(&mut self, previous: &Self) {
        for (uid, state) in &mut self.states {
            if let CorootState::Unreachable { last, .. } = state
                && let Some(CorootState::Read(report)) = previous.states.get(uid)
            {
                *last = Some(report.clone());
            }
        }
    }
    /// Nothing asked, for each of `uids`.
    pub fn absent(uids: impl IntoIterator<Item = String>, why: Absence) -> Self {
        Self {
            states: uids
                .into_iter()
                .map(|uid| (uid, CorootState::NoCoroot(why)))
                .collect(),
        }
    }
}

/// Splits a report's checks into the three groups the Inspector shows.
fn judge(report: &AppReport) -> (Vec<Fired>, Vec<String>, Vec<String>) {
    let (mut fired, mut ok, mut unused) = (Vec::new(), Vec::new(), Vec::new());
    for check in &report.checks {
        match check.status {
            Status::Warning | Status::Critical => fired.push(fire(check)),
            Status::Ok | Status::Info => ok.push(check.title.clone()),
            Status::Unknown => unused.push(check.title.clone()),
        }
    }
    fired.sort_by_key(|f| std::cmp::Reverse(f.status));
    (fired, ok, unused)
}
fn fire(check: &Check) -> Fired {
    Fired {
        title: check.title.clone(),
        message: check.message.clone(),
        status: check.status,
    }
}

/// The newest sample of each series of the report's replication lag chart,
/// in seconds, when the sample is within two steps of the chart's end.
fn lag(report: &AppReport) -> BTreeMap<String, Lag> {
    let mut out = BTreeMap::new();
    for widget in &report.widgets {
        let charts: &[crate::coroot::AppChart] = match &widget.kind {
            WidgetKind::Chart(chart) => std::slice::from_ref(chart),
            WidgetKind::ChartGroup { charts, .. } => charts,
            _ => continue,
        };
        for chart in charts {
            if !chart.title.to_lowercase().contains("replication lag") {
                continue;
            }
            let Some(history) = chart.history.as_deref() else {
                continue;
            };
            for series in &history.series {
                let newest = series
                    .samples
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(ix, sample)| sample.map(|value| (ix, value)));
                let Some((ix, seconds)) = newest else {
                    continue;
                };
                let stale = series.samples.len().saturating_sub(ix + 1) > 2;
                if !stale && seconds.is_finite() && seconds >= 0. {
                    out.insert(
                        series.name.clone(),
                        Lag {
                            seconds,
                            source: LagSource::Coroot,
                        },
                    );
                }
            }
        }
    }
    out
}

/// Projects Coroot's page for one database. None when the page holds no
/// report for the engine, which Coroot gives an application it does not
/// recognise as one.
pub fn project(view: &AppView, engine: Engine, read_at: DateTime<Utc>) -> Option<DatabaseReport> {
    let report = view
        .reports
        .iter()
        .find(|report| report.name == report_name(engine))?;
    let (fired, ok, unused) = judge(report);
    let slo = view
        .reports
        .iter()
        .find(|report| report.name == "SLO")
        .map(|slo| {
            slo.checks
                .iter()
                .filter(|check| matches!(check.status, Status::Warning | Status::Critical))
                .map(fire)
                .collect()
        })
        .unwrap_or_default();
    let archiving = report
        .checks
        .iter()
        .find(|check| check.id == ARCHIVING_CHECK)
        .map(|check| check.status);
    Some(DatabaseReport {
        app: view.map.app.id.clone(),
        status: view.map.app.status,
        fired,
        ok,
        unused,
        slo,
        archiving,
        lag: lag(report),
        read_at,
    })
}

/// A database whose page is wanted.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Wanted {
    pub uid: String,
    pub engine: Engine,
    pub namespace: String,
    pub name: String,
}

/// Reads the applications list once and then each wanted page, at most
/// [`MAX_VIEWS`] of them, over `link`'s project and cluster. A page that
/// fails is Unreachable for its database; the others still answer.
pub async fn read(
    provider: &Provider,
    link: &Link,
    wanted: Vec<Wanted>,
    now: DateTime<Utc>,
) -> Evidence {
    let work = async {
        let source = provider.link_source(link);
        let range = TimeRange::between(now - Duration::seconds(WINDOW), now);
        let apps = match provider.applications(&source, range).await {
            Ok(apps) => apps,
            Err(why) => return failed(&wanted, why),
        };
        let mut states = BTreeMap::new();
        let mut asks = Vec::new();
        for (ix, want) in wanted.iter().enumerate() {
            let app = app_id(link.cluster(), want.engine, &want.namespace, &want.name);
            if !apps.iter().any(|listed| listed.id == app) {
                states.insert(want.uid.clone(), CorootState::AppNotFound);
            } else if asks.len() >= MAX_VIEWS {
                states.insert(want.uid.clone(), CorootState::NotRead);
            } else {
                asks.push((ix, app));
            }
        }
        let pages = futures::future::join_all(asks.iter().map(|(_, app)| {
            let source = source.clone();
            async move { provider.app_view(&source, range, app).await }
        }))
        .await;
        for ((ix, _), page) in asks.into_iter().zip(pages) {
            let want = &wanted[ix];
            let state = match page {
                Ok(view) => match project(&view, want.engine, now) {
                    Some(report) => CorootState::Read(Box::new(report)),
                    None => CorootState::NoReport,
                },
                Err(ReadError::Missing) => CorootState::AppNotFound,
                Err(why) => CorootState::Unreachable { why, last: None },
            };
            states.insert(want.uid.clone(), state);
        }
        Evidence { states }
    };
    tokio::time::timeout(DEADLINE, work)
        .await
        .unwrap_or_else(|_| failed(&wanted, ReadError::Timeout))
}

fn failed(wanted: &[Wanted], why: ReadError) -> Evidence {
    Evidence {
        states: wanted
            .iter()
            .map(|want| {
                let state = match why {
                    ReadError::Missing => CorootState::AppNotFound,
                    why => CorootState::Unreachable { why, last: None },
                };
                (want.uid.clone(), state)
            })
            .collect(),
    }
}
