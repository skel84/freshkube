//! Undo for a reversible action: offered for a short time after the write is
//! accepted, computed from the object as it was before the send, and sent
//! through the same preflight and runner as any write. It is not offered a
//! send once someone else has written the fields it would put back; it says
//! who.

use std::time::Duration;

use serde_json::Value;

use super::{
    action::{ActionDescription, Destination, ObjectChange, Observation},
    preflight::{Block, Prepared, Verdict},
    requirement::{Tier, requirement},
    run::Runner,
    send::{FIELD_MANAGER, Refusal, get},
};

/// A way back, held for [`super::UNDO_WINDOW`].
#[derive(Clone, Debug, PartialEq)]
pub struct UndoOffer {
    /// The action it undoes.
    pub action: String,
    pub sentence: String,
    pub window: Duration,
    /// The other cluster the write reached, so Undo is held to the same level.
    pub(super) destinations: Vec<Destination>,
    /// One inverse per object, each guarded.
    pub(super) items: Vec<(ObjectChange, Observation)>,
}

impl UndoOffer {
    pub fn objects(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|(item, _)| item.target.label())
            .collect()
    }
}

/// Why Undo cannot be sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoRefused {
    /// The window passed.
    Expired,
    /// Someone else wrote what Undo would put back, or deleted the object.
    /// `by` is the field manager that wrote last, when the object says.
    Changed { object: String, by: Option<String> },
    /// The cluster did not answer, or preflight blocked the inverse.
    Blocked(Block),
}

impl UndoRefused {
    /// A sentence for the Undo button's place, so it never just vanishes.
    pub fn text(&self) -> String {
        match self {
            UndoRefused::Expired => "Undo is no longer offered.".to_owned(),
            UndoRefused::Changed {
                object,
                by: Some(by),
            } => {
                format!("Can't undo: {object} was changed by {by} since.")
            }
            UndoRefused::Changed { object, by: None } => {
                format!("Can't undo: {object} was changed since.")
            }
            UndoRefused::Blocked(block) => format!("Can't undo: {}", block.text()),
        }
    }
}

/// The manager that wrote an object last, other than this app.
fn last_other_writer(object: &Value) -> Option<String> {
    object["metadata"]["managedFields"]
        .as_array()?
        .iter()
        .filter(|entry| {
            entry["manager"]
                .as_str()
                .is_some_and(|m| m != FIELD_MANAGER)
        })
        .max_by_key(|entry| entry["time"].as_str().unwrap_or_default().to_owned())
        .and_then(|entry| entry["manager"].as_str().map(str::to_owned))
}

fn holds(guard: &Observation, object: &Value) -> bool {
    match guard {
        Observation::FieldEquals { pointer, value } => object.pointer(pointer) == Some(value),
        Observation::Gone => false,
        // Nothing can be checked, so nothing is known to be safe: not holding.
        Observation::Exists | Observation::Unobservable => false,
    }
}

impl Runner {
    /// Gets the inverse ready: checks the offer is open, that the object still
    /// has what the write gave it, and runs preflight on the inverse with the
    /// object's current UID and resourceVersion. `elapsed` is how long ago
    /// the offer was made, on the caller's clock.
    pub async fn prepare_undo(
        &self,
        offer: &UndoOffer,
        elapsed: Duration,
    ) -> Result<Prepared, UndoRefused> {
        if elapsed > offer.window {
            return Err(UndoRefused::Expired);
        }
        let mut changes = vec![];
        for (item, guard) in &offer.items {
            let object = item.target.label();
            let current = match get(&self.client, &item.target).await {
                Ok(current) => current,
                Err(Refusal::Changed(_)) => return Err(UndoRefused::Changed { object, by: None }),
                Err(other) => {
                    return Err(UndoRefused::Blocked(Block::NoAnswer {
                        object,
                        message: format!("{other:?}"),
                    }));
                }
            };
            let same = current["metadata"]["uid"].as_str() == Some(item.target.uid.as_str());
            if !same || !holds(guard, &current) {
                return Err(UndoRefused::Changed {
                    object,
                    by: last_other_writer(&current),
                });
            }
            let mut change = item.clone();
            change.target.resource_version = current["metadata"]["resourceVersion"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            changes.push(change);
        }
        let action = ActionDescription {
            kind: format!("undo:{}", offer.action),
            verb: "Undo".to_owned(),
            tier: Tier::Reversible,
            sentence: format!("Undo: {}", offer.sentence),
            changes,
            destinations: offer.destinations.clone(),
            no_confirmation: false,
            subjects: vec![],
            preview: vec![],
        };
        // Undo is a write like any other at its level: a protected cluster
        // asks for the name, a guarded one for a confirmation, an open one
        // for nothing, and a read-only one blocks it.
        let level = self.level_for(&action);
        let requirement = requirement(Tier::Reversible, level);
        match self.preflight_with(action, requirement, level, true).await {
            Verdict::Ready(prepared) => Ok(*prepared),
            Verdict::Blocked(block) => Err(UndoRefused::Blocked(block)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_last_other_writer_is_named_and_this_app_never_is() {
        let object = json!({"metadata": {"managedFields": [
            {"manager": "freshkube", "time": "2026-10-22T10:00:09Z"},
            {"manager": "kubectl", "time": "2026-10-22T10:00:01Z"},
            {"manager": "hpa", "time": "2026-10-22T10:00:05Z"},
        ]}});
        assert_eq!(last_other_writer(&object).as_deref(), Some("hpa"));
        assert_eq!(last_other_writer(&json!({})), None);
    }

    #[test]
    fn a_guard_that_cannot_be_checked_does_not_hold() {
        let object = json!({"spec": {"replicas": 3}});
        assert!(!holds(&Observation::Unobservable, &object));
        assert!(!holds(&Observation::Gone, &object));
        assert!(holds(
            &Observation::FieldEquals {
                pointer: "/spec/replicas".into(),
                value: json!(3)
            },
            &object
        ));
        assert!(!holds(
            &Observation::FieldEquals {
                pointer: "/spec/replicas".into(),
                value: json!(4)
            },
            &object
        ));
    }

    #[test]
    fn a_refusal_always_has_a_sentence() {
        let changed = UndoRefused::Changed {
            object: "Deployment/web".into(),
            by: Some("hpa".into()),
        };
        assert_eq!(
            changed.text(),
            "Can't undo: Deployment/web was changed by hpa since."
        );
        assert!(!UndoRefused::Expired.text().is_empty());
    }
}
