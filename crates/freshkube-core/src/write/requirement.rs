//! What a person must do before a write is sent: the tier and the level
//! together decide it, here, and a page only draws the answer
//! (`docs/WRITE_MODEL.md`, "The tiers").

use std::time::Duration;

use super::{Protection, action::ActionDescription};

/// How risky an action is. The action declares it; a page never does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tier {
    /// Exec and port forwarding: an explicit Start, not a change to an object.
    Session,
    /// Scale, restart a workload, suspend a CronJob, cordon.
    Reversible,
    /// Edit and apply YAML, change an image or an environment.
    SpecChange,
    /// Delete, evict, force-delete.
    Destructive,
    /// Creates an object that runs code and can't be unsent (a CronJob's Run
    /// now): always confirms, with a preview of what it creates, and never
    /// asks for a typed name, counts down or offers Undo.
    Run,
}

impl Tier {
    pub const ALL: [Tier; 5] = [
        Tier::Session,
        Tier::Reversible,
        Tier::SpecChange,
        Tier::Destructive,
        Tier::Run,
    ];

    /// The word the audit log spells it with.
    pub fn key(self) -> &'static str {
        match self {
            Tier::Session => "session",
            Tier::Reversible => "reversible",
            Tier::SpecChange => "spec-change",
            Tier::Destructive => "destructive",
            Tier::Run => "run",
        }
    }
}

/// How long a destructive action can still be cancelled after it is confirmed.
pub const CANCEL_WINDOW: Duration = Duration::from_secs(5);

/// How long Undo is offered after a reversible write.
pub const UNDO_WINDOW: Duration = Duration::from_secs(30);

/// What the tier and the level require before the write is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirement {
    /// The level refuses the write; the reason a disabled control gives.
    pub blocked: Option<&'static str>,
    /// A dialog asks first. False sends at once: `open` and a reversible
    /// action, which says in its toast that nothing was asked.
    pub confirm: bool,
    /// The dialog asks for the object's name (the entry's id for several).
    pub type_name: bool,
    /// The dialog shows the server-side dry run's diff.
    pub diff: bool,
    /// The dialog lists what the action reaches.
    pub impact: bool,
    /// After confirming, the write waits this long and can be cancelled.
    pub cancel_window: Option<Duration>,
    /// Undo is offered after the write is accepted.
    pub undo: bool,
}

/// The table in `docs/WRITE_MODEL.md` as a pure function. The access review,
/// the UID and resourceVersion check and the dry run are not in it: they are
/// never skipped, at any level.
pub fn requirement(tier: Tier, level: Protection) -> Requirement {
    let none = Requirement {
        blocked: None,
        confirm: false,
        type_name: false,
        diff: false,
        impact: false,
        cancel_window: None,
        undo: false,
    };
    if let Some(reason) = level.refusal() {
        return Requirement {
            blocked: Some(reason),
            ..none
        };
    }
    let typed = level == Protection::Protected;
    match tier {
        Tier::Session => none,
        Tier::Reversible => Requirement {
            confirm: level != Protection::Open,
            type_name: typed,
            undo: true,
            ..none
        },
        Tier::SpecChange => Requirement {
            confirm: true,
            type_name: typed,
            diff: true,
            ..none
        },
        Tier::Destructive => Requirement {
            confirm: true,
            type_name: typed,
            impact: true,
            cancel_window: Some(CANCEL_WINDOW),
            ..none
        },
        // It creates rather than destroys, so there is no impact list or
        // countdown; the action's own preview is what the person confirms.
        Tier::Run => Requirement {
            confirm: true,
            ..none
        },
    }
}

