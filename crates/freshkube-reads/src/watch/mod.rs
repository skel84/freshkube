//! One stream's work for one identity and the delivery that hands what it
//! sends to its owner, a batch at a time.
//!
//! A list's watch, a tab's pod watch and a log's container streams each
//! did this by hand: a Tokio job that sends into a channel, a GPUI task
//! that drains it, and a generation or epoch that drops what a replaced
//! stream still sends. A [`Watch`] keeps that pair and that check; the
//! owner keeps what the stream builds, as a [`Loader`](crate::Loader)'s
//! owner reads its data.

use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context as TaskContext, Poll},
    time::{Duration, Instant},
};

use freshkube_core::job::OwnedJob;
use gpui_kit::*;
use tokio::{runtime::Handle, sync::mpsc};

/// How a watch hands what its stream sends to the owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pace {
    /// How many items the channel holds before the stream waits.
    pub capacity: usize,
    /// The most items one batch carries; the rest wait for the next. A
    /// batch is drained on the UI thread, so a fast stream with no cap
    /// holds it as long as the channel stays full.
    pub most: usize,
    /// The least time between two batches: one sent sooner waits, and
    /// whatever arrives meanwhile joins it.
    pub every: Duration,
}

/// Which start a delivery belongs to.
#[derive(Clone, Debug)]
struct Ticket<I> {
    identity: I,
    generation: u64,
}

/// One stream for one identity, such as a kind's list and watch: the Tokio
/// work that sends items and the GPUI task that delivers them in batches.
/// Starting again or stopping drops both, and a batch from an earlier
/// start, or for another identity, never lands.
pub struct Watch<T, I> {
    identity: Option<I>,
    generation: u64,
    job: Option<OwnedJob>,
    task: Option<Task<()>>,
    /// The running stream's channel, held weakly so that it never keeps
    /// the channel open by itself; see [`sender`](Self::sender).
    sender: Option<mpsc::WeakSender<T>>,
    /// The sender that holds an [`open`](Self::open)ed stream's channel
    /// open, as `start`'s work holds its own.
    held: Option<mpsc::Sender<T>>,
    /// Rises with each batch delivered, so an owner derives its display
    /// data again only then.
    revision: u64,
    /// Whether the owner shows it; see [`set_visible`](Self::set_visible).
    visible: bool,
    /// Whether hiding stopped a running stream, which showing starts again.
    stopped_by_hiding: bool,
    items: PhantomData<fn(T)>,
}

impl<T, I> Default for Watch<T, I> {
    fn default() -> Self {
        Self {
            identity: None,
            generation: 0,
            job: None,
            task: None,
            sender: None,
            held: None,
            revision: 0,
            visible: true,
            stopped_by_hiding: false,
            items: PhantomData,
        }
    }
}

