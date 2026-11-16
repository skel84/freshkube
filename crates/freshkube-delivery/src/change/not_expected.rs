//! "Don't expect it" (#823): a Stage the setup lists as one an application
//! isn't expected in turns what wasn't found there into Not used here. Only
//! Not found changes: Waiting, Can't read and Not set up stay as they were,
//! and nothing is read or written.

use super::{Cause, Change, Phase, Shows};

/// Applies `delivery.applications[].not_expected_in` of the change's
/// project to `change`, and returns the names that match no Stage of the
/// change, for a warning. With no Stage read there is nothing to match them
/// to, so none is called unknown.
pub fn not_expected_in(change: &mut Change, stages: &[&str]) -> Vec<String> {
    let mut unknown = Vec::new();
    for name in stages {
        let Some(ix) = change.stages.iter().position(|stage| stage.name == *name) else {
            if !change.stages.is_empty() {
                unknown.push((*name).to_owned());
            }
            continue;
        };
        let cause = || {
            Cause::not_used_here(format!(
                "{} isn't expected in {name}, as freshkube.yaml says",
                change.project
            ))
        };
        let new = cause();
        let swap = |slot: &mut Option<Cause>| {
            if matches!(slot, Some(Cause::NotFound { .. })) {
                *slot = Some(new.clone());
            }
        };
        let groups: Vec<usize> = change
            .groups
            .iter()
            .enumerate()
            .filter(|(_, group)| group.phase == Phase::Stage(ix))
            .map(|(at, _)| at)
            .collect();
        for hop in change
            .hops
            .iter_mut()
            .filter(|hop| groups.contains(&hop.group))
        {
            swap(&mut hop.cause);
            if let Shows::Hop(detail) = &mut hop.shows
                && let Some(link) = &mut detail.link
            {
                swap(&mut link.cause);
            }
        }
        let stage = &mut change.stages[ix];
        if let Some(link) = &mut stage.link {
            swap(&mut link.cause);
        }
        if matches!(
            &stage.next_step,
            super::NextStep::Unknown {
                cause: Some(Cause::NotFound { .. }),
                ..
            }
        ) {
            stage.next_step = super::NextStep::NoRollout;
        }
    }
    unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::example::{Shape, change};

    fn change_for_tests() -> Change {
        change(chrono::Utc::now())
    }

    fn causes(change: &Change, stage: &str) -> Vec<(String, &'static str)> {
        let ix = change.stages.iter().position(|s| s.name == stage).unwrap();
        let groups: Vec<usize> = change
            .groups
            .iter()
            .enumerate()
            .filter(|(_, g)| g.phase == Phase::Stage(ix))
            .map(|(at, _)| at)
            .collect();
        change
            .hops
            .iter()
            .filter(|hop| groups.contains(&hop.group))
            .filter_map(|hop| Some((hop.key.clone(), hop.cause.as_ref()?.word())))
            .collect()
    }

    #[test]
    fn a_listed_stage_turns_not_found_into_not_used_here_and_nothing_else() {
        let found = Shape::ProdLonNotFound.apply(change_for_tests()).unwrap();
        let before = causes(&found, "prod-lon");
        assert!(!before.is_empty() && before.iter().all(|(_, w)| *w == "Not found"));
        let mut changed = found.clone();
        let unknown = not_expected_in(&mut changed, &["prod-lon"]);
        assert!(unknown.is_empty());
        let after = causes(&changed, "prod-lon");
        assert_eq!(after.len(), before.len());
        assert!(
            after.iter().all(|(_, w)| *w == "Not used here"),
            "{after:?}"
        );
        // The Next step too, and the other Stages are as they were.
        assert_eq!(
            changed.stages[3].next_step,
            crate::change::NextStep::NoRollout
        );
        for stage in ["dev", "stage", "prod-ams"] {
            assert_eq!(causes(&changed, stage), causes(&found, stage), "{stage}");
        }
        // The listed Stage's gates, which have no cause, are untouched.
        assert_eq!(changed.stages[3].promotion, found.stages[3].promotion);
        // Not used here is no problem, so nothing is left to fix there.
        assert!(
            crate::change::fixes(&changed)
                .iter()
                .all(|fix| !fix.hop.starts_with("prod-lon-")),
            "{:?}",
            crate::change::fixes(&changed)
        );
    }

    #[test]
    fn only_not_found_changes_waiting_and_cant_read_stay() {
        let causes_change = Shape::Causes.apply(change_for_tests()).unwrap();
        let mut changed = causes_change.clone();
        // prod-ams waits for promotion; prod-lon can't read its pods.
        not_expected_in(&mut changed, &["prod-ams", "prod-lon", "dev", "stage"]);
        for stage in ["dev", "stage", "prod-ams", "prod-lon"] {
            let words = |c: &Change| {
                causes(c, stage)
                    .into_iter()
                    .map(|(_, w)| w)
                    .collect::<Vec<_>>()
            };
            assert_eq!(words(&changed), words(&causes_change), "{stage}");
        }
        // The build's Not found is no Stage's, so it stays too.
        assert_eq!(
            changed.hop("policy").unwrap().cause,
            causes_change.hop("policy").unwrap().cause
        );
    }

    #[test]
    fn a_name_that_matches_no_stage_is_unknown_and_changes_nothing() {
        let found = Shape::ProdLonNotFound.apply(change_for_tests()).unwrap();
        let mut changed = found.clone();
        let unknown = not_expected_in(&mut changed, &["prod-lan", "prod-lon", "prod-lan"]);
        assert_eq!(unknown, ["prod-lan", "prod-lan"]);
        assert_eq!(
            causes(&changed, "prod-lon")
                .iter()
                .filter(|(_, w)| *w == "Not used here")
                .count(),
            causes(&found, "prod-lon").len()
        );
        // With no Stage at all, nothing is called unknown.
        let mut empty = found;
        empty.stages.clear();
        assert!(not_expected_in(&mut empty, &["anything"]).is_empty());
    }
}
