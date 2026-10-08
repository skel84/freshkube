//! The shell owns one cancellable Kubernetes observation session per applied
//! cluster/configuration. Foreground Talos node changes do not replace it.
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use freshkube_core::{
    kubernetes_summary::{
        DEBOUNCE, ObservationFailure, Publication, Session, SessionIdentity, Source,
        SubscriptionKey,
    },
    resources::FailureKind,
};
use gpui_kit::{Context, Window};

use super::Pilot;
use crate::{
    backend::{OwnedJob, Target},
    resources::{KubeAccess, example},
    screens::WorkloadData,
};

pub(super) struct SummarySession {
    pub(super) core: Session,
    target: Target,
    fixture_at: chrono::DateTime<chrono::Utc>,
    fixture_clock: Instant,
    applied_revision: u64,
}
impl SummarySession {
    fn fixture_now(&self, now: Instant) -> chrono::DateTime<chrono::Utc> {
        fixture_time(self.fixture_at, self.fixture_clock, now)
    }
}

/// The example session's time: its start, moved on by the executor's clock.
fn fixture_time(
    at: chrono::DateTime<chrono::Utc>,
    clock: Instant,
    now: Instant,
) -> chrono::DateTime<chrono::Utc> {
    at + chrono::Duration::from_std(now.saturating_duration_since(clock)).unwrap_or_default()
}

impl Pilot {
    pub(super) fn deliver_workloads(
        &self,
        data: Result<Arc<WorkloadData>, String>,
        cx: &mut Context<Self>,
    ) {
        self.health.update(cx, |screen, cx| {
            screen.apply_summary(
                self.applied.context.as_deref().unwrap_or_default(),
                data,
                cx,
            );
        });
    }

    pub(super) fn deliver_summary_nodes(&self, cx: &mut Context<Self>) {
        let subscription = (self.page == super::Page::Lifecycle)
            .then_some(self.summary_session.as_ref())
            .flatten()
            .and_then(|session| {
                session.core.subscribe(SubscriptionKey::summary(
                    session.core.identity().clone(),
                    Source::Nodes,
                ))
            });
        self.lifecycle
            .update(cx, |screen, cx| screen.observe_nodes(subscription, cx));
    }

    pub(super) fn stop_summary(&mut self) {
        self.summary_job = None;
        self.summary_task = None;
        self.summary_session = None;
        self.summary_epoch = self.summary_epoch.wrapping_add(1);
    }

