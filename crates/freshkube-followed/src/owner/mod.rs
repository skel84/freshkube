//! [`Followed`], the app's follows. Like `ForwardList` it belongs to the
//! app, not to a page: a follow reads from the moment the person turns it
//! on until they stop it, through page and context changes and while the
//! window is closed, and `stop_all` at quit stops every read. This is a
//! user-started lifetime beside dock log tabs (docs/STATE_LIFETIMES.md).
//!
//! What it reads is bounded: at most [`MAX_FOLLOWS`] follows, one watch per
//! object or Kargo project however many follows share it, and Coroot once a
//! minute. Every read is the user's own, through the connection the shell
//! offered for the follow's context; a follow with none, or one that can't
//! be read, is "Not watching". The follows are saved in `follows.json`.
//!
//! The owner does not draw. It emits [`Event::Toast`] for a notice that
//! shows, and the shell, which has the window, pushes it. The bell lists
//! every notice. macOS banners and firing alerts plug in here later: the
//! first as another way to show a notice, the second as another `WatchKey`.

mod feeds;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Timelike, Utc};
use freshkube_core::cluster_source::ClusterAccess;
use freshkube_prefs::Document;
use gpui_kit::*;
use tokio::runtime::Handle;

use crate::example::Script;
use crate::live::Coroot;
use crate::model::{Follow, MAX_FOLLOWS, Refused, Subject, WatchKey};
use crate::notice::{Hub, Inbox, Notice, Settings, Words};
use crate::rules::{Seen, Status, Tracker, Update};
use crate::saved::{self, Saved};
use feeds::{Feed, FeedKey};

/// How a cluster is reached for a follow. The shell offers one for each
/// context it has open; the follow keeps reading with the last one offered.
#[derive(Clone)]
pub struct Connection {
    pub context: String,
    pub access: ClusterAccess,
    /// A protected or read-only cluster: Hide details is on by default.
    pub sensitive: bool,
    /// Where Coroot's incidents are read, once Observability has connected.
    pub coroot: Option<Coroot>,
}

/// What the owner tells the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Show this as a toast.
    Toast { words: Words, failure: bool },
}

/// One follow and where it stands.
pub struct Item {
    pub follow: Follow,
    pub status: Status,
    tracker: Tracker,
}

/// What the owner needs to start.
pub struct Options {
    pub runtime: Handle,
    /// `preferences.json`'s path; `follows.json` is beside it. Without one,
    /// as in tests, nothing is read or written.
    pub preferences: Option<std::path::PathBuf>,
    /// Example data: the reads are scripted and nothing is contacted.
    pub script: Option<Script>,
}

pub struct Followed {
    runtime: Handle,
    file: Option<Document<Saved>>,
    save_failed: bool,
    next_id: u64,
    pub(crate) items: Vec<Item>,
    pub(crate) settings: Settings,
    hub: Hub,
    pub(crate) inbox: Inbox,
    pub(crate) connections: HashMap<String, Connection>,
    pub(crate) feeds: HashMap<FeedKey, Feed>,
    /// Takes a late delivery for a feed that is gone.
    orphan: Feed,
    epoch: u64,
    /// Set by `stop_all`: nothing starts again.
    stopped: bool,
    pub(crate) script: Option<Script>,
    pub(crate) seeded: bool,
    origin: Option<(Instant, DateTime<Utc>)>,
    flush: Option<Task<()>>,
    pub(crate) local_minute: Arc<dyn Fn() -> u16 + Send + Sync>,
}

impl EventEmitter<Event> for Followed {}

struct Installed(Entity<Followed>);

impl Global for Installed {}

/// Makes the app's follows and loads the saved ones. They read once the
/// shell offers their cluster's connection.
pub fn install(options: Options, cx: &mut App) -> Entity<Followed> {
    let followed = cx.new(|_| Followed::new(options));
    cx.set_global(Installed(followed.clone()));
    followed
}

/// The app's follows, once installed.
pub fn followed(cx: &App) -> Option<Entity<Followed>> {
    cx.try_global::<Installed>().map(|g| g.0.clone())
}

