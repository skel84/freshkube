//! What could not be read, in the existing source gap wording.

use super::*;

/// What may be missing from the change, one sentence each, in the words
/// that name the entry: a part whose entry couldn't be read at all, a
/// destination that couldn't be, and a kind refused or not served where it
/// was asked.
pub(super) fn missing(
    place: &Place,
    evidence: &Evidence,
    sides: &Sides,
    reached: &Reached,
) -> Vec<Missing> {
    fn kind<T>(source: &Source<T>, what: &str, at: &At) -> Option<Missing> {
        let entry = &at.label;
        match source {
            Source::Refused(why) => Some(Missing::new(
                Gap::Refused,
                words::were_refused(what, entry, why),
            )),
            Source::NotInstalled(_) => Some(Missing::new(
                Gap::NotServed,
                words::arent_served(what, entry),
            )),
            Source::Unreadable(why) => Some(Missing::new(
                Gap::Unreadable,
                words::unreadable(what, entry, why),
            )),
            Source::Read(_) | Source::Capped(..) => None,
        }
    }
    let mut lines: Vec<Missing> = Vec::new();
    for (unread, kinds) in [
        (
            &reached.builds,
            // No build is looked for without a commit.
            vec![
                (!evidence.sha.is_empty())
                    .then(|| kind(&evidence.builds, "PipelineRuns", &sides.builds))
                    .flatten(),
            ],
        ),
        (
            &reached.kargo,
            vec![
                kind(&evidence.kargo.stages, "Kargo Stages", &sides.kargo),
                kind(&evidence.kargo.promotions, "Kargo Promotions", &sides.kargo),
            ],
        ),
        (
            &reached.argocd,
            vec![kind(
                &evidence.applications,
                "Argo CD Applications",
                &sides.argocd,
            )],
        ),
    ] {
        // The Stages and Promotions a change would follow its Freight with.
        let after = std::ptr::eq(unread, &reached.kargo).then_some("freight");
        let hang = |line: Missing| match after {
            Some(key) => line.after(key),
            None => line,
        };
        match unread {
            Some(unread) => lines.push(hang(Missing::new(Gap::Unreadable, unread.words()))),
            None => lines.extend(kinds.into_iter().flatten().map(hang)),
        }
    }
    for (context, read) in &evidence.environments {
        let at = place.at_context(context);
        match read {
            Err(why) => lines.push(Missing::new(Gap::Unreadable, why.message.clone())),
            Ok(workloads) => {
                let pods = workloads
                    .pods
                    .values()
                    .chain(workloads.deployment_pods.values())
                    .map(|read| &read.pods)
                    .chain(workloads.namespace_pods.values())
                    .find(|pods| matches!(pods, Source::Refused(_) | Source::NotInstalled(_)));
                // Rollouts not served are only Argo Rollouts not installed.
                let rollouts = kind(&workloads.rollouts, "Rollouts", &at)
                    .filter(|line| line.gap != Gap::NotServed);
                lines.extend(
                    [
                        rollouts,
                        kind(&workloads.deployments, "Deployments", &at),
                        pods.and_then(|pods| kind(pods, "Pods", &at)),
                    ]
                    .into_iter()
                    .flatten(),
                );
            }
        }
    }
    if !evidence.sha.is_empty() {
        lines.extend(history::missing(evidence, &sides.builds.label));
    }
    lines.dedup();
    lines
}
