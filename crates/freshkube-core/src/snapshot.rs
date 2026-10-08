//! The last answer of one read, kept for one identity, and the request
//! generations that may replace it. The identity says whose data this is (a
//! node, a configuration, a provider); the generation says which request may
//! publish, so a superseded or duplicate completion changes nothing. A
//! failed refresh for the same identity keeps the data, marked stale.
//!
//! It cancels nothing and inspects no credentials: the caller owns the work
//! ([`crate::job::OwnedJob`]) and chooses the identity. Desktop names it
//! `state::Snapshot`, with its applied configuration as the default identity.

use std::time::SystemTime;

/// A completion token belongs to one identity and one request generation.
#[derive(Clone, Debug)]
pub struct Request<I> {
    identity: I,
    generation: u64,
}

/// Retains the last successful value only while its target identity is unchanged.
pub struct Snapshot<T, I> {
    identity: Option<I>,
    generation: u64,
    data: Option<T>,
    loading: bool,
    error: Option<String>,
    stale: bool,
    last_successful: Option<SystemTime>,
    last_failure: Option<SystemTime>,
}

impl<T, I> Default for Snapshot<T, I> {
    fn default() -> Self {
        Self {
            identity: None,
            generation: 0,
            data: None,
            loading: false,
            error: None,
            stale: false,
            last_successful: None,
            last_failure: None,
        }
    }
}

impl<T, I: Clone + Eq> Snapshot<T, I> {
    pub fn begin(&mut self, identity: I) -> Request<I> {
        if self.identity.as_ref() != Some(&identity) {
            self.data = None;
            self.last_successful = None;
            self.last_failure = None;
        }
        self.generation = self
            .generation
            .checked_add(1)
            .expect("request generation exhausted");
        self.identity = Some(identity.clone());
        self.loading = true;
        self.error = None;
        self.stale = self.data.is_some();
        Request {
            identity,
            generation: self.generation,
        }
    }

    /// Puts back an answer kept from an earlier time, marked stale until a
    /// read of the same identity replaces it, with the time it was taken.
    /// Nothing is loading; a [`begin`](Self::begin) for another identity
    /// drops it, as it drops any data.
    pub fn restore(&mut self, identity: I, data: T, taken: SystemTime) {
        self.identity = Some(identity);
        self.data = Some(data);
        self.loading = false;
        self.error = None;
        self.stale = true;
        self.last_successful = Some(taken);
        self.last_failure = None;
    }

    pub fn is_current(&self, request: &Request<I>) -> bool {
        request.generation == self.generation && self.identity.as_ref() == Some(&request.identity)
    }

    /// Returns false without mutation for an obsolete or already-applied request.
    pub fn apply(&mut self, request: &Request<I>, result: Result<T, String>) -> bool {
        if !self.loading || !self.is_current(request) {
            return false;
        }
        self.loading = false;
        match result {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
                self.stale = false;
                self.last_successful = Some(SystemTime::now());
                self.last_failure = None;
            }
            Err(error) => {
                self.error = Some(error);
                self.last_failure = Some(SystemTime::now());
                self.stale = self.data.is_some();
            }
        }
        true
    }

    pub fn data(&self) -> Option<&T> {
        self.data.as_ref()
    }

    /// Updates an independent projection without changing the request, its
    /// coverage, error or success/failure timestamps.
    pub fn data_mut(&mut self) -> Option<&mut T> {
        self.data.as_mut()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn is_stale(&self) -> bool {
        self.stale
    }

    pub fn last_successful(&self) -> Option<SystemTime> {
        self.last_successful
    }

    /// When the most recent request for this target failed, if it did.
    pub fn last_failure(&self) -> Option<SystemTime> {
        self.last_failure
    }
}

#[cfg(test)]
mod tests {
    use super::Snapshot;

    #[test]
    fn same_target_failure_retains_data_and_success_time() {
        let mut snapshot = Snapshot::<u32, &str>::default();
        let initial = snapshot.begin("node-a");
        assert!(snapshot.apply(&initial, Ok(7)));
        let successful = snapshot.last_successful();
        assert!(successful.is_some());
        assert!(!snapshot.is_stale());

        let refresh = snapshot.begin("node-a");
        assert_eq!(snapshot.data(), Some(&7));
        assert!(snapshot.is_loading());
        assert!(snapshot.is_stale());
        assert!(snapshot.apply(&refresh, Err("offline".into())));
        assert_eq!(snapshot.data(), Some(&7));
        assert_eq!(snapshot.last_successful(), successful);
        assert!(snapshot.last_failure().is_some());
        assert_eq!(snapshot.error(), Some("offline"));
        assert!(snapshot.is_stale());
        assert!(!snapshot.is_loading());

        let recovery = snapshot.begin("node-a");
        assert!(snapshot.apply(&recovery, Ok(8)));
        assert_eq!(snapshot.data(), Some(&8));
        assert_eq!(snapshot.error(), None);
        assert!(!snapshot.is_stale());
        assert!(snapshot.last_failure().is_none());
    }

    #[test]
    fn target_switch_clears_data_and_rejects_late_completions() {
        let mut snapshot = Snapshot::<u32, &str>::default();
        let initial = snapshot.begin("node-a");
        assert!(snapshot.apply(&initial, Ok(7)));
        let old = snapshot.begin("node-a");
        let current = snapshot.begin("node-b");
        assert_eq!(snapshot.data(), None);
        assert_eq!(snapshot.last_successful(), None);
        assert!(!snapshot.is_stale());
        assert!(!snapshot.apply(&old, Ok(99)));
        assert!(snapshot.is_loading());
        assert!(snapshot.apply(&current, Err("unavailable".into())));
        assert_eq!(snapshot.data(), None);
        assert!(!snapshot.is_stale());
    }

    #[test]
    fn a_restored_answer_is_stale_with_its_own_time_until_another_identity_drops_it() {
        let taken = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        let mut snapshot = Snapshot::<u32, &str>::default();
        snapshot.restore("node-a", 7, taken);
        assert_eq!(snapshot.data(), Some(&7));
        assert!(snapshot.is_stale() && !snapshot.is_loading());
        assert_eq!(snapshot.last_successful(), Some(taken));

        let refresh = snapshot.begin("node-a");
        assert_eq!(snapshot.data(), Some(&7));
        assert!(snapshot.apply(&refresh, Ok(8)));
        assert!(!snapshot.is_stale());

        snapshot.restore("node-a", 9, taken);
        snapshot.begin("node-b");
        assert_eq!(snapshot.data(), None);
        assert_eq!(snapshot.last_successful(), None);
    }

    #[test]
    fn superseded_generation_and_duplicate_completion_are_ignored() {
        let mut snapshot = Snapshot::<u32, &str>::default();
        let old = snapshot.begin("node-a");
        let current = snapshot.begin("node-a");
        assert!(!snapshot.apply(&old, Err("late failure".into())));
        assert_eq!(snapshot.error(), None);
        assert!(snapshot.apply(&current, Ok(4)));
        assert!(!snapshot.apply(&current, Ok(5)));
        assert_eq!(snapshot.data(), Some(&4));

        let switched = snapshot.begin("node-b");
        let returned = snapshot.begin("node-a");
        assert!(!snapshot.apply(&switched, Ok(6)));
        assert!(!snapshot.apply(&old, Ok(7)));
        assert!(snapshot.apply(&returned, Ok(8)));
    }
}