/// What an action must do before it is sent at `level`: the table's row for
/// its tier, except that an action which declares `no_confirmation` (it only
/// adds an annotation) asks nothing at any level `read-only` doesn't block,
/// and a reversible action with no way back asks even at `open`.
pub fn requirement_for(action: &ActionDescription, level: Protection) -> Requirement {
    let given = requirement(action.tier, level);
    if action.no_confirmation && given.blocked.is_none() {
        return Requirement {
            confirm: false,
            type_name: false,
            undo: false,
            ..given
        };
    }
    // A reversible action that has no way back (Restart) is not undoable, so
    // `open` does not skip its confirmation: a stray key must not roll out.
    if action.tier == Tier::Reversible
        && given.blocked.is_none()
        && action.changes.iter().all(|change| change.inverse.is_none())
    {
        return Requirement {
            confirm: true,
            undo: false,
            ..given
        };
    }
    given
}

impl Requirement {
    /// Whether `given` meets this requirement for objects named `names`,
    /// asked under the entry `entry`.
    pub fn satisfied_by(&self, given: &Confirmation, names: &[&str], entry: &str) -> bool {
        if self.blocked.is_some() {
            return false;
        }
        if !self.confirm {
            return true;
        }
        if self.type_name {
            let expected = expected_name(names, entry);
            return matches!(given, Confirmation::Typed(typed) if typed == expected);
        }
        !matches!(given, Confirmation::NotAsked)
    }
}

/// The text a protected cluster asks for: the object's name, or the entry's
/// id, typed once, for several objects.
pub fn expected_name<'a>(names: &[&'a str], entry: &'a str) -> &'a str {
    match names {
        [only] => only,
        _ => entry,
    }
}

