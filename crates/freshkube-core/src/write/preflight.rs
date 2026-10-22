//! Preflight: everything that is checked before a person is asked. The level's
//! block, the access review, the UID and resourceVersion the person saw, and
//! a server-side dry run. None of it is skipped at any level, and none of it
//! changes anything.

use super::{
    Protection,
    access::{self, Access},
    action::{ActionDescription, ObjectChange},
    audit::{Phase, RunId, scrub},
    diff::{FieldChange, diff},
    requirement::{Requirement, expected_name, requirement},
    run::Runner,
    send::{ChangedWhat, Conflict, Refusal, get, send},
};

/// Who is writing, where, and under what level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Who {
    /// The cluster entry's id.
    pub entry: String,
    pub context: String,
    pub protection: Protection,
    pub actor: String,
}

/// Why an action cannot go ahead. Each is shown; none can be forced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// The level refuses writes. No request was made.
    ReadOnly(&'static str),
    /// The access review said no.
    AccessDenied {
        object: String,
        verb: &'static str,
        reason: String,
    },
    /// The access review could not be asked, so nothing is known.
    AccessUnknown { object: String, message: String },
    /// The object is not as the person saw it.
    Changed { object: String, what: ChangedWhat },
    /// Another field manager owns fields the write sets.
    Conflict { object: String, conflict: Conflict },
    /// The server-side dry run was refused.
    DryRunFailed { object: String, message: String },
    /// The cluster did not answer a read.
    NoAnswer { object: String, message: String },
    /// The action touches nothing.
    Empty,
}

impl Block {
    /// A sentence for a dialog or a tooltip.
    pub fn text(&self) -> String {
        match self {
            Block::ReadOnly(reason) => (*reason).to_owned(),
            Block::AccessDenied {
                object,
                verb,
                reason,
            } => {
                format!("You are not allowed to {verb} {object}: {reason}")
            }
            Block::AccessUnknown { object, .. } => {
                format!("Whether you may change {object} could not be checked.")
            }
            Block::Changed { object, what } => format!("{object}: {}", what.text()),
            Block::Conflict { object, conflict } => format!(
                "{object}: {} own fields this write sets. Nothing was sent.",
                conflict.managers.join(", ")
            ),
            Block::DryRunFailed { object, message } => {
                format!("The cluster would refuse this for {object}: {message}")
            }
            Block::NoAnswer { object, .. } => format!("The cluster did not answer for {object}."),
            Block::Empty => "There is nothing to change.".to_owned(),
        }
    }
}

/// What the dry run showed for one object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectDiff {
    pub object: String,
    pub changes: Vec<FieldChange>,
}

/// An action that passed preflight and waits for the person. Consumed by
/// [`Runner::execute`], so it is sent at most once.
#[derive(Debug)]
pub struct Prepared {
    pub(super) description: ActionDescription,
    pub(super) requirement: Requirement,
    pub(super) run: RunId,
    pub(super) diffs: Vec<ObjectDiff>,
    pub(super) undo: bool,
}

impl Prepared {
    pub fn description(&self) -> &ActionDescription {
        &self.description
    }

    /// What the level asks of the person first.
    pub fn requirement(&self) -> &Requirement {
        &self.requirement
    }

    pub fn run(&self) -> &RunId {
        &self.run
    }

    /// The dry run's diffs, one per object, for a spec change's dialog.
    pub fn diffs(&self) -> &[ObjectDiff] {
        &self.diffs
    }

    /// The objects a destructive dialog lists.
    pub fn impact(&self) -> Vec<String> {
        self.description
            .changes
            .iter()
            .map(|change| change.target.label())
            .collect()
    }

    /// What a protected dialog asks to be typed.
    pub fn expected_name(&self, entry: &str) -> String {
        expected_name(&self.description.names(), entry).to_owned()
    }
}

/// The answer to preflight.
#[derive(Debug)]
pub enum Verdict {
    Ready(Box<Prepared>),
    Blocked(Block),
}

