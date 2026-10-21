use super::*;
use crate::change::Role;
use crate::change::example::{self, Shape};
use chrono::{TimeZone, Utc};

fn example() -> Change {
    let now = Utc.with_ymd_and_hms(2026, 10, 21, 12, 0, 0).unwrap();
    example::change(now)
}

/// Every Stage runs the change and nothing is Unknown or pending.
fn healthy() -> Change {
    let mut change = example();
    for hop in &mut change.hops {
        hop.cause = None;
        hop.state = HealthIndicator::Healthy;
    }
    for stage in &mut change.stages {
        stage.state = HealthIndicator::Healthy;
        stage.reach = Reach::Linked(Confidence::Confirmed);
        stage.eligible = Eligible::Warehouse { at: None };
        stage.promotion = Promotion::Automatic { at: None };
    }
    change
}

#[test]
fn a_healthy_change_runs_everywhere_and_has_nothing_to_fix() {
    let change = healthy();
    assert_eq!(fixes(&change), []);
    assert_eq!(
        headline(&change),
        "Runs in dev, stage, prod-ams and prod-lon."
    );
}

#[test]
fn a_change_with_all_four_causes_lists_each_to_fix() {
    let change = Shape::Causes.apply(example()).expect("change");
    let items: Vec<(String, String)> = fixes(&change)
        .into_iter()
        .map(|fix| (fix.hop.clone(), fix.words()))
        .collect();
    assert_eq!(
        items,
        [
            ("pr-run", "Not found: Run checkout-pr-418-m2q8"),
            ("policy", "Not found: Policy acme-prod"),
            ("stage-argocd", "Not set up: stage checkout-stage"),
            ("prod-ams-pods", "Can't read: prod-ams pods"),
        ]
        .map(|(hop, words)| (hop.to_owned(), words.to_owned()))
    );
    // Waiting is no problem: no item, though prod-ams waits.
    assert!(change.hops.iter().any(|hop| {
        hop.cause
            .as_ref()
            .is_some_and(|cause| matches!(cause, Cause::Waiting { .. }))
    }));
}

#[test]
fn the_headline_names_what_failed_can_not_be_confirmed_and_waits_in_that_order() {
    let change = Shape::Causes.apply(example()).expect("change");
    assert_eq!(
        headline(&change),
        "Runs in dev, stage and prod-lon. prod-lon failed (verification failed); \
         the build can't be confirmed (run not found, policy not found); \
         stage can't be confirmed (delivery.stage_naming not set); and 2 more."
    );
}

#[test]
fn a_failing_change_says_which_stage_failed_and_which_waits() {
    let mut change = example();
    change.hops.iter_mut().for_each(|hop| hop.cause = None);
    assert_eq!(
        headline(&change),
        "Runs in dev, stage and prod-lon. prod-lon failed (verification failed); \
         prod-ams waits for promotion."
    );
}

#[test]
fn a_refused_kind_is_forbidden_and_a_retryable_one_unreadable() {
    let hop = &example().hops[0];
    let role = Role::new("", "pods", Some("shop"));
    assert_eq!(
        reason(&Cause::cant_read("x", Some(role.clone()), true), hop),
        "pods forbidden"
    );
    assert_eq!(
        reason(&Cause::cant_read("x", Some(role), false), hop),
        "pods unreadable"
    );
    assert_eq!(
        reason(&Cause::cant_read("x", None, false), hop),
        "unreadable"
    );
    assert_eq!(reason(&Cause::cant_read("x", None, true), hop), "refused");
}

#[test]
fn a_change_that_runs_nowhere_says_so() {
    let mut change = healthy();
    for stage in &mut change.stages {
        stage.reach = Reach::NotReached;
    }
    assert_eq!(headline(&change), "Runs in no Stage yet.");
}

#[test]
fn more_than_three_clauses_are_counted_not_spelled_out() {
    let mut change = healthy();
    for stage in &mut change.stages {
        stage.promotion = Promotion::Waiting;
        stage.reach = Reach::NotReached;
    }
    assert_eq!(
        headline(&change),
        "Runs in no Stage yet. dev waits for promotion; stage waits for promotion; \
         prod-ams waits for promotion; and 1 more."
    );
}

#[test]
fn a_list_reads_as_words() {
    let names = |items: &[&str]| list(&items.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert_eq!(names(&["a"]), "a");
    assert_eq!(names(&["a", "b"]), "a and b");
    assert_eq!(names(&["a", "b", "c"]), "a, b and c");
}
