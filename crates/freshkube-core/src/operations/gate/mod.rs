//! What a cluster's protection level asks before a node operation starts
//! (#852, `docs/WRITE_MODEL.md`, "Talos Operations").
//!
//! The requirement comes from the operation's tier through
//! [`write::requirement`](crate::write::requirement), never from a rule of
//! this module's own: a frontend draws the answer and hands back what the
//! person typed, and [`confirm_operation`] is the only way to build the
//! confirmation a run needs.

use thiserror::Error;

use super::{NodeTarget, OperationConfirmation, OperationKind};
use crate::write::{Protection, Tier, expected_name, requirement};

impl OperationKind {
    /// The write model's tier for the operation. Cordon and uncordon are
    /// undone by each other; a drain evicts pods, and a reboot or shutdown
    /// takes the node down, so none of the three can be taken back.
    pub const fn tier(self) -> Tier {
        match self {
            Self::Cordon | Self::Uncordon => Tier::Reversible,
            Self::Drain | Self::Reboot | Self::Shutdown => Tier::Destructive,
        }
    }
}

/// What the level asks of an operation before it is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationGate {
    /// The level refuses it; the reason a disabled control gives.
    Blocked(&'static str),
    /// The operation's own confirmation, as before.
    Confirm,
    /// The confirmation also asks for `expected`: the node's name, or the
    /// cluster entry's id once for several nodes.
    TypeName { expected: String },
}

/// Why no confirmation was made.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OperationRefusal {
    /// The level refuses writes.
    #[error("{0}")]
    Blocked(&'static str),
    /// No node was named.
    #[error("no node was named")]
    NoTargets,
    /// The name the level asks for was not typed exactly.
    #[error("the name was not typed exactly")]
    NameMismatch,
}

/// What the level asks of `kind` on `targets` in the cluster entry
/// `entry_id`. One node is typed by its name, several by the entry's id once.
pub fn operation_gate(
    kind: OperationKind,
    level: Protection,
    targets: &[NodeTarget],
    entry_id: &str,
) -> OperationGate {
    let needs = requirement(kind.tier(), level);
    if let Some(reason) = needs.blocked {
        return OperationGate::Blocked(reason);
    }
    if !needs.type_name {
        return OperationGate::Confirm;
    }
    let names: Vec<&str> = targets.iter().map(|target| target.name.as_str()).collect();
    OperationGate::TypeName {
        expected: expected_name(&names, entry_id).to_owned(),
    }
}

/// The confirmation for a run, or why there is none. `level` is the level as
/// it is at send time, so one that became stricter after a dialog opened
/// sends nothing. Matching is exact and case-sensitive.
pub fn confirm_operation(
    kind: OperationKind,
    level: Protection,
    targets: &[NodeTarget],
    entry_id: &str,
    typed: &str,
) -> Result<OperationConfirmation, OperationRefusal> {
    if targets.is_empty() {
        return Err(OperationRefusal::NoTargets);
    }
    match operation_gate(kind, level, targets, entry_id) {
        OperationGate::Blocked(reason) => Err(OperationRefusal::Blocked(reason)),
        OperationGate::Confirm => Ok(OperationConfirmation::confirmed()),
        OperationGate::TypeName { expected } => {
            if !expected.is_empty() && typed == expected {
                Ok(OperationConfirmation::confirmed())
            } else {
                Err(OperationRefusal::NameMismatch)
            }
        }
    }
}

#[cfg(test)]
mod tests;
