//! `agent.json` beside the preferences: the clusters a person let an agent
//! read, and the audit line each change leaves.
//!
//! It is not in `freshkube.yaml`: that file is what a person might share or
//! keep in dotfiles, while sending a cluster's data to their model provider
//! is their own consent, as with `follows.json`. A missing or unreadable file
//! grants nothing.
//!
//! [`Access::set_access`] is the one way to change a grant. It saves the file,
//! appends to `<preferences>/audit/agent.jsonl` and updates the gate. Turning
//! access on does all three or none; turning it off ends access first and
//! reports a failed save, never keeping access on.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use freshkube_core::agent_reads::AgentAccess;
use freshkube_core::write::AuditStore;
use freshkube_prefs::{Dir, Document, Layout};
use serde::{Deserialize, Serialize};

const NAME: &str = "agent.json";
const AUDIT_NAME: &str = "agent.jsonl";
/// The file is read to at most this many bytes, as `follows.json` is.
const LIMIT: u64 = 1024 * 1024;
/// More grants than a workspace has entries are ignored.
const MAX_GRANTS: usize = 256;

/// One cluster an agent may read, while its entry opens `context`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub entry: String,
    pub context: String,
    pub since: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub grants: Vec<Grant>,
}

/// A line of `audit/agent.jsonl`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditLine {
    pub at: DateTime<Utc>,
    pub entry: String,
    pub context: String,
    /// `on` or `off`.
    pub access: String,
    /// Where the change was made, such as `settings` or `egress card`.
    pub from: String,
}

/// The saved grants and the gate that enforces them.
pub struct Access {
    document: Document<Saved>,
    audit: AuditStore,
    gate: AgentAccess,
    saved: Mutex<Saved>,
}

impl Access {
    /// Loads the grants beside `preferences` into `gate`. Nothing is granted
    /// if the file is missing; one that cannot be read or parsed grants
    /// nothing and says why once.
    pub fn open(preferences: &Path, gate: &AgentAccess) -> Self {
        let dir = Dir::beside(preferences);
        let document: Document<Saved> = dir.document(NAME, Layout::Pretty, 0o600).limited_to(LIMIT);
        let loaded = document.load();
        if let Some(problem) = &loaded.problem {
            tracing::warn!("{problem}; agent access is off everywhere");
        }
        let mut saved = loaded.value;
        saved.grants.truncate(MAX_GRANTS);
        let wanted: Vec<(String, String)> = saved
            .grants
            .iter()
            .map(|grant| (grant.entry.clone(), grant.context.clone()))
            .collect();
        gate.replace(&wanted);
        Self {
            document,
            audit: AuditStore::named(audit_dir(preferences), AUDIT_NAME),
            gate: gate.clone(),
            saved: Mutex::new(saved),
        }
    }

    /// The gate this keeps.
    pub fn gate(&self) -> &AgentAccess {
        &self.gate
    }

    /// The saved grants.
    pub fn grants(&self) -> Vec<Grant> {
        self.state().grants.clone()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, Saved> {
        self.saved
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Turns agent access for `entry`, opening `context`, on or off. `from`
    /// says where the change was made.
    ///
    /// **On is all or none:** the file, the audit line and the gate all
    /// change, or the error is returned, the file is put back and the gate is
    /// unchanged. **Off fails closed:** the gate is turned off first, which
    /// ends the reads in flight on the cluster, and then the file and the
    /// audit line are written; if either fails the error is returned but
    /// access stays off, and the saved state in memory drops the grant so the
    /// next save writes it out.
    pub fn set_access(&self, entry: &str, context: &str, on: bool, from: &str) -> io::Result<()> {
        let mut saved = self.state();
        let before = saved.clone();
        let mut after = before.clone();
        after.grants.retain(|grant| grant.entry != entry);
        if on {
            after.grants.push(Grant {
                entry: entry.to_owned(),
                context: context.to_owned(),
                since: Utc::now(),
            });
        }
        let line = AuditLine {
            at: Utc::now(),
            entry: entry.to_owned(),
            context: context.to_owned(),
            access: if on { "on" } else { "off" }.to_owned(),
            from: from.to_owned(),
        };
        if !on {
            self.gate.revoke(entry);
            *saved = after.clone();
            let written = self.document.save(&after);
            let audited = self.audit.append_json(&line);
            return written.and(audited);
        }
        self.document.save(&after)?;
        if let Err(error) = self.audit.append_json(&line) {
            // The change is not made without its line.
            if let Err(undo) = self.document.save(&before) {
                tracing::warn!("agent access could not be put back after a failed audit: {undo}");
            }
            return Err(error);
        }
        self.gate.allow(entry, context);
        *saved = after;
        Ok(())
    }

    /// The audit log's directory, for a test or a viewer.
    pub fn audit_path(&self) -> PathBuf {
        self.audit.path()
    }

    pub fn file_path(&self) -> &Path {
        self.document.path()
    }
}

fn audit_dir(preferences: &Path) -> PathBuf {
    preferences.with_file_name("audit")
}

impl std::fmt::Debug for Access {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Access")
            .field("grants", &self.state().grants.len())
            .finish()
    }
}
