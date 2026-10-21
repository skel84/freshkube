//! The verdict an Inspector opens with: one state, why it is so, and what
//! to look at next, computed from the evidence by one pure function.
use super::coroot::{CorootState, DatabaseReport};
use super::state::severity;
use super::*;
use crate::HealthIndicator;
use crate::coroot::Status;
use chrono::{DateTime, Duration, Utc};

/// A requested primary that differs from the current one for this long is stuck.
pub const SWITCHOVER_STUCK_AFTER: Duration = Duration::minutes(10);

/// One reason for the state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Warning or Error count against the state; Info is only said.
    pub health: HealthIndicator,
    pub source: Source,
    pub why: String,
    pub next: Option<String>,
}
impl Finding {
    fn new(health: HealthIndicator, source: Source, why: impl Into<String>) -> Self {
        Self {
            health,
            source,
            why: why.into(),
            next: None,
        }
    }
    fn next(mut self, next: impl Into<String>) -> Self {
        self.next = Some(next.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub state: RowState,
    /// The one sentence the Inspector opens with.
    pub headline: String,
    /// Worst first.
    pub findings: Vec<Finding>,
}
impl Verdict {
    /// Warnings and errors only: what makes a row need a look.
    pub fn problems(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|finding| severity(finding.health) >= severity(HealthIndicator::Warning))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switchover {
    /// The engine has no switchover this reads.
    NotApplicable,
    Settled,
    InProgress(Duration),
    Stuck(Duration),
    /// A primary or the time it changed isn't reported.
    Unknown,
}
/// The MariaDB operator's switchover: the requested primary differs from the
/// current one. Never stuck without a time to compare.
pub fn switchover(db: &Database, now: DateTime<Utc>) -> Switchover {
    if db.engine != Engine::MariaDb {
        return Switchover::NotApplicable;
    }
    match (db.current_primary.known(), db.target_primary.known()) {
        (Some(current), Some(requested)) if current == requested => Switchover::Settled,
        (Some(_), Some(_)) => match db.primary_changed {
            Some(since) if since <= now => {
                let age = now - since;
                if age > SWITCHOVER_STUCK_AFTER {
                    Switchover::Stuck(age)
                } else {
                    Switchover::InProgress(age)
                }
            }
            _ => Switchover::Unknown,
        },
        _ => Switchover::Unknown,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CertificateState {
    Valid,
    /// Expires in less than the warning period.
    Expiring(Duration),
    Expired,
    /// The expiry isn't reported.
    Unknown,
}
pub fn certificate(cert: &Certificate, now: DateTime<Utc>) -> CertificateState {
    match cert.expires {
        None => CertificateState::Unknown,
        Some(at) if at <= now => CertificateState::Expired,
        Some(at) if at - now < Duration::days(CERTIFICATE_WARNING_DAYS) => {
            CertificateState::Expiring(at - now)
        }
        Some(_) => CertificateState::Valid,
    }
}

/// Lag by instance: Coroot's measurement first; Prometheus is not read, so
/// nothing else fills it.
pub fn lag_of(instance: &Instance, coroot: Option<&DatabaseReport>) -> Fact<Lag> {
    if let Some(lag) = coroot.and_then(|report| report.lag.get(&instance.name)) {
        return Fact::Known(*lag);
    }
    instance.lag
}

fn days(duration: Duration) -> String {
    let days = duration.num_days();
    if days >= 1 {
        format!("{days} day{}", if days == 1 { "" } else { "s" })
    } else {
        format!("{} hours", duration.num_hours().max(1))
    }
}

pub fn verdict(
    db: &Database,
    inventory: &Inventory,
    coroot: &CorootState,
    now: DateTime<Utc>,
) -> Verdict {
    use HealthIndicator::{Error, Info, Warning};
    let report = coroot.report();
    let found = coverage(db, inventory, inventory.backups.observed_at);
    let mut findings = Vec::new();
    let operator = Source::Operator;

    let ready = db
        .conditions
        .iter()
        .find(|condition| condition.kind == "Ready" && condition.current);
    if let Some(ready) = ready.filter(|condition| condition.status == "False") {
        findings.push(
            Finding::new(Warning, operator, format!("Not ready: {}", ready.reason))
                .next("Open the Conditions below, then the pods in Resources"),
        );
    }
    if let (Some(ready), Some(desired)) = (db.ready_instances.known(), db.desired_instances.known())
        && ready < desired
    {
        findings.push(
            Finding::new(
                Warning,
                operator,
                format!("{ready} of {desired} instances ready"),
            )
            .next("Open the instances' pods in Resources"),
        );
    }
    match switchover(db, now) {
        Switchover::Stuck(age) => findings.push(
            Finding::new(
                Error,
                operator,
                format!(
                    "Switchover stuck for {} min: {} requested, {} still primary",
                    age.num_minutes(),
                    db.target_primary.known().map_or("?", String::as_str),
                    db.current_primary.known().map_or("?", String::as_str),
                ),
            )
            .next("Read the primary's conditions and the operator's events in Resources"),
        ),
        Switchover::InProgress(age) => findings.push(Finding::new(
            Info,
            operator,
            format!("Switchover in progress for {} min", age.num_minutes()),
        )),
        _ => {}
    }
    match found.freshness {
        Freshness::Old => findings.push(
            Finding::new(Warning, operator, "Last good backup is older than 24 hours")
                .next("Check the newest backup attempt and the schedule"),
        ),
        Freshness::None => findings.push(
            Finding::new(Warning, operator, "No completed backup")
                .next("Check the schedule and the backup configuration"),
        ),
        _ => {}
    }
    if let Some(newest) = found.newest
        && newest.state == BackupState::Failed
        && found
            .completed
            .is_none_or(|done| done.completed < newest.started.or(newest.created))
    {
        findings.push(
            Finding::new(
                Warning,
                operator,
                format!(
                    "The newest backup attempt, {}, failed",
                    newest.identity.name
                ),
            )
            .next("Open the backup in Resources for its status"),
        );
    }
    if found.no_schedule {
        findings.push(Finding::new(Info, operator, "No backup schedule"));
    }
    let archiving = archiving(db, report);
    match archiving {
        Archiving::Failing => findings.push(
            Finding::new(Warning, operator, "WAL archiving is failing")
                .next("Restores reach only the last completed backup until it works"),
        ),
        Archiving::Disagree => findings.push(
            Finding::new(Warning, operator, Archiving::Disagree.label())
                .next("The restore window stops at the last completed backup"),
        ),
        _ => {}
    }
    for cert in db.certificates.known().into_iter().flatten() {
        match certificate(cert, now) {
            CertificateState::Expired => findings.push(
                Finding::new(
                    Error,
                    operator,
                    format!("Certificate {} has expired", cert.name),
                )
                .next("Renew it; the operator rotates certificates it manages"),
            ),
            CertificateState::Expiring(left) => findings.push(
                Finding::new(
                    Warning,
                    operator,
                    format!("Certificate {} expires in {}", cert.name, days(left)),
                )
                .next("Check that its issuer is renewing it"),
            ),
            _ => {}
        }
    }
    if let Some(Some(proxy)) = db.proxy.known() {
        let name = proxy.kind.label();
        if proxy.running < proxy.desired {
            findings.push(Finding::new(
                if proxy.running == 0 { Error } else { Warning },
                operator,
                format!(
                    "{name}: {} of {} pods running",
                    proxy.running, proxy.desired
                ),
            ));
        } else if proxy.ready == Fact::Known(false) {
            findings.push(Finding::new(
                Warning,
                operator,
                format!("{name} is not ready"),
            ));
        }
    }
    for instance in &db.instances {
        if let Fact::Known(lag) = lag_of(instance, report)
            && lag.seconds > LAG_WARNING_SECONDS
        {
            findings.push(Finding::new(
                Warning,
                Source::Coroot,
                format!(
                    "{} lags {:.0} s behind ({})",
                    instance.name,
                    lag.seconds,
                    lag.source.label()
                ),
            ));
        }
    }
    if let Some(report) = report {
        for fired in &report.fired {
            let health = if fired.status == Status::Critical {
                Error
            } else {
                Warning
            };
            let why = if fired.message.is_empty() {
                fired.title.clone()
            } else {
                format!("{}: {}", fired.title, fired.message)
            };
            findings.push(Finding::new(health, Source::Coroot, why));
        }
    }
    findings.sort_by_key(|finding| std::cmp::Reverse(severity(finding.health)));

    let worst = |source: Source| {
        findings
            .iter()
            .filter(|finding| finding.source == source)
            .map(|finding| finding.health)
            .max_by_key(|health| severity(*health))
    };
    let base = crate::HasHealth::health(db);
    let operator_health = match worst(Source::Operator) {
        Some(found) if severity(found) > severity(base) && severity(found) >= 3 => found,
        _ => base,
    };
    let coroot_health = match (coroot.health(), worst(Source::Coroot)) {
        (HealthIndicator::Unknown, _) => HealthIndicator::Unknown,
        (health, Some(found)) if severity(found) > severity(health) => found,
        (health, _) => health,
    };
    let state = worse_of(operator_health, coroot_health);
    let headline = match findings.first().filter(|f| severity(f.health) >= 3) {
        Some(finding) => finding.why.clone(),
        None => match state.health {
            HealthIndicator::Healthy => "Healthy".into(),
            _ => "State unknown: the operator reports no readiness".into(),
        },
    };
    Verdict {
        state,
        headline,
        findings,
    }
}

/// A backup whose database is gone is a warning that names the database;
/// only a complete Cluster read can say so.
pub fn orphan(reference: Option<&ClusterRef>, inventory: &Inventory) -> Option<Finding> {
    let reference = reference?;
    (link(Some(reference), &inventory.databases) == LinkState::Orphaned).then(|| {
        Finding::new(
            HealthIndicator::Warning,
            Source::Operator,
            format!(
                "Orphaned: its database {} is gone or was replaced",
                reference.name
            ),
        )
        .next(
            "Check that the database was removed on purpose; nothing running is protected by this",
        )
    })
}
