//! Reading the sources: a GET-only [`Client`] for one Alertmanager or
//! vmalert, the [`Sources`] a cluster has, and the reads the pages and the
//! session make of them. Every answer and every list is bounded.
use serde_json::Value;

use super::super::scope::{Place, Resolved};
use super::super::{Matcher, Prometheus, PrometheusService, QueryError};
use super::model::{Alert, AlertState, Kind, RuleGroup, Silence};
use super::parse;
use super::state::Unknown;

/// The largest answer read from an alert source.
pub const MAX_ANSWER: usize = 8 * 1024 * 1024;
/// Alerts kept from one read for a page.
pub const MAX_PAGE_ALERTS: usize = 2000;
/// Alerts the session keeps; the rest are counted, not held.
pub const MAX_ACTIVE: usize = 500;

/// How the open entry's alerts are looked for, from its metrics scope
/// ([`Resolved`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The open cluster's own Alertmanager and vmalert.
    Cluster,
    /// One Alertmanager and vmalert beside the central store, whose alerts
    /// carry the matcher's label.
    Central {
        /// The namespace of the store's Service.
        namespace: String,
        matcher: Matcher,
        /// The workspace entry whose cluster hosts the store's Service,
        /// when it isn't the open one: another host is another read.
        host: Option<String>,
    },
    /// No source can be looked for, and why.
    Unsupported(String),
}

impl Scope {
    /// The scope of an entry's metrics. A store given as a URL names no
    /// namespace to look beside, so it names no alert source either.
    pub fn of(resolved: &Resolved) -> Self {
        match resolved {
            Resolved::Central {
                place: Place::Service(service),
                matcher,
                host,
            } => Self::Central {
                namespace: service.namespace.clone(),
                matcher: matcher.clone(),
                host: host.clone(),
            },
            Resolved::Central {
                place: Place::Url { .. },
                ..
            } => Self::Unsupported(
                "The central store is a URL, so there is no Service to look for \
                 Alertmanager or vmalert beside"
                    .into(),
            ),
            Resolved::Discovery | Resolved::Own(_) | Resolved::Unknown(_) => Self::Cluster,
        }
    }

    /// The namespace to look in, when it is one.
    pub fn beside(&self) -> Option<&str> {
        match self {
            Self::Central { namespace, .. } => Some(namespace),
            _ => None,
        }
    }

    pub fn matcher(&self) -> Option<&Matcher> {
        match self {
            Self::Central { matcher, .. } => Some(matcher),
            _ => None,
        }
    }
}

/// One confirmed Alertmanager or vmalert.
#[derive(Clone)]
pub struct Client {
    kind: Kind,
    prometheus: Prometheus,
    version: Option<String>,
    replicas: Option<usize>,
}

impl Client {
    pub fn new(kind: Kind, prometheus: Prometheus) -> Self {
        Self {
            kind,
            prometheus,
            version: None,
            replicas: None,
        }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Where it is, as the page shows it.
    pub fn label(&self) -> String {
        self.prometheus.endpoint().label()
    }

    pub fn service(&self) -> Option<&PrometheusService> {
        self.prometheus.service()
    }

    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// The peers Alertmanager's cluster reports, itself included.
    pub fn replicas(&self) -> Option<usize> {
        self.replicas
    }

    /// Confirms the server answers as its kind, and learns its version and
    /// replicas, with a read of its own.
    pub(super) async fn confirm(mut self) -> Result<Self, QueryError> {
        match self.kind {
            Kind::Alertmanager => {
                let status = self.get("api/v2/status").await?;
                let (version, replicas) = parse::alertmanager_status(&status);
                self.version = version;
                self.replicas = replicas;
                // The status says a server is there; its alerts say it is
                // Alertmanager's API.
                self.alertmanager_alerts(false).await?;
            }
            Kind::Vmalert => {
                self.get("api/v1/alerts")
                    .await
                    .and_then(|value| parse::vmalert_alerts(&value).map_err(bad))?;
            }
            Kind::PrometheusRule => {}
        }
        Ok(self)
    }

    async fn get(&self, target: &str) -> Result<Value, QueryError> {
        self.prometheus.get_plain(target, MAX_ANSWER).await
    }

    /// Alertmanager's alerts: the active ones, or with the muted too.
    async fn alertmanager_alerts(&self, muted: bool) -> Result<Vec<Alert>, QueryError> {
        let target = format!(
            "api/v2/alerts?active=true&unprocessed=true&silenced={muted}&inhibited={muted}"
        );
        let value = self.get(&target).await?;
        parse::alertmanager_alerts(&value).map_err(bad)
    }

    /// The alerts the source holds. `muted` asks Alertmanager for its
    /// silenced and inhibited alerts too; `active` ones only otherwise.
    pub async fn alerts(&self, muted: bool) -> Result<Vec<Alert>, QueryError> {
        match self.kind {
            Kind::Alertmanager => self.alertmanager_alerts(muted).await,
            Kind::Vmalert => {
                let value = self.get("api/v1/alerts").await?;
                parse::vmalert_alerts(&value).map_err(bad)
            }
            Kind::PrometheusRule => Ok(Vec::new()),
        }
    }

    pub async fn silences(&self) -> Result<Vec<Silence>, QueryError> {
        let value = self.get("api/v2/silences").await?;
        parse::silences(&value).map_err(bad)
    }

    pub async fn rules(&self) -> Result<Vec<RuleGroup>, QueryError> {
        let value = self.get("api/v1/rules").await?;
        parse::vmalert_rules(&value).map_err(bad)
    }
}

fn bad(message: String) -> QueryError {
    QueryError::new(super::super::ErrorKind::BadAnswer, message)
}

/// The sources a cluster has: at most one of each.
#[derive(Clone, Default)]
pub struct Sources {
    pub alertmanager: Option<Client>,
    pub vmalert: Option<Client>,
}

impl Sources {
    pub fn is_empty(&self) -> bool {
        self.alertmanager.is_none() && self.vmalert.is_none()
    }

