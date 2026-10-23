//! Invented source reads for `--fixture`: acme's `checkout`, with a deploy
//! at -2 h 10 min, errors from ten minutes after it, and the cluster's Events
//! kept for the last hour. Times count back from `now`.
use super::{EVENT_RETENTION, Item, Severity, Source, SourceRead};

/// `FRESHKUBE_TIMELINE`: what the example sources could and couldn't read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    /// Every source read, with something in most hours.
    #[default]
    Busy,
    /// Every source read, with nothing in the last hours.
    Quiet,
    /// Coroot refused, Argo CD not installed.
    Partial,
    /// Events kept for only the last 15 minutes.
    ShortEvents,
    /// No source could be read.
    NoneRead,
}

impl Shape {
    /// The words `FRESHKUBE_TIMELINE` takes.
    pub const NAMES: [&'static str; 5] = ["busy", "quiet", "partial", "short-events", "none-read"];

    pub fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "busy" => Self::Busy,
            "quiet" => Self::Quiet,
            "partial" => Self::Partial,
            "short-events" => Self::ShortEvents,
            "none-read" => Self::NoneRead,
            _ => return None,
        })
    }
}

const CLUSTER: &str = "core-fra";

fn item(now: i64, ago: i64, severity: Severity, title: &str, detail: &str) -> Item {
    Item {
        at: now - ago,
        severity,
        title: title.into(),
        detail: detail.into(),
    }
}

const MIN: i64 = 60;
const HOUR: i64 = 3600;

/// The example reads of `checkout`, taken at `now`.
pub fn reads(now: i64, shape: Shape) -> Vec<SourceRead> {
    if shape == Shape::NoneRead {
        return Source::ALL
            .into_iter()
            .map(|source| SourceRead::not_read(source, CLUSTER, now, "the cluster didn't answer"))
            .collect();
    }
    let quiet = shape == Shape::Quiet;
    let promotions = if quiet {
        Vec::new()
    } else {
        vec![
            item(
                now,
                2 * HOUR + 12 * MIN,
                Severity::Info,
                "Promoted wonky-otter to prod-ams",
                "Kargo Promotion prod-ams.01hx succeeded",
            ),
            item(
                now,
                2 * HOUR + 40 * MIN,
                Severity::Info,
                "Promoted wonky-otter to stage",
                "Kargo Promotion stage.01hw succeeded",
            ),
        ]
    };
    let syncs = if quiet {
        Vec::new()
    } else {
        vec![item(
            now,
            2 * HOUR + 11 * MIN,
            Severity::Info,
            "Synced checkout-prod-ams",
            "Argo CD Application checkout-prod-ams, Synced and Healthy",
        )]
    };
    let incidents = if quiet {
        Vec::new()
    } else {
        vec![item(
            now,
            2 * HOUR,
            Severity::Bad,
            "Error rate above 5% on checkout/api",
            "Coroot incident, still open",
        )]
    };
    let deploys = if quiet {
        Vec::new()
    } else {
        vec![item(
            now,
            2 * HOUR + 10 * MIN,
            Severity::Info,
            "deploy/api → 1.8.2",
            "ReplicaSet api-7d9f created, revision 14",
        )]
    };
    let events = if quiet {
        Vec::new()
    } else {
        vec![
            item(
                now,
                8 * MIN,
                Severity::Warn,
                "BackOff: api-7d9f-x2k4",
                "Back-off restarting failed container api",
            ),
            item(
                now,
                12 * MIN,
                Severity::Warn,
                "Unhealthy: api-7d9f-x2k4",
                "Readiness probe failed: HTTP 503",
            ),
        ]
    };
    let restarts = if quiet {
        Vec::new()
    } else {
        vec![item(
            now,
            9 * MIN,
            Severity::Bad,
            "api-7d9f-x2k4 restarted",
            "Container api, OOMKilled, 5 restarts",
        )]
    };
    let nodes = if quiet {
        Vec::new()
    } else {
        vec![item(
            now,
            4 * HOUR,
            Severity::Warn,
            "worker-2 NotReady",
            "Node worker-2 hosts 2 of the application's pods",
        )]
    };
    let retention = if shape == Shape::ShortEvents {
        15 * MIN
    } else {
        EVENT_RETENTION
    };
    let mut reads = vec![
        SourceRead::read(Source::Promotions, "Kargo in cicd-fra", now, promotions),
        SourceRead::read(Source::Syncs, "Argo CD in core-fra", now, syncs),
        SourceRead::read(Source::Incidents, "Coroot", now, incidents),
        SourceRead::read(Source::Deploys, CLUSTER, now, deploys).state_only(),
        SourceRead::read(Source::Events, CLUSTER, now, events).kept_from(now - retention),
        SourceRead::read(Source::Restarts, CLUSTER, now, restarts).state_only(),
        SourceRead::read(Source::Nodes, CLUSTER, now, nodes).state_only(),
    ];
    if shape == Shape::Partial {
        reads[1] = SourceRead::not_read(
            Source::Syncs,
            "Argo CD in core-fra",
            now,
            "Argo CD isn't installed",
        );
        reads[2] = SourceRead::not_read(Source::Incidents, "Coroot", now, "refused (403)");
    }
    reads
}
