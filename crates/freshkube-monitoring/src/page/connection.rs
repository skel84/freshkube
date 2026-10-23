//! Finding Prometheus for the chosen context. The source chosen in
//! Settings is the only one tried; without one, the Service remembered for
//! the context goes first, then discovery's ranked candidates. Each outcome
//! is a state the page shows on its own: looking, found, none found (with
//! what was looked for and the candidates to pick from), refused and failed.
use freshkube_core::cluster_source::ClusterAccess;
use freshkube_core::monitoring::scope::{self, Place, Resolved};
use freshkube_core::monitoring::{
    Backend, Candidate, Discovery, Endpoint, ErrorKind, LOOKED_FOR, Prometheus, PrometheusService,
    QueryError, Tried, cluster_client, confirm, confirm_url, discover, forget_after,
};
use freshkube_ui::read_state::ReadState;
use gpui_kit::{Context, EntityId, SharedString, WeakEntity};

use super::{MonitoringEvent, MonitoringPage, Read, ReadKind, Request};
use crate::history::{HistoryKind, HistorySource, HistoryView};
use crate::store::{Choice, token_account};

/// These owners outlive the display states, so a repeated connection or
/// confirmation cannot reuse a superseded completion token.
#[derive(Default)]
pub(super) struct ConnectionReads {
    pub(super) looking: Read,
    pub(super) confirming: Read,
    pub(super) rebuild: Read,
}

impl ConnectionReads {
    pub(super) fn forget(&mut self) {
        self.looking.resolve(None, Ok(()));
        self.confirming.resolve(None, Ok(()));
        self.rebuild.resolve(None, Ok(()));
    }
}

pub(super) enum Connection {
    /// Not looked for on this source yet, or the page hid while looking.
    None,
    /// `--fixture`: example data answers.
    Example,
    Looking {
        _request: Request,
    },
    Ready {
        prometheus: Prometheus,
        /// `namespace/name:port` or the URL, and the server and its
        /// version for the tooltip.
        label: SharedString,
        version: SharedString,
        rebuild: Rebuild,
    },
    /// Nothing answered as Prometheus.
    Missing(Missing),
    Refused(SharedString),
    Failed(SharedString),
    /// The setup file leaves this entry without a store, and says why.
    /// Nothing was asked, so there is nothing to retry.
    Unknown(SharedString),
}

pub(super) struct Missing {
    /// Ranked Services, each with its label, to pick from.
    pub(super) candidates: Vec<(PrometheusService, SharedString)>,
    /// What was tried and why it didn't answer, one line each.
    pub(super) tried: Option<SharedString>,
    /// A candidate being confirmed.
    pub(super) confirming: Option<(PrometheusService, Request)>,
    /// What the empty state says, derived once with the answer.
    pub(super) state: ReadState,
    /// What was looked for and tried, for the details box.
    pub(super) details: String,
}

/// Discovery found no Prometheus. Prometheus may run under a name
/// Freshkube doesn't search for, so this says "not found", not "not
/// installed".
pub(super) fn not_found_state(cluster: &str) -> ReadState {
    ReadState::Empty {
        looked_for: "Prometheus".into(),
        cluster: cluster.into(),
        hint: Some(
            "Prometheus may run under a name Freshkube doesn't look for. It is optional: it adds charts of CPU, memory and your own queries, and the rest of Freshkube works without it."
                .into(),
        ),
    }
}

/// The setup names no metrics store for this cluster.
pub(super) fn not_set_up_state() -> &'static ReadState {
    static STATE: std::sync::OnceLock<ReadState> = std::sync::OnceLock::new();
    STATE.get_or_init(|| ReadState::NotSetUp {
        what: "A metrics source".into(),
        gap: "Freshkube doesn't know which metrics store to read for this cluster, so it asked nothing. Prometheus is optional; the rest of Freshkube works without it.".into(),
        setting: Some("Monitoring".into()),
    })
}

