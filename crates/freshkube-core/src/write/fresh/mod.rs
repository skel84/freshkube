//! The level a write is sent under, read again from `freshkube.yaml` at the
//! send step (#1055, `docs/WRITE_MODEL.md`, "The level at send time").
//!
//! The app defers reading a changed setup file while a dialog is open, so a
//! dialog opened under `guarded` and confirmed after the file was edited to
//! `read-only` or `protected` would still send under the level the app held.
//! [`SetupLevels::read`] reads the file once, [`at_send`] takes the stricter
//! of what it says and the held level, and a file that can't say refuses.
//! A looser level in the file never loosens anything: the held level stands
//! until the person reviews again.
//!
//! No UI types, and the notice names the level only: never the file, a path
//! or a parse error's text.

use std::path::PathBuf;

use super::Protection;
use crate::workspace::MAX_BYTES;
use crate::{BoundedReadError, read_bounded_regular_file, setup};

/// Whose level the setup file is asked for, fixed when the dialog opens so
/// the re-read is against the identity the dialog was for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Subject {
    /// A workspace entry, by its id. An entry the file no longer lists is
    /// gone, which refuses.
    Entry(String),
    /// A kubeconfig context the window has no entry for: the stricter of the
    /// entries now naming it, `guarded` when none does.
    Context(String),
}

/// Why the file could not answer, so nothing is sent. The reason is kept for
/// tests and the audit; the notice never repeats it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unreadable {
    /// The file is missing and the dialog was for an entry.
    Missing,
    /// It is too large, not a regular file, or could not be read.
    NotReadable,
    /// It does not parse.
    Invalid,
    /// It no longer lists the entry (or skipped its row).
    EntryGone,
}

/// Why a send is refused after the re-read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendRefusal {
    /// The file's level is stricter than the held one.
    Stricter(Protection),
    /// The file could not say.
    Unreadable(Unreadable),
}

impl SendRefusal {
    /// The notice: the level only, no file, path or parse error.
    pub fn text(self) -> String {
        match self {
            SendRefusal::Stricter(level) => {
                format!("This cluster is now {}; review again.", level.key())
            }
            SendRefusal::Unreadable(Unreadable::EntryGone) => {
                "This cluster is no longer in freshkube.yaml; review again.".to_owned()
            }
            SendRefusal::Unreadable(_) => {
                "This cluster's level can't be read from freshkube.yaml; review again.".to_owned()
            }
        }
    }
}

/// The setup file and the identity whose level is read from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupLevels {
    file: PathBuf,
    subject: Subject,
}

impl SetupLevels {
    pub fn new(file: impl Into<PathBuf>, subject: Subject) -> Self {
        Self {
            file: file.into(),
            subject,
        }
    }

    pub fn subject(&self) -> &Subject {
        &self.subject
    }

    /// The subject's level in the file as it is now, from one bounded read
    /// and one lenient parse of those bytes, as Settings reads the file: a
    /// skipped unrelated row doesn't refuse everything. Blocking: call it off
    /// the UI thread.
    pub fn read(&self) -> Result<Protection, Unreadable> {
        self.read_with(&[]).map(|(own, _)| own)
    }

    /// [`Self::read`], and the level of each entry in `entries` from the same
    /// parse, so the strictest of an action's cluster and its destinations is
    /// decided on one reading. An entry the file doesn't list is gone.
    pub fn read_with(
        &self,
        entries: &[String],
    ) -> Result<(Protection, Vec<Protection>), Unreadable> {
        let bytes = match read_bounded_regular_file(&self.file, MAX_BYTES) {
            Ok(bytes) => Some(bytes),
            Err(BoundedReadError::NotFound) => None,
            Err(_) => return Err(Unreadable::NotReadable),
        };
        let parsed = Parsed::of(bytes.as_deref())?;
        let own = parsed.level(&self.subject)?;
        let others = entries
            .iter()
            .map(|id| parsed.level(&Subject::Entry(id.clone())))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((own, others))
    }

    /// [`Self::read`] on bytes already read; `None` is a missing file.
    pub fn level_in(&self, bytes: Option<&[u8]>) -> Result<Protection, Unreadable> {
        Parsed::of(bytes)?.level(&self.subject)
    }
}

/// The file's bytes, parsed once.
enum Parsed {
    Missing,
    File(Box<setup::Recovered>),
}

impl Parsed {
    fn of(bytes: Option<&[u8]>) -> Result<Self, Unreadable> {
        match bytes {
            None => Ok(Parsed::Missing),
            Some(bytes) => setup::read(bytes)
                .map(|recovered| Parsed::File(Box::new(recovered)))
                .map_err(|_| Unreadable::Invalid),
        }
    }

    fn level(&self, subject: &Subject) -> Result<Protection, Unreadable> {
        let Parsed::File(recovered) = self else {
            return match subject {
                // No file, no workspace: a context no entry names is guarded.
                Subject::Context(_) => Ok(Protection::default()),
                Subject::Entry(_) => Err(Unreadable::Missing),
            };
        };
        let (workspace, recovery) = (&recovered.setup.workspace, &recovered.source_recovery);
        match subject {
            Subject::Context(context) => Ok(workspace.level_of(context, recovery)),
            Subject::Entry(id) => {
                let entry = workspace
                    .clusters
                    .iter()
                    .find(|entry| &entry.id == id)
                    .ok_or(Unreadable::EntryGone)?;
                // The stricter of the entry's own and what a skipped row or
                // another entry on its context sets.
                Ok(entry
                    .level()
                    .stricter(workspace.level_of(&entry.context, recovery)))
            }
        }
    }
}

/// The level a write is sent under, from the level the app holds and what the
/// file said just now. The file never loosens: equal or looser gives `held`.
/// Stricter, or a file that can't say, is a refusal.
pub fn at_send(
    held: Protection,
    fresh: Result<Protection, Unreadable>,
) -> Result<Protection, SendRefusal> {
    match fresh {
        Ok(level) if level < held => Err(SendRefusal::Stricter(level)),
        Ok(_) => Ok(held),
        Err(why) => Err(SendRefusal::Unreadable(why)),
    }
}

#[cfg(test)]
mod tests;
