//! The reads behind the follows: one [`Feed`] for each object or Kargo
//! project of a context, shared by every follow that needs it.

use std::time::Duration;

use freshkube_reads::{Pace, Watch};
use gpui_kit::*;

use super::Followed;
use crate::example::Play;
use crate::live;
use crate::model::{Follow, WatchKey};
use crate::rules::{Seen, Status};

/// How a feed hands what it saw to the owner.
const PACE: Pace = Pace {
    capacity: 8,
    most: usize::MAX,
    every: Duration::from_millis(200),
};

/// Which read: a context's pod, Kargo project, or Coroot.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FeedKey {
    pub(crate) context: String,
    pub(crate) watch: WatchKey,
}

impl FeedKey {
    pub(crate) fn of(follow: &Follow) -> Self {
        Self {
            context: follow.context.clone(),
            watch: follow.subject.watch_key(),
        }
    }
}

#[derive(Default)]
pub(crate) struct Feed {
    pub(crate) watch: Watch<Seen, u64>,
    /// Which start this is, so a test tells a restarted read.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) epoch: u64,
    /// An example feed plays its script on the GPUI clock.
    player: Option<Task<()>>,
}

/// Finds a feed's watch again inside the owner; a feed that is gone leaves
/// the late delivery to the orphan.
fn slot_of(
    key: FeedKey,
) -> impl for<'a> Fn(&'a mut Followed) -> &'a mut Watch<Seen, u64> + 'static {
    move |this| {
        if this.feeds.contains_key(&key) {
            &mut this.feeds.get_mut(&key).expect("checked").watch
        } else {
            &mut this.orphan.watch
        }
    }
}

impl Followed {
    /// Starts the read for `key`, in place of any earlier one.
    pub(super) fn start_feed(&mut self, key: FeedKey, cx: &mut Context<Self>) {
        let Some(connection) = self.connections.get(&key.context).cloned() else {
            return;
        };
        self.epoch += 1;
        let epoch = self.epoch;
        let mut feed = Feed {
            epoch,
            ..Feed::default()
        };
        let slot = slot_of(key.clone());
        let apply = {
            let key = key.clone();
            move |this: &mut Followed, batch: Vec<Seen>, cx: &mut Context<Followed>| {
                this.deliver(&key, batch, cx)
            }
        };
        if connection.access.is_example() {
            let sender = feed.watch.open(epoch, PACE, slot, apply, cx);
            let subjects: Vec<_> = self
                .items
                .iter()
                .filter(|i| FeedKey::of(&i.follow) == key)
                .map(|i| i.follow.subject.clone())
                .collect();
            let plays = self
                .script
                .as_ref()
                .map(|script| script.plays(&key.watch, &subjects))
                .unwrap_or_else(|| crate::example::Script::default().plays(&key.watch, &subjects));
            feed.player = Some(cx.spawn(async move |_, cx| {
                for Play { after, seen } in plays {
                    cx.background_executor().timer(after).await;
                    if sender.send(seen).await.is_err() {
                        return;
                    }
                }
                // Keep the stream open: an example read goes on until
                // stopped.
                std::future::pending::<()>().await;
            }));
        } else {
            match key.watch.clone() {
                WatchKey::Pod { namespace, name } => {
                    let access = connection.access.clone();
                    feed.watch.start(
                        epoch,
                        &self.runtime,
                        PACE,
                        move |tx| live::watch_pod(access, namespace, name, tx),
                        slot,
                        apply,
                        cx,
                    );
                }
                WatchKey::Kargo { project } => {
                    let access = connection.access.clone();
                    feed.watch.start(
                        epoch,
                        &self.runtime,
                        PACE,
                        move |tx| live::watch_project(access, project, tx),
                        slot,
                        apply,
                        cx,
                    );
                }
                WatchKey::Coroot => match connection.coroot.clone() {
                    Some(coroot) => feed.watch.start(
                        epoch,
                        &self.runtime,
                        PACE,
                        move |tx| live::poll_incidents(coroot, tx),
                        slot,
                        apply,
                        cx,
                    ),
                    None => {
                        self.feeds.remove(&key);
                        self.set_status(
                            &key,
                            Status::NotWatching("Coroot isn't connected.".to_owned()),
                        );
                        return;
                    }
                },
            }
        }
        self.feeds.insert(key, feed);
    }

    /// Lets the read go when no follow needs it.
    pub(super) fn release(&mut self, follow: &Follow) {
        let key = FeedKey::of(follow);
        if !self.items.iter().any(|i| FeedKey::of(&i.follow) == key) {
            self.feeds.remove(&key);
        }
    }

    fn set_status(&mut self, key: &FeedKey, status: Status) {
        for item in self
            .items
            .iter_mut()
            .filter(|i| FeedKey::of(&i.follow) == *key)
        {
            item.status = status.clone();
        }
    }
}
