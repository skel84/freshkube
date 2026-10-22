use chrono::{DateTime, Duration, TimeZone, Utc};

use super::*;
use crate::model::Follow;

fn at(minutes: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 22, 9, 0, 0).unwrap() + Duration::minutes(minutes)
}

fn follow(subject: Subject) -> Follow {
    Follow {
        id: 1,
        subject,
        context: "dev-fra".into(),
        sensitive: false,
        since: at(0),
    }
}

fn crash_loop() -> Tracker {
    Tracker::new(&follow(Subject::CrashLoop {
        namespace: "shop".into(),
        pod: "api-7c9".into(),
    }))
}

fn pod(crash_looping: bool, ready: bool) -> Seen {
    Seen::Pod(Some(PodSeen {
        crash_looping,
        ready,
        restarts: 4,
    }))
}

#[test]
fn a_crash_loop_fires_once() {
    let mut tracker = crash_loop();
    let first = tracker.observe(&pod(true, false), at(0));
    assert_eq!(first.updates.len(), 1);
    assert!(first.updates[0].failure);
    for minute in 1..10 {
        let again = tracker.observe(&pod(true, false), at(minute));
        assert!(again.updates.is_empty(), "minute {minute}");
    }
}

#[test]
fn a_crash_loop_fires_again_only_after_five_minutes_ready() {
    let mut tracker = crash_loop();
    assert_eq!(tracker.observe(&pod(true, false), at(0)).updates.len(), 1);
    // Ready for 4 minutes is not enough.
    tracker.observe(&pod(false, true), at(1));
    let early = tracker.observe(&pod(true, false), at(5));
    assert!(early.updates.is_empty());
    // Ready from minute 6 and still at minute 11: five minutes.
    tracker.observe(&pod(false, true), at(6));
    tracker.observe(&pod(false, true), at(11));
    let again = tracker.observe(&pod(true, false), at(12));
    assert_eq!(again.updates.len(), 1);
    // And once more, not on every observation.
    assert!(
        tracker
            .observe(&pod(true, false), at(13))
            .updates
            .is_empty()
    );
}

#[test]
fn a_long_ready_spell_with_no_event_in_it_rearms_on_the_next_one() {
    let mut tracker = crash_loop();
    assert_eq!(tracker.observe(&pod(true, false), at(0)).updates.len(), 1);
    tracker.observe(&pod(false, true), at(1));
    // Nothing is seen for an hour; the next event is the crash loop.
    let again = tracker.observe(&pod(true, false), at(61));
    assert_eq!(again.updates.len(), 1);
}

#[test]
fn a_pod_that_is_gone_is_not_watching_once() {
    let mut tracker = crash_loop();
    let first = tracker.observe(&Seen::Pod(None), at(0));
    assert!(matches!(first.status, Status::NotWatching(_)));
    assert_eq!(first.updates.len(), 1);
    assert_eq!(first.updates[0].headline, "Not watching");
    assert!(!first.updates[0].failure);
    let second = tracker.observe(&Seen::Pod(None), at(1));
    assert!(matches!(second.status, Status::NotWatching(_)));
    assert!(second.updates.is_empty());
}

#[test]
fn reading_again_re_arms_the_not_watching_report() {
    let mut tracker = crash_loop();
    tracker.observe(&Seen::Unreadable("refused".into()), at(0));
    let back = tracker.observe(&pod(false, true), at(1));
    assert_eq!(back.status, Status::Watching);
    let again = tracker.observe(&Seen::Unreadable("refused".into()), at(2));
    assert_eq!(again.updates.len(), 1);
}

fn stage(name: &str, current: &[&str], verified: &[(&str, &str)]) -> StageSeen {
    StageSeen {
        name: name.into(),
        current_freight: current.iter().map(|s| (*s).to_owned()).collect(),
        verifications: verified
            .iter()
            .map(|(freight, phase)| (vec![(*freight).to_owned()], Some((*phase).to_owned())))
            .collect(),
    }
}

