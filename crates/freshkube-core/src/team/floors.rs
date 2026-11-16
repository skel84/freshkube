//! The team's floors as the level overlay reads them (#849 slice 5C): for each
//! of your kubeconfig contexts, the loosest level the team allows it.
//!
//! A floor is only ever combined with [`Protection::stricter`], so it can
//! tighten a context and never loosen one. It is built from three things and
//! nothing else: the version you reviewed and applied, the floors held at
//! the stricter level, and the context you chose for each team cluster. A
//! waiting version has no floor until it is applied, a failed or stale fetch
//! changes nothing, and a cluster you left out or never mapped has none.
//! A floor is never inferred from a context's name.
//!
//! When the saved team can't be read, the floors can't be known, so every
//! context is held at `read-only`: the one level that cannot loosen a floor.

use std::collections::BTreeMap;
use std::path::Path;

use super::mapping::Choice;
use super::membership::Membership;
use super::merge;
use super::saved::SavedTeam;
use crate::write::Protection;
use crate::{BoundedReadError, read_bounded_regular_file};

/// The most a saved team is read to, as the page's store reads it.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// The saved file's version this app writes and reads.
const FILE_VERSION: u64 = 1;

/// What the team holds each context to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TeamFloors {
    by_context: BTreeMap<String, Protection>,
    /// The saved team couldn't be read: every context is `read-only`.
    unreadable: bool,
}

impl TeamFloors {
    /// No team: no floor anywhere.
    pub fn none() -> Self {
        Self::default()
    }

    /// The saved team can't be read, so every context is held `read-only`.
    pub fn unreadable() -> Self {
        Self {
            by_context: BTreeMap::new(),
            unreadable: true,
        }
    }

    pub fn is_unreadable(&self) -> bool {
        self.unreadable
    }

    /// The floors of a membership: only what is applied, held or mapped.
    pub fn from_membership(membership: &Membership) -> Self {
        let mut floors = Self::none();
        if membership.applied().is_none() {
            return floors;
        }
        let values = membership.team_values();
        for (id, choice) in membership.mapping().clusters() {
            let Some(Choice::Context(context)) = choice else {
                continue;
            };
            if let Some(floor) = merge::floor(id, &values) {
                floors.raise(context, floor);
            }
        }
        floors
    }

    /// The floors of what the file says, read at the point of use: nothing
    /// for a file that isn't there, every context `read-only` for one that is
    /// and can't be read, is from a newer app, or can't be taken up.
    pub fn read(file: &Path) -> Self {
        let bytes = match read_bounded_regular_file(file, MAX_BYTES) {
            Ok(bytes) => bytes,
            Err(BoundedReadError::NotFound) => return Self::none(),
            Err(_) => return Self::unreadable(),
        };
        Self::from_bytes(&bytes)
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        let Ok(root) = serde_json::from_slice::<serde_json::Value>(bytes) else {
            return Self::unreadable();
        };
        // A file with no version is version 1; one from a newer app is not
        // ours to read.
        if root
            .get("version")
            .is_some_and(|v| v.as_u64() != Some(FILE_VERSION))
        {
            return Self::unreadable();
        }
        let Ok(saved) = serde_json::from_value::<SavedTeam>(root) else {
            return Self::unreadable();
        };
        match saved.membership {
            None => Self::none(),
            Some(saved) => match Membership::from_saved(saved) {
                Ok(membership) => Self::from_membership(&membership),
                Err(_) => Self::unreadable(),
            },
        }
    }

    /// Holds `context` to at least `floor`.
    fn raise(&mut self, context: &str, floor: Protection) {
        let level = self
            .by_context
            .get(context)
            .map_or(floor, |held| held.stricter(floor));
        self.by_context.insert(context.to_string(), level);
    }

    /// The stricter of two sets of floors, context by context.
    pub fn and(&self, other: &TeamFloors) -> TeamFloors {
        let mut both = self.clone();
        both.unreadable |= other.unreadable;
        for (context, floor) in &other.by_context {
            both.raise(context, *floor);
        }
        both
    }

    /// What the team holds `context` to, if anything: `read-only` for every
    /// context when the saved team can't be read.
    pub fn floor(&self, context: &str) -> Option<Protection> {
        if self.unreadable {
            return Some(Protection::ReadOnly);
        }
        self.by_context.get(context).copied()
    }

