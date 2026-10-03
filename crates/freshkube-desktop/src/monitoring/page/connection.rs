//! Finding Prometheus for the chosen context: the Service remembered for
//! it first, then discovery's ranked candidates. Each outcome is a state
//! the page shows on its own: looking, found, none found (with what was
//! looked for and the candidates to pick from), refused and failed.
use freshkube_core::monitoring::{
    BuildInfo, Candidate, Discovery, ErrorKind, Prometheus, PrometheusService, QueryError, Tried,
    confirm, discover,
};
use gpui_kit::{Context, SharedString};

use super::{MonitoringEvent, MonitoringPage, Request};
use crate::monitoring::history::{HistoryKind, HistorySource};
use crate::resources::KubeAccess;

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
        /// `namespace/name:port`, and its version for the tooltip.
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
    pub(crate) fn history(&self) -> Option<HistorySource> {
        let source = self.source.as_ref()?;
        let kind = match (&self.connection, &source.access) {
            (_, KubeAccess::Example) => HistoryKind::Example,
            (Connection::Ready { prometheus, .. }, _) => HistoryKind::Ready(prometheus.clone()),
            (Connection::None | Connection::Looking { .. }, access) => HistoryKind::Remembered {
                access: access.clone(),
                service: self.saved.services.get(&source.context)?.clone(),
            },
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
        if let KubeAccess::Example = source.access {
            self.connection = Connection::Example;
            self.resume(cx);
            return;
        }
        let remembered = self.saved.services.get(&source.context).cloned();
        let access = source.access.clone();
        let request = self.run(
            async move {
                let client = access
                    .client()
                    .await
                    .map_err(|message| QueryError::new(ErrorKind::Unavailable, message))?;
                discover(&client, remembered).await
            },
            cx,
            |this, result, cx| this.discovered(result, cx),
        );
        self.connection = Connection::Looking { _request: request };
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
            Err(error) if error.kind == ErrorKind::Refused => {
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
                let client = access
                    .client()
                    .await
                    .map_err(|message| QueryError::new(ErrorKind::Unavailable, message))?;
                confirm(&client, confirming).await
            },
            cx,
            |this, result, cx| match result {
                Ok((prometheus, build)) => this.use_prometheus(prometheus, build, cx),
                Err(error) if error.kind == ErrorKind::Refused => this.discovered(Err(error), cx),
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
        let service = prometheus.service().clone();
        if let Some(context) = self.context()
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
            label: service.label().into(),
            version: format!("Prometheus {}", build.version).into(),
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