/// Building a Ready Service's cluster client again after a read through it
/// failed in a way that implicates the client: once, until Refresh or Try
/// again, so a Service that is down doesn't rebuild it on every read.
#[derive(Default)]
pub(super) enum Rebuild {
    #[default]
    Unused,
    Building {
        _request: Request,
        /// A shown history asked for this rebuild, possibly after the page.
        history: bool,
    },
    Used,
}

impl Connection {
    pub(super) fn usable(&self) -> bool {
        matches!(self, Self::Example | Self::Ready { .. })
    }

    /// Hiding drops a search in progress; showing starts it again.
    pub(super) fn hide(&mut self, history_visible: bool, reads: &mut ConnectionReads) {
        match self {
            Self::Looking { .. } => {
                reads.looking.resolve(None, Ok(()));
                *self = Self::None;
            }
            Self::Missing(missing) => {
                reads.confirming.resolve(None, Ok(()));
                missing.confirming = None;
            }
            // The next failure may build it again.
            Self::Ready {
                rebuild: Rebuild::Building { history: true, .. },
                ..
            } if history_visible => {}
            Self::Ready {
                rebuild: rebuild @ Rebuild::Building { .. },
                ..
            } => {
                reads.rebuild.resolve(None, Ok(()));
                *rebuild = Rebuild::Unused;
            }
            _ => {}
        }
    }

    fn missing(candidates: Vec<Candidate>, tried: Vec<Tried>, cluster: &str) -> Self {
        let tried_lines: Option<String> = (!tried.is_empty()).then(|| {
            tried
                .iter()
                .map(|tried| format!("{}: {}", tried.service.label(), tried.error))
                .collect::<Vec<_>>()
                .join("\n")
        });
        Self::Missing(Missing {
            candidates: candidates
                .into_iter()
                .map(|candidate| {
                    let label = candidate.service.label().into();
                    (candidate.service, label)
                })
                .collect(),
            tried: tried_lines.clone().map(Into::into),
            confirming: None,
            state: not_found_state(cluster),
            details: {
                let mut details = format!("Looked for {LOOKED_FOR}.");
                if let Some(tried) = &tried_lines {
                    details = format!("{details}\n{tried}");
                }
                details
            },
        })
    }
}

