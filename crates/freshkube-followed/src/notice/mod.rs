//! From updates to what the person sees: grouping within 2 minutes, quiet
//! hours that hold all but failures, and the words of a notice with or
//! without its details.

use std::collections::VecDeque;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::rules::Update;

/// Updates within this long of the first one join one notice.
pub const GROUP_WINDOW_SECONDS: i64 = 2 * 60;
/// The most notices the bell keeps in each list.
const KEPT: usize = 30;

/// Quiet hours: from a time of day to another, in minutes since midnight,
/// local time. It may wrap midnight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Quiet {
    pub enabled: bool,
    pub from: u16,
    pub until: u16,
}

impl Default for Quiet {
    fn default() -> Self {
        Self {
            enabled: false,
            from: 22 * 60,
            until: 7 * 60,
        }
    }
}

impl Quiet {
    /// Whether quiet hours hold at `minute` of the day.
    pub fn holds(&self, minute: u16) -> bool {
        if !self.enabled || self.from == self.until {
            return false;
        }
        if self.from < self.until {
            (self.from..self.until).contains(&minute)
        } else {
            minute >= self.from || minute < self.until
        }
    }

    /// "22:00".
    pub fn clock(minute: u16) -> String {
        format!("{:02}:{:02}", (minute / 60) % 24, minute % 60)
    }
}

/// How notices are made. Saved with the follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Whether updates within [`GROUP_WINDOW_SECONDS`] join one notice.
    pub grouping: bool,
    pub quiet: Quiet,
    /// `None` is the default: on for a protected or read-only cluster.
    pub hide_details: Option<bool>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            grouping: true,
            quiet: Quiet::default(),
            hide_details: None,
        }
    }
}

impl Settings {
    /// Whether a notice of updates from a cluster of this kind shows no
    /// details.
    pub fn hides(&self, sensitive: bool) -> bool {
        self.hide_details.unwrap_or(sensitive)
    }
}

/// One toast or bell entry: one update, or the updates of a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub id: u64,
    pub at: DateTime<Utc>,
    pub updates: Vec<Update>,
    /// Held by quiet hours: in the bell, not shown.
    pub held: bool,
}

/// What a notice says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Words {
    pub title: String,
    pub body: Option<String>,
}

impl Notice {
    pub fn has_failure(&self) -> bool {
        self.updates.iter().any(|u| u.failure)
    }

    /// Whether any update comes from a protected or read-only cluster.
    pub fn sensitive(&self) -> bool {
        self.updates.iter().any(|u| u.sensitive)
    }

    /// "Freshkube · 1 update".
    pub fn summary(&self) -> String {
        match self.updates.len() {
            1 => "Freshkube · 1 update".to_owned(),
            n => format!("Freshkube · {n} updates"),
        }
    }

    /// What a toast (and later a banner) says: with Hide details, the
    /// summary alone.
    pub fn words(&self, settings: &Settings) -> Words {
        if settings.hides(self.sensitive()) {
            return Words {
                title: self.summary(),
                body: None,
            };
        }
        match self.updates.as_slice() {
            [only] => Words {
                title: only.headline.clone(),
                body: Some(only.detail.clone()),
            },
            many => Words {
                title: self.summary(),
                body: Some(
                    many.iter()
                        .map(|u| u.headline.as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            },
        }
    }
}

/// Collects updates into notices. The first update of a window is told at
/// once; the ones after it, until the window ends, join one notice.
#[derive(Debug, Default)]
pub struct Hub {
    window_start: Option<DateTime<Utc>>,
    pending: Vec<Update>,
    next_id: u64,
}

impl Hub {
    fn notice(&mut self, updates: Vec<Update>, at: DateTime<Utc>) -> Notice {
        self.next_id += 1;
        Notice {
            id: self.next_id,
            at,
            updates,
            held: false,
        }
    }

    /// Takes an update. Returns the notice to tell now, if any.
    pub fn push(&mut self, update: Update, grouping: bool, now: DateTime<Utc>) -> Option<Notice> {
        // A failure waits for no window: it is told now, with whatever
        // waited, and the window it fell in goes on.
        if update.failure {
            let mut updates = std::mem::take(&mut self.pending);
            updates.push(update);
            return Some(self.notice(updates, now));
        }
        if grouping
            && let Some(start) = self.window_start
            && now < start + Duration::seconds(GROUP_WINDOW_SECONDS)
        {
            self.pending.push(update);
            return None;
        }
        // A window that ended with updates waiting, before its flush
        // woke, tells them with this one.
        self.window_start = grouping.then_some(now);
        let mut updates = std::mem::take(&mut self.pending);
        updates.push(update);
        Some(self.notice(updates, now))
    }

    /// The notice for the updates that waited, once the window has ended.
    pub fn flush(&mut self, now: DateTime<Utc>) -> Option<Notice> {
        let start = self.window_start?;
        if now < start + Duration::seconds(GROUP_WINDOW_SECONDS) {
            return None;
        }
        self.window_start = None;
        if self.pending.is_empty() {
            return None;
        }
        let updates = std::mem::take(&mut self.pending);
        Some(self.notice(updates, now))
    }

    /// When the open window ends, if updates wait for it.
    pub fn due(&self) -> Option<DateTime<Utc>> {
        let start = self.window_start?;
        (!self.pending.is_empty()).then(|| start + Duration::seconds(GROUP_WINDOW_SECONDS))
    }
}

/// The bell's two lists, newest first.
#[derive(Debug, Default)]
pub struct Inbox {
    pub new: VecDeque<Notice>,
    pub earlier: VecDeque<Notice>,
}

impl Inbox {
    pub fn push(&mut self, notice: Notice) {
        self.new.push_front(notice);
        while self.new.len() > KEPT {
            self.new.pop_back();
        }
    }

    /// Opening the bell: New becomes Earlier.
    pub fn mark_seen(&mut self) {
        for notice in self.new.drain(..).rev() {
            self.earlier.push_front(notice);
        }
        while self.earlier.len() > KEPT {
            self.earlier.pop_back();
        }
    }
}

#[cfg(test)]
mod tests;
