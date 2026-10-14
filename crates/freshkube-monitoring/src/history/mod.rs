//! CPU and memory over the last hour for one pod or node, drawn by the
//! Monitoring panels, in a pod's Overview and the node pane.
//!
//! It shows only while the context has a Prometheus: the one the Monitoring
//! page confirmed, or the Service it remembers for the context. Without
//! one it draws nothing and reads nothing, so those screens look as they
//! did. It reads only while shown, again each minute, and dropping the
//! subject, the source or the screen drops the reads.
#[cfg(test)]
mod tests;

use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use freshkube_core::cluster_source::ClusterAccess;
use freshkube_core::job::spawn_job;
use freshkube_core::monitoring::{
    Endpoint, ErrorKind, ExampleSource, PanelResult, Prometheus, PrometheusService, QueryError,
    Source, cluster_client,
    history::{self, Subject},
    model::{data::QueryContext, time::TimeWindow},
    prometheus::Variables,
};
use freshkube_core::snapshot::Request;
use freshkube_reads::{Loader, OnShow};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AppContext, Context, Entity, EntityId, IntoElement, Render, SharedString, StyleRefinement,
    Subscription, Task, TestSupportExt, WeakEntity, Window, div,
};
use tokio::runtime::Handle;

use super::panel::{Linked, PanelEvent, PanelView};
use crate::page::{MonitoringEvent, MonitoringPage};
use crate::ui::{self, dp};

/// The window drawn, and how often it is read again while shown.
const SPAN: u64 = 3600;
const EVERY: Duration = Duration::from_secs(60);
const POINTS: usize = 120;
const DEADLINE: Duration = Duration::from_secs(30);
const TIMED_OUT: &str = "Prometheus didn't answer in time";
const PANEL_NAMES: [&str; 2] = ["cpu", "memory"];

/// Where the history comes from, as the Monitoring page knows it.
#[derive(Clone)]
pub struct HistorySource {
    /// The shell's connection id, so another cluster's answers never show.
    pub(crate) id: String,
    pub(crate) kind: HistoryKind,
    /// Counts the page's rebuilt clients, so a Ready source built again
    /// reads through the new one.
    pub(crate) client: u64,
    /// The one bounded recovery owner, without keeping its page alive.
    pub(crate) owner: Option<WeakEntity<MonitoringPage>>,
}

#[derive(Clone)]
pub(crate) enum HistoryKind {
    Example,
    /// Confirmed on the Monitoring page.
    Ready(Prometheus),
    /// Remembered for the context and not looked for again yet.
    Remembered {
        access: ClusterAccess,
        service: PrometheusService,
    },
}

impl HistorySource {
    /// What tells two sources apart: the connection, the endpoint, whether
    /// it was confirmed and its client.
    pub(crate) fn key(&self) -> (String, Option<Endpoint>, bool, u64, Option<EntityId>) {
        let id = self.id.clone();
        let owner = self.owner.as_ref().map(|owner| owner.entity_id());
        match &self.kind {
            HistoryKind::Example => (id, None, true, self.client, owner),
            HistoryKind::Ready(prometheus) => (
                id,
                Some(prometheus.endpoint().clone()),
                true,
                self.client,
                owner,
            ),
            HistoryKind::Remembered { service, .. } => (
                id,
                Some(Endpoint::Service(service.clone())),
                false,
                self.client,
                owner,
            ),
        }
    }

    fn example(&self) -> bool {
        matches!(self.kind, HistoryKind::Example)
    }

    /// The source to query for `subject`.
    async fn source(self, subject: Subject) -> Result<Source, QueryError> {
        match self.kind {
            HistoryKind::Example => Ok(Source::Example(ExampleSource::new(&subject.seed()))),
            HistoryKind::Ready(prometheus) => Ok(Source::Prometheus(prometheus)),
            HistoryKind::Remembered { access, service } => {
                let client = cluster_client(&access).await?;
                Ok(Source::Prometheus(Prometheus::new(client, service)))
            }
        }
    }
}

/// The same subject through a replaced client is a different read.
#[derive(Clone, PartialEq, Eq)]
struct HistoryIdentity {
    source: (String, Option<Endpoint>, bool, u64, Option<EntityId>),
    subject: Subject,
    panel: &'static str,
}

