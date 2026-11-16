//! A Stage's Next step, from the Rollout link of its run.

use crate::change::NextStep;
use crate::join::{Confidence, Hop as Joined, Link, Workloads};

/// The Rollout the run's Application manages, as the change read it. A
/// Rollout that wasn't read, was refused or isn't in a capped listing is
/// Unknown with the link's words and cause, never blank.
pub(super) fn next_step(workloads: Option<&Workloads>, run: &[&Link]) -> NextStep {
    let Some(link) = run.iter().find(|link| link.to == Joined::Rollout) else {
        return NextStep::NoRollout;
    };
    if link.confidence == Confidence::Unknown {
        return NextStep::Unknown {
            why: link.reason.clone(),
            cause: link.cause.clone(),
        };
    }
    let found = workloads
        .and_then(|read| read.rollouts.read())
        .and_then(|all| {
            all.iter()
                .find(|r| format!("{}/{}", r.namespace, r.name) == link.subject)
        });
    match found {
        Some(rollout) => NextStep::of(rollout),
        None => NextStep::Unknown {
            why: "the Rollout was not read".into(),
            cause: None,
        },
    }
}
