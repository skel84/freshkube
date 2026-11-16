//! `team.json` beside the preferences (#849 slice 5): the membership a
//! restart keeps. It is a preference of this person, so it lives here and
//! never in `freshkube.yaml`, which a team can share. A token is never in it:
//! that goes in the key store.

use std::path::Path;

use freshkube_core::team::{Membership, SavedTeam};
use freshkube_prefs::{Dir, Document, Layout};

const NAME: &str = crate::mutation::TEAM_FILE;
/// The file is read to at most this many bytes.
const LIMIT: u64 = 2 * 1024 * 1024;

/// The saved membership, read once at start.
pub(super) struct Restored {
    pub(super) membership: Option<Membership>,
    /// Why a file that exists wasn't used, in words with no path or address.
    pub(super) unreadable: Option<String>,
}

pub(super) struct Store {
    document: Document<SavedTeam>,
}

impl Store {
    /// The document beside `preferences`.
    pub(super) fn beside(preferences: &Path) -> Self {
        Self {
            document: Dir::beside(preferences)
                .document(NAME, Layout::Pretty, 0o600)
                .limited_to(LIMIT),
        }
    }

    /// What the file holds. A file that is there and can't be used is said,
    /// once, and the app starts with no team; the file is left for the person
    /// (a copy of a corrupt one is kept by the preferences crate).
    pub(super) fn load(&self) -> Restored {
        let loaded = self.document.load();
        if let Some(problem) = &loaded.problem {
            tracing::warn!("{problem}; starting without a team");
            return Restored {
                membership: None,
                unreadable: Some("The file that keeps your team setup couldn’t be read.".into()),
            };
        }
        let Some(saved) = loaded.value.membership else {
            return Restored {
                membership: None,
                unreadable: None,
            };
        };
        match Membership::from_saved(saved) {
            Ok(membership) => Restored {
                membership: Some(membership),
                unreadable: None,
            },
            Err(why) => {
                tracing::warn!("the saved team setup was not used: {why}");
                Restored {
                    membership: None,
                    unreadable: Some("Your saved team setup couldn’t be used.".into()),
                }
            }
        }
    }

    /// Saves the membership, or nothing for none. A failure is returned for
    /// the page to say; the membership stays in memory either way.
    pub(super) fn save(&self, membership: Option<&Membership>) -> std::io::Result<()> {
        self.document.save(&SavedTeam {
            membership: membership.and_then(Membership::to_saved),
        })
    }
}
