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
//! The file is read as untrusted text. The source is parsed again, the
//! version is rebuilt by parsing the team file's own text again
//! ([`Version::parse`], so the allowlist runs whoever wrote the file), the
//! keys of your own values must be ones a team file could set, and a held
//! level that isn't a level reads as read-only.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::file::{KEY_FILE, Version, key_allowed};
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

/// A version as saved: the file's text and what the fetch read beside it. What
/// the team sets and what it ignores are never saved: they are worked out
/// again from the text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedVersion {
    pub id: String,
    pub file: String,
    pub key_fingerprints: BTreeMap<String, String>,
}

impl SavedVersion {
    pub fn of(version: &Version) -> Option<Self> {
        Some(Self {
            id: version.id().to_string(),
            file: String::from_utf8(version.bytes().to_vec()).ok()?,
            key_fingerprints: version.key_fingerprints().clone(),
        })
    }

    /// The version again, by the rules a fetched file goes through. A
    /// fingerprint is kept only for the key file the text names.
    pub fn parse(&self) -> Result<Version, Unusable> {
        let parsed = Version::parse(&self.id, self.file.as_bytes(), BTreeMap::new())
            .map_err(|_| Unusable::Version)?;
        let named = parsed.values().get(KEY_FILE).and_then(Value::as_str);
        let fingerprints = self
            .key_fingerprints
            .iter()
            .filter(|(path, _)| Some(path.as_str()) == named)
            .map(|(path, print)| (path.clone(), print.clone()))
            .collect();
        Version::parse(&self.id, self.file.as_bytes(), fingerprints).map_err(|_| Unusable::Version)
    }
}

/// Your own values, kept only under keys a team file could set.
pub fn allowed_overrides(saved: BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    saved
        .into_iter()
        .filter(|(key, _)| key_allowed(key))
        .collect()
}

/// One membership as saved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedMembership {
    pub source: SavedSource,
    pub applied: Option<SavedVersion>,
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
    /// The team file's text isn't one the rules take.
    Version,
    /// More keys than any team file has.
    TooLarge,
    /// It names no applied version, so there is nothing to keep.
    NoVersion,
}

impl std::fmt::Display for Unusable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Unusable::Source => "its address is one that joining refuses",
            Unusable::Version => "its team file is not one the rules take",
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
        if applied.file.len() as u64 > crate::workspace::MAX_BYTES
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
        let text = serde_json::to_string(&saved.source).unwrap();
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

    /// A saved team whose team file says `text`, as a person might edit it.
    fn with_file(text: &str) -> SavedMembership {
        let mut saved = joined().to_saved().expect("saved");
        let applied = saved.applied.as_mut().unwrap();
        applied.file = text.to_string();
        applied.id = "edited".into();
        saved
    }

    #[test]
    fn a_saved_version_is_parsed_again_so_a_forbidden_key_comes_back_without_it() {
        // The text carries what a team may never set: a context, agent
        // access, a token, what writes may do, and a path on this computer.
        let saved = with_file(
            "version: 1\nclusters:\n  - id: core-fra\n    role: core\n    protection: protected\n    context: evil-ctx\n    agent:\n      enabled: true\n    token: secret\n    writes: all\n    talosconfig: /etc/passwd\n",
        );
        // A saved copy of what was read and what was ignored is not a thing
        // the file can carry: it only has the text.
        let after = Membership::from_saved(saved).expect("the rules take the text");
        let version = after.applied().unwrap();
        for forbidden in ["context", "agent", "token", "writes", "talosconfig"] {
            assert!(
                version.values().keys().all(|key| !key.contains(forbidden)),
                "{forbidden} came back: {:?}",
                version.values().keys().collect::<Vec<_>>()
            );
        }
        assert!(
            version
                .values()
                .contains_key("clusters[core-fra].protection")
        );
        // And it says what it ignored, worked out from the text.
        assert!(version.ignored().len() >= 5, "{:?}", version.ignored());
    }

    #[test]
    fn a_saved_version_the_rules_refuse_makes_the_file_unusable() {
        for text in ["not: [a, team", "version: 2\n", "clusters: 3\n"] {
            assert_eq!(
                Membership::from_saved(with_file(text)).unwrap_err(),
                Unusable::Version,
                "{text}"
            );
        }
    }

    #[test]
    fn over_https_a_saved_id_must_be_the_texts_hash() {
        let mut saved = with_file("version: 1\n");
        saved.source = SavedSource {
            address: "https://setup.acme.example/team/freshkube.yaml".into(),
            git_ref: String::new(),
            path: String::new(),
        };
        // The id of an edited file that claims a newer fetch is refused.
        assert_eq!(
            Membership::from_saved(saved.clone()).unwrap_err(),
            Unusable::Version
        );
        saved.applied.as_mut().unwrap().id = crate::team::fetch::https::content_id(b"version: 1\n");
        assert!(Membership::from_saved(saved).is_ok());
    }

    #[test]
    fn your_values_are_kept_only_under_keys_a_team_file_could_set() {
        let mut saved = joined().to_saved().expect("saved");
        saved.overrides.clear();
        saved.before_joining.clear();
        for key in [
            "clusters[core-fra].protection",
            "coroot.project",
            "clusters[core-fra].metrics.source.url",
        ] {
            saved.overrides.insert(key.into(), "x".into());
            saved.before_joining.insert(key.into(), "x".into());
        }
        for key in [
            "clusters[core-fra].context",
            "clusters[core-fra].agent.enabled",
            "clusters[core-fra]",
            "clusters[core-fra].protection.extra",
            "kubeconfig",
            "writes",
            "clusters[../x].protection",
            "coroot",
            "..",
            "",
        ] {
            saved.overrides.insert(key.into(), "x".into());
            saved.before_joining.insert(key.into(), "x".into());
        }
        let after = Membership::from_saved(saved).unwrap();
        let kept: Vec<&str> = after.overrides().keys().map(String::as_str).collect();
        assert_eq!(
            kept,
            [
                "clusters[core-fra].metrics.source.url",
                "clusters[core-fra].protection",
                "coroot.project"
            ]
        );
        let back = after.leave(crate::team::Leave::GoBack);
        assert_eq!(back.len(), 3, "{back:?}");
    }
}
