//! Starting a change to a Kubernetes object from a page (#853,
//! `docs/WRITE_MODEL.md`). A page describes an action and hands it to
//! [`start`]; core decides what is asked (`freshkube_core::write::Dialog`) and
//! this draws it: the dialog, the countdown, the toast that tells accepted
//! from observed, and Undo. Nothing here builds a request or decides a level.
//!
//! **Lifetime.** Checking an action is an ordinary read: it belongs to the
//! dialog and dies with it. A *submitted* write does not. Confirming takes the
//! app-wide operation slot ([`Operations`]) and spawns the runner on Tokio
//! with no owner; a detached task holds the ticket until the runner ends. So
//! hiding the page, or closing the dialog's page, never stops or repeats a
//! write, and the slot is freed exactly once, when the runner has finished
//! (docs/STATE_LIFETIMES.md, "Kubernetes writes").

mod dialog;
mod report;
#[cfg(test)]
mod stage_tests;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::{Duration, Instant};

use freshkube_core::write::{
    ActionDescription, Confirmation, Dialog, Event, Finished, NotSent, Prepared, Runner, Status,
    UndoOffer, UndoRefused, Verdict,
};
use gpui_kit::component::{
    WindowExt,
    input::{InputEvent, InputState},
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::backend::{OwnedJob, spawn_job};
use crate::mutation::{OperationTicket, Operations, Refused};
use report::WriteReport;

/// How long checking an action may take before it is reported as not
/// answering.
const CHECK_DEADLINE: Duration = Duration::from_secs(30);

/// The flows a page started and that are still open, kept so the dialog's
/// state lives while it shows. A closed one is dropped when the next starts.
#[derive(Default)]
struct Flows(Vec<Entity<WriteFlow>>);

impl Global for Flows {}

/// Keeps `flow` alive until it closes.
pub(crate) fn retain(flow: Entity<WriteFlow>, cx: &mut App) {
    let kept = std::mem::take(&mut cx.default_global::<Flows>().0);
    let mut open: Vec<Entity<WriteFlow>> = kept
        .into_iter()
        .filter(|kept| kept.read(cx).stage != Stage::Closed)
        .collect();
    open.push(flow);
    cx.default_global::<Flows>().0 = open;
}

/// What a page hands over.
pub(crate) struct WriteRequest {
    pub(crate) runner: Arc<Runner>,
    pub(crate) runtime: Handle,
    pub(crate) action: ActionDescription,
    /// Set for Undo: every check of it, a re-check under a stricter level
    /// included, goes through `prepare_undo` so its guard is checked again
    /// and Review can never put the old value back over someone else's write.
    pub(crate) undo: Option<UndoSource>,
}

/// What an Undo flow re-checks from, and when its window started counting.
#[derive(Clone)]
pub(crate) struct UndoSource {
    pub(crate) offer: UndoOffer,
    pub(crate) elapsed: Duration,
    pub(crate) at: Instant,
}

/// Where the dialog is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Stage {
    /// Preflight is running.
    Checking,
    /// Core's answer, drawn.
    Shown(Dialog),
    /// A destructive action was confirmed and is in its cancel window. The
    /// dialog's confirm has turned into Cancel, with the time left.
    Countdown {
        started: std::time::Instant,
        total: Duration,
    },
    /// Sent or finished; the dialog is closed.
    Closed,
}

/// What a runner tells the task that watches it.
pub(super) enum Msg {
    Event(Event),
    Done(Box<Finished>),
}

/// The dialog's state: the action, core's answer and what the person typed.
pub(crate) struct WriteFlow {
    request: WriteRequest,
    pub(crate) stage: Stage,
    /// Kept between preflight and confirm; taken (so used once) on confirm.
    prepared: Option<Box<Prepared>>,
    typed: Option<Entity<InputState>>,
    typed_matches: bool,
    dialog_open: bool,
    check: Option<(OwnedJob, Task<()>)>,
    _subscriptions: Vec<Subscription>,
}