impl MonitoringPage {
    /// The connection's state in a word, for tests.
    #[cfg(test)]
    pub(super) fn connection_name(&self) -> &'static str {
        match &self.connection {
            Connection::None => "none",
            Connection::Example => "example",
            Connection::Looking { .. } => "looking",
            Connection::Ready { .. } => "ready",
            Connection::Missing(_) => "missing",
            Connection::Refused(_) => "refused",
            Connection::Failed(_) => "failed",
            Connection::Unknown(_) => "unknown",
        }
    }

    /// The Ready connection's client rebuild in a word, for tests.
    #[cfg(test)]
    pub(super) fn rebuild_name(&self) -> &'static str {
        match &self.connection {
            Connection::Ready { rebuild, .. } => match rebuild {
                Rebuild::Unused => "unused",
                Rebuild::Building { .. } => "building",
                Rebuild::Used => "used",
            },
            _ => "not ready",
        }
    }

    /// Where pod and node history read: example data, the Prometheus this
    /// page confirmed, or the Service remembered for the context while it
    /// hasn't looked again. None when it looked and found none.
    pub fn history(&self) -> Option<HistorySource> {
        let source = self.source.as_ref()?;
        let kind = match (&self.connection, &source.access) {
            (_, ClusterAccess::Example(_)) => HistoryKind::Example,
            (Connection::Ready { prometheus, .. }, _) => HistoryKind::Ready(prometheus.clone()),
            // A scoped source is never guessed from what was remembered.
            _ if !self.scope.resolved.is_discovery() => return None,
            (Connection::None | Connection::Looking { .. }, access) => {
                // A URL's token is read only when the page connects.
                let service = match self.saved.choices.get(&source.context) {
                    Some(Choice::Service(service)) => service,
                    Some(Choice::Url { .. }) => return None,
                    None => self.saved.services.get(&source.context)?,
                };
                HistoryKind::Remembered {
                    access: access.clone(),
                    service: service.clone(),
                }
            }
            _ => return None,
        };
        Some(HistorySource {
            id: source.id.clone(),
            kind,
            client: self.clients,
            owner: Some(self.owner.clone()),
        })
    }

    /// Registers only visible histories; they keep history-requested
    /// recovery alive independently of the dashboard's reads.
    pub(crate) fn show_history(
        &mut self,
        history: WeakEntity<HistoryView>,
        source: &HistorySource,
        visible: bool,
    ) {
        self.histories
            .retain(|shown| shown.entity_id() != history.entity_id() && shown.upgrade().is_some());
        if visible && self.current_history(source) {
            self.histories.push(history);
        }
        self.cancel_history_rebuild();
    }

    pub(crate) fn hide_history(&mut self, id: EntityId) {
        self.histories
            .retain(|history| history.entity_id() != id && history.upgrade().is_some());
        self.cancel_history_rebuild();
    }

    fn cancel_history_rebuild(&mut self) {
        if !self.visible
            && self.histories.is_empty()
            && let Connection::Ready {
                rebuild: rebuild @ Rebuild::Building { history: true, .. },
                ..
            } = &mut self.connection
        {
            self.connection_reads.rebuild.resolve(None, Ok(()));
            *rebuild = Rebuild::Unused;
        }
    }

    /// A history reports only failures and retries for the client it read.
    pub(crate) fn history_failed(
        &mut self,
        source: &HistorySource,
        error: &QueryError,
        cx: &mut Context<Self>,
    ) {
        if !self.current_history(source) || self.histories.is_empty() {
            return;
        }
        self.read_failed(error, cx);
        if error.implicates_client()
            && let Connection::Ready {
                rebuild: Rebuild::Building { history, .. },
                ..
            } = &mut self.connection
        {
            *history = true;
        }
    }

    pub(crate) fn retry_history(&mut self, source: &HistorySource) {
        if self.current_history(source) && !self.histories.is_empty() {
            self.allow_rebuild();
        }
    }

    fn current_history(&self, source: &HistorySource) -> bool {
        self.history()
            .is_some_and(|current| current.key() == source.key())
    }

    /// Looks for Prometheus when the page shows and hasn't yet on this
    /// source; once usable, reads what the dashboard needs.
    pub(super) fn connect(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        if self.connection.usable() {
            self.resume(cx);
            return;
        }
        if !matches!(self.connection, Connection::None) {
            return;
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        if source.access.is_example() {
            self.connection = Connection::Example;
            self.resume(cx);
            return;
        }
        if !self.scope.resolved.is_discovery() {
            self.connect_scoped(source.access.clone(), cx);
            return;
        }
        let request = match self.saved.choices.get(&source.context).cloned() {
            Some(Choice::Service(service)) => {
                let access = source.access.clone();
                self.run(
                    self.read_identity(ReadKind::Connect {
                        endpoint: Some(Endpoint::Service(service.clone())),
                    }),
                    |this| Some(&mut this.connection_reads.looking),
                    async move {
                        let client = cluster_client(&access).await?;
                        confirm(&client, service)
                            .await
                            .inspect_err(|error| forget_after(&access, error))
                    },
                    cx,
                    |this, result, cx| this.confirmed(result, cx),
                )
            }
            Some(Choice::Url { url, token }) => {
                let token = self.token_reader(&url, token);
                self.run(
                    self.read_identity(ReadKind::Connect {
                        endpoint: Some(Endpoint::Url(url.clone())),
                    }),
                    |this| Some(&mut this.connection_reads.looking),
                    async move { confirm_url(url, token.await?).await },
                    cx,
                    |this, result, cx| this.confirmed(result, cx),
                )
            }
            None => {
                let remembered = self.saved.services.get(&source.context).cloned();
                let access = source.access.clone();
                self.run(
                    self.read_identity(ReadKind::Connect { endpoint: None }),
                    |this| Some(&mut this.connection_reads.looking),
                    async move {
                        let client = cluster_client(&access).await?;
                        discover(&client, remembered)
                            .await
                            .inspect_err(|error| forget_after(&access, error))
                    },
                    cx,
                    |this, result, cx| this.discovered(result, cx),
                )
            }
        };
        self.connection = Connection::Looking { _request: request };
    }

    /// Reads the store the setup file names, and nothing else: no discovery,
    /// no source chosen in Settings. A Service is reached through the cluster
    /// that hosts it. A store shared by every entry is scoped to this entry's
    /// series before anything is asked of it.
    fn connect_scoped(&mut self, open: ClusterAccess, cx: &mut Context<Self>) {
        let (place, access, matcher, hosted_by) = match self.scope.resolved.clone() {
            Resolved::Own(place) => (place, open, None, None),
            Resolved::Central {
                place,
                host,
                matcher,
            } => {
                let access = match (&place, &host) {
                    (Place::Service(_), Some(_)) => self.scope.host.clone(),
                    _ => Some(open),
                };
                let Some(access) = access else {
                    self.connection = Connection::Failed(
                        "The cluster that hosts the metrics store isn't connected".into(),
                    );
                    return;
                };
                (place, access, Some(matcher), host)
            }
            Resolved::Unknown(why) => {
                self.connection = Connection::Unknown(why.into());
                return;
            }
            Resolved::Discovery => return,
        };
        let (endpoint, token) = match &place {
            Place::Service(service) => (Endpoint::Service(service.clone()), None),
            Place::Url { url, token } => (
                Endpoint::Url(url.clone()),
                Some(self.token_reader(url, *token)),
            ),
        };
        let request = self.run(
            self.read_identity(ReadKind::Connect {
                endpoint: Some(endpoint),
            }),
            |this| Some(&mut this.connection_reads.looking),
            async move {
                let token = match token {
                    Some(token) => token.await?,
                    None => None,
                };
                scope::connect(&place, &access, token).await
            },
            cx,
            move |this, result, cx| match result {
                Ok((prometheus, version)) => {
                    let prometheus = match matcher {
                        Some(matcher) => prometheus.with_matcher(matcher),
                        None => prometheus,
                    };
                    this.use_prometheus(prometheus, version, cx)
                }
                Err(error) => this.discovered(Err(through_host(error, hosted_by.as_deref())), cx),
            },
        );
        self.connection = Connection::Looking { _request: request };
    }

    /// The cluster a Ready connection's Service is reached through.
    fn reading_access(&self) -> Option<ClusterAccess> {
        match (&self.scope.resolved, &self.scope.host) {
            (Resolved::Central { host: Some(_), .. }, Some(host)) => Some(host.clone()),
            _ => self.source.as_ref().map(|source| source.access.clone()),
        }
    }

    /// The token for a URL: typed this session, else read from the
    /// credential store off the async threads, else none.
    pub(super) fn token_reader(
        &self,
        url: &str,
        saved: bool,
    ) -> impl std::future::Future<Output = Result<Option<String>, QueryError>> + Send + 'static
    {
        let typed = self.tokens.get(url).cloned();
        let secrets = self.secrets.clone().filter(|_| saved && typed.is_none());
        let account = token_account(url);
        async move {
            let Some(secrets) = secrets else {
                return Ok(typed);
            };
            tokio::task::spawn_blocking(move || secrets.read(&account))
                .await
                .map_err(|_| QueryError::new(ErrorKind::Unavailable, "The token read stopped"))?
                .map_err(|message| {
                    QueryError::new(
                        ErrorKind::Unavailable,
                        format!("Couldn't read the token: {message}"),
                    )
                })
        }
    }

    /// The answer for a source chosen in Settings: nothing else is tried.
    fn confirmed(
        &mut self,
        result: Result<(Prometheus, Option<String>), QueryError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok((prometheus, version)) => self.use_prometheus(prometheus, version, cx),
            Err(error) => {
                self.discovered(Err(error), cx);
            }
        }
    }

    pub(super) fn discovered(
        &mut self,
        result: Result<Discovery, QueryError>,
        cx: &mut Context<Self>,
    ) {
        self.connection_reads.looking.resolve(None, Ok(()));
        self.connection_reads.confirming.resolve(None, Ok(()));
        match result {
            Ok(Discovery::Found {
                prometheus,
                version,
                ..
            }) => self.use_prometheus(prometheus, version, cx),
            Ok(Discovery::Missing { candidates, tried }) => {
                let cluster = self.source.as_ref().map_or_else(
                    || "this cluster".to_owned(),
                    |source| source.context.to_string(),
                );
                self.connection = Connection::missing(candidates, tried, &cluster);
            }
            Err(error) if error.is_refused() => {
                self.connection = Connection::Refused(error.message.into());
            }
            Err(error) => self.connection = Connection::Failed(error.message.into()),
        }
        cx.emit(MonitoringEvent::History);
        cx.notify();
    }

    /// Confirms a Service the user picked when discovery found none.
    pub(super) fn pick_service(&mut self, service: PrometheusService, cx: &mut Context<Self>) {
        let Connection::Missing(_) = &self.connection else {
            return;
        };
        let Some(access) = self.source.as_ref().map(|source| source.access.clone()) else {
            return;
        };
        let confirming = service.clone();
        let request = self.run(
            self.read_identity(ReadKind::Confirm {
                service: service.clone(),
            }),
            |this| Some(&mut this.connection_reads.confirming),
            async move {
                let client = cluster_client(&access).await?;
                confirm(&client, confirming)
                    .await
                    .inspect_err(|error| forget_after(&access, error))
            },
            cx,
            |this, result, cx| match result {
                Ok((prometheus, version)) => this.use_prometheus(prometheus, version, cx),
                Err(error) if error.is_refused() => this.discovered(Err(error), cx),
                Err(error) => this.confirm_failed(error, cx),
            },
        );
        if let Connection::Missing(missing) = &mut self.connection {
            missing.confirming = Some((service, request));
        }
        cx.notify();
    }

    /// A picked Service that didn't answer joins what was tried; the
    /// candidates stay to pick another.
    fn confirm_failed(&mut self, error: QueryError, cx: &mut Context<Self>) {
        self.connection_reads.confirming.resolve(None, Ok(()));
        if let Connection::Missing(missing) = &mut self.connection
            && let Some((service, _)) = missing.confirming.take()
        {
            let line = format!("{}: {error}", service.label());
            missing.tried = Some(match missing.tried.take() {
                Some(tried) => format!("{tried}\n{line}").into(),
                None => line.into(),
            });
        }
        cx.notify();
    }

    pub(super) fn use_prometheus(
        &mut self,
        prometheus: Prometheus,
        version: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let label = prometheus.endpoint().label().into();
        let version = describe(prometheus.endpoint().backend(), version.as_deref()).into();
        if let Some(context) = self.context()
            && self.scope.resolved.is_discovery()
            && !self.saved.choices.contains_key(&context)
            && let Some(service) = prometheus.service().cloned()
            && self.saved.services.get(&context) != Some(&service)
        {
            self.saved.services.insert(context.clone(), service.clone());
            let remembered = service.clone();
            self.save(
                move |saved| {
                    saved.services.insert(context, remembered);
                },
                cx,
            );
        }
        self.connection_reads.forget();
        self.connection = Connection::Ready {
            prometheus,
            label,
            version,
            rebuild: Rebuild::Unused,
        };
        cx.emit(MonitoringEvent::History);
        self.resume(cx);
        cx.notify();
    }

    /// A read through the Ready connection failed. When the cluster's
    /// client may be the cause, it is forgotten and built again once, then
    /// the variables and every panel are read again. A URL doesn't use the
    /// client, and a 403 keeps it.
    pub(super) fn read_failed(&mut self, error: &QueryError, cx: &mut Context<Self>) {
        let Some(access) = self.reading_access() else {
            return;
        };
        let Connection::Ready {
            prometheus,
            rebuild: Rebuild::Unused,
            ..
        } = &self.connection
        else {
            return;
        };
        if !error.implicates_client() || prometheus.service().is_none() || access.is_example() {
            return;
        }
        let prometheus = prometheus.clone();
        let Some(source) = self.history() else {
            return;
        };
        access.forget();
        let request = self.run(
            self.read_identity(ReadKind::Rebuild {
                source: source.key(),
            }),
            |this| Some(&mut this.connection_reads.rebuild),
            async move { Ok(prometheus.with_client(cluster_client(&access).await?)) },
            cx,
            move |this, result, cx| {
                if this.current_history(&source)
                    && matches!(
                        this.connection,
                        Connection::Ready {
                            rebuild: Rebuild::Building { .. },
                            ..
                        }
                    )
                {
                    this.client_rebuilt(result, cx);
                }
            },
        );
        if let Connection::Ready { rebuild, .. } = &mut self.connection {
            *rebuild = Rebuild::Building {
                _request: request,
                history: false,
            };
        }
    }

    /// The Ready Service through its new client, which pod and node history
    /// take too. Without one, the panels keep their errors.
    fn client_rebuilt(&mut self, result: Result<Prometheus, QueryError>, cx: &mut Context<Self>) {
        self.connection_reads.rebuild.resolve(None, Ok(()));
        let Connection::Ready {
            prometheus,
            rebuild,
            ..
        } = &mut self.connection
        else {
            return;
        };
        *rebuild = Rebuild::Used;
        if let Ok(rebuilt) = result {
            *prometheus = rebuilt;
            self.clients += 1;
            cx.emit(MonitoringEvent::History);
            self.resolve_variables(cx);
        }
        cx.notify();
    }

    /// Refresh lets a failure build the client again.
    pub(super) fn allow_rebuild(&mut self) {
        if let Connection::Ready {
            rebuild: rebuild @ Rebuild::Used,
            ..
        } = &mut self.connection
        {
            *rebuild = Rebuild::Unused;
        }
    }
}