/// What the front end collected from the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirmation {
    /// Nothing was asked: a send the requirement allows without a dialog.
    NotAsked,
    /// The person pressed the dialog's confirm.
    Confirmed,
    /// The person typed this where the dialog asked for the name.
    Typed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = include_str!("../../../../docs/WRITE_MODEL.md");

    /// The cells of the tier table's row that starts with `**name`.
    fn row(name: &str) -> Vec<String> {
        let line = DOC
            .lines()
            .find(|line| line.starts_with(&format!("| **{name}")))
            .unwrap_or_else(|| panic!("no row {name}"));
        line.split('|')
            .skip(2)
            .take(4)
            .map(|cell| cell.trim().to_lowercase())
            .collect()
    }

    fn words(requirement: Requirement) -> Vec<&'static str> {
        let mut words = vec![];
        if requirement.blocked.is_some() {
            words.push("blocked");
        }
        if requirement.type_name {
            words.push("type the");
        }
        if requirement.diff {
            words.push("dry-run diff");
        }
        if requirement.impact {
            words.push("impact preview");
        }
        if requirement.cancel_window.is_some() {
            words.push("5 s to cancel");
        }
        if requirement.undo {
            words.push("undo for 30 s");
        }
        words
    }

    #[test]
    fn the_function_says_what_the_docs_table_says() {
        for (name, tier) in [
            ("Reversible", Tier::Reversible),
            ("Spec change", Tier::SpecChange),
            ("Destructive", Tier::Destructive),
            ("Run", Tier::Run),
        ] {
            let cells = row(name);
            for (cell, level) in cells.iter().zip(Protection::ALL) {
                if cell.starts_with("as `guarded`") {
                    assert_eq!(
                        requirement(tier, level),
                        requirement(tier, Protection::Guarded)
                    );
                    continue;
                }
                let given = requirement(tier, level);
                for word in words(given) {
                    assert!(
                        cell.contains(word),
                        "{name} at {level:?}: {cell} lacks {word}"
                    );
                }
                // And nothing the cell names is missing from the function.
                for (word, present) in [
                    ("type the", given.type_name),
                    ("dry-run diff", given.diff),
                    ("impact preview", given.impact),
                    ("5 s to cancel", given.cancel_window.is_some()),
                    ("undo for 30 s", given.undo),
                    ("blocked", given.blocked.is_some()),
                ] {
                    if cell.contains(word) {
                        assert!(present, "{name} at {level:?}: {cell} names {word}");
                    }
                }
            }
        }
    }

    #[test]
    fn read_only_blocks_every_tier_with_its_reason() {
        for tier in Tier::ALL {
            let given = requirement(tier, Protection::ReadOnly);
            assert_eq!(given.blocked, Some("This cluster is read-only."));
            assert!(!given.satisfied_by(&Confirmation::Confirmed, &["a"], "e"));
        }
    }

    #[test]
    fn open_skips_only_the_reversible_confirmation() {
        assert!(!requirement(Tier::Reversible, Protection::Open).confirm);
        assert!(requirement(Tier::Reversible, Protection::Open).undo);
        for tier in [Tier::SpecChange, Tier::Destructive] {
            assert_eq!(
                requirement(tier, Protection::Open),
                requirement(tier, Protection::Guarded),
                "{tier:?}"
            );
        }
    }

    #[test]
    fn protected_asks_for_the_name_at_every_tier_that_confirms() {
        for tier in [Tier::Reversible, Tier::SpecChange, Tier::Destructive] {
            assert!(
                requirement(tier, Protection::Protected).type_name,
                "{tier:?}"
            );
            assert!(
                !requirement(tier, Protection::Guarded).type_name,
                "{tier:?}"
            );
        }
    }

    #[test]
    fn only_destructive_has_a_cancel_window_and_only_reversible_undo() {
        for tier in Tier::ALL {
            for level in Protection::ALL {
                let given = requirement(tier, level);
                if given.blocked.is_some() {
                    continue;
                }
                assert_eq!(given.cancel_window.is_some(), tier == Tier::Destructive);
                assert_eq!(given.undo, tier == Tier::Reversible);
                assert_eq!(given.diff, tier == Tier::SpecChange);
            }
        }
    }

    #[test]
    fn the_typed_name_is_exact_and_the_entry_id_for_several_objects() {
        let protected = requirement(Tier::Destructive, Protection::Protected);
        assert!(protected.satisfied_by(&Confirmation::Typed("web".into()), &["web"], "prod"));
        for wrong in ["Web", "web ", "", "prod"] {
            assert!(!protected.satisfied_by(&Confirmation::Typed(wrong.into()), &["web"], "prod"));
        }
        assert!(!protected.satisfied_by(&Confirmation::Confirmed, &["web"], "prod"));
        assert!(protected.satisfied_by(&Confirmation::Typed("prod".into()), &["a", "b"], "prod"));
        assert!(!protected.satisfied_by(&Confirmation::Typed("a".into()), &["a", "b"], "prod"));
    }

    #[test]
    fn run_always_confirms_and_never_types_counts_down_or_undoes() {
        for level in [Protection::Protected, Protection::Guarded, Protection::Open] {
            assert_eq!(
                requirement(Tier::Run, level),
                Requirement {
                    blocked: None,
                    confirm: true,
                    type_name: false,
                    diff: false,
                    impact: false,
                    cancel_window: None,
                    undo: false,
                },
                "{level:?}"
            );
        }
    }

    #[test]
    fn a_dialog_needs_a_confirmation_and_no_dialog_needs_none() {
        let guarded = requirement(Tier::Reversible, Protection::Guarded);
        assert!(!guarded.satisfied_by(&Confirmation::NotAsked, &["a"], "e"));
        assert!(guarded.satisfied_by(&Confirmation::Confirmed, &["a"], "e"));
        let open = requirement(Tier::Reversible, Protection::Open);
        assert!(open.satisfied_by(&Confirmation::NotAsked, &["a"], "e"));
    }

    #[test]
    fn undo_follows_the_requirement_of_any_write_at_its_level() {
        // Undo is a reversible write: a protected cluster asks for the name.
        let protected = requirement(Tier::Reversible, Protection::Protected);
        assert!(protected.type_name && protected.confirm);
        assert!(requirement(Tier::Reversible, Protection::Guarded).confirm);
        assert!(!requirement(Tier::Reversible, Protection::Open).confirm);
        assert!(
            requirement(Tier::Reversible, Protection::ReadOnly)
                .blocked
                .is_some()
        );
    }
}