/// Opens the flow for `request`: the dialog when the level asks anything,
/// otherwise nothing until there is something to say.
pub(crate) fn start(request: WriteRequest, window: &mut Window, cx: &mut App) -> Entity<WriteFlow> {
    let asks = freshkube_core::write::requirement_for(
        &request.action,
        request.runner.level_for(&request.action),
    );
    let flow = cx.new(|_| WriteFlow {
        request,
        stage: Stage::Checking,
        prepared: None,
        typed: None,
        typed_matches: false,
        dialog_open: false,
        check: None,
        _subscriptions: vec![],
    });
    if asks.confirm || asks.blocked.is_some() {
        dialog::open(&flow, window, cx);
        flow.update(cx, |flow, _| flow.dialog_open = true);
    }
    flow.update(cx, |flow, cx| flow.check(window, cx));
    flow
}

/// Opens the flow for an action that has already passed preflight, such as
/// Undo's: it asks what its level asks, like any write, and sends nothing
/// until that is met.
pub(crate) fn start_prepared(
    request: WriteRequest,
    prepared: Prepared,
    window: &mut Window,
    cx: &mut App,
) -> Entity<WriteFlow> {
    let flow = cx.new(|_| WriteFlow {
        request,
        stage: Stage::Checking,
        prepared: None,
        typed: None,
        typed_matches: false,
        dialog_open: false,
        check: None,
        _subscriptions: vec![],
    });
    flow.update(cx, |flow, cx| {
        flow.checked(Ok(Verdict::Ready(Box::new(prepared))), window, cx)
    });
    flow
}

impl WriteFlow {
    /// Runs preflight (access review, UID and resourceVersion, dry run) and
    /// shows what core answers. Replacing a check drops the one before.
    pub(crate) fn check(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stage = Stage::Checking;
        self.prepared = None;
        let (runner, action) = (self.request.runner.clone(), self.request.action.clone());
        let undo = self.request.undo.clone().map(|source| {
            let spent = source.elapsed
                + cx.background_executor()
                    .now()
                    .saturating_duration_since(source.at);
            (source.offer, spent)
        });
        let (job, receiver) = spawn_job(
            &self.request.runtime,
            CHECK_DEADLINE,
            "The cluster did not answer in time. Nothing was sent.".to_owned(),
            async move {
                Ok::<_, String>(match undo {
                    None => runner.preflight(action).await,
                    Some((offer, spent)) => match runner.prepare_undo(&offer, spent).await {
                        Ok(prepared) => Verdict::Ready(Box::new(prepared)),
                        Err(UndoRefused::Blocked(block)) => Verdict::Blocked(block),
                        Err(refused) => return Err(refused.text()),
                    },
                })
            },
        );
        let task = cx.spawn_in(window, async move |this, cx| {
            let verdict = receiver
                .await
                .unwrap_or_else(|_| Err("Checking stopped unexpectedly. Nothing was sent.".into()));
            _ = this.update_in(cx, |flow, window, cx| flow.checked(verdict, window, cx));
        });
        self.check = Some((job, task));
        cx.notify();
    }

    fn checked(
        &mut self,
        verdict: Result<Verdict, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entry = self.request.runner.who().entry.clone();
        let (dialog, prepared) = match verdict {
            Ok(verdict) => {
                let dialog = Dialog::for_verdict(&verdict, &entry);
                match verdict {
                    Verdict::Ready(prepared) => (dialog, Some(prepared)),
                    Verdict::Blocked(_) => (dialog, None),
                }
            }
            Err(text) => (Dialog::Refused { text }, None),
        };
        self.prepared = prepared;
        self.typed = None;
        self.typed_matches = false;
        match &dialog {
            Dialog::SendWithoutAsking { notice } => {
                let notice = *notice;
                self.send(Confirmation::NotAsked, Some(notice), window, cx);
                return;
            }
            Dialog::Ask(ask) if ask.type_name.is_some() => {
                let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type the name"));
                let expected = ask.type_name.clone().unwrap_or_default();
                let subscription =
                    cx.subscribe(&input, move |flow, input, event: &InputEvent, cx| {
                        if matches!(event, InputEvent::Change) {
                            flow.typed_matches = input.read(cx).value().as_ref() == expected;
                            cx.notify();
                        }
                    });
                self._subscriptions = vec![subscription];
                self.typed = Some(input);
            }
            _ => {}
        }
        self.stage = Stage::Shown(dialog);
        if !self.dialog_open {
            // Nothing was asked, but there is something to say: a refusal.
            self.dialog_open = true;
            dialog::open(&cx.entity(), window, cx);
        }
        cx.notify();
    }