pub struct HistoryView {
    runtime: Handle,
    /// Tells this view's panels apart from another's, as
    /// `monitoring-panel-<prefix>-cpu`.
    prefix: &'static str,
    source: Option<HistorySource>,
    subject: Option<Subject>,
    visible: bool,
    /// The CPU and memory panels for the subject.
    panels: Vec<Entity<PanelView>>,
    /// One panel's cursor on the other.
    linked: Linked,
    /// When the panels were last asked, in Unix seconds.
    asked: Option<i64>,
    /// Persistent completion owners: replacement panels never reuse an
    /// earlier read's token, even when the source or subject returns.
    reads: [Loader<(), HistoryIdentity>; 2],
    timer: Option<Task<()>>,
    /// Connection changes reach history without a shell or dashboard read.
    _connection: Option<Subscription>,
    _release: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
    /// Unix seconds now. Tests fix it, so example data is the same each run.
    now: fn() -> i64,
}

impl HistoryView {
    pub fn new(runtime: Handle, prefix: &'static str) -> Self {
        Self {
            runtime,
            prefix,
            source: None,
            subject: None,
            visible: false,
            panels: Vec::new(),
            linked: Linked::default(),
            asked: None,
            reads: std::array::from_fn(|_| Loader::default()),
            timer: None,
            _connection: None,
            _release: None,
            _subscriptions: Vec::new(),
            now: || chrono::Utc::now().timestamp(),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_now(&mut self, now: fn() -> i64) {
        self.now = now;
    }

    /// Whether there is anything to draw: a source and a subject.
    pub fn shows(&self) -> bool {
        self.source.is_some() && !self.panels.is_empty()
    }

    /// The Monitoring page's Prometheus, or None when it has none.
    pub fn set_source(&mut self, source: Option<HistorySource>, cx: &mut Context<Self>) {
        let key = |source: &Option<HistorySource>| source.as_ref().map(HistorySource::key);
        if key(&self.source) == key(&source) {
            return;
        }
        let was = self.source.as_ref().map(|source| source.id.clone());
        let now = source.as_ref().map(|source| source.id.clone());
        self.leave_connection(cx);
        self.source = source;
        self.watch_connection(cx);
        if was != now {
            // Another cluster's answers never show as this one's.
            self.rebuild(cx);
        } else {
            self.forget(cx);
        }
        self.sync_consumer(cx);
        self.read(cx);
        cx.notify();
    }

    /// The pod or node to show, or None.
    pub fn set_subject(&mut self, subject: Option<Subject>, cx: &mut Context<Self>) {
        if self.subject == subject {
            return;
        }
        self.subject = subject;
        self.rebuild(cx);
        self.sync_consumer(cx);
        self.read(cx);
        cx.notify();
    }

    /// Whether the screen shows the history now. Only a shown view reads.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.sync_consumer(cx);
        // Freshness stays with `asked`, including failed answers. The
        // loader's success-time threshold would change that rule.
        let cut = self.reads.iter().any(Loader::is_loading);
        for read in &mut self.reads {
            read.set_visible(visible, OnShow::Never, cx);
        }
        if visible {
            self.read(cx);
        } else {
            // A read cut short is asked again on showing; a recent answer
            // is kept.
            if cut {
                self.asked = None;
            }
            self.timer = None;
        }
    }

    /// The screen's Refresh.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.visible
            && self.subject.is_some()
            && let Some(source) = &self.source
            && let Some(owner) = &source.owner
        {
            _ = owner.update(cx, |page, _| page.retry_history(source));
        }
        self.asked = None;
        self.read(cx);
    }

    /// Drops what was asked; the panels keep their answers.
    fn forget(&mut self, cx: &mut Context<Self>) {
        for read in &mut self.reads {
            read.set_visible(false, OnShow::Never, cx);
        }
        self.asked = None;
    }

    fn leave_connection(&mut self, cx: &mut Context<Self>) {
        if let Some(owner) = self
            .source
            .as_ref()
            .and_then(|source| source.owner.as_ref())
        {
            let id = cx.entity_id();
            _ = owner.update(cx, |page, _| page.hide_history(id));
        }
        self._connection = None;
    }

    fn watch_connection(&mut self, cx: &mut Context<Self>) {
        let Some(owner) = self
            .source
            .as_ref()
            .and_then(|source| source.owner.as_ref())
            .and_then(WeakEntity::upgrade)
        else {
            return;
        };
        self._connection = Some(
            cx.subscribe(&owner, |this, page, event: &MonitoringEvent, cx| {
                if *event == MonitoringEvent::History {
                    let source = page.read(cx).history();
                    this.set_source(source, cx);
                }
            }),
        );
        if self._release.is_none() {
            let id = cx.entity_id();
            self._release = Some(cx.on_release(move |this, cx| {
                if let Some(owner) = this
                    .source
                    .as_ref()
                    .and_then(|source| source.owner.as_ref())
                {
                    _ = owner.update(cx, |page, _| page.hide_history(id));
                }
            }));
        }
    }