    pub(super) fn set(&mut self, client: Client) {
        match client.kind() {
            Kind::Alertmanager => self.alertmanager = Some(client),
            Kind::Vmalert => self.vmalert = Some(client),
            Kind::PrometheusRule => {}
        }
    }

    pub(super) fn has(&self, kind: Kind) -> bool {
        self.service(kind).is_some()
    }

    pub(super) fn service(&self, kind: Kind) -> Option<&PrometheusService> {
        match kind {
            Kind::Alertmanager => self.alertmanager.as_ref(),
            Kind::Vmalert => self.vmalert.as_ref(),
            Kind::PrometheusRule => None,
        }
        .and_then(Client::service)
    }

    /// What was read, for the "all quiet" line to name.
    pub fn read(&self) -> Vec<ReadFrom> {
        [&self.alertmanager, &self.vmalert]
            .into_iter()
            .flatten()
            .map(|client| ReadFrom {
                kind: client.kind(),
                label: client.label(),
                service: client.service().cloned(),
                version: client.version().map(str::to_owned),
                replicas: client.replicas(),
            })
            .collect()
    }
}

/// A source an answer was read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadFrom {
    pub kind: Kind,
    pub label: String,
    /// The Service it was read through; none for a URL and for example data.
    pub service: Option<PrometheusService>,
    pub version: Option<String>,
    pub replicas: Option<usize>,
}

impl ReadFrom {
    /// `Alertmanager 0.27.0 (3 replicas) at monitoring/alertmanager-operated:9093`.
    pub fn words(&self) -> String {
        let mut words = self.kind.label().to_owned();
        if let Some(version) = &self.version {
            words.push(' ');
            words.push_str(version);
        }
        if let Some(replicas) = self.replicas.filter(|replicas| *replicas > 1) {
            words.push_str(&format!(" ({replicas} replicas)"));
        }
        format!("{words} at {}", self.label)
    }
}

/// An error read from a source as the state that says it.
pub(super) fn unknown(error: QueryError) -> Unknown {
    if error.is_refused() {
        Unknown::Refused(error.message)
    } else {
        Unknown::Unread(error.message)
    }
}

/// Keeps the alerts of the matcher's cluster. A store whose alerts carry no
/// such label at all can't say whose they are, so nothing is kept and the
/// absence is reported: never every cluster's alerts.
pub fn scoped(alerts: Vec<Alert>, matcher: Option<&Matcher>) -> Result<Vec<Alert>, Unknown> {
    let Some(matcher) = matcher else {
        return Ok(alerts);
    };
    if !alerts.is_empty()
        && !alerts
            .iter()
            .any(|alert| alert.labels.contains_key(matcher.label()))
    {
        return Err(Unknown::NoClusterLabel(matcher.label().to_owned()));
    }
    Ok(alerts
        .into_iter()
        .filter(|alert| alert.label(matcher.label()) == Some(matcher.value()))
        .collect())
}

/// What the session reads: the active alerts, bounded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Active {
    /// Firing and pending alerts, most urgent first, at most [`MAX_ACTIVE`].
    pub alerts: Vec<Alert>,
    /// Active alerts beyond the bound, counted and not held.
    pub more: usize,
    pub read: Vec<ReadFrom>,
    /// Other Services of the same kind that also answered.
    pub others: Vec<String>,
}

