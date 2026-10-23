//! The per-cluster access gate. Agent access is off for every cluster until a
//! person turns it on, and `read` asks this gate for a lease before it builds
//! any client, so a cluster that is off sends nothing.
//!
//! The state is in memory and cheap to clone. The grants are `(entry,
//! context)` pairs: a grant counts only while the entry opens the context it
//! was given for, so a renamed entry or a changed context reads as off.
//! Turning a cluster off ends its leases, and a read holding one answers
//! `not_enabled` and drops what it had.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use super::refusal::{Code, Refusal};
use crate::constants::agent::MAX_READS_PER_SESSION;

struct Grant {
    entry: String,
    context: String,
    /// Sent `true` when the grant ends, which ends its leases.
    ended: watch::Sender<bool>,
}

#[derive(Default)]
struct Grants(Vec<Grant>);

/// Which clusters an agent may read, and how many reads a session has left.
/// Clones share both.
#[derive(Clone, Default)]
pub struct AgentAccess {
    grants: Arc<Mutex<Grants>>,
    reads: Arc<AtomicU32>,
}

impl AgentAccess {
    /// A gate that grants nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same grants with a read count of their own: one per session, so
    /// a session's 500 reads are its own and a change to a grant reaches it.
    pub fn for_session(&self) -> Self {
        Self {
            grants: Arc::clone(&self.grants),
            reads: Arc::default(),
        }
    }

    fn grants(&self) -> std::sync::MutexGuard<'_, Grants> {
        self.grants
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Grants `entry` while it opens `context`. A grant for the same entry
    /// and another context replaces it and ends the leases of the old one.
    pub fn allow(&self, entry: &str, context: &str) {
        let mut grants = self.grants();
        if let Some(at) = grants.0.iter().position(|grant| grant.entry == entry) {
            if grants.0[at].context == context {
                return;
            }
            let old = grants.0.remove(at);
            old.ended.send_replace(true);
        }
        grants.0.push(Grant {
            entry: entry.to_owned(),
            context: context.to_owned(),
            ended: watch::channel(false).0,
        });
    }

    /// Ends the grant for `entry` and every read in flight under it.
    pub fn revoke(&self, entry: &str) {
        let mut grants = self.grants();
        grants.0.retain(|grant| {
            let keep = grant.entry != entry;
            if !keep {
                grant.ended.send_replace(true);
            }
            keep
        });
    }

    /// Makes the grants exactly `wanted`, ending what is no longer in it.
    pub fn replace(&self, wanted: &[(String, String)]) {
        let stale: Vec<String> = self
            .grants()
            .0
            .iter()
            .filter(|grant| {
                !wanted
                    .iter()
                    .any(|(entry, context)| *entry == grant.entry && *context == grant.context)
            })
            .map(|grant| grant.entry.clone())
            .collect();
        for entry in stale {
            self.revoke(&entry);
        }
        for (entry, context) in wanted {
            self.allow(entry, context);
        }
    }

    /// The grants, as `(entry, context)` pairs.
    pub fn granted(&self) -> Vec<(String, String)> {
        self.grants()
            .0
            .iter()
            .map(|grant| (grant.entry.clone(), grant.context.clone()))
            .collect()
    }

    /// Whether `entry` opening `context` is granted now.
    pub fn is_on(&self, entry: &str, context: &str) -> bool {
        self.lease(entry, context).is_some()
    }

    /// A lease on `entry` opening `context`, or `None` when it is off, and
    /// also when the entry is granted for another context.
    pub(super) fn lease(&self, entry: &str, context: &str) -> Option<Lease> {
        let grants = self.grants();
        let grant = grants
            .0
            .iter()
            .find(|grant| grant.entry == entry && grant.context == context)?;
        Some(Lease(grant.ended.subscribe()))
    }

    /// Counts a read against the session's budget; the 501st is refused.
    /// Only a read that holds a lease is counted.
    pub(super) fn spend(&self) -> Result<(), Refusal> {
        let used = self.reads.fetch_add(1, Ordering::SeqCst);
        if used >= MAX_READS_PER_SESSION {
            // Not counted past the limit, so the counter cannot wrap.
            self.reads.fetch_sub(1, Ordering::SeqCst);
            return Err(Refusal::new(
                Code::LimitReached,
                "this session has used its reads",
            ));
        }
        Ok(())
    }
}

impl std::fmt::Debug for AgentAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentAccess")
            .field("granted", &self.grants().0.len())
            .field("reads", &self.reads.load(Ordering::SeqCst))
            .finish()
    }
}

/// The right to read one cluster while its grant lasts.
pub(super) struct Lease(watch::Receiver<bool>);

impl Lease {
    pub(super) fn is_live(&self) -> bool {
        !*self.0.borrow()
    }

    /// Completes when the grant ends.
    pub(super) async fn ended(&mut self) {
        let _ = self.0.wait_for(|ended| *ended).await;
    }
}