    fn sync_consumer(&self, cx: &mut Context<Self>) {
        let Some(source) = &self.source else {
            return;
        };
        let Some(owner) = &source.owner else {
            return;
        };
        let history = cx.weak_entity();
        let visible = self.visible && self.subject.is_some();
        _ = owner.update(cx, |page, _| page.show_history(history, source, visible));
    }

    /// New panels for the subject, with nothing answered.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.forget(cx);
        self.panels.clear();
        self.linked = Linked::default();
        self._subscriptions.clear();
        let (Some(_), Some(subject)) = (&self.source, &self.subject) else {
            return;
        };
        let dashboard = history::dashboard(subject);
        self.panels = dashboard
            .panels
            .iter()
            .zip(PANEL_NAMES)
            .map(|(spec, name)| {
                let id = format!("{}-{name}", self.prefix);
                let spec = Rc::new(spec.clone());
                cx.new(|cx| PanelView::new(id, spec, cx))
            })
            .collect();
        // One chart's cursor shows on the other, drawn here so that the
        // other panel doesn't redraw.
        self._subscriptions = self
            .panels
            .iter()
            .map(|panel| {
                cx.subscribe(panel, |this, from, event: &PanelEvent, cx| {
                    let PanelEvent::Cursor(time) = event;
                    this.linked.show(from.entity_id(), *time, &this.panels, cx);
                    cx.notify();
                })
            })
            .collect();
    }

    /// Asks both panels, unless they were asked within the minute, and
    /// asks again each minute while shown.
    fn read(&mut self, cx: &mut Context<Self>) {
        let (true, Some(source), Some(subject)) =
            (self.visible, self.source.clone(), self.subject.clone())
        else {
            return;
        };
        if self.panels.is_empty() {
            return;
        }
        let now = (self.now)();
        let fresh = self
            .asked
            .is_some_and(|asked| now - asked < EVERY.as_secs() as i64);
        if !fresh {
            self.ask(source, subject, now, cx);
        }
        if self.timer.is_none() {
            self.timer = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(EVERY).await;
                    let asked = this.update(cx, |this, cx| {
                        this.asked = None;
                        this.read(cx);
                    });
                    if asked.is_err() {
                        break;
                    }
                }
            }));
        }
    }

    fn ask(&mut self, source: HistorySource, subject: Subject, now: i64, cx: &mut Context<Self>) {
        let window = TimeWindow::new(now, SPAN, POINTS);
        let dashboard = history::dashboard(&subject);
        self.asked = Some(now);
        freshkube_probe::probe::hit("monitoring.history.ask");
        for (index, (spec, name)) in dashboard.panels.into_iter().zip(PANEL_NAMES).enumerate() {
            let identity = HistoryIdentity {
                source: source.key(),
                subject: subject.clone(),
                panel: name,
            };
            let (source, subject) = (source.clone(), subject.clone());
            let example = source.example();
            let work = async move {
                let source = source.source(subject).await?;
                let context = QueryContext {
                    window,
                    variables: Default::default(),
                };
                source
                    .query_panel(&spec, &context, &Variables::default())
                    .await
            };
            self.run_panel(index, identity, window, example, work, cx);
        }
    }

    /// The panels own their data; the loader owns only completion and the
    /// work. Keep the typed error until the accepted answer reaches them.
    fn run_panel(
        &mut self,
        index: usize,
        identity: HistoryIdentity,
        window: TimeWindow,
        example: bool,
        work: impl Future<Output = Result<PanelResult, QueryError>> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.reads[index].set_visible(true, OnShow::Never, cx);
        let request = self.reads[index].start(identity.clone());
        let done = move |this: &mut Self, result, cx: &mut Context<Self>| {
            if this.visible
                && this
                    .source
                    .as_ref()
                    .is_some_and(|source| source.key() == identity.source)
                && this.subject.as_ref() == Some(&identity.subject)
            {
                this.land_panel(index, &request, window, result, cx);
            }
        };
        let (job, task) = if example {
            let work = cx.background_spawn(work);
            let task = cx.spawn(async move |this, cx| {
                let result = work.await;
                _ = this.update(cx, |this, cx| done(this, result, cx));
            });
            (None, task)
        } else {
            let (job, receiver) = spawn_job(
                &self.runtime,
                DEADLINE,
                QueryError::new(ErrorKind::TimedOut, TIMED_OUT),
                work,
            );
            let task = cx.spawn(async move |this, cx| {
                let Ok(result) = receiver.await else {
                    return;
                };
                _ = this.update(cx, |this, cx| done(this, result, cx));
            });
            (Some(job), task)
        };
        self.reads[index].run(job, task);
    }

    fn land_panel(
        &mut self,
        index: usize,
        request: &Request<HistoryIdentity>,
        window: TimeWindow,
        result: Result<PanelResult, QueryError>,
        cx: &mut Context<Self>,
    ) {
        let completion = result
            .as_ref()
            .map(|_| ())
            .map_err(|error| error.message.clone());
        if self.reads[index].land(request, completion, cx) {
            self.answered(index, window, result, cx);
        }
    }

    fn answered(
        &mut self,
        index: usize,
        window: TimeWindow,
        result: Result<PanelResult, QueryError>,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.panels.get(index) else {
            return;
        };
        let error = result.as_ref().err().cloned();
        panel.update(cx, |panel, cx| match result {
            Ok(result) => panel.set_result(result, window, cx),
            Err(error) => panel.set_error(&error, cx),
        });
        if self.linked.refresh(&self.panels, cx) {
            cx.notify();
        }
        if let Some(error) = error
            && let Some(source) = &self.source
            && let Some(owner) = &source.owner
        {
            _ = owner.update(cx, |page, cx| page.history_failed(source, &error, cx));
        }
    }

    /// Debug fixture checks: answers both panels from example data now, so
    /// a screenshot taken before the executor runs shows them.
    #[cfg(any(debug_assertions, feature = "stress"))]
    pub fn answer_example_now(&mut self, cx: &mut Context<Self>) {
        let (Some(source), Some(subject)) = (&self.source, self.subject.clone()) else {
            return;
        };
        if !source.example() {
            return;
        }
        self.forget(cx);
        let now = (self.now)();
        let window = TimeWindow::new(now, SPAN, POINTS);
        let example = ExampleSource::new(&subject.seed());
        let context = QueryContext {
            window,
            variables: Default::default(),
        };
        self.asked = Some(now);
        for (panel, spec) in self.panels.iter().zip(history::dashboard(&subject).panels) {
            let result = example.query_panel(&spec, &context, &Variables::default());
            panel.update(cx, |panel, cx| match result {
                Ok(result) => panel.set_result(result, window, cx),
                Err(error) => panel.set_error(&error, cx),
            });
        }
    }

    /// Whether a read is in flight, for tests.
    #[cfg(any(test, feature = "testing"))]
    pub fn reading(&self) -> bool {
        self.reads.iter().any(Loader::is_loading)
    }

    #[cfg(test)]
    pub(crate) fn panels(&self) -> &[Entity<PanelView>] {
        &self.panels
    }
}