/// The server and its version, as the source's tooltip says it.
/// VictoriaMetrics answers `buildinfo` with the Prometheus version it
/// imitates, so its own name stands alone.
pub(super) fn describe(backend: Backend, version: Option<&str>) -> String {
    match (version, backend) {
        (Some(version), Backend::Prometheus | Backend::Thanos | Backend::Mimir) => {
            format!("{} {version}", backend.label())
        }
        (Some(version), Backend::Compatible) => format!("Prometheus API {version}"),
        _ => backend.label().to_owned(),
    }
}

/// A failure reading a store that another entry's cluster hosts says so, or
/// it reads as this entry's own cluster failing.
fn through_host(error: QueryError, host: Option<&str>) -> QueryError {
    match host {
        Some(host) => QueryError::new(
            error.kind,
            format!(
                "Couldn't reach the metrics store through {host}: {}",
                error.message
            ),
        ),
        None => error,
    }
}

#[cfg(test)]
mod read_state_tests {
    use super::*;

    #[test]
    fn nothing_found_says_not_found_and_that_prometheus_is_optional() {
        let state = not_found_state("prod-fra");
        assert_eq!(state.title(), "No Prometheus found");
        let text = state.description();
        assert!(
            text.contains("Looked for Prometheus in prod-fra and found none"),
            "{text}"
        );
        assert!(text.contains("optional"), "{text}");
        assert!(text.contains("works without it"), "{text}");
        assert!(!text.contains("installed"), "{text}");
    }

    #[test]
    fn no_named_store_is_not_set_up_and_points_at_settings() {
        let state = not_set_up_state();
        assert_eq!(state.title(), "A metrics source isn't set up");
        let text = state.description();
        assert!(text.contains("Settings › Monitoring"), "{text}");
        assert!(text.contains("optional"), "{text}");
        assert!(!text.contains("freshkube.yaml"), "{text}");
    }
}
