//! Which value wins: yours over the team's over the defaults. Contexts are
//! always yours and never pass through here. Protection is the exception: the
//! team's level is a floor, so a cluster may be made stricter, never looser.

use serde_json::Value;

use super::file::Values;
use super::membership::Overrides;
use crate::write::Protection;

/// Where a resolved value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Default,
    Team,
    /// Yours; `over_team` when the team sets the same key.
    Yours {
        over_team: bool,
    },
}

impl Origin {
    /// The chip on a row or an Inspector field.
    pub fn chip(self) -> Option<&'static str> {
        match self {
            Origin::Default => None,
            Origin::Team => Some("Team"),
            Origin::Yours { .. } => Some("Yours"),
        }
    }

    /// The words under a value that overrides the team's.
    pub fn words(self) -> &'static str {
        match self {
            Origin::Default => "Default",
            Origin::Team => "Team",
            Origin::Yours { over_team: false } => "Yours",
            Origin::Yours { over_team: true } => "Yours · over the team’s",
        }
    }

    /// Whether "Use the team's value" does anything.
    pub fn can_use_team(self) -> bool {
        matches!(self, Origin::Yours { over_team: true })
    }
}

/// The value that applies at a key, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub value: Option<Value>,
    pub origin: Origin,
    /// Your override was looser than the team's floor, so the floor applies.
    pub floor_held: bool,
}

/// The key of a cluster's protection level.
pub fn protection_key(id: &str) -> String {
    format!("clusters[{id}].protection")
}

fn is_protection(key: &str) -> bool {
    key.starts_with("clusters[") && key.ends_with("].protection")
}

/// The level a value says. An unknown level counts as guarded, as the write
/// model does; no value is no level.
fn level(value: Option<&Value>) -> Option<Protection> {
    let value = value?;
    Some(serde_json::from_value(value.clone()).unwrap_or_default())
}

/// The floor the team sets a cluster: its level, else guarded when the team
/// names the cluster. A cluster the team doesn't name has no floor.
pub fn floor(id: &str, team: &Values) -> Option<Protection> {
    match level(team.get(&protection_key(id))) {
        Some(floor) => Some(floor),
        None => team
            .contains_key(&format!("clusters[{id}]"))
            .then_some(Protection::Guarded),
    }
}

/// Resolves one key.
pub fn resolve(key: &str, team: &Values, yours: &Overrides, defaults: &Values) -> Resolved {
    let team_value = team.get(key);
    if let Some(mine) = yours.get(key) {
        if is_protection(key)
            && let Some(id) = key
                .strip_prefix("clusters[")
                .and_then(|rest| rest.strip_suffix("].protection"))
            && let (Some(floor), Some(mine_level)) = (floor(id, team), level(Some(mine)))
            && mine_level > floor
        {
            return Resolved {
                value: Some(Value::String(floor.key().into())),
                origin: Origin::Team,
                floor_held: true,
            };
        }
        return Resolved {
            value: Some(mine.clone()),
            origin: Origin::Yours {
                over_team: team_value.is_some(),
            },
            floor_held: false,
        };
    }
    match team_value {
        Some(value) => Resolved {
            value: Some(value.clone()),
            origin: Origin::Team,
            floor_held: false,
        },
        None => Resolved {
            value: defaults.get(key).cloned(),
            origin: Origin::Default,
            floor_held: false,
        },
    }
}

/// The levels a cluster may be set to: the floor and every stricter level.
/// `None` is a cluster the team doesn't name: no floor.
pub fn allowed_levels(team_level: Option<Protection>) -> Vec<Protection> {
    Protection::ALL
        .into_iter()
        .filter(|candidate| team_level.is_none_or(|floor| *candidate <= floor))
        .collect()
}

/// Why the looser levels are off in the Inspector.
pub fn floor_words(team_level: Protection) -> String {
    format!(
        "Your team sets {} as the loosest level for this cluster. You can make it stricter, not looser.",
        team_level.label()
    )
}

/// How one cluster stands against the team, for its row and its Inspector.
#[derive(Clone, Debug, PartialEq)]
pub struct ClusterView {
    /// The chip on the row: the team names the cluster, or you set one of its
    /// keys. None when neither does.
    pub row: Option<Origin>,
    /// The loosest level the team allows; none when the team sets no floor.
    pub floor: Option<Protection>,
    /// Where the protection level comes from.
    pub level: Origin,
    /// Your level was looser than the floor, so the floor applies.
    pub floor_held: bool,
}

impl ClusterView {
    /// The levels the Inspector offers.
    pub fn allowed(&self) -> Vec<Protection> {
        allowed_levels(self.floor)
    }

    /// Why the looser levels are off; none without a floor.
    pub fn floor_words(&self) -> Option<String> {
        self.floor.map(floor_words)
    }
}

/// The cluster `id` against the team's values and yours. None when neither
/// the team nor you set anything for it.
pub fn cluster_view(id: &str, team: &Values, yours: &Overrides) -> Option<ClusterView> {
    let mark = format!("clusters[{id}]");
    let mine = |key: &String| {
        key.starts_with(&mark) && matches!(key[mark.len()..].chars().next(), None | Some('.'))
    };
    let yours_set = yours.keys().any(mine);
    let team_set = team.keys().any(mine);
    let row = match (yours_set, team_set) {
        (false, false) => return None,
        (true, over_team) => Origin::Yours { over_team },
        (false, true) => Origin::Team,
    };
    let level = resolve(&protection_key(id), team, yours, &Values::new());
    Some(ClusterView {
        row: Some(row),
        floor: floor(id, team),
        level: level.origin,
        floor_held: level.floor_held,
    })
}
