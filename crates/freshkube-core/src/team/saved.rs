//! What a restart keeps of a membership (#849): the source, the version the
//! person reviewed and applied, which context each team cluster is, the
//! floors held, and the overrides (yours now, and from before joining).
//!
//! It is a person's own preference, so it is saved beside the preferences by
//! the app and never in `freshkube.yaml`, which a team can share. It holds no
//! credential: the source is its address, branch and path (an address with
//! user info or a query is refused at joining and again here), and a token
//! lives only in the key store. A waiting version and a failure are not
//! saved: the next fetch brings them again.
//!
//! The file is read as untrusted text. The source is parsed again, and a
//! saved level is read as the stricter reading (an unknown word is guarded).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::file::Version;
use super::mapping::Choice;
use super::source::{Source, redact};

/// The most keys a saved version or set of overrides may have: far above any
/// team file (which is capped at 256 KiB), and a bound on a damaged file.
const MAX_KEYS: usize = 20_000;

/// The file: nothing, or one membership.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedTeam {
    pub membership: Option<SavedMembership>,
}

/// Where the team file comes from, as typed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSource {
    pub address: String,
    pub git_ref: String,
    pub path: String,
}

impl SavedSource {
    pub fn of(source: &Source) -> Self {
        match source {
            Source::Git(git) => Self {
                address: git.url().to_string(),
                git_ref: git.git_ref().to_string(),
                path: git.path().to_string(),
            },
            Source::Https(https) => Self {
                address: https.url().to_string(),
                git_ref: String::new(),
                path: String::new(),
            },
        }
    }

    /// The source again, through the same rules as joining. `None` for an
    /// address those rules refuse.
    pub fn parse(&self) -> Option<Source> {
        Source::parse(&self.address, &self.git_ref, &self.path).ok()
    }
}

/// One membership as saved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedMembership {
    pub source: SavedSource,
    pub applied: Option<Version>,
    pub applied_at: Option<DateTime<Utc>>,
    pub overrides: BTreeMap<String, Value>,
    pub before_joining: BTreeMap<String, Value>,
    /// Held floors by cluster id, spelled as the file spells a level.
    pub held: BTreeMap<String, String>,
    /// The contexts chosen, by cluster id.
    pub mapping: BTreeMap<String, Choice>,
}

/// Why a saved membership was not taken. The words never repeat the address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unusable {
    /// The address is one that joining refuses.
    Source,
    /// More keys than any team file has.
    TooLarge,
    /// It names no applied version, so there is nothing to keep.
    NoVersion,
}

impl std::fmt::Display for Unusable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Unusable::Source => "its address is one that joining refuses",
            Unusable::TooLarge => "it holds more than a team file can",
            Unusable::NoVersion => "it holds no reviewed version",
        })
    }
}

impl SavedMembership {
    /// Checks what can be checked without a kubeconfig, and returns the
    /// source it parses to.
    pub fn check(&self) -> Result<Source, Unusable> {
        let applied = self.applied.as_ref().ok_or(Unusable::NoVersion)?;
        if applied.values().len() > MAX_KEYS
            || self.overrides.len() > MAX_KEYS
            || self.before_joining.len() > MAX_KEYS
            || self.held.len() > MAX_KEYS
            || self.mapping.len() > MAX_KEYS
        {
            return Err(Unusable::TooLarge);
        }
        self.source.parse().ok_or(Unusable::Source)
    }

    /// The address as a log may show it.
    pub fn shown_address(&self) -> String {
        redact(&self.source.address)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::team::example::{self, Shape};
    use crate::team::{Choice, Membership};
    use crate::write::Protection;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 16, 9, 0, 0).unwrap()
    }

    fn joined() -> Membership {
        let (mut membership, _) = example::membership(Shape::Joined, now()).expect("joined");
        membership.set_override("clusters[core-fra].role", Value::String("edge".into()));
        membership
    }

    fn through_text(saved: &SavedTeam) -> SavedTeam {
        serde_json::from_str(&serde_json::to_string_pretty(saved).unwrap()).unwrap()
    }

    #[test]
    fn a_membership_survives_a_restart_with_what_was_reviewed() {
        let before = joined();
        let saved = SavedTeam {
            membership: before.to_saved(),
        };
        let after = Membership::from_saved(through_text(&saved).membership.unwrap())
            .expect("a saved membership");
        assert_eq!(after.source(), before.source());
        assert_eq!(after.applied(), before.applied());
        assert_eq!(after.applied_at(), before.applied_at());
        assert_eq!(after.overrides(), before.overrides());
        assert_eq!(after.mapping(), before.mapping());
        assert_eq!(after.held_floors(), before.held_floors());
        // The copy to go back to is kept too.
        assert_eq!(
            after.clone().leave(crate::team::Leave::GoBack),
            before.clone().leave(crate::team::Leave::GoBack)
        );
        // Nothing waits and nothing failed after a restart.
        assert!(after.pending().is_none() && after.failure().is_none());
    }

    #[test]
    fn a_membership_with_nothing_applied_is_not_saved() {
        let joining = Membership::join(example::source(), Default::default());
        assert!(joining.to_saved().is_none());
    }

    #[test]
    fn the_saved_source_holds_no_credential_and_is_read_again_through_the_rules() {
        let saved = joined().to_saved().expect("saved");
        let text = serde_json::to_string(&saved).unwrap();
        assert!(!text.contains('@') || !text.contains("://"));
        assert!(!text.contains('?') && !text.contains('#'));
        for address in [
            "http://git.acme.example/ops/setup",
            "https://user:secret@git.acme.example/ops/setup.git",
            "https://git.acme.example/ops/setup.git?token=abc",
            "/home/someone/setup",
        ] {
            let mut tampered = saved.clone();
            tampered.source.address = address.into();
            let refused = Membership::from_saved(tampered).expect_err("refused");
            assert_eq!(refused, Unusable::Source, "{address}");
            // The words never repeat it.
            assert!(!refused.to_string().contains("secret"));
        }
    }

    #[test]
    fn a_held_level_that_is_not_a_level_reads_read_only() {
        let mut saved = joined().to_saved().expect("saved");
        saved.held.insert("core-fra".into(), "protected".into());
        saved.held.insert("cicd-fra".into(), "sideways".into());
        let after = Membership::from_saved(saved).expect("saved");
        assert_eq!(after.held_floors()["core-fra"], Protection::Protected);
        assert_eq!(after.held_floors()["cicd-fra"], Protection::ReadOnly);
    }

    #[test]
    fn a_choice_for_a_cluster_the_version_no_longer_names_is_dropped() {
        let mut saved = joined().to_saved().expect("saved");
        saved.mapping.insert("gone-fra".into(), Choice::LeftOut);
        let after = Membership::from_saved(saved).expect("saved");
        assert!(after.mapping().clusters().all(|(id, _)| id != "gone-fra"));
        assert!(!after.mapping().chosen().contains_key("gone-fra"));
    }

    #[test]
    fn a_file_without_a_reviewed_version_or_with_too_much_is_unusable() {
        let mut saved = joined().to_saved().expect("saved");
        let mut none = saved.clone();
        none.applied = None;
        assert_eq!(
            Membership::from_saved(none).unwrap_err(),
            Unusable::NoVersion
        );
        for index in 0..=MAX_KEYS {
            saved
                .overrides
                .insert(format!("k{index}"), Value::Bool(true));
        }
        assert_eq!(
            Membership::from_saved(saved).unwrap_err(),
            Unusable::TooLarge
        );
    }
}
