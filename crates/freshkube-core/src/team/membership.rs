//! Joining, failing and leaving. The applied version only changes through a
//! finished [`super::review::Review`]; a failed fetch keeps it.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::file::{Values, Version, VersionError};
use super::review::{NotReviewed, Review};
use super::source::Source;

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
    pub source: Source,
    pub applied: Option<Version>,
    pub applied_at: Option<DateTime<Utc>>,
    /// A fetched version that waits for review.
    pub pending: Option<Version>,
    pub failure: Option<Failure>,
    /// Your overrides now.
    pub overrides: Overrides,
    /// Your overrides from before joining.
    before_joining: Overrides,
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
        }
    }

    /// The team's applied values; none before the first review.
    pub fn team_values(&self) -> Values {
        self.applied
            .as_ref()
            .map(|v| v.values.clone())
            .unwrap_or_default()
    }

    /// Takes the outcome of a fetch. A failure keeps what is applied and says
    /// since when; a version equal to the applied one is no update.
    pub fn fetched(&mut self, result: Result<Version, FetchFailed>, now: DateTime<Utc>) {
        match result {
            Ok(version) => {
                self.failure = None;
                if self.applied.as_ref().is_some_and(|a| a.id == version.id) {
                    self.pending = None;
                } else {
                    self.pending = Some(version);
                }
            }
            Err(error) => {
                let since = self.failure.as_ref().map_or(now, |f| f.since);
                self.failure = Some(Failure {
                    since,
                    last_try: now,
                    reason: error.0,
                });
            }
        }
    }

    /// The review of the waiting version, if any.
    pub fn review(&self) -> Option<Review> {
        let pending = self.pending.clone()?;
        Some(Review::new(self.applied.as_ref(), pending))
    }

    /// Applies a finished review. Nothing else changes the applied version.
    pub fn apply(&mut self, review: Review, now: DateTime<Utc>) -> Result<(), NotReviewed> {
        let version = review.apply()?;
        self.applied = Some(version);
        self.applied_at = Some(now);
        self.pending = None;
        Ok(())
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

/// A fetch or parse that failed. The text never repeats the source's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchFailed(pub String);

impl From<VersionError> for FetchFailed {
    fn from(error: VersionError) -> Self {
        FetchFailed(error.to_string())
    }
}
