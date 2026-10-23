//! Joining, failing and leaving. The applied version only changes through a
//! finished [`super::review::Review`]; a failed fetch keeps it.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::file::{Values, Version, VersionError};
use super::merge;
use super::review::{NotReviewed, Review};
use super::source::Source;
use crate::write::Protection;

/// Your own values over the team's, as saved in your `freshkube.yaml`.
pub type Overrides = BTreeMap<String, Value>;

/// Why the last fetch could not give a version, and since when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub since: DateTime<Utc>,
    pub last_try: DateTime<Utc>,
    pub reason: String,
}

/// How to leave the team.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leave {
    /// Keep a copy of the team's values as your own.
    KeepCopy,
    /// Go back to your values from before joining.
    GoBack,
}

/// A person's membership of one team.
#[derive(Clone, Debug, PartialEq)]
pub struct Membership {
    source: Source,
    applied: Option<Version>,
    applied_at: Option<DateTime<Utc>>,
    /// A fetched version that waits for review.
    pending: Option<Version>,
    failure: Option<Failure>,
    /// Your overrides now.
    overrides: Overrides,
    /// Your overrides from before joining.
    before_joining: Overrides,
    /// Floors an applied version would have loosened, held at the stricter
    /// level for as long as this membership lasts. A floor tightens and
    /// never relaxes.
    held: BTreeMap<String, Protection>,
}

impl Membership {
    /// Joins: remembers `overrides` as the copy to go back to.
    pub fn join(source: Source, overrides: Overrides) -> Self {
        Self {
            source,
            applied: None,
            applied_at: None,
            pending: None,
            failure: None,
            before_joining: overrides.clone(),
            overrides,
            held: BTreeMap::new(),
        }
    }

    pub fn source(&self) -> &Source {
        &self.source
    }

    pub fn applied(&self) -> Option<&Version> {
        self.applied.as_ref()
    }

    pub fn applied_at(&self) -> Option<DateTime<Utc>> {
        self.applied_at
    }

    pub fn pending(&self) -> Option<&Version> {
        self.pending.as_ref()
    }

    pub fn failure(&self) -> Option<&Failure> {
        self.failure.as_ref()
    }

    pub fn overrides(&self) -> &Overrides {
        &self.overrides
    }

    /// Sets one of your own values.
    pub fn set_override(&mut self, key: impl Into<String>, value: Value) {
        self.overrides.insert(key.into(), value);
    }

    /// The team's applied values; none before the first review. A floor that
    /// a later version loosened stays at the stricter level it had.
    pub fn team_values(&self) -> Values {
        let mut values = self
            .applied
            .as_ref()
            .map(|v| v.values().clone())
            .unwrap_or_default();
        for (id, level) in &self.held {
            values.insert(merge::protection_key(id), Value::String(level.key().into()));
        }
        values
    }

    /// The clusters whose floor is held above what the applied version says,
    /// with the level it is held at.
    pub fn held_floors(&self) -> &BTreeMap<String, Protection> {
        &self.held
    }

    /// Takes the outcome of a fetch. A failure keeps what is applied and says
    /// since when; a version equal to the applied one is no update. True when
    /// a different version now waits than the one that waited before, so a
    /// choice to review later no longer holds.
    pub fn fetched(&mut self, result: Result<Version, FetchFailed>, now: DateTime<Utc>) -> bool {
        match result {
            Ok(version) => {
                self.failure = None;
                if self
                    .applied
                    .as_ref()
                    .is_some_and(|a| a.id() == version.id())
                {
                    self.pending = None;
                    false
                } else {
                    let new = self.pending.as_ref().map(|p| p.id()) != Some(version.id());
                    self.pending = Some(version);
                    new
                }
            }
            Err(error) => {
                let since = self.failure.as_ref().map_or(now, |f| f.since);
                self.failure = Some(Failure {
                    since,
                    last_try: now,
                    reason: error.0,
                });
                false
            }
        }
    }

    /// Fetches again and takes the outcome; true as for [`Self::fetched`].
    pub fn refresh(&mut self, fetch: &dyn Fetch, now: DateTime<Utc>) -> bool {
        let result = fetch.fetch(&self.source);
        self.fetched(result, now)
    }

    /// The review of the waiting version, if any.
    pub fn review(&self) -> Option<Review> {
        let pending = self.pending.clone()?;
        Some(Review::new(self.applied.as_ref(), pending))
    }

    /// Applies a finished review. Nothing else changes the applied version.
    pub fn apply(&mut self, review: Review, now: DateTime<Utc>) -> Result<(), NotReviewed> {
        let version = review.apply(self.applied.as_ref(), self.pending.as_ref())?;
        self.hold_floors(&version);
        self.applied = Some(version);
        self.applied_at = Some(now);
        self.pending = None;
        Ok(())
    }

    /// A version may only make a cluster's level stricter for the session:
    /// where it loosens a floor, or stops setting one, the stricter floor is
    /// held; where it tightens past a held one, the hold ends.
    fn hold_floors(&mut self, next: &Version) {
        let before = self.team_values();
        let ids: BTreeSet<String> = before
            .keys()
            .filter_map(|key| {
                let rest = key.strip_prefix("clusters[")?;
                rest.split_once(']').map(|(id, _)| id.to_string())
            })
            .collect();
        for id in ids {
            let Some(was) = merge::floor(&id, &before) else {
                continue;
            };
            match merge::floor(&id, next.values()) {
                Some(now) if now <= was => {
                    self.held.remove(&id);
                }
                _ => {
                    self.held.insert(id, was);
                }
            }
        }
    }

    /// Leaves the team, returning the overrides to save.
    pub fn leave(self, how: Leave) -> Overrides {
        match how {
            Leave::GoBack => self.before_joining,
            Leave::KeepCopy => {
                let mut kept = self.team_values();
                // Cluster markers (`clusters[id]`) are structure, not values.
                kept.retain(|key, _| !(key.starts_with("clusters[") && key.ends_with(']')));
                kept.extend(self.overrides);
                kept
            }
        }
    }
}

/// Something that fetches the team file. Slice 1 has one implementation, the
/// fake in [`super::example`]: nothing reads the network.
pub trait Fetch {
    fn fetch(&self, source: &Source) -> Result<Version, FetchFailed>;
}

/// A fetch or parse that failed. The text never repeats the source's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchFailed(pub String);

impl From<VersionError> for FetchFailed {
    fn from(error: VersionError) -> Self {
        FetchFailed(error.to_string())
    }
}