/// Stops every read of every follow, at quit. The follows stay saved.
pub fn stop_all(cx: &mut App) {
    if let Some(followed) = followed(cx) {
        followed.update(cx, |followed, cx| followed.stop_all(cx));
    }
}

impl Followed {
    fn new(options: Options) -> Self {
        let file = options.preferences.as_deref().map(saved::document);
        let saved = file.as_ref().map(saved::load).unwrap_or_default();
        let items = saved
            .follows
            .into_iter()
            .map(|follow| Item {
                tracker: Tracker::new(&follow),
                status: Status::NotWatching(NOT_CONNECTED.to_owned()),
                follow,
            })
            .collect();
        Self {
            runtime: options.runtime,
            file,
            save_failed: false,
            next_id: saved.next_id.max(1),
            items,
            settings: saved.settings,
            hub: Hub::default(),
            inbox: Inbox::default(),
            connections: HashMap::new(),
            feeds: HashMap::new(),
            orphan: Feed::default(),
            epoch: 0,
            stopped: false,
            script: options.script,
            seeded: false,
            origin: None,
            flush: None,
            local_minute: Arc::new(|| {
                let now = chrono::Local::now();
                (now.hour() * 60 + now.minute()) as u16
            }),
        }
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    pub fn inbox(&self) -> &Inbox {
        &self.inbox
    }

    /// How many are followed, for the bell's "Notifying · n".
    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// How many reads run: one per object or Kargo project, and Coroot.
    pub fn reads_running(&self) -> usize {
        self.feeds.values().filter(|f| f.watch.is_running()).count()
    }

    /// Whether `subject` is followed in `context`.
    pub fn is_following(&self, subject: &Subject, context: &str) -> bool {
        self.items
            .iter()
            .any(|i| &i.follow.subject == subject && i.follow.context == context)
    }

    /// Follows `subject` in `context`, through the connection the shell
    /// offered for it.
    pub fn follow_in(
        &mut self,
        subject: Subject,
        context: &str,
        cx: &mut Context<Self>,
    ) -> Result<u64, Refused> {
        let connection = self
            .connections
            .get(context)
            .cloned()
            .ok_or(Refused::NotOpen)?;
        self.follow(subject, connection, cx)
    }

    /// Follows `subject` through `connection`, which the follow keeps
    /// reading with. Refused at [`MAX_FOLLOWS`] and for a repeat.
    pub fn follow(
        &mut self,
        subject: Subject,
        connection: Connection,
        cx: &mut Context<Self>,
    ) -> Result<u64, Refused> {
        if self.items.len() >= MAX_FOLLOWS {
            return Err(Refused::Full);
        }
        if self.is_following(&subject, &connection.context) {
            return Err(Refused::Duplicate);
        }
        let id = self.next_id;
        self.next_id += 1;
        let follow = Follow {
            id,
            subject,
            context: connection.context.clone(),
            sensitive: connection.sensitive,
            since: self.now(cx),
        };
        self.items.push(Item {
            tracker: Tracker::new(&follow),
            status: Status::Starting,
            follow,
        });
        self.offer(connection, cx);
        self.save();
        cx.notify();
        Ok(id)
    }

    /// Stops following; the read stops with its last follow.
    pub fn stop(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(at) = self.items.iter().position(|i| i.follow.id == id) else {
            return;
        };
        let item = self.items.remove(at);
        self.release(&item.follow);
        self.save();
        cx.notify();
    }

    /// The shell has a connection to `connection.context`: follows there
    /// read with it from now on.
    pub fn offer(&mut self, connection: Connection, cx: &mut Context<Self>) {
        if self.stopped {
            return;
        }
        let context = connection.context.clone();
        let changed = self
            .connections
            .get(&context)
            .is_none_or(|known| known.coroot.is_some() != connection.coroot.is_some());
        self.connections.insert(context.clone(), connection);
        let keys: Vec<FeedKey> = self
            .items
            .iter()
            .filter(|i| i.follow.context == context)
            .map(|i| FeedKey::of(&i.follow))
            .collect();
        for key in keys {
            let reading = self.feeds.get(&key).is_some_and(|f| f.watch.is_running());
            // Coroot starts again when it was not connected before.
            if !reading || (changed && key.watch == WatchKey::Coroot) {
                self.start_feed(key, cx);
            }
        }
        cx.notify();
    }

    /// Stops every read, at quit. The follows stay saved and the owner
    /// starts nothing again.
    pub fn stop_all(&mut self, cx: &mut Context<Self>) {
        self.stopped = true;
        self.feeds.clear();
        self.flush = None;
        cx.notify();
    }

    pub fn set_settings(&mut self, settings: Settings, cx: &mut Context<Self>) {
        self.settings = settings;
        self.save();
        cx.notify();
    }

    /// Opening the bell: what was new is earlier.
    pub fn mark_seen(&mut self, cx: &mut Context<Self>) {
        if !self.inbox.new.is_empty() {
            self.inbox.mark_seen();
            cx.notify();
        }
    }

    /// Takes what a read saw for `key`: applied to every follow that reads
    /// it.
    fn deliver(&mut self, key: &FeedKey, batch: Vec<Seen>, cx: &mut Context<Self>) {
        let now = self.now(cx);
        let mut updates = Vec::new();
        let mut finished = Vec::new();
        for seen in &batch {
            for item in self
                .items
                .iter_mut()
                .filter(|i| FeedKey::of(&i.follow) == *key)
            {
                let step = item.tracker.observe(seen, now);
                item.status = step.status;
                updates.extend(step.updates);
                if step.finished {
                    finished.push(item.follow.id);
                }
            }
        }
        for update in updates {
            self.tell(update, now, cx);
        }
        for id in finished {
            self.stop(id, cx);
        }
        cx.notify();
    }

    /// Groups an update and shows or holds the notice it makes.
    fn tell(&mut self, update: Update, now: DateTime<Utc>, cx: &mut Context<Self>) {
        if let Some(notice) = self.hub.push(update, self.settings.grouping, now) {
            self.show(notice, cx);
        }
        self.arm_flush(cx);
    }

    /// Lists a notice in the bell, and toasts it unless quiet hours hold it
    /// and it is no failure.
    fn show(&mut self, mut notice: Notice, cx: &mut Context<Self>) {
        let quiet = self.settings.quiet.holds((self.local_minute)());
        notice.held = quiet && !notice.has_failure();
        if !notice.held {
            cx.emit(Event::Toast {
                words: notice.words(&self.settings),
                failure: notice.has_failure(),
            });
        }
        self.inbox.push(notice);
        cx.notify();
    }

    /// Wakes when the open group window ends, to tell what waited in it.
    fn arm_flush(&mut self, cx: &mut Context<Self>) {
        let Some(due) = self.hub.due() else {
            self.flush = None;
            return;
        };
        let wait = (due - self.now(cx)).to_std().unwrap_or_default();
        self.flush = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            _ = this.update(cx, |this, cx| {
                let now = this.now(cx);
                if let Some(notice) = this.hub.flush(now) {
                    this.show(notice, cx);
                }
                this.arm_flush(cx);
            });
        }));
    }

    /// The time, on the executor's clock so a test steps it.
    pub(crate) fn now(&mut self, cx: &App) -> DateTime<Utc> {
        let instant = cx.background_executor().now();
        let (origin_instant, origin) = *self.origin.get_or_insert((instant, Utc::now()));
        origin + chrono::Duration::from_std(instant - origin_instant).unwrap_or_default()
    }

    fn save(&mut self) {
        let Some(file) = &self.file else {
            return;
        };
        let saved = Saved {
            follows: self.items.iter().map(|i| i.follow.clone()).collect(),
            next_id: self.next_id,
            settings: self.settings,
        };
        match file.save(&saved) {
            Ok(()) => self.save_failed = false,
            Err(error) => {
                if !std::mem::replace(&mut self.save_failed, true) {
                    tracing::warn!("{} not saved: {error}", file.path().display());
                }
            }
        }
    }
}

/// Why a saved follow reads nothing before its cluster is open.
pub(crate) const NOT_CONNECTED: &str = "This cluster isn't open in this session.";
