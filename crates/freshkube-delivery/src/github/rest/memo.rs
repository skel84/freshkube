//! What was read, kept for the client's life: the parsed answer, its `ETag`
//! and when it was read. It lives inside the client, so it shares the
//! client's identity and is dropped with it: no answer read with one token
//! outlives that token.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::time::Instant;

/// At most this many answers are kept; the oldest is dropped first.
pub(super) const MOST_ENTRIES: usize = 512;

struct Entry {
    etag: Option<String>,
    body: Arc<Value>,
    read: Instant,
    read_at: DateTime<Utc>,
    /// How long it stays fresh; for the client's life when `None`.
    keep: Option<Duration>,
}

#[derive(Default)]
pub(super) struct Memo {
    entries: HashMap<String, Entry>,
    /// Oldest first.
    order: VecDeque<String>,
}

impl Memo {
    /// The answer for `path` while it is fresh.
    pub fn fresh(&self, path: &str, now: Instant) -> Option<Arc<Value>> {
        let entry = self.entries.get(path)?;
        let fresh = entry
            .keep
            .is_none_or(|keep| now.saturating_duration_since(entry.read) < keep);
        fresh.then(|| entry.body.clone())
    }

    /// The `ETag` of the answer kept for `path`, fresh or not.
    pub fn etag(&self, path: &str) -> Option<&str> {
        self.entries.get(path)?.etag.as_deref()
    }

    /// The answer kept for `path`, fresh or not, and when it was read.
    pub fn kept(&self, path: &str) -> Option<(Arc<Value>, DateTime<Utc>)> {
        self.entries
            .get(path)
            .map(|entry| (entry.body.clone(), entry.read_at))
    }

    pub fn store(
        &mut self,
        path: &str,
        etag: Option<String>,
        body: Arc<Value>,
        keep: Option<Duration>,
        now: Instant,
    ) {
        if self.entries.remove(path).is_some() {
            self.order.retain(|kept| kept != path);
        }
        while self.entries.len() >= MOST_ENTRIES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
        self.order.push_back(path.to_owned());
        self.entries.insert(
            path.to_owned(),
            Entry {
                etag,
                body,
                read: now,
                read_at: Utc::now(),
                keep,
            },
        );
    }

    /// A 304 says the kept answer still holds: its expiry starts again.
    pub fn renew(&mut self, path: &str, keep: Option<Duration>, now: Instant) {
        if let Some(entry) = self.entries.get_mut(path) {
            entry.read = now;
            entry.read_at = Utc::now();
            entry.keep = keep;
        }
    }

    /// Makes every kept answer `by` older.
    #[cfg(test)]
    pub fn age(&mut self, by: Duration) {
        for entry in self.entries.values_mut() {
            entry.read -= by;
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_answer_expires_when_its_time_is_up_and_not_before() {
        let start = Instant::now();
        let mut memo = Memo::default();
        memo.store(
            "a",
            Some("\"e\"".into()),
            Arc::new(json!(1)),
            Some(Duration::from_secs(60)),
            start,
        );
        assert!(memo.fresh("a", start + Duration::from_secs(59)).is_some());
        assert!(memo.fresh("a", start + Duration::from_secs(60)).is_none());
        // Stale, it still labels a conditional request and can be reused.
        assert_eq!(memo.etag("a"), Some("\"e\""));
        assert!(memo.kept("a").is_some());
        memo.renew("a", Some(Duration::from_secs(60)), start + Duration::from_secs(61));
        assert!(memo.fresh("a", start + Duration::from_secs(100)).is_some());
        memo.store("b", None, Arc::new(json!(2)), None, start);
        assert!(memo.fresh("b", start + Duration::from_secs(10_000)).is_some());
    }

    #[test]
    fn the_oldest_answer_goes_first_past_the_cap() {
        let now = Instant::now();
        let mut memo = Memo::default();
        for n in 0..MOST_ENTRIES + 3 {
            memo.store(&n.to_string(), None, Arc::new(json!(n)), None, now);
        }
        assert_eq!(memo.len(), MOST_ENTRIES);
        for gone in 0..3 {
            assert!(memo.kept(&gone.to_string()).is_none());
        }
        assert!(memo.kept("3").is_some());
        // Storing again moves an entry to the newest end.
        memo.store("3", None, Arc::new(json!(3)), None, now);
        memo.store("new", None, Arc::new(json!(0)), None, now);
        assert!(memo.kept("3").is_some());
        assert!(memo.kept("4").is_none());
    }
}
