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
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use freshkube_core::agent_reads::AgentAccess;
use freshkube_core::write::AuditStore;
use freshkube_prefs::{Dir, Document, Layout};
use serde::{Deserialize, Serialize};

const NAME: &str = "agent.json";
/// Beside `agent.json`, written when turning access off could not be saved,
/// and cleared only by a save that succeeds. It does not depend on
/// `agent.json`, so a file that still says `on` is not trusted while it
/// stands.
const PENDING_NAME: &str = "agent-pending.json";
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

/// Where opencode keeps its logins and history for an agent session. `Own` is
/// a directory Freshkube owns; `Shared` is the person's real one, so the
/// investigations land in their own opencode history and any config saved
/// there loads (`profile::Data`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataMode {
    #[default]
    Own,
    Shared,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub grants: Vec<Grant>,
    /// `data: shared` in the file switches opencode to the person's own data.
    pub data: DataMode,
    /// The opencode the person chose with "Choose a file", tried first.
    pub opencode: Option<PathBuf>,
    /// The model the person chose, applied to every new session.
    pub model: Option<String>,
}

/// A revocation that has not reached `agent.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revoked {
    pub entry: String,
    pub context: String,
    pub since: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Pending {
    revoked: Vec<Revoked>,
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
    pending: Document<Pending>,
    /// The entries whose access was held off at open because a revocation
    /// had not been saved; the dock says why.
    held_off: Vec<String>,
    audit: AuditStore,
    gate: AgentAccess,
    saved: Mutex<Saved>,
    /// Turning access off could not be written: the file may still hold the
    /// grant and would turn it back on at the next start. The gate is off
    /// regardless; [`Access::retry_save`] writes the file again.
    unsaved: AtomicBool,
    /// The marker could not be written either; see
    /// [`Access::revocation_remembered`].
    unremembered: AtomicBool,
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
        // A revocation that never reached the file wins over what the file
        // says: those entries start off. A marker that can't be read is not
        // known to be empty, so nothing is granted.
        let pending: Document<Pending> = dir
            .document(PENDING_NAME, Layout::Compact, 0o600)
            .limited_to(LIMIT);
        let marker = pending.load();
        let mut held_off = Vec::new();
        let mut unsaved = false;
        if let Some(problem) = &marker.problem {
            tracing::warn!("{problem}; agent access is off everywhere");
            held_off.extend(saved.grants.iter().map(|grant| grant.entry.clone()));
            saved.grants.clear();
            unsaved = true;
        }
        for revoked in &marker.value.revoked {
            if saved
                .grants
                .iter()
                .any(|grant| grant.entry == revoked.entry)
            {
                held_off.push(revoked.entry.clone());
            }
            saved.grants.retain(|grant| grant.entry != revoked.entry);
            unsaved = true;
        }
        held_off.dedup();
        let wanted: Vec<(String, String)> = saved
            .grants
            .iter()
            .map(|grant| (grant.entry.clone(), grant.context.clone()))
            .collect();
        gate.replace(&wanted);
        let access = Self {
            document,
            pending,
            held_off,
            audit: AuditStore::named(audit_dir(preferences), AUDIT_NAME),
            gate: gate.clone(),
            saved: Mutex::new(saved),
            unsaved: AtomicBool::new(unsaved),
            unremembered: AtomicBool::new(false),
        };
        // Try the file again now; the marker stays until a save succeeds.
        if unsaved {
            let _ = access.retry_save();
        }
        access
    }

    /// The entries held off at open because turning their access off was
    /// never saved. They start off, whatever `agent.json` said.
    pub fn held_off(&self) -> &[String] {
        &self.held_off
    }

    /// The revocations not yet saved, for a test or a viewer.
    pub fn pending_revocations(&self) -> Vec<Revoked> {
        self.pending.load().value.revoked
    }

    /// A save of `agent.json` succeeded: nothing is pending any more.
    fn saved_ok(&self) {
        self.unsaved.store(false, Ordering::SeqCst);
        self.unremembered.store(false, Ordering::SeqCst);
        if self.pending.path().exists() {
            let cleared = self.pending.save(&Pending::default());
            if let Err(error) = cleared {
                tracing::warn!("the agent access marker could not be cleared: {error}");
            }
        }
    }

    /// Writes the revocation of `entry` to the marker, apart from
    /// `agent.json`, before `agent.json` is saved: a crash or a failed save
    /// after it leaves the cluster off at the next start. Returns whether the
    /// marker was written.
    fn remember(&self, entry: &str, context: &str) -> bool {
        let mut pending = self.pending.load().value;
        pending.revoked.retain(|revoked| revoked.entry != entry);
        pending.revoked.push(Revoked {
            entry: entry.to_owned(),
            context: context.to_owned(),
            since: Utc::now(),
        });
        match self.pending.save(&pending) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!("the agent access marker for {entry} could not be written: {error}");
                self.unremembered.store(true, Ordering::SeqCst);
                false
            }
        }
    }

    /// Whether the last revocation that could not be saved is remembered
    /// beside the file, so the cluster stays off at the next start. False
    /// when the marker could not be written either (a full disk, a folder
    /// that can't be written): then access comes back at the next start
    /// unless a save succeeds first.
    pub fn revocation_remembered(&self) -> bool {
        !self.unremembered.load(Ordering::SeqCst)
    }

    /// Where opencode keeps its data in this setup.
    pub fn data_mode(&self) -> DataMode {
        self.state().data
    }

    /// The opencode file the person chose, if any.
    pub fn opencode_path(&self) -> Option<PathBuf> {
        self.state().opencode.clone()
    }

    /// Keeps the opencode file the person chose (or forgets it) in `agent.json`.
    pub fn set_opencode_path(&self, path: Option<PathBuf>) -> io::Result<()> {
        let mut saved = self.state();
        let mut after = saved.clone();
        after.opencode = path;
        self.document.save(&after)?;
        *saved = after;
        self.saved_ok();
        Ok(())
    }

    /// The model the person chose, if any.
    pub fn model(&self) -> Option<String> {
        self.state().model.clone()
    }

    /// Keeps the model the person chose in `agent.json`. A failed save is
    /// returned; the caller keeps the choice for the session.
    pub fn set_model(&self, model: Option<String>) -> io::Result<()> {
        let mut saved = self.state();
        let mut after = saved.clone();
        after.model = model;
        self.document.save(&after)?;
        *saved = after;
        self.saved_ok();
        Ok(())
    }

    /// Whether a change that turned access off could not be written, so that
    /// the file still holds a grant the gate has ended. It would come back at
    /// the next start unless [`Access::retry_save`] succeeds first.
    pub fn save_pending(&self) -> bool {
        self.unsaved.load(Ordering::SeqCst)
    }

    /// Writes the grants as they are now, after a failed save.
    pub fn retry_save(&self) -> io::Result<()> {
        let saved = self.state();
        self.document.save(&saved)?;
        self.saved_ok();
        Ok(())
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
            // The marker first: whatever happens to the file, the cluster
            // stays off at the next start.
            self.remember(entry, context);
            let written = self.document.save(&after);
            match &written {
                Ok(()) => self.saved_ok(),
                Err(_) => self.unsaved.store(true, Ordering::SeqCst),
            }
            let audited = self.audit.append_json(&line);
            return written.and(audited);
        }
        self.document.save(&after)?;
        self.saved_ok();
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
