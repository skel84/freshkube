use super::*;

#[tokio::test]
async fn checks_roll_up_to_one_state_and_say_when_they_stopped_at_the_cap() {
    let api = card(|_| None);
    let detail = client(&api, None).detail("acme/shop", 418).await.unwrap();
    // A run still going makes them pending; 3 of 112 runs were read.
    assert_eq!(detail.checks.state, ChecksState::Pending);
    assert_eq!(detail.checks.runs.len(), 3);
    assert_eq!(detail.checks.runs[0].state, RunState::Passed);
    assert_eq!(detail.checks.runs[1].state, RunState::Running);
    assert_eq!(detail.checks.runs[2].state, RunState::Neutral);
    assert_eq!(
        detail.checks.capped,
        Some(Capped {
            read: 3,
            total: 112
        })
    );
    assert_eq!(detail.checks.statuses[0].state, StatusState::Passed);
    let roll = |runs: Value, statuses: Value| {
        super::detail::checks(&json!({"check_runs": runs}), &json!({"statuses": statuses})).state
    };
    let run = |status: &str, conclusion: Option<&str>| json!({"name": "x", "status": status, "conclusion": conclusion});
    let ended = |conclusion: &str| run("completed", Some(conclusion));
    assert_eq!(roll(json!([]), json!([])), ChecksState::None);
    assert_eq!(
        roll(json!([ended("success")]), json!([])),
        ChecksState::Passing
    );
    // A skipped or neutral run beside a pass is fine; alone, nothing passed.
    assert_eq!(
        roll(
            json!([ended("success"), ended("skipped"), ended("neutral")]),
            json!([])
        ),
        ChecksState::Passing
    );
    for only in ["skipped", "neutral", "cancelled"] {
        assert_eq!(
            roll(json!([ended(only)]), json!([])),
            ChecksState::Ended,
            "{only}"
        );
    }
    // A cancelled run beside a pass did not run: not passing.
    assert_eq!(
        roll(json!([ended("success"), ended("cancelled")]), json!([])),
        ChecksState::Ended
    );
    assert_eq!(
        roll(json!([ended("failure"), run("queued", None)]), json!([])),
        ChecksState::Failing
    );
    assert_eq!(
        roll(
            json!([ended("success")]),
            json!([{"context": "c", "state": "pending"}])
        ),
        ChecksState::Pending
    );
    assert_eq!(
        roll(json!([]), json!([{"context": "c", "state": "error"}])),
        ChecksState::Failing
    );
    assert_eq!(
        roll(json!([]), json!([{"context": "c", "state": "success"}])),
        ChecksState::Passing
    );
    // Cut at the cap, what was read can't say passing: the rest could fail.
    let cut = |runs: Value, statuses: Value, runs_total: u64, statuses_total: u64| {
        super::detail::checks(
            &json!({"check_runs": runs, "total_count": runs_total}),
            &json!({"statuses": statuses, "total_count": statuses_total}),
        )
    };
    let passing = json!([ended("success")]);
    let cut_runs = cut(passing.clone(), json!([]), 112, 0);
    assert_eq!(cut_runs.state, ChecksState::Partial);
    assert_eq!(
        cut_runs.capped,
        Some(Capped {
            read: 1,
            total: 112
        })
    );
    // …by the combined status's own count, too.
    let cut_statuses = cut(
        passing,
        json!([{"context": "c", "state": "success"}]),
        1,
        31,
    );
    assert_eq!(cut_statuses.state, ChecksState::Partial);
    assert_eq!(
        cut_statuses.statuses_capped,
        Some(Capped { read: 1, total: 31 })
    );
    // A failure among what was read is a failure whatever was not.
    let failing = cut(json!([ended("failure")]), json!([]), 112, 0);
    assert_eq!(failing.state, ChecksState::Failing);
}
