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
use freshkube_reads::Pace;
use gpui_kit::{Context, Window};

use super::Pilot;
use crate::{
    backend::Target,
    resources::{ClusterAccess, example},
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
        self.pages.health.update(cx, |screen, cx| {
            screen.apply_summary(
                self.applied.context.as_deref().unwrap_or_default(),
                data,
                cx,
            );
        });
    }

    pub(super) fn deliver_summary_nodes(&self, cx: &mut Context<Self>) {
        let subscription = (self.navigation.page == super::Page::Lifecycle)
            .then_some(self.switch.registry.active().summary_session.as_ref())
            .flatten()
            .and_then(|session| {
                session.core.subscribe(SubscriptionKey::summary(
                    session.core.identity().clone(),
                    Source::Nodes,
                ))
            });
        self.pages
            .lifecycle
            .update(cx, |screen, cx| screen.observe_nodes(subscription, cx));
    }

    pub(super) fn stop_summary(&mut self) {
        self.switch.registry.summary_watch.stop();
        let session = self.switch.registry.active_mut();
        session.summary_fixture = None;
        session.summary_session = None;
    }

    /// Manual Refresh relists every source. The automatic Talos cycle calls
    /// ensure_summary instead, which leaves an existing session alone.
    pub(super) fn refresh_summary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = &self.switch.registry.active().summary_session {
            session.core.relist();
            if self.mode().is_example() && !self.mode().holds_talos() {
                self.publish_fixture(window, cx);
            }
        } else {
            self.ensure_summary(window, cx);
        }
    }

    pub(super) fn ensure_summary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_summary_session(window, cx);
        // A link into this entry waits for its source, which is known here.
        self.open_pending_link(window, cx);
    }

    fn ensure_summary_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source().kube else {
            return;
        };
        if self
            .switch
            .registry
            .active()
            .summary_session
            .as_ref()
            .is_some_and(|session| session.core.identity().connection() == source.id)
        {
            return;
        }
        self.stop_summary();
        // The reducer keeps its applied connection identity. Capture the
        // shell's existing target revision once; later node selections do
        // not change this session. Watch owns the driver's request sequence.
        let identity = SessionIdentity::new(source.id.clone(), self.epoch);
        let session = Session::new(identity);
        let target = Target {
            epoch: self.epoch,
            context: source.context.clone(),
            node: String::new(),
            address: source.id,
        };
        self.switch.registry.active_mut().summary_session = Some(SummarySession {
            core: session.clone(),
            target,
            fixture_at: chrono::Utc::now(),
            fixture_clock: cx.background_executor().now(),
            applied_revision: 0,
        });
        self.sync_unread_health(cx);
        if matches!(source.access, ClusterAccess::Example(_)) {
            self.start_fixture_summary(window, cx);
            return;
        }
        if let Some(super::kubernetes_only::KubernetesOnly {
            connection: super::kubernetes_only::KubeConnection::Connected { version },
            ..
        }) = self.mode().kubernetes_only()
        {
            session.version(0, version.clone(), chrono::Utc::now());
        }
        let runtime = self.runtime.clone();
        self.switch.registry.summary_watch.start(
            (source.context, session.identity().connection().to_owned()),
            &runtime,
            SUMMARY_PACE,
            move |sender| run_summary_driver(session, source.access, sender),
            |view: &mut Self| &mut view.switch.registry.summary_watch,
            Self::deliver_summary,
            cx,
        );
        self.deliver_summary_nodes(cx);
    }

    /// The example session: nothing publishes while the hold keeps its data
    /// back, otherwise it is seeded and watched.
    fn start_fixture_summary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode().holds_talos() {
            return;
        }
        self.publish_fixture(window, cx);
    }

    /// Hands the latest publication to the shell in its window. The watch
    /// rejects replaced streams; apply_summary retains the reducer's checks
    /// when this deferred window update runs.
    fn deliver_summary(&mut self, batch: Vec<SummaryFeed>, cx: &mut Context<Self>) {
        let Some(feed) = batch.last() else {
            return;
        };
        let Some((publication, health)) = feed.borrow().clone() else {
            return;
        };
        let this = cx.weak_entity();
        let handle = self.window;
        cx.defer(move |cx| {
            _ = handle.update(cx, |_, window, cx| {
                _ = this.update(cx, |view, cx| {
                    view.apply_summary(publication, health, window, cx);
                });
            });
        });
    }

    /// Seeds the example session, publishes what it derives at its own
    /// time and keeps watching it.
    fn publish_fixture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self
            .switch
            .registry
            .active()
            .summary_session
            .as_ref()
            .unwrap();
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
        let session = self
            .switch
            .registry
            .active()
            .summary_session
            .as_ref()
            .unwrap();
        let core = session.core.clone();
        let (at, clock) = (session.fixture_at, session.fixture_clock);
        let mut changes = core.changes();
        let (latest, feed) = tokio::sync::watch::channel(None);
        let sender = self.switch.registry.summary_watch.open(
            (
                session.target.context.clone(),
                core.identity().connection().to_owned(),
            ),
            SUMMARY_PACE,
            |view: &mut Self| &mut view.switch.registry.summary_watch,
            Self::deliver_summary,
            cx,
        );
        self.switch.registry.active_mut().summary_fixture =
            Some(cx.spawn_in(window, async move |_, cx| {
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
                    latest.send_replace(Some((publication, health)));
                    if sender.is_closed() {
                        break;
                    }
                    // A full notification channel already points at this
                    // latest value, just as the live driver's does.
                    _ = sender.try_send(feed.clone());
                }
            }));
    }

    pub(super) fn apply_summary(
        &mut self,
        publication: Arc<Publication>,
        health: Result<Arc<WorkloadData>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self.switch.registry.active_mut();
        let Some(session) = &mut current.summary_session else {
            return;
        };
        if &publication.identity != session.core.identity()
            || publication.generation != session.core.generation()
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
        let request = self
            .switch
            .registry
            .active_mut()
            .kubernetes_summary
            .begin(self.applied.clone());
        self.switch
            .registry
            .active_mut()
            .kubernetes_summary
            .apply(&request, Ok(publication.summary.clone()));
        if self.switch.restored_taken.take().is_some() {
            self.switch
                .age_label
                .update(cx, |label, cx| label.set(None, cx));
        }
        self.switch.registry.active_mut().summary_health = Some(health.clone());
        self.deliver_workloads(health, cx);
        self.rebuild_joined_nodes(cx);
        self.pages.push_node_facts(cx);
        self.prepare_context_display(window, cx);
        self.deliver_summary_nodes(cx);
        cx.notify();
    }
}

/// What the driver sends the shell: a publication and the workload health
/// derived from it.
type SummaryAnswer = Option<(Arc<Publication>, Result<Arc<WorkloadData>, String>)>;

/// Notifications point at one latest value, preserving the driver's
/// coalescing watch channel without blocking the reducer on UI delivery.
pub(super) type SummaryFeed = tokio::sync::watch::Receiver<SummaryAnswer>;

const SUMMARY_PACE: Pace = Pace {
    capacity: 1,
    most: 1,
    every: Duration::ZERO,
};

/// Runs one cluster's observation session on Tokio: waits for a client,
/// drives the reflectors and sends each publication down the channel.
async fn run_summary_driver(
    session: Session,
    access: ClusterAccess,
    sender: tokio::sync::mpsc::Sender<SummaryFeed>,
) {
    let mut publications = session.publications();
    let (latest, feed) = tokio::sync::watch::channel(None);
    let mut backoff = Duration::from_secs(1);
    let client = loop {
        match tokio::time::timeout(Duration::from_secs(45), access.client()).await {
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
                latest.send_replace(Some((publication, health)));
                _ = sender.try_send(feed.clone());
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
                    latest.send_replace(Some((publication, health)));
                    _ = sender.try_send(feed.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