    /// Manual Refresh relists every source. The automatic Talos cycle calls
    /// ensure_summary instead, which leaves an existing session alone.
    pub(super) fn refresh_summary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = &self.summary_session {
            session.core.relist();
            if self.fixture && !self.fixture_hold {
                self.publish_fixture(window, cx);
            }
        } else {
            self.ensure_summary(window, cx);
        }
    }

    pub(super) fn ensure_summary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.kube_source() else {
            return;
        };
        if self
            .summary_session
            .as_ref()
            .is_some_and(|session| session.core.identity().connection() == source.id)
        {
            return;
        }
        self.stop_summary();
        let identity = SessionIdentity::new(source.id.clone(), self.summary_epoch);
        let session = Session::new(identity);
        let target = Target {
            epoch: self.summary_epoch,
            context: source.context.clone(),
            node: String::new(),
            address: source.id,
        };
        self.summary_session = Some(SummarySession {
            core: session.clone(),
            target,
            fixture_at: chrono::Utc::now(),
            fixture_clock: cx.background_executor().now(),
            applied_revision: 0,
        });
        self.sync_unread_health(cx);
        if matches!(source.access, KubeAccess::Example) {
            if self.fixture_hold {
                return;
            }
            self.publish_fixture(window, cx);
            return;
        }
        if let Some(super::kubernetes_only::KubernetesOnly {
            connection: super::kubernetes_only::KubeConnection::Connected { version },
            ..
        }) = &self.kubernetes_only
        {
            session.version(0, version.clone(), chrono::Utc::now());
        }
        let mut publications = session.publications();
        let (sender, mut receiver) = tokio::sync::watch::channel(None);
        self.summary_job = Some(OwnedJob::new(self.runtime.spawn(async move {
            let mut backoff = Duration::from_secs(1);
            let client = loop {
                match tokio::time::timeout(Duration::from_secs(45), source.access.client()).await {
                    Ok(Ok(client)) => break client,
                    result => {
                        let failure = ObservationFailure::Read(if result.is_err() {
                            FailureKind::Timeout
                        } else {
                            FailureKind::Config
                        });
                        for source in Source::WATCHED.into_iter().chain([Source::Version]) {
                            session.fail(session.generation(), source, failure.clone());
                        }
                        let publication = session.derive(chrono::Utc::now());
                        let health = WorkloadData::from_outcome(&publication.summary.workloads);
                        session.publish(publication.clone());
                        sender.send_replace(Some((publication, health)));
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            };
            let driver = session.run(client);
            tokio::pin!(driver);
            loop {
                tokio::select! {
                    _ = &mut driver => break,
                    changed = publications.changed() => {
                        if changed.is_err() { break; }
                        let publication = publications.borrow_and_update().clone();
                        if let Some(publication) = publication {
                            let health = WorkloadData::from_outcome(&publication.summary.workloads);
                            sender.send_replace(Some((publication, health)));
                        }
                    }
                }
            }
        })));
        self.summary_task = Some(cx.spawn_in(window, async move |this, cx| {
            while receiver.changed().await.is_ok() {
                let answer = receiver.borrow_and_update().clone();
                if let Some((publication, health)) = answer
                    && this
                        .update_in(cx, |view, window, cx| {
                            view.apply_summary(publication, health, window, cx)
                        })
                        .is_err()
                {
                    break;
                }
            }
        }));
        self.deliver_summary_nodes(cx);
    }

    /// Seeds the example session, publishes what it derives at its own
    /// time and keeps watching it.
    fn publish_fixture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.summary_session.as_ref().unwrap();
        example::seed_summary(
            &session.core,
            &session.target.context,
            session.fixture_at.timestamp(),
        );
        let publication = session
            .core
            .derive(session.fixture_now(cx.background_executor().now()));
        session.core.publish(publication.clone());
        self.apply_summary(
            publication.clone(),
            WorkloadData::from_outcome(&publication.summary.workloads),
            window,
            cx,
        );
        self.watch_fixture_summary(window, cx);
    }

    fn watch_fixture_summary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.summary_session.as_ref().unwrap();
        let core = session.core.clone();
        let (at, clock) = (session.fixture_at, session.fixture_clock);
        let mut changes = core.changes();
        self.summary_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                let now = fixture_time(at, clock, cx.background_executor().now());
                let expiry = core.next_warning_change(now);
                let dirty = changes.changed();
                let timer = cx
                    .background_executor()
                    .timer(expiry.unwrap_or(Duration::from_secs(24 * 60 * 60)));
                match futures::future::select(Box::pin(dirty), Box::pin(timer)).await {
                    futures::future::Either::Left((Err(_), _)) => break,
                    futures::future::Either::Left((Ok(()), _)) => {
                        cx.background_executor().timer(DEBOUNCE).await
                    }
                    futures::future::Either::Right(_) => {}
                }
                changes.borrow_and_update();
                let now = fixture_time(at, clock, cx.background_executor().now());
                let publication = core.derive(now);
                core.publish(publication.clone());
                let health = WorkloadData::from_outcome(&publication.summary.workloads);
                if this
                    .update_in(cx, |view, window, cx| {
                        view.apply_summary(publication, health, window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn apply_summary(
        &mut self,
        publication: Arc<Publication>,
        health: Result<Arc<WorkloadData>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = &mut self.summary_session else {
            return;
        };
        if &publication.identity != session.core.identity()
            || publication.generation != session.core.generation()
            || session.target.epoch != self.summary_epoch
            || publication.revision <= session.applied_revision
        {
            return;
        }
        session.applied_revision = publication.revision;
        crate::perf::value(
            "summary.live_sources",
            Source::WATCHED
                .iter()
                .filter(|source| publication.summary.observations[*source].is_current())
                .count() as f64,
        );
        if let Some(changed_at) = publication.changed_at {
            crate::perf::value("summary.lag", changed_at.elapsed().as_secs_f64() * 1000.);
        }
        crate::perf::value(
            "summary.staging_peak_mib",
            publication.staging_peak as f64 / (1024. * 1024.),
        );
        crate::desktop::probe::hit("summary.apply");
        let _span = crate::perf::span("summary.apply");
        crate::perf::value(
            "summary.tokio",
            publication.derive_duration.as_secs_f64() * 1000.,
        );
        crate::perf::value(
            "summary.retained_mib",
            publication.retained_bytes as f64 / (1024. * 1024.),
        );
        if publication.pod_count > 0 {
            crate::perf::value(
                "summary.bytes_per_pod",
                publication.pod_bytes as f64 / publication.pod_count as f64,
            );
        }
        let request = self.kubernetes_summary.begin(self.applied.clone());
        self.kubernetes_summary
            .apply(&request, Ok(publication.summary.clone()));
        self.summary_health = Some(health.clone());
        self.deliver_workloads(health, cx);
        self.rebuild_joined_nodes(cx);
        self.push_node_rows(cx);
        self.prepare_context_display(window, cx);
        self.deliver_summary_nodes(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests;