impl Runner {
    /// Checks everything that can be checked without writing, in order: the
    /// level, then for each object the access review, the UID and
    /// resourceVersion, and the dry run.
    pub async fn preflight(&self, action: ActionDescription) -> Verdict {
        let requirement = requirement(action.tier, self.who.protection);
        self.preflight_with(action, requirement, false).await
    }

    pub(super) async fn preflight_with(
        &self,
        action: ActionDescription,
        requirement: Requirement,
        undo: bool,
    ) -> Verdict {
        if let Some(reason) = requirement.blocked {
            return Verdict::Blocked(Block::ReadOnly(reason));
        }
        if action.changes.is_empty() {
            return Verdict::Blocked(Block::Empty);
        }
        let run = RunId::new();
        let mut diffs = vec![];
        for change in &action.changes {
            match self.check_object(change).await {
                Ok(diff) => diffs.push(ObjectDiff {
                    object: change.target.label(),
                    changes: diff,
                }),
                Err(block) => {
                    self.audit_block(&run, &action, change, &block);
                    return Verdict::Blocked(block);
                }
            }
        }
        Verdict::Ready(Box::new(Prepared {
            description: action,
            requirement,
            run,
            diffs,
            undo,
        }))
    }

    async fn check_object(&self, change: &ObjectChange) -> Result<Vec<FieldChange>, Block> {
        let target = &change.target;
        let object = target.label();
        let verb = change.change.verb();
        match access::review(&self.client, target, verb).await {
            Access::Allowed => {}
            Access::Denied { reason } => {
                return Err(Block::AccessDenied {
                    object,
                    verb,
                    reason,
                });
            }
            Access::Unknown { message } => return Err(Block::AccessUnknown { object, message }),
        }
        let before = match get(&self.client, target).await {
            Ok(before) => before,
            Err(Refusal::Changed(what)) => return Err(Block::Changed { object, what }),
            Err(Refusal::Denied(reason)) => {
                return Err(Block::AccessDenied {
                    object,
                    verb: "get",
                    reason,
                });
            }
            Err(Refusal::NoAnswer(message) | Refusal::Rejected { message, .. }) => {
                return Err(Block::NoAnswer { object, message });
            }
            Err(Refusal::Conflict(_)) => unreachable!("a read does not conflict"),
        };
        if before["metadata"]["uid"].as_str() != Some(target.uid.as_str()) {
            return Err(Block::Changed {
                object,
                what: ChangedWhat::Recreated,
            });
        }
        if before["metadata"]["resourceVersion"].as_str() != Some(target.resource_version.as_str())
        {
            return Err(Block::Changed {
                object,
                what: ChangedWhat::Modified,
            });
        }
        match send(&self.client, target, &change.change, true).await {
            Ok(after) => Ok(diff(&before, &after, target.is_secret())),
            Err(Refusal::Conflict(conflict)) => Err(Block::Conflict { object, conflict }),
            Err(Refusal::Changed(what)) => Err(Block::Changed { object, what }),
            Err(Refusal::Denied(reason)) => Err(Block::AccessDenied {
                object,
                verb,
                reason,
            }),
            Err(Refusal::Rejected { message, .. } | Refusal::NoAnswer(message)) => {
                Err(Block::DryRunFailed { object, message })
            }
        }
    }

    /// A dry run the cluster refused is a request the app made, so it is
    /// written down; the blocks that made no request, or only read, are not.
    fn audit_block(
        &self,
        run: &RunId,
        action: &ActionDescription,
        change: &ObjectChange,
        block: &Block,
    ) {
        let phase = match block {
            Block::DryRunFailed { .. } => Phase::DryRunFailed,
            Block::Conflict { .. } => Phase::Conflict,
            _ => return,
        };
        let _ = self.record(run, action, &[change], phase, &scrub(&block.text()));
    }
}
