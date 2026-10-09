//! The page's read: example data's change, or the live one core's
//! `delivery::change::live` reads on the open connection. It reads while
//! the page shows: on opening, on showing again once what it has is older
//! than [`FRESH_FOR`], and on Refresh. Hiding drops the read in flight; an
//! answer for another change, or a superseded one, never lands. A failed
//! refresh keeps the last answer, marked stale.
use std::time::Duration;

use chrono::{DateTime, Utc};
use freshkube_core::delivery::change::live::{self, Place};
use freshkube_core::delivery::change::{self as delivery_change, Change};
use freshkube_core::delivery::read::ReadOnlyClient;
use freshkube_core::snapshot::Request;

use super::*;
use crate::backend;
use crate::resources::KubeAccess;

/// A change read this recent is shown again without reading.
const FRESH_FOR: Duration = Duration::from_secs(30);
/// How long one change's read may take: a few dozen lists.
const DEADLINE: Duration = Duration::from_secs(60);

/// Where the page's change comes from.
#[derive(Clone)]
pub(in crate::applications) enum Fetch {
    /// Example data's, its times counted from `now`; it answers after
    /// `delay`, which only tests set, so a read stays in flight.
    Example {
        project: String,
        stage: String,
        now: DateTime<Utc>,
        delay: Duration,
    },
    /// The open connection's.
    Live {
        runtime: tokio::runtime::Handle,
        access: KubeAccess,
        place: Box<Place>,
    },
}

impl Fetch {
    /// Which change this is: an answer for another is never shown as its.
    fn identity(&self) -> String {
        match self {
            Self::Example { project, stage, .. } => format!("example/{project}/{stage}"),
            Self::Live { place, .. } => {
                format!("{}/{}/{}", place.cluster, place.project, place.freight)
            }
        }
    }

    /// The change before anything was read: its project and Freight alone.
    pub(super) fn unread(&self) -> Change {
        let (project, freight, cluster) = match self {
            Self::Example { project, stage, .. } => (
                project.clone(),
                delivery_change::example::freight_of(project, stage)
                    .unwrap_or_default()
                    .to_owned(),
                String::new(),
            ),
            Self::Live { place, .. } => (
                place.project.clone(),
                place.freight.clone(),
                place.label.clone(),
            ),
        };
        Change {
            project,
            freight,
            kargo_cluster: cluster,
            observed_at: DateTime::<Utc>::MIN_UTC,
            page: Err("Nothing is read yet".into()),
            groups: Vec::new(),
            stages: Vec::new(),
            hops: Vec::new(),
        }
    }
}

