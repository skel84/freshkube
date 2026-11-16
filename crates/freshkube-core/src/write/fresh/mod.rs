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
use crate::team::TeamFloors;
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
    /// `None` when a live window has no setup file to read: it can't say, so
    /// a send refuses (never "send without reading").
    file: Option<PathBuf>,
    subject: Subject,
    /// Whether the entries an action's destinations name are listed in the
    /// file. A legacy `workspace.json` user's entries came from the migration,
    /// so there is nothing in a file to read a destination's level from.
    destinations_listed: bool,
    /// The team's floors the window held when the dialog opened (#849).
    held_floors: TeamFloors,
    /// The saved team, read again at the send step: a floor that appeared
    /// since the dialog opened (a team joined and applied in Settings)
    /// stops this send too.
    team_file: Option<PathBuf>,
}

impl SetupLevels {
    pub fn new(file: impl Into<PathBuf>, subject: Subject) -> Self {
        Self {
            file: Some(file.into()),
            subject,
            destinations_listed: true,
            held_floors: TeamFloors::none(),
            team_file: None,
        }
    }

    /// The floors the team holds, as the window holds them now, and the file
    /// they are saved in, which the send step reads again. The level a send
    /// goes under is then the stricter of the setup file, the floors now
    /// saved, the floors held at opening and the level the app holds; a team
    /// file that can't be read holds every context `read-only`.
    pub fn with_team(mut self, held: TeamFloors, file: Option<PathBuf>) -> Self {
        self.held_floors = held;
        self.team_file = file;
        self
    }

    /// The floors a send is under: those held at opening and those saved now.
    fn floors(&self) -> TeamFloors {
        match &self.team_file {
            Some(file) => self.held_floors.and(&TeamFloors::read(file)),
            None => self.held_floors.clone(),
        }
    }

    /// A legacy `workspace.json` user, whose entries came from the migration
    /// and not from `freshkube.yaml`: their own level is read by kubeconfig
    /// `context`, and an action's destinations have no entry in a file to be
    /// read from.
    pub fn legacy(file: impl Into<PathBuf>, context: impl Into<String>) -> Self {
        Self {
            file: Some(file.into()),
            subject: Subject::Context(context.into()),
            destinations_listed: false,
            held_floors: TeamFloors::none(),
            team_file: None,
        }
    }

    /// A live window with no setup file behind it: [`Self::read`] can't say,
    /// so nothing is sent.
    pub fn unavailable(subject: Subject) -> Self {
        Self {
            file: None,
            subject,
            destinations_listed: true,
            held_floors: TeamFloors::none(),
            team_file: None,
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
    /// decided on one reading. An entry the file doesn't list is gone; where
    /// the entries aren't in the file at all ([`Self::legacy`]) the file has
    /// nothing to say about them (`None`), and they are never refused for it.
    pub fn read_with(
        &self,
        entries: &[String],
    ) -> Result<(Protection, Vec<Option<Protection>>), Unreadable> {
        let Some(file) = &self.file else {
            return Err(Unreadable::NotReadable);
        };
        let bytes = match read_bounded_regular_file(file, MAX_BYTES) {
            Ok(bytes) => Some(bytes),
            Err(BoundedReadError::NotFound) => None,
            Err(_) => return Err(Unreadable::NotReadable),
        };
        let parsed = Parsed::of(bytes.as_deref())?;
        let floors = self.floors();
        let own = parsed.level(&self.subject, &floors)?;
        let others = entries
            .iter()
            .map(|id| {
                if self.destinations_listed {
                    parsed.level(&Subject::Entry(id.clone()), &floors).map(Some)
                } else {
                    Ok(None)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((own, others))
    }

    /// [`Self::read`] on bytes already read; `None` is a missing file.
    pub fn level_in(&self, bytes: Option<&[u8]>) -> Result<Protection, Unreadable> {
        Parsed::of(bytes)?.level(&self.subject, &self.floors())
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

    fn level(&self, subject: &Subject, floors: &TeamFloors) -> Result<Protection, Unreadable> {
        let Parsed::File(recovered) = self else {
            return match subject {
                // No file, no workspace: a context no entry names is guarded,
                // or what the team holds it to.
                Subject::Context(context) => Ok(floors.apply(context, Protection::default())),
                Subject::Entry(_) => Err(Unreadable::Missing),
            };
        };
        let workspace = &recovered.setup.workspace;
        let mut with_floors = recovered.source_recovery.clone();
        with_floors.set_team_floors(floors.clone());
        let recovery = &with_floors;
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

/// How long the setup file may take to read before a send refuses: it is a
/// few kilobytes, so a read that takes this long is not going to answer.
pub(super) const READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// Runs a blocking `read` off the async threads and gives up after
/// `deadline`: a read that doesn't answer is a file that can't say, and holds
/// nothing.
pub(super) async fn read_within<T: Send + 'static>(
    deadline: std::time::Duration,
    read: impl FnOnce() -> Result<T, Unreadable> + Send + 'static,
) -> Result<T, Unreadable> {
    match tokio::time::timeout(deadline, tokio::task::spawn_blocking(read)).await {
        Ok(Ok(answer)) => answer,
        _ => Err(Unreadable::NotReadable),
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
