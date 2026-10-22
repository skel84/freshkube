//! The toast for a submitted write: one toast from sending to the end, with
//! an observation bar while the cluster is being watched, and Undo with its
//! time left on the button. Accepted and observed are told apart in words;
//! only an observed success may go away by itself, and Not observed and
//! Unknown stay until closed.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use freshkube_core::write::{Event, Finished, Headline, Runner, Status, UNDO_WINDOW, UndoOffer};
use gpui_kit::component::{
    Sizable, WindowExt, button::Button, h_flex, notification::Notification, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;

use super::{WriteRequest, start_prepared};
use crate::backend::spawn_job;
use crate::palette::palette;
use crate::ui::{self, Tone, dp};

static NEXT: AtomicU64 = AtomicU64::new(1);

struct WriteToast;

/// What the toast says about the write itself.
#[derive(Clone, Debug)]
pub(crate) enum Progress {
    Sending,
    /// The API took it; the cluster has not been seen to reach the state.
    Accepted,
    Observing {
        waited: Duration,
        limit: Duration,
    },
    Finished(Box<Finished>),
}

/// What the toast says about Undo. The button is never just dropped: it is
/// replaced by the reason there is none.
#[derive(Clone, Debug)]
pub(crate) enum Undo {
    Unavailable,
    Offered {
        offer: Box<UndoOffer>,
        at: Instant,
    },
    Running,
    /// Undo continues in its own dialog and notice.
    Handed,
    /// What came of it, and how to tone it.
    Result {
        text: String,
        good: bool,
    },
}

pub(crate) struct WriteReport {
    sentence: String,
    notice: Option<&'static str>,
    runner: Arc<Runner>,
    runtime: Handle,
    pub(crate) progress: Progress,
    pub(crate) undo: Undo,
    key: SharedString,
    shown: bool,
    _ticker: Option<Task<()>>,
}

/// A run whose worker ended without saying how. The request may have been
/// sent, so it reads as unknown, never as not sent.
pub(super) fn stopped() -> Finished {
    use freshkube_core::write::{ObjectResult, Step};
    Finished {
        run: freshkube_core::write::RunId::new(),
        action: String::new(),
        status: Status::Sent,
        objects: vec![ObjectResult {
            object: "The object".into(),
            step: Step::Unknown("the worker stopped unexpectedly".into()),
        }],
        undo: None,
        audit_error: None,
    }
}

impl WriteReport {
    pub(super) fn new(
        sentence: String,
        notice: Option<&'static str>,
        runner: Arc<Runner>,
        runtime: Handle,
    ) -> Self {
        Self {
            sentence,
            notice,
            runner,
            runtime,
            progress: Progress::Sending,
            undo: Undo::Unavailable,
            key: format!("write-{}", NEXT.fetch_add(1, Ordering::Relaxed)).into(),
            shown: false,
            _ticker: None,
        }
    }

    pub(super) fn event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Accepted(_) => self.progress = Progress::Accepted,
            Event::Observing { waited, limit } => {
                self.progress = Progress::Observing { waited, limit }
            }
            Event::Sending | Event::CancelWindow(_) => {}
        }
        cx.notify();
    }

    pub(super) fn finished(&mut self, done: Finished, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(offer) = done.undo.clone() {
            let now = cx.background_executor().now();
            self.undo = Undo::Offered {
                offer: Box::new(offer),
                at: now,
            };
            self.tick(window, cx);
        }
        self.progress = Progress::Finished(Box::new(done));
        // Re-pushed under the same id so it replaces the toast and takes its
        // new autohide: only an observed success without Undo goes away.
        self.push(window, cx);
    }

    pub(super) fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.shown {
            self.push(window, cx);
        }
    }

    /// Redraws once a second while Undo counts down.
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._ticker = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let offered = this
                    .update(cx, |report, cx| {
                        cx.notify();
                        matches!(report.undo, Undo::Offered { .. })
                    })
                    .unwrap_or(false);
                if !offered {
                    break;
                }
            }
        }));
    }

    fn goes_away_by_itself(&self) -> bool {
        matches!(&self.progress, Progress::Finished(done) if !done.sticky())
            && !matches!(self.undo, Undo::Offered { .. } | Undo::Running)
    }

    fn push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shown = true;
        let report = cx.entity();
        let weak = report.downgrade();
        let observing = Rc::new(Cell::new(false));
        let note = Notification::new()
            .id1::<WriteToast>(self.key.clone())
            .title(self.sentence.clone())
            .autohide(self.goes_away_by_itself())
            .content(move |_, _, cx| {
                if !observing.replace(true) {
                    cx.observe(&report, |_, _, cx| cx.notify()).detach();
                }
                report.read(cx).render_toast(weak.clone(), cx)
            });
        window.push_notification(note, cx);
    }

    /// Undo: the same path as any write. Preflight again, then run.
    pub(crate) fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Undo::Offered { offer, at } = &self.undo else {
            return;
        };
        let (offer, elapsed) = (
            (**offer).clone(),
            cx.background_executor()
                .now()
                .saturating_duration_since(*at),
        );
        self.undo = Undo::Running;
        let runner = self.runner.clone();
        let (job, receiver) = spawn_job(
            &self.runtime,
            Duration::from_secs(30),
            "The cluster did not answer in time. Nothing was sent.".to_owned(),
            async move { Ok::<_, String>(runner.prepare_undo(&offer, elapsed).await) },
        );
        cx.spawn_in(window, async move |this, cx| {
            let _job = job;
            let prepared = receiver
                .await
                .unwrap_or_else(|_| Err("Checking stopped unexpectedly. Nothing was sent.".into()));
            _ = this.update_in(cx, |report, window, cx| match prepared {
                Ok(Ok(prepared)) => report.run_undo(prepared, window, cx),
                Ok(Err(refused)) => report.undone(refused.text(), false, cx),
                Err(text) => report.undone(format!("Can't undo: {text}"), false, cx),
            });
        })
        .detach();
        cx.notify();
    }

    fn undone(&mut self, text: String, good: bool, cx: &mut Context<Self>) {
        self.undo = Undo::Result { text, good };
        cx.notify();
    }

    /// Undo is a write like any other at its level: it goes to its own flow,
    /// which asks what the level asks (the name on a protected cluster) and
    /// reports in its own notice.
    fn run_undo(
        &mut self,
        prepared: freshkube_core::write::Prepared,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let request = WriteRequest {
            runner: self.runner.clone(),
            runtime: self.runtime.clone(),
            action: prepared.description().clone(),
        };
        start_prepared(request, prepared, window, cx);
        self.undo = Undo::Handed;
        cx.notify();
    }

    // ---- drawing ----

    fn status_line(&self) -> (SharedString, Tone) {
        match &self.progress {
            Progress::Sending => ("Sending…".into(), Tone::Info),
            Progress::Accepted => (
                "Accepted. Waiting for the cluster to reach the new state…".into(),
                Tone::Info,
            ),
            Progress::Observing { waited, limit } => (
                format!(
                    "Accepted. Waiting for the cluster to reach the new state… {} s of {} s",
                    waited.as_secs(),
                    limit.as_secs()
                )
                .into(),
                Tone::Info,
            ),
            Progress::Finished(done) => {
                let tone = match done.headline() {
                    Headline::Done => Tone::Good,
                    Headline::Cancelled => Tone::Unknown,
                    Headline::NotObserved
                    | Headline::Unknown
                    | Headline::Diverged
                    | Headline::Partial => Tone::Warn,
                    _ => Tone::Crit,
                };
                (done.message().into(), tone)
            }
        }
    }

    fn render_toast(&self, this: WeakEntity<Self>, cx: &App) -> AnyElement {
        let p = palette(cx);
        let (line, tone) = self.status_line();
        let bar = match &self.progress {
            Progress::Observing { waited, limit } if !limit.is_zero() => {
                Some((waited.as_secs_f32() / limit.as_secs_f32()).clamp(0., 1.))
            }
            Progress::Accepted => Some(0.),
            _ => None,
        };
        v_flex()
            .id("write-toast")
            .test_support()
            .role(Role::Status)
            .aria_label(line.clone())
            .gap(dp(6.))
            .text_size(dp(13.))
            .child(
                div()
                    .id("write-toast-status")
                    .test_support()
                    .aria_label(line.clone())
                    .child(ui::tag(tone, None, line, cx)),
            )
            .when_some(bar, |this, fraction| {
                this.child(
                    div()
                        .id("write-observation-bar")
                        .test_support()
                        .aria_label(format!("Observing, {:.0}%", fraction * 100.))
                        .h(dp(3.))
                        .w_full()
                        .rounded_full()
                        .bg(p.line)
                        .child(
                            div()
                                .h_full()
                                .w(relative(fraction))
                                .rounded_full()
                                .bg(p.accent),
                        ),
                )
            })
            .when_some(self.notice, |this, notice| {
                this.child(
                    div()
                        .id("write-toast-notice")
                        .test_support()
                        .aria_label(notice)
                        .text_size(dp(12.))
                        .text_color(p.muted)
                        .child(notice),
                )
            })
            .children(self.render_undo(this, cx))
            .into_any_element()
    }

    fn render_undo(&self, this: WeakEntity<Self>, cx: &App) -> Option<AnyElement> {
        let p = palette(cx);
        match &self.undo {
            Undo::Unavailable => None,
            Undo::Offered { at, .. } => {
                let left = UNDO_WINDOW.saturating_sub(
                    cx.background_executor()
                        .now()
                        .saturating_duration_since(*at),
                );
                if left.is_zero() {
                    Some(note(
                        "write-undo-note",
                        "Undo is no longer offered.",
                        p.muted,
                    ))
                } else {
                    let label = format!("Undo ({} s)", left.as_secs().max(1));
                    Some(
                        h_flex()
                            .child(
                                Button::new("write-undo")
                                    .label(label)
                                    .small()
                                    .outline()
                                    .on_click(move |_, window, cx| {
                                        _ = this.update(cx, |report, cx| report.undo(window, cx));
                                    }),
                            )
                            .into_any_element(),
                    )
                }
            }
            Undo::Running => Some(note("write-undo-note", "Undoing…", p.muted)),
            Undo::Handed => Some(note(
                "write-undo-note",
                "Undo continues in its own notice.",
                p.muted,
            )),
            Undo::Result { text, good } => Some(note(
                "write-undo-note",
                text,
                if *good { p.good_ink } else { p.warn_ink },
            )),
        }
    }
}

fn note(id: &'static str, text: impl Into<SharedString>, color: Hsla) -> AnyElement {
    let text = text.into();
    div()
        .id(id)
        .test_support()
        .aria_label(text.clone())
        .text_size(dp(12.))
        .text_color(color)
        .child(text)
        .into_any_element()
}