impl ChangePage {
    /// Reads the change again, whatever is in flight. With a window at
    /// hand, example data answers at once.
    pub(super) fn read(&mut self, window: Option<&Window>, cx: &mut Context<Self>) {
        self.stop();
        let request = self.snapshot.begin(self.fetch.identity());
        self.pending = true;
        match self.fetch.clone() {
            Fetch::Example {
                project,
                stage,
                now,
                delay,
            } => {
                let result = delivery_change::example::for_stage(&project, &stage, now)
                    .ok_or_else(|| format!("Example data has no change for Stage {stage}"));
                match window {
                    Some(window) if delay.is_zero() => self.answer(&request, result, window, cx),
                    _ => {
                        let handle = self.window;
                        self.task = Some(cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(delay).await;
                            deliver(this, handle, request, result, cx);
                        }));
                    }
                }
            }
            Fetch::Live {
                runtime,
                access,
                place,
            } => {
                let (job, receiver) = backend::spawn_job(
                    &runtime,
                    DEADLINE,
                    "Reading the change timed out".into(),
                    async move {
                        let client = access.client().await?;
                        let reader = ReadOnlyClient::new(client);
                        live::read(&reader, &place, Utc::now()).await
                    },
                );
                self.job = Some(job);
                let handle = self.window;
                self.task = Some(cx.spawn(async move |this, cx| {
                    let result = receiver
                        .await
                        .unwrap_or_else(|_| Err("Reading the change stopped".into()));
                    deliver(this, handle, request, result, cx);
                }));
            }
        }
        cx.notify();
    }

    /// An answer, published only while it is the one in flight. The first
    /// selects the Stage the page was opened on.
    fn answer(
        &mut self,
        request: &Request<String>,
        result: Result<Change, String>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let answered = result.is_ok();
        if !self.pending || !self.snapshot.apply(request, result) {
            return;
        }
        self.pending = false;
        self.job = None;
        self.task = None;
        if answered {
            self.read_at = Some(cx.background_executor().now());
        }
        self.stale = self.snapshot.data().and(self.snapshot.error()).map(|why| {
            let text = format!("Showing the last read. {why}");
            let label = format!("Couldn't read again: {text}");
            (text.into(), label.into())
        });
        if let Some(change) = self.snapshot.data().filter(|_| answered).cloned() {
            self.show(change);
            if let Some(stage) = self.stage.take() {
                self.select_stage(&stage, window, cx);
            }
        }
        cx.notify();
    }

    /// Drops the read in flight; what was read stays.
    fn stop(&mut self) {
        self.pending = false;
        self.job = None;
        self.task = None;
    }

    /// Shown, the page reads unless what it has is fresh; hidden, it drops
    /// the read in flight.
    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        if !visible {
            self.stop();
            cx.notify();
            return;
        }
        let now = cx.background_executor().now();
        let fresh = self.snapshot.data().is_some()
            && !self.snapshot.is_stale()
            && self
                .read_at
                .is_some_and(|at| now.saturating_duration_since(at) < FRESH_FOR);
        if !fresh {
            self.read(None, cx);
        }
    }

    /// Refresh: reads again while shown.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.visible {
            self.read(None, cx);
        }
    }

    /// Why the change couldn't be read, when nothing was.
    pub(super) fn failure(&self) -> Option<&str> {
        self.snapshot
            .data()
            .is_none()
            .then(|| self.snapshot.error())
            .flatten()
    }

    /// The loading rows, until the first answer.
    pub(super) fn loading_rows(&self) -> Option<&kit::LoadingRows> {
        (self.pending && self.snapshot.data().is_none() && self.snapshot.error().is_none())
            .then_some(&self.loading)
    }

    /// The motion over the loading rows, which the shell mounts beside the
    /// cached page.
    pub(crate) fn loading_motion(&self) -> Option<Entity<kit::LoadingMotion>> {
        self.loading_rows()
            .is_some()
            .then(|| self.loading_motion.clone())
    }
}

/// Hands an answer to the page in its window, if both are still there.
fn deliver(
    this: WeakEntity<ChangePage>,
    handle: AnyWindowHandle,
    request: Request<String>,
    result: Result<Change, String>,
    cx: &mut AsyncApp,
) {
    _ = cx.update_window(handle, |_, window, cx| {
        _ = this.update(cx, |page, cx| page.answer(&request, result, window, cx));
    });
}

#[cfg(test)]
impl ChangePage {
    pub(crate) fn is_reading(&self) -> bool {
        self.pending
    }

    /// Whether an answer was shown.
    pub(crate) fn answered(&self) -> bool {
        self.snapshot.data().is_some()
    }

    /// The selection's actions, as the Inspector's footer labels them.
    pub(crate) fn action_labels(&self) -> Vec<String> {
        self.detail
            .as_ref()
            .map(|detail| detail.action_labels())
            .unwrap_or_default()
    }

    /// Why the selection's action labelled `label` is greyed out, if it is.
    pub(crate) fn action_why(&self, label: &str) -> Option<String> {
        self.detail.as_ref()?.action_why(label)
    }

    /// The change shown.
    pub(crate) fn shown(&self) -> &Change {
        &self.change
    }

    /// An answer, as a read brings it.
    pub(crate) fn read_answer(&mut self, change: Change, window: &Window, cx: &mut Context<Self>) {
        let request = self.snapshot.begin(self.fetch.identity());
        self.pending = true;
        self.answer(&request, Ok(change), window, cx);
    }

    /// A read that failed, as a refused or unreachable cluster answers.
    pub(crate) fn fail_read(&mut self, why: &str, window: &Window, cx: &mut Context<Self>) {
        let request = self.snapshot.begin(self.fetch.identity());
        self.pending = true;
        self.answer(&request, Err(why.into()), window, cx);
    }
}