impl<T: Send + 'static, I: Clone + Eq + 'static> Watch<T, I> {
    /// Runs `work` on Tokio for `identity`, with the sender it sends its
    /// items into, dropping any earlier stream. Each batch lands through
    /// `apply` only while this is still the latest start for that identity;
    /// `slot` finds the watch again inside the view. `apply` may stop or
    /// start the watch again, but never replaces it with a new one, whose
    /// generations start over and could match this start's. When `work`
    /// returns, the watch stops.
    #[allow(clippy::too_many_arguments)]
    pub fn start<V: 'static, Work, Fut>(
        &mut self,
        identity: I,
        runtime: &Handle,
        pace: Pace,
        work: Work,
        slot: impl Fn(&mut V) -> &mut Self + 'static,
        apply: impl Fn(&mut V, Vec<T>, &mut Context<V>) + 'static,
        cx: &mut Context<V>,
    ) where
        Work: FnOnce(mpsc::Sender<T>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let sender = self.begin(identity, pace, slot, apply, cx);
        self.job = Some(OwnedJob::new(runtime.spawn(work(sender))));
    }

    /// Opens a stream for `identity` with no work of its own, fed only
    /// through [`sender`](Self::sender), and delivered as `start`'s is:
    /// many containers' jobs that share one delivery, or example data
    /// written on the GPUI clock, which no Tokio thread should wake. It
    /// stays open until the watch stops or starts again.
    pub fn open<V: 'static>(
        &mut self,
        identity: I,
        pace: Pace,
        slot: impl Fn(&mut V) -> &mut Self + 'static,
        apply: impl Fn(&mut V, Vec<T>, &mut Context<V>) + 'static,
        cx: &mut Context<V>,
    ) {
        self.held = Some(self.begin(identity, pace, slot, apply, cx));
    }

    /// Drops any earlier stream and starts the delivery of a new one for
    /// `identity`, returning the sender that feeds it.
    fn begin<V: 'static>(
        &mut self,
        identity: I,
        pace: Pace,
        slot: impl Fn(&mut V) -> &mut Self + 'static,
        apply: impl Fn(&mut V, Vec<T>, &mut Context<V>) + 'static,
        cx: &mut Context<V>,
    ) -> mpsc::Sender<T> {
        self.stop();
        self.stopped_by_hiding = false;
        self.identity = Some(identity.clone());
        let ticket = Ticket {
            identity,
            generation: self.generation,
        };
        let (sender, mut receiver) = mpsc::channel(pace.capacity.max(1));
        self.sender = Some(sender.downgrade());
        self.task = Some(cx.spawn(async move |this, cx| {
            let mut last = None;
            while let Some(first) = receiver.recv().await {
                if let Some(due) = last.map(|at: Instant| at + pace.every) {
                    let now = cx.background_executor().now();
                    if now < due {
                        cx.background_executor().timer(due - now).await;
                    }
                }
                let mut batch = vec![first];
                while batch.len() < pace.most {
                    let Ok(item) = receiver.try_recv() else {
                        break;
                    };
                    batch.push(item);
                }
                let delivered = this
                    .update(cx, |view, cx| {
                        Self::deliver(view, &ticket, batch, &slot, &apply, cx)
                    })
                    .unwrap_or(false);
                if !delivered {
                    return;
                }
                last = Some(cx.background_executor().now());
                // However busy the stream and short `every`, let a frame
                // draw between two batches.
                YieldNow(false).await;
            }
            // The stream ended by itself.
            _ = this.update(cx, |view, cx| {
                let watch = slot(view);
                if watch.is_current(&ticket) {
                    watch.job = None;
                    watch.sender = None;
                    watch.held = None;
                    if let Some(task) = watch.task.take() {
                        // This runs in that task: let it finish.
                        task.detach();
                    }
                    cx.notify();
                }
            });
        }));
        sender
    }

    /// Lands one batch if `ticket` is still the latest start. Returns
    /// whether its delivery goes on.
    fn deliver<V>(
        view: &mut V,
        ticket: &Ticket<I>,
        batch: Vec<T>,
        slot: &impl Fn(&mut V) -> &mut Self,
        apply: &impl Fn(&mut V, Vec<T>, &mut Context<V>),
        cx: &mut Context<V>,
    ) -> bool {
        let watch = slot(view);
        if !watch.is_current(ticket) {
            return false;
        }
        watch.revision += 1;
        // Held here while `apply` runs, so that stopping or starting again
        // from it doesn't drop the task it runs in.
        let task = watch.task.take();
        apply(view, batch, cx);
        let watch = slot(view);
        if watch.is_current(ticket) {
            watch.task = task;
            true
        } else {
            if let Some(task) = task {
                task.detach();
            }
            false
        }
    }

    fn is_current(&self, ticket: &Ticket<I>) -> bool {
        ticket.generation == self.generation && self.identity.as_ref() == Some(&ticket.identity)
    }

    /// Drops the stream and its delivery; what it sent stays with the
    /// owner, and nothing more of it lands.
    pub fn stop(&mut self) {
        self.generation += 1;
        self.job = None;
        self.task = None;
        self.sender = None;
        self.held = None;
    }

    /// Another sender into the running stream's channel, for work beside
    /// the one `start` ran that feeds the same delivery: each container's
    /// log among a workload's, or an example writer on the GPUI clock.
    /// What it sends lands only while this start is current, as the
    /// stream's own items do; once the watch stops or starts again,
    /// nothing it sends lands, and its sends fail as soon as the old
    /// delivery is gone. `None` when no stream runs.
    ///
    /// The watch holds the channel only weakly: while `start`'s work or a
    /// sender taken here lives, the stream hasn't ended, so a writer that
    /// takes one for each item lets it end.
    pub fn sender(&self) -> Option<mpsc::Sender<T>> {
        self.sender.as_ref()?.upgrade()
    }

    /// Whether a stream runs: started or opened, and neither stopped nor
    /// ended.
    pub fn is_running(&self) -> bool {
        self.job.is_some() || self.held.is_some()
    }

    /// The identity of the last start, kept after the stream stops.
    pub fn identity(&self) -> Option<&I> {
        self.identity.as_ref()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Rises with each start and stop, so a test tells whether a stream
    /// was dropped. For tests: an owner checks nothing by it.
    #[cfg(any(test, feature = "testing"))]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Hiding stops a running stream and keeps what it sent as last known;
    /// showing again returns whether the owner should start it again, which
    /// it should whenever hiding stopped it, however recently: a stopped
    /// stream holds nothing fresh. A stream that wasn't running when it hid
    /// isn't started again; one the owner stops while hidden still asks for
    /// a start on show, so an owner that wants it to stay stopped says no.
    /// Returns false when nothing changed.
    ///
    /// An owner whose stream reads on while its page hides, as a dock's log
    /// tab does, never calls this.
    pub fn set_visible(&mut self, visible: bool) -> bool {
        if self.visible == visible {
            return false;
        }
        self.visible = visible;
        if !visible {
            if self.is_running() {
                self.stop();
                self.stopped_by_hiding = true;
            }
            return false;
        }
        std::mem::take(&mut self.stopped_by_hiding)
    }

    /// Whether the owner shows it, as [`set_visible`](Self::set_visible)
    /// last said; a new watch is shown. For tests: an owner keeps its own.
    #[cfg(any(test, feature = "testing"))]
    pub fn is_visible(&self) -> bool {
        self.visible
    }
}

/// Gives the executor one turn before going on: a channel that is never
/// empty would otherwise keep the delivery in one poll.
struct YieldNow(bool);

impl Future for YieldNow {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<()> {
        if self.0 {
            return Poll::Ready(());
        }
        self.0 = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

#[cfg(test)]
mod tests;