#[test]
fn a_landing_says_so_once_and_ends() {
    let mut tracker = Tracker::new(&follow(Subject::Landing {
        project: "checkout".into(),
        stage: "prod-ams".into(),
        freight: "wonky-otter".into(),
    }));
    let waiting = tracker.observe(&Seen::Stages(vec![stage("prod-ams", &["old"], &[])]), at(0));
    assert!(waiting.updates.is_empty() && !waiting.finished);
    let landed = tracker.observe(
        &Seen::Stages(vec![stage("prod-ams", &["wonky-otter"], &[])]),
        at(1),
    );
    assert_eq!(landed.updates.len(), 1);
    assert!(!landed.updates[0].failure);
    assert!(landed.finished);
    let again = tracker.observe(
        &Seen::Stages(vec![stage("prod-ams", &["wonky-otter"], &[])]),
        at(2),
    );
    assert!(again.updates.is_empty());
}

#[test]
fn a_failed_verification_is_a_failure_and_a_good_one_ends_quietly() {
    let subject = Subject::Verification {
        project: "checkout".into(),
        stage: "prod-ams".into(),
        freight: "wonky-otter".into(),
    };
    let mut failing = Tracker::new(&follow(subject.clone()));
    let step = failing.observe(
        &Seen::Stages(vec![stage("prod-ams", &[], &[("wonky-otter", "Failed")])]),
        at(0),
    );
    assert_eq!(step.updates.len(), 1);
    assert!(step.updates[0].failure && step.finished);

    let mut passing = Tracker::new(&follow(subject));
    let step = passing.observe(
        &Seen::Stages(vec![stage(
            "prod-ams",
            &[],
            &[("wonky-otter", "Successful")],
        )]),
        at(0),
    );
    assert!(step.updates.is_empty() && step.finished);
}

#[test]
fn a_missing_stage_is_not_watching() {
    let mut tracker = Tracker::new(&follow(Subject::Landing {
        project: "checkout".into(),
        stage: "prod-ams".into(),
        freight: "wonky-otter".into(),
    }));
    let step = tracker.observe(&Seen::Stages(vec![stage("dev", &[], &[])]), at(0));
    assert!(matches!(step.status, Status::NotWatching(_)));
}

fn incident(key: &str, app: &str, open: bool) -> IncidentSeen {
    IncidentSeen {
        key: key.into(),
        application: app.into(),
        critical: true,
        open,
        description: "high latency".into(),
    }
}

#[test]
fn incidents_open_at_the_start_are_a_baseline_not_news() {
    let mut tracker = Tracker::new(&follow(Subject::Incident { application: None }));
    let first = tracker.observe(
        &Seen::Incidents(vec![incident("a", "checkout", true)]),
        at(0),
    );
    assert!(first.updates.is_empty());
    let next = tracker.observe(
        &Seen::Incidents(vec![
            incident("a", "checkout", true),
            incident("b", "checkout", true),
        ]),
        at(1),
    );
    assert_eq!(next.updates.len(), 1);
    assert_eq!(next.updates[0].state, "incident:b");
    // The same incident is not told again.
    let again = tracker.observe(
        &Seen::Incidents(vec![
            incident("a", "checkout", true),
            incident("b", "checkout", true),
        ]),
        at(2),
    );
    assert!(again.updates.is_empty());
}

#[test]
fn an_incident_follow_for_one_application_ignores_the_others() {
    let mut tracker = Tracker::new(&follow(Subject::Incident {
        application: Some("checkout".into()),
    }));
    tracker.observe(&Seen::Incidents(vec![]), at(0));
    let step = tracker.observe(
        &Seen::Incidents(vec![
            incident("a", "payments", true),
            incident("b", "checkout", true),
            incident("c", "checkout", false),
        ]),
        at(1),
    );
    assert_eq!(step.updates.len(), 1);
    assert_eq!(step.updates[0].state, "incident:b");
}
