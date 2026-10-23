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