impl Render for HistoryView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.shows() {
            return div().into_any_element();
        }
        let label: SharedString = match self.source.as_ref().map(|source| &source.kind) {
            Some(HistoryKind::Example) => "Example data".into(),
            Some(HistoryKind::Ready(prometheus)) => prometheus.endpoint().label().into(),
            Some(HistoryKind::Remembered { service, .. }) => service.label().into(),
            None => SharedString::default(),
        };
        let p = crate::palette::palette(cx);
        v_flex()
            .id(SharedString::from(format!("{}-history", self.prefix)))
            .gap(dp(8.))
            .child(
                h_flex()
                    .gap(dp(8.))
                    .child(ui::caption("Last hour", cx))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(ui::MONO_FONT)
                            .text_size(dp(11.))
                            .text_color(p.muted)
                            .child(label),
                    ),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(dp(12.))
                    .children(self.panels.iter().enumerate().map(|(index, panel)| {
                        div()
                            .relative()
                            .flex_1()
                            .min_w(dp(180.))
                            .h(dp(184.))
                            .child(panel.clone().cached(StyleRefinement::default().size_full()))
                            .children(panel.read(cx).cursor_overlay())
                            .children(self.linked.element(index))
                    })),
            )
            .test_support()
            .into_any_element()
    }
}
