//! Where a Stage's Rollout is: the Overview's Next step column. It is what
//! the Rollout the Stage's Application manages says, read with the rest of
//! the change, never a second look.

use super::Cause;
use crate::rollouts::Rollout;

/// A Stage's Next step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NextStep {
    /// The Stage's Application was read, in this cluster, and manages no
    /// Rollout: nothing to read.
    NoRollout,
    /// Nothing looked for this Stage's Application: the change hasn't got
    /// this far.
    NotReached,
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
    /// The Rollout is Degraded, on a step when it says so, and not aborted.
    Degraded {
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
    /// The Rollout's own words: its steps and phase. A Rollout that is
    /// aborted or Degraded says so before any step, and one with steps but
    /// no current step reported is Unknown, never "Step 1".
    pub(crate) fn of(rollout: &Rollout) -> Self {
        let steps = rollout.steps.filter(|steps| *steps > 0);
        let step = steps
            .zip(rollout.step_index)
            .filter(|(of, index)| index < of)
            .map(|(of, index)| (index + 1, of));
        let phase = rollout.phase.as_deref();
        if rollout.aborted {
            return Self::Aborted;
        }
        if phase == Some("Degraded") {
            return Self::Degraded { step };
        }
        if rollout.paused {
            return Self::Paused { step };
        }
        let healthy = phase == Some("Healthy");
        match (steps, rollout.step_index) {
            (Some(of), Some(index)) if index >= of => Self::Done,
            (Some(_), _) | (None, _) if healthy => Self::Done,
            (Some(of), Some(index)) => Self::Step { at: index + 1, of },
            (Some(_), None) => Self::Unknown {
                why: "the Rollout reports no current step yet".into(),
                cause: None,
            },
            (None, _) => match phase {
                Some("Progressing") => Self::Progressing,
                Some(phase) => Self::Unknown {
                    why: format!("the Rollout reports the phase {phase}, with no step"),
                    cause: None,
                },
                None => Self::Unknown {
                    why: "the Rollout reports no status yet".into(),
                    cause: None,
                },
            },
        }
    }

    /// The cell's word: at most 14 characters.
    pub fn word(&self) -> String {
        match self {
            Self::NoRollout => "No Rollout".into(),
            Self::NotReached => "Not reached".into(),
            Self::Step { at, of } => format!("Step {at} of {of}"),
            Self::Progressing => "Progressing".into(),
            Self::Paused { .. } => "Paused".into(),
            Self::Degraded { .. } => "Degraded".into(),
            Self::Done => "Done".into(),
            Self::Aborted => "Aborted".into(),
            Self::Unknown { cause, .. } => cause.as_ref().map_or("Unknown", Cause::word).into(),
        }
    }

    /// The cell's tooltip.
    pub fn detail(&self) -> String {
        let at = |step: &Option<(u32, u32)>| {
            step.map(|(at, of)| format!(" on step {at} of {of}"))
                .unwrap_or_default()
        };
        match self {
            Self::NoRollout => "No Argo Rollout manages this Stage's workload".into(),
            Self::NotReached => "Nothing has looked for this Stage's Application yet".into(),
            Self::Step { at, of } => format!("The Rollout is on step {at} of {of}"),
            Self::Progressing => "The Rollout is rolling out; it lists no steps".into(),
            Self::Paused { step } => format!("The Rollout is paused{}", at(step)),
            Self::Degraded { step } => format!("The Rollout is degraded{}", at(step)),
            Self::Done => "The Rollout has gone through every step".into(),
            Self::Aborted => "The Rollout was aborted".into(),
            Self::Unknown { why, .. } => why.clone(),
        }
    }
}