    /// The confirm button.
    pub(crate) fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Stage::Shown(Dialog::Ask(ask)) = &self.stage else {
            return;
        };
        let confirmation = match &ask.type_name {
            Some(expected) => {
                let typed = self
                    .typed
                    .as_ref()
                    .map(|input| input.read(cx).value().to_string())
                    .unwrap_or_default();
                if &typed != expected {
                    return;
                }
                Confirmation::Typed(typed)
            }
            None => Confirmation::Confirmed,
        };
        let window_for_cancel = ask.cancel_window;
        if let Some(total) = window_for_cancel {
            let now = cx.background_executor().now();
            self.stage = Stage::Countdown {
                started: now,
                total,
            };
            self.tick(window, cx);
        }
        self.send(confirmation, None, window, cx);
    }

    /// Redraws the countdown once a second while it runs.
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let running = this
                    .update(cx, |flow, cx| {
                        cx.notify();
                        matches!(flow.stage, Stage::Countdown { .. })
                    })
                    .unwrap_or(false);
                if !running {
                    break;
                }
            }
        })
        .detach();
    }

    /// "Review the new version": re-bases on the object as it is now and
    /// checks again. Sends nothing.
    pub(crate) fn review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let runner = self.request.runner.clone();
        let action = self.request.action.clone();
        self.stage = Stage::Checking;
        let (job, receiver) = spawn_job(
            &self.request.runtime,
            CHECK_DEADLINE,
            "The cluster did not answer in time. Nothing was sent.".to_owned(),
            async move { Ok::<_, String>(runner.review_new_version(action).await) },
        );
        let task = cx.spawn_in(window, async move |this, cx| {
            let rebased = receiver
                .await
                .unwrap_or_else(|_| Err("Checking stopped unexpectedly. Nothing was sent.".into()));
            _ = this.update_in(cx, |flow, window, cx| match rebased {
                Ok(Ok(action)) => {
                    flow.request.action = action;
                    flow.check(window, cx);
                }
                Ok(Err(block)) => {
                    let verdict = Verdict::Blocked(block);
                    flow.checked(Ok(verdict), window, cx);
                }
                Err(text) => flow.checked(Err(text), window, cx),
            });
        });
        self.check = Some((job, task));
        cx.notify();
    }

    /// Cancels a destructive action during its window. Through the
    /// operation slot's cancellation, the same one the status bar offers.
    pub(crate) fn cancel_countdown(&mut self, cx: &mut Context<Self>) {
        if matches!(self.stage, Stage::Countdown { .. }) {
            Operations::global(cx).update(cx, |operations, cx| operations.request_cancel(cx));
        }
    }

    /// Takes the slot and sends. The prepared action is consumed here, so a
    /// second press finds nothing to send.
    fn send(
        &mut self,
        confirmation: Confirmation,
        notice: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.prepared.is_none() {
            return;
        }
        let sentence = self.request.action.sentence.clone();
        let operations = Operations::global(cx);
        let ticket = match operations
            .update(cx, |operations, cx| operations.begin(sentence.clone(), cx))
        {
            Ok(ticket) => ticket,
            Err(refused) => {
                let text = match refused {
                    Refused::Busy(running) => format!(
                        "{} is running. Nothing was sent; wait for it to finish, or cancel it from the status bar.",
                        running.label
                    ),
                    Refused::ReadOnly(why) => format!("{why} Nothing was sent."),
                };
                self.stage = Stage::Shown(Dialog::Refused { text });
                if !self.dialog_open {
                    self.dialog_open = true;
                    dialog::open(&cx.entity(), window, cx);
                }
                cx.notify();
                return;
            }
        };
        let Some(prepared) = self.prepared.take() else {
            return;
        };
        let report = cx.new(|_| {
            WriteReport::new(
                sentence,
                notice,
                self.request.runner.clone(),
                self.request.runtime.clone(),
            )
        });
        let mut messages = start_worker(
            &self.request.runner,
            &self.request.runtime,
            *prepared,
            confirmation,
            &ticket,
        );
        let flow = cx.entity().downgrade();
        // Detached on purpose: it owns the ticket, so the slot is freed only
        // when the runner has really ended, whatever became of the page, the
        // dialog or the window.
        cx.spawn_in(window, async move |_, cx| {
            let mut finished = None;
            while let Some(message) = messages.recv().await {
                match message {
                    Msg::Event(Event::Sending) => {
                        _ = flow
                            .update_in(cx, |flow, window, cx| flow.sending(&report, window, cx));
                    }
                    Msg::Event(event) => {
                        report.update(cx, |report, cx| report.event(event, cx));
                    }
                    Msg::Done(done) => {
                        finished = Some(done);
                        break;
                    }
                }
            }
            let done = *finished.unwrap_or_else(|| Box::new(report::stopped()));
            // A level that got stricter sends nothing and asks again; there
            // is nothing to report.
            let again = matches!(done.status, Status::NotSent(NotSent::LevelChanged(_)));
            _ = flow.update_in(cx, |flow, window, cx| {
                flow.finished(&report, again, window, cx)
            });
            if !again {
                _ = report.update_in(cx, |report, window, cx| report.finished(done, window, cx));
            }
            operations.update(cx, |operations, cx| operations.finish(ticket, cx));
        })
        .detach();
        cx.notify();
    }

    /// The `sending` line is written and requests begin: the dialog has done
    /// its part, and the toast takes over.
    fn sending(
        &mut self,
        report: &Entity<WriteReport>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close(window, cx);
        report.update(cx, |report, cx| report.show(window, cx));
    }

    /// A run that ended without sending (cancelled, refused, unconfirmed)
    /// closes the dialog and reports in the toast, which stays.
    fn finished(
        &mut self,
        report: &Entity<WriteReport>,
        again: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if again {
            // Back to the dialog, checked under the new level.
            self.check(window, cx);
            if !self.dialog_open {
                self.dialog_open = true;
                dialog::open(&cx.entity(), window, cx);
            }
            return;
        }
        if self.stage != Stage::Closed {
            self.close(window, cx);
            report.update(cx, |report, cx| report.show(window, cx));
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stage = Stage::Closed;
        if std::mem::take(&mut self.dialog_open) {
            window.close_dialog(cx);
        }
        cx.notify();
    }
}

/// Spawns the runner on Tokio with no owner and streams what it says. The
/// run is never aborted when anything here goes away.
pub(super) fn start_worker(
    runner: &Arc<Runner>,
    runtime: &Handle,
    prepared: Prepared,
    confirmation: Confirmation,
    ticket: &OperationTicket,
) -> mpsc::UnboundedReceiver<Msg> {
    let (tx, rx) = mpsc::unbounded_channel();
    let (runner, cancel) = (runner.clone(), ticket.cancellation());
    runtime.spawn(worker(runner, prepared, confirmation, cancel, tx));
    rx
}

async fn worker(
    runner: Arc<Runner>,
    prepared: Prepared,
    confirmation: Confirmation,
    cancel: impl Fn() -> bool + Send,
    tx: mpsc::UnboundedSender<Msg>,
) {
    let events = tx.clone();
    let finished = runner
        .execute(prepared, confirmation, cancel, move |event| {
            let _ = events.send(Msg::Event(event));
        })
        .await;
    let _ = tx.send(Msg::Done(Box::new(finished)));
}
