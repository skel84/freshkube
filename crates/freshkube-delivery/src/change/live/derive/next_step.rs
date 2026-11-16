//! A Stage's Next step, from the links of its run.

use super::Away;
use crate::change::{Cause, NextStep, Role};
use crate::join::{Confidence, Hop as Joined, Link, Workloads};

/// Where the Stage's Rollout is, as the change read it.
///
/// The Rollout link decides when there is one; only the first Rollout the
/// Application manages counts. With none, `NoRollout` is only for an
/// Application that was read, in this cluster, and manages no Rollout: an
/// Application that wasn't read, wasn't found or is ambiguous is Unknown
/// with its link's words and cause, one that deploys to a cluster nothing
/// was read in is Unknown, and a Stage nothing looked an Application up for
/// is not reached. A Rollout that wasn't read, was refused or isn't in a
/// capped listing is Unknown, never blank.
pub(super) fn next_step(
    workloads: Option<&Workloads>,
    run: &[&Link],
    away: Option<&Away<'_>>,
) -> NextStep {
    let unknown = |link: &Link| NextStep::Unknown {
        why: link.reason.clone(),
        cause: link.cause.clone(),
    };
    let Some(link) = run.iter().find(|link| link.to == Joined::Rollout) else {
        let Some(app) = run.iter().find(|link| link.to == Joined::Application) else {
            return NextStep::NotReached;
        };
        if app.confidence == Confidence::Unknown {
            return unknown(app);
        }
        return match away {
            Some(away) => NextStep::Unknown {
                why: match away.why {
                    Some(why) => format!(
                        "the Application deploys to {}, whose Rollouts couldn't be read: {why}",
                        away.entry
                    ),
                    None => format!(
                        "the Application deploys to {}, whose Rollouts weren't read",
                        away.entry
                    ),
                },
                cause: away.why.map(|_| {
                    Cause::cant_read(
                        "the destination cluster's Rollouts, to see what the Application runs",
                        Some(Role::new("argoproj.io", "rollouts", None)),
                        away.refused,
                    )
                }),
            },
            None => NextStep::NoRollout,
        };
    };
    if link.confidence == Confidence::Unknown {
        return unknown(link);
    }
    let found = link.subject.split_once('/').and_then(|(namespace, name)| {
        workloads
            .and_then(|read| read.rollouts.read())
            .and_then(|all| {
                all.iter()
                    .find(|r| r.namespace == namespace && r.name == name)
            })
    });
    match found {
        Some(rollout) => NextStep::of(rollout),
        None => NextStep::Unknown {
            why: "the Rollout was not read".into(),
            cause: None,
        },
    }
}
