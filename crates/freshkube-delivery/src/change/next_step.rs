//! Where a Stage's Rollout is: the Overview's Next step column. It is what
//! the Rollout the Stage's Application manages says, read with the rest of
//! the change, never a second look.

use super::Cause;
use crate::rollouts::Rollout;

/// A Stage's Next step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NextStep {
    /// No Rollout is part of this Stage's chain: nothing to read.
    NoRollout,
    /// On step `at` (from 1) of `of`.
    Step {
        at: u32,
        of: u32,
    },
    /// The Rollout is rolling out with no steps to count.
    Progressing,
    /// The Rollout waits, on step `at` of `of` when it says so.
    Paused {
        step: Option<(u32, u32)>,
    },
    Done,
    Aborted,
    /// What would say couldn't be read, or isn't there: never blank.
    Unknown {
        why: String,
        cause: Option<Cause>,
    },
}

impl NextStep {
    /// The Rollout's own words: its steps and phase. `None` is for the
    /// caller to say why nothing could be.
    pub(crate) fn of(rollout: &Rollout) -> Self {
        let steps = rollout.steps.filter(|steps| *steps > 0);
        let at = steps.zip(rollout.step_index);
        if rollout.aborted {
            return Self::Aborted;
        }
        if rollout.paused {
            return Self::Paused {
                step: at
                    .filter(|(of, index)| index < of)
                    .map(|(of, index)| (index + 1, of)),
            };
        }
        let healthy = rollout.phase.as_deref() == Some("Healthy");
        match (steps, rollout.step_index) {
            (Some(of), Some(index)) if index >= of => Self::Done,
            (Some(_), _) if healthy => Self::Done,
            (Some(of), Some(index)) => Self::Step { at: index + 1, of },
            (Some(of), None) => Self::Step { at: 1, of },
            (None, _) if healthy => Self::Done,
            (None, _) => match rollout.phase.as_deref() {
                Some("Progressing") => Self::Progressing,
                Some(phase) => Self::Unknown {
                    why: format!("the Rollout reports the phase {phase}, with no step"),
                    cause: None,
                },
                None => Self::Unknown {
                    why: "the Rollout reports no phase".into(),
                    cause: None,
                },
            },
        }
    }

    /// The cell's word: at most 14 characters.
    pub fn word(&self) -> String {
        match self {
            Self::NoRollout => "No Rollout".into(),
            Self::Step { at, of } => format!("Step {at} of {of}"),
            Self::Progressing => "Progressing".into(),
            Self::Paused { .. } => "Paused".into(),
            Self::Done => "Done".into(),
            Self::Aborted => "Aborted".into(),
            Self::Unknown { cause, .. } => cause.as_ref().map_or("Unknown", Cause::word).into(),
        }
    }

    /// The cell's tooltip.
    pub fn detail(&self) -> String {
        match self {
            Self::NoRollout => "No Argo Rollout manages this Stage's workload".into(),
            Self::Step { at, of } => format!("The Rollout is on step {at} of {of}"),
            Self::Progressing => "The Rollout is rolling out; it lists no steps".into(),
            Self::Paused {
                step: Some((at, of)),
            } => format!("The Rollout is paused on step {at} of {of}"),
            Self::Paused { step: None } => "The Rollout is paused".into(),
            Self::Done => "The Rollout has gone through every step".into(),
            Self::Aborted => "The Rollout was aborted".into(),
            Self::Unknown { why, .. } => why.clone(),
        }
    }
}