    /// The level `base` becomes under the floor of `context`: the stricter of
    /// the two, so a floor never loosens it.
    pub fn apply(&self, context: &str, base: Protection) -> Protection {
        match self.floor(context) {
            Some(floor) => base.stricter(floor),
            None => base,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::BTreeMap;

    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::team::example;
    use crate::team::{FetchFailed, Overrides, Version};
    use crate::workspace::SourceRecovery;

    fn at(day: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, 9, 0, 0).unwrap()
    }

    fn version(id: &str, text: &str) -> Version {
        Version::parse(id, text.as_bytes(), BTreeMap::new()).unwrap()
    }

    pub(crate) const TEAM: &str = "version: 1\nclusters:\n  - id: prod-ams\n    role: core\n    protection: read-only\n  - id: stage-lon\n    role: core\n    protection: protected\n  - id: dev-fra\n    role: core\n";

    /// A team applied with each cluster mapped to the context given.
    pub(crate) fn applied(text: &str, map: &[(&str, &str)]) -> Membership {
        let mut m = Membership::join(example::source(), Overrides::new());
        m.fetched(Ok(version("v1", text)), at(1));
        let available: Vec<String> = map.iter().map(|(_, context)| context.to_string()).collect();
        for (id, context) in map {
            m.map_context(id, Choice::Context(context.to_string()), &available)
                .unwrap();
        }
        // The clusters not named here are left out.
        let rest: Vec<String> = m
            .mapping()
            .clusters()
            .filter(|(_, choice)| choice.is_none())
            .map(|(id, _)| id.to_string())
            .collect();
        for id in rest {
            m.map_context(&id, Choice::LeftOut, &available).unwrap();
        }
        let mut review = m.review().unwrap();
        example::confirm_all(&mut review);
        m.apply(review, at(2)).unwrap();
        m
    }

    #[test]
    fn a_floor_binds_to_the_context_you_chose_and_only_that_one() {
        let m = applied(TEAM, &[("prod-ams", "ams-ctx"), ("stage-lon", "lon-ctx")]);
        let floors = TeamFloors::from_membership(&m);
        assert_eq!(floors.floor("ams-ctx"), Some(Protection::ReadOnly));
        assert_eq!(floors.floor("lon-ctx"), Some(Protection::Protected));
        // dev-fra was left out: no floor, and none is found by its name.
        assert_eq!(floors.floor("dev-fra"), None);
        assert_eq!(floors.floor("unrelated"), None);
    }

    #[test]
    fn a_floor_only_ever_tightens() {
        let m = applied(TEAM, &[("stage-lon", "lon-ctx")]);
        let floors = TeamFloors::from_membership(&m);
        // Over a looser level it holds; over a stricter one it does not move.
        assert_eq!(
            floors.apply("lon-ctx", Protection::Open),
            Protection::Protected
        );
        assert_eq!(
            floors.apply("lon-ctx", Protection::Guarded),
            Protection::Protected
        );
        assert_eq!(
            floors.apply("lon-ctx", Protection::ReadOnly),
            Protection::ReadOnly
        );
        // A context with no floor is as it was.
        assert_eq!(floors.apply("other", Protection::Open), Protection::Open);
    }

    #[test]
    fn a_team_that_sets_open_does_not_loosen_the_default() {
        let m = applied(
            "version: 1\nclusters:\n  - id: dev-fra\n    role: core\n    protection: open\n",
            &[("dev-fra", "dev-ctx")],
        );
        let floors = TeamFloors::from_membership(&m);
        assert_eq!(floors.floor("dev-ctx"), Some(Protection::Open));
        // A context nothing else names is guarded, and a floor of open is
        // not a level to go to: the stricter stays.
        assert_eq!(
            floors.apply("dev-ctx", Protection::Guarded),
            Protection::Guarded
        );
    }

    #[test]
    fn a_waiting_version_has_no_floor_and_cannot_take_one_away() {
        let mut m = applied(TEAM, &[("prod-ams", "ams-ctx"), ("stage-lon", "lon-ctx")]);
        let before = TeamFloors::from_membership(&m);
        // A newer version waits, and it drops prod-ams and adds a strict one.
        m.fetched(
            Ok(version(
                "v2",
                "version: 1\nclusters:\n  - id: stage-lon\n    role: core\n    protection: protected\n  - id: new-nyc\n    role: core\n    protection: read-only\n",
            )),
            at(3),
        );
        assert!(m.pending().is_some());
        let waiting = TeamFloors::from_membership(&m);
        assert_eq!(waiting, before, "nothing unreviewed sets or drops a floor");
        assert_eq!(waiting.floor("ams-ctx"), Some(Protection::ReadOnly));
    }

    #[test]
    fn a_failed_fetch_keeps_the_floors_of_the_applied_version() {
        let mut m = applied(TEAM, &[("prod-ams", "ams-ctx")]);
        let before = TeamFloors::from_membership(&m);
        m.fetched(Err(FetchFailed("the host didn’t answer".into())), at(3));
        assert_eq!(TeamFloors::from_membership(&m), before);
        assert_eq!(before.floor("ams-ctx"), Some(Protection::ReadOnly));
    }

    #[test]
    fn a_floor_a_later_version_drops_is_held() {
        let mut m = applied(TEAM, &[("prod-ams", "ams-ctx")]);
        m.fetched(Ok(version("v2", "version: 1\n")), at(3));
        let mut review = m.review().unwrap();
        example::confirm_all(&mut review);
        m.apply(review, at(4)).unwrap();
        // The team stopped naming prod-ams, but its floor is held, and so is
        // the choice of context it holds on.
        let floors = TeamFloors::from_membership(&m);
        assert_eq!(floors.floor("ams-ctx"), Some(Protection::ReadOnly));
    }

    #[test]
    fn a_context_that_is_not_in_the_kubeconfig_any_more_is_harmless() {
        let m = applied(TEAM, &[("prod-ams", "gone-ctx")]);
        let floors = TeamFloors::from_membership(&m);
        assert_eq!(floors.floor("gone-ctx"), Some(Protection::ReadOnly));
        // Nothing else moves.
        assert_eq!(floors.floor("ams-ctx"), None);
    }

    #[test]
    fn two_clusters_on_one_context_take_the_stricter() {
        let m = applied(TEAM, &[("prod-ams", "shared"), ("stage-lon", "shared")]);
        assert_eq!(
            TeamFloors::from_membership(&m).floor("shared"),
            Some(Protection::ReadOnly)
        );
    }

    #[test]
    fn nothing_applied_is_no_floor() {
        let mut m = Membership::join(example::source(), Overrides::new());
        m.fetched(Ok(version("v1", TEAM)), at(1));
        assert_eq!(TeamFloors::from_membership(&m), TeamFloors::none());
    }

    #[test]
    fn an_unreadable_team_holds_every_context_read_only() {
        let floors = TeamFloors::unreadable();
        for context in ["ams-ctx", "never-heard-of"] {
            assert_eq!(floors.floor(context), Some(Protection::ReadOnly));
            assert_eq!(
                floors.apply(context, Protection::Open),
                Protection::ReadOnly
            );
        }
    }

    #[test]
    fn two_sets_of_floors_take_the_stricter_and_unreadable_wins() {
        let a = TeamFloors::from_membership(&applied(TEAM, &[("stage-lon", "lon-ctx")]));
        let b = TeamFloors::from_membership(&applied(TEAM, &[("prod-ams", "lon-ctx")]));
        assert_eq!(a.and(&b).floor("lon-ctx"), Some(Protection::ReadOnly));
        assert_eq!(b.and(&a).floor("lon-ctx"), Some(Protection::ReadOnly));
        assert!(a.and(&TeamFloors::unreadable()).is_unreadable());
        assert_eq!(a.and(&TeamFloors::none()), a);
    }

    pub(crate) fn saved_text(m: &Membership) -> String {
        let saved = crate::team::SavedTeam {
            membership: m.to_saved(),
        };
        let mut value = serde_json::to_value(saved).unwrap();
        value["version"] = 1.into();
        value.to_string()
    }

    #[test]
    fn the_saved_file_is_read_at_the_point_of_use() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("team.json");
        // No file: no team.
        assert_eq!(TeamFloors::read(&file), TeamFloors::none());
        // A saved team: its floors.
        let m = applied(TEAM, &[("prod-ams", "ams-ctx")]);
        std::fs::write(&file, saved_text(&m)).unwrap();
        assert_eq!(
            TeamFloors::read(&file).floor("ams-ctx"),
            Some(Protection::ReadOnly)
        );
        // A saved file with no membership is no team.
        std::fs::write(&file, r#"{"version":1,"membership":null}"#).unwrap();
        assert_eq!(TeamFloors::read(&file), TeamFloors::none());
    }

    #[test]
    fn a_file_that_cannot_be_read_holds_every_context_read_only() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("team.json");
        let m = applied(TEAM, &[("prod-ams", "ams-ctx")]);
        let mut newer: serde_json::Value = serde_json::from_str(&saved_text(&m)).unwrap();
        newer["version"] = 99.into();
        let mut refused: serde_json::Value = serde_json::from_str(&saved_text(&m)).unwrap();
        refused["membership"]["source"]["address"] = "http://git.acme.example/x.git".into();
        for (why, text) in [
            ("not json", "{ not json".to_string()),
            ("a newer app's", newer.to_string()),
            ("an address joining refuses", refused.to_string()),
            ("the wrong shape", r#"{"membership": 7}"#.to_string()),
            ("empty", String::new()),
        ] {
            std::fs::write(&file, &text).unwrap();
            let floors = TeamFloors::read(&file);
            assert!(floors.is_unreadable(), "{why}");
            assert_eq!(
                floors.floor("any-context"),
                Some(Protection::ReadOnly),
                "{why}"
            );
        }
        // Not a regular file: the same.
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(TeamFloors::read(&file).is_unreadable());
    }

    #[test]
    fn level_of_takes_the_floor_and_nothing_loosens() {
        let setup = crate::setup::parse(
            b"version: 1\nclusters:\n  - id: a\n    role: core\n    context: ams-ctx\n    protection: open\n  - id: b\n    role: core\n    context: lon-ctx\n    protection: read-only\n",
        )
        .unwrap();
        let m = applied(TEAM, &[("prod-ams", "ams-ctx"), ("stage-lon", "lon-ctx")]);
        let mut recovery = SourceRecovery::default();
        recovery.set_team_floors(TeamFloors::from_membership(&m));
        let level = |context| setup.workspace.level_of(context, &recovery);
        assert_eq!(level("ams-ctx"), Protection::ReadOnly, "open held up");
        assert_eq!(level("lon-ctx"), Protection::ReadOnly, "read-only stays");
        assert_eq!(
            level("elsewhere"),
            Protection::Guarded,
            "no floor, no entry"
        );
        // The file's recovery is the same whatever the floors say.
        assert!(recovery.same_file_as(&SourceRecovery::default()));
    }
}