impl Active {
    /// The Services it was read through, to try first the next time.
    pub fn remembered(&self) -> super::discover::Remembered {
        let service = |kind| {
            self.read
                .iter()
                .find(|from| from.kind == kind)
                .and_then(|from| from.service.clone())
        };
        super::discover::Remembered {
            alertmanager: service(Kind::Alertmanager),
            vmalert: service(Kind::Vmalert),
        }
    }
}

/// Orders alerts for display: most urgent first, then by name, firing before
/// pending, then by labels, so each alert name's alerts sit together.
pub fn sort(alerts: &mut [Alert]) {
    alerts.sort_by(|a, b| {
        (a.severity, &a.name, a.state, &a.fingerprint).cmp(&(
            b.severity,
            &b.name,
            b.state,
            &b.fingerprint,
        ))
    });
}

/// Reads the active alerts of every source: Alertmanager's firing alerts
/// and vmalert's, except that with both, vmalert's firing ones are
/// Alertmanager's already and only its pending ones are added. Any source
/// failing fails the read, since an alert it holds may be missing.
pub async fn active(
    sources: &Sources,
    others: &[String],
    scope: &Scope,
) -> Result<Active, Unknown> {
    let mut alerts = Vec::new();
    if let Some(manager) = &sources.alertmanager {
        alerts.extend(manager.alerts(false).await.map_err(unknown)?);
    }
    if let Some(vmalert) = &sources.vmalert {
        let held = vmalert.alerts(false).await.map_err(unknown)?;
        let both = sources.alertmanager.is_some();
        alerts.extend(
            held.into_iter()
                .filter(|alert| !both || alert.state == AlertState::Pending),
        );
    }
    let mut alerts = scoped(parse::dedupe(alerts), scope.matcher())?;
    alerts.retain(|alert| alert.state.is_active());
    sort(&mut alerts);
    let more = alerts.len().saturating_sub(MAX_ACTIVE);
    alerts.truncate(MAX_ACTIVE);
    Ok(Active {
        alerts,
        more,
        read: sources.read(),
        others: others.to_vec(),
    })
}

/// What the Alerts page shows: every state, bounded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertsData {
    pub alerts: Vec<Alert>,
    pub more: usize,
    pub read: Vec<ReadFrom>,
    pub others: Vec<String>,
}

/// The Alerts page's read: Alertmanager's alerts with the muted ones, and
/// vmalert's pending ones.
pub async fn alerts_page(
    sources: &Sources,
    others: &[String],
    scope: &Scope,
) -> Result<AlertsData, Unknown> {
    let mut alerts = Vec::new();
    if let Some(manager) = &sources.alertmanager {
        alerts.extend(manager.alerts(true).await.map_err(unknown)?);
    }
    if let Some(vmalert) = &sources.vmalert {
        let held = vmalert.alerts(true).await.map_err(unknown)?;
        let both = sources.alertmanager.is_some();
        alerts.extend(
            held.into_iter()
                .filter(|alert| !both || alert.state == AlertState::Pending),
        );
    }
    let mut alerts = scoped(parse::dedupe(alerts), scope.matcher())?;
    sort(&mut alerts);
    let more = alerts.len().saturating_sub(MAX_PAGE_ALERTS);
    alerts.truncate(MAX_PAGE_ALERTS);
    Ok(AlertsData {
        alerts,
        more,
        read: sources.read(),
        others: others.to_vec(),
    })
}

/// What the Silences page shows: the silences, and the alerts they mute now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SilencesData {
    pub silences: Vec<Silence>,
    pub alerts: Vec<Alert>,
    pub read: Vec<ReadFrom>,
}

/// The Silences page's read. Silences belong to Alertmanager; vmalert has
/// none. In a central store a silence isn't scoped to one cluster, so each
/// is shown with the alerts of this cluster it mutes and the others are
/// left to their matchers, which are shown as written.
pub async fn silences_page(sources: &Sources, scope: &Scope) -> Result<SilencesData, Unknown> {
    let Some(manager) = &sources.alertmanager else {
        return Err(Unknown::NotFound(
            "No Alertmanager was found, and only it holds silences".into(),
        ));
    };
    let silences = manager.silences().await.map_err(unknown)?;
    let alerts = scoped(
        parse::dedupe(manager.alerts(true).await.map_err(unknown)?),
        scope.matcher(),
    )?;
    Ok(SilencesData {
        silences,
        alerts,
        read: sources.read(),
    })
}

/// What the Rules page shows.
#[derive(Clone, Debug, PartialEq)]
pub struct RulesData {
    pub groups: Vec<RuleGroup>,
    /// Whether a server evaluated them. Without one, every health is
    /// Unknown, and the page says so.
    pub evaluated: bool,
    pub read: Vec<ReadFrom>,
    /// Why some rules aren't shown, for the page to say.
    pub note: Option<String>,
}
