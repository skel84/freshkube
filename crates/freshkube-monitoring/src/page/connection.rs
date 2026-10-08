//! Finding Prometheus for the chosen context. The source chosen in
//! Settings is the only one tried; without one, the Service remembered for
//! the context goes first, then discovery's ranked candidates. Each outcome
//! is a state the page shows on its own: looking, found, none found (with
//! what was looked for and the candidates to pick from), refused and failed.
use freshkube_core::cluster_source::ClusterAccess;
use freshkube_core::monitoring::{
    Backend, BuildInfo, Candidate, Discovery, ErrorKind, Prometheus, PrometheusService, QueryError,
    Tried, cluster_client, confirm, confirm_url, discover, forget_after,
};
use gpui_kit::{Context, SharedString};

use super::{MonitoringEvent, MonitoringPage, Request};
use crate::history::{HistoryKind, HistorySource};
use crate::store::{Choice, token_account};

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
    },
    /// Nothing answered as Prometheus.
    Missing(Missing),
    Refused(SharedString),
    Failed(SharedString),
}

pub(super) struct Missing {
    /// Ranked Services, each with its label, to pick from.
    pub(super) candidates: Vec<(PrometheusService, SharedString)>,
    /// What was tried and why it didn't answer, one line each.
    pub(super) tried: Option<SharedString>,
    /// A candidate being confirmed.
    pub(super) confirming: Option<(PrometheusService, Request)>,
}

impl Connection {
    pub(super) fn usable(&self) -> bool {
        matches!(self, Self::Example | Self::Ready { .. })
    }

    /// Hiding drops a search in progress; showing starts it again.
    pub(super) fn hide(&mut self) {
        match self {
            Self::Looking { .. } => *self = Self::None,
            Self::Missing(missing) => missing.confirming = None,
            _ => {}
        }
    }

    fn missing(candidates: Vec<Candidate>, tried: Vec<Tried>) -> Self {
        Self::Missing(Missing {
            candidates: candidates
                .into_iter()
                .map(|candidate| {
                    let label = candidate.service.label().into();
                    (candidate.service, label)
                })
                .collect(),
            tried: (!tried.is_empty()).then(|| {
                tried
                    .iter()
                    .map(|tried| format!("{}: {}", tried.service.label(), tried.error))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into()
            }),
            confirming: None,
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
        }
    }

    /// Where pod and node history read: example data, the Prometheus this
    /// page confirmed, or the Service remembered for the context while it
    /// hasn't looked again. None when it looked and found none.
    pub fn history(&self) -> Option<HistorySource> {
        let source = self.source.as_ref()?;
        let kind = match (&self.connection, &source.access) {
            (_, ClusterAccess::Example) => HistoryKind::Example,
            (Connection::Ready { prometheus, .. }, _) => HistoryKind::Ready(prometheus.clone()),
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
        })
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
        let request = match self.saved.choices.get(&source.context).cloned() {
            Some(Choice::Service(service)) => {
                let access = source.access.clone();
                self.run(
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
                    async move { confirm_url(url, token.await?).await },
                    cx,
                    |this, result, cx| this.confirmed(result, cx),
                )
            }
            None => {
                let remembered = self.saved.services.get(&source.context).cloned();
                let access = source.access.clone();
                self.run(
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
        result: Result<(Prometheus, BuildInfo), QueryError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok((prometheus, build)) => self.use_prometheus(prometheus, build, cx),
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
        match result {
            Ok(Discovery::Found {
                prometheus, build, ..
            }) => self.use_prometheus(prometheus, build, cx),
            Ok(Discovery::Missing { candidates, tried }) => {
                self.connection = Connection::missing(candidates, tried);
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
            async move {
                let client = cluster_client(&access).await?;
                confirm(&client, confirming)
                    .await
                    .inspect_err(|error| forget_after(&access, error))
            },
            cx,
            |this, result, cx| match result {
                Ok((prometheus, build)) => this.use_prometheus(prometheus, build, cx),
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

    fn use_prometheus(&mut self, prometheus: Prometheus, build: BuildInfo, cx: &mut Context<Self>) {
        let label = prometheus.endpoint().label().into();
        let version = describe(prometheus.endpoint().backend(), &build).into();
        if let Some(context) = self.context()
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
        self.connection = Connection::Ready {
            prometheus,
            label,
            version,
        };
        cx.emit(MonitoringEvent::History);
        self.resume(cx);
        cx.notify();
    }

    /// Asks again after a failure, from the start.
    pub(super) fn retry(&mut self, cx: &mut Context<Self>) {
        self.connection = Connection::None;
        cx.emit(MonitoringEvent::History);
        self.connect(cx);
        cx.notify();
    }
}

/// The server and its version, as the source's tooltip says it.
/// VictoriaMetrics answers `buildinfo` with the Prometheus version it
/// imitates, so its own name stands alone.
pub(super) fn describe(backend: Backend, build: &BuildInfo) -> String {
    match (&build.version, backend) {
        (Some(version), Backend::Prometheus | Backend::Thanos | Backend::Mimir) => {
            format!("{} {version}", backend.label())
        }
        (Some(version), Backend::Compatible) => format!("Prometheus API {version}"),
        _ => backend.label().to_owned(),
    }
}
