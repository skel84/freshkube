//! What GitHub says is left of the rate limit, from the headers of every
//! response. One client has one budget, shared by the list and the page.

use chrono::{DateTime, Duration, Utc};
use http::HeaderMap;

/// The rate limit as the last response reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RateLimit {
    pub limit: u32,
    pub remaining: u32,
    pub reset: DateTime<Utc>,
    pub authenticated: bool,
}

/// Where the limit is reached, until when.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Blocked {
    pub until: DateTime<Utc>,
    pub secondary: bool,
}

#[derive(Debug, Default)]
pub(super) struct Budget {
    last: Option<RateLimit>,
    blocked: Option<Blocked>,
}

fn number(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

impl Budget {
    pub fn rate(&self) -> Option<&RateLimit> {
        self.last.as_ref()
    }

    /// Records a response's headers. `remaining: 0` blocks until the reset.
    pub fn observe(&mut self, headers: &HeaderMap, authenticated: bool, now: DateTime<Utc>) {
        let (Some(limit), Some(remaining), Some(reset)) = (
            number(headers, "x-ratelimit-limit"),
            number(headers, "x-ratelimit-remaining"),
            number(headers, "x-ratelimit-reset"),
        ) else {
            return;
        };
        let Some(reset) = i64::try_from(reset)
            .ok()
            .and_then(|reset| DateTime::from_timestamp(reset, 0))
        else {
            return;
        };
        let rate = RateLimit {
            limit: u32::try_from(limit).unwrap_or(u32::MAX),
            remaining: u32::try_from(remaining).unwrap_or(u32::MAX),
            reset,
            authenticated,
        };
        if rate.remaining == 0 && reset > now {
            self.blocked = Some(Blocked {
                until: reset,
                secondary: false,
            });
        }
        self.last = Some(rate);
    }

    /// A 403 or 429 with `retry-after`, or a 429 without one.
    pub fn block_for(&mut self, seconds: u64, now: DateTime<Utc>) {
        let seconds = i64::try_from(seconds.min(24 * 3600)).unwrap_or(60);
        self.blocked = Some(Blocked {
            until: now + Duration::seconds(seconds),
            secondary: true,
        });
    }

    /// Blocked until the reset or the retry time, if it hasn't passed.
    pub fn blocked(&mut self, now: DateTime<Utc>) -> Option<Blocked> {
        match self.blocked {
            Some(blocked) if blocked.until > now => Some(blocked),
            _ => {
                self.blocked = None;
                None
            }
        }
    }

    /// Fewer than `reserve` calls left before the reset, as the last
    /// response said.
    pub fn low(&self, reserve: u32, now: DateTime<Utc>) -> Option<(u32, DateTime<Utc>)> {
        self.last
            .as_ref()
            .filter(|rate| rate.remaining < reserve && rate.reset > now)
            .map(|rate| (rate.remaining, rate.reset))
    }
}
