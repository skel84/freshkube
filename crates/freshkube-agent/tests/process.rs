//! The ACP client against the scripted agent as a process: what it is given,
//! and what happens to it and to what it starts.
#![cfg(unix)]

mod support;

use std::time::{Duration, Instant};

use freshkube_agent::investigation::{Entry, Event, Investigation, Outcome, StepState};
use serde_json::Value;
use support::{said, start_binary, stub, until_ended};

async fn environment(args: &[&str]) -> (Value, support::Stub) {
    let stub = stub().await;
    let (session, mut rx) = start_binary(args, stub.endpoint()).await;
    session.prompt("describe your environment").unwrap();
    let mut inv = Investigation::default();
    until_ended(&mut rx, &mut inv).await;
    assert_eq!(inv.outcome(), Outcome::Answered);
    let report = serde_json::from_str(&said(&inv)).expect("the agent reports JSON");
    drop(session);
    (report, stub)
}

#[tokio::test]
async fn the_token_reaches_the_agent_only_in_session_new() {
    let stub = stub().await;
    let token = stub.token.clone();
    let (session, mut rx) = start_binary(&[], stub.endpoint()).await;

    // What the process was given: neither its arguments nor its environment
    // hold the token.
    session.prompt("describe your environment").unwrap();
    let mut inv = Investigation::default();
    until_ended(&mut rx, &mut inv).await;
    let report = said(&inv);
    assert!(
        !report.contains(&token),
        "the token is in the agent's argv or environment"
    );
    let report: Value = serde_json::from_str(&report).unwrap();
    assert_eq!(
        report["args"].as_array().unwrap().len(),
        1,
        "only its own path"
    );

    // And yet it can read: the one place it learned the token is session/new,
    // where it is a bearer header the endpoint accepts.
    session.prompt("checkout is degraded").unwrap();
    until_ended(&mut rx, &mut inv).await;
    let step = inv.entries().iter().find_map(|e| match e {
        Entry::Step(step) if step.id == "t1" => Some(step.state),
        _ => None,
    });
    assert_eq!(step, Some(StepState::Done));
    let seen = stub.seen.lock().unwrap();
    assert!(!seen.is_empty());
    assert!(
        seen.iter().all(|h| h == &format!("Bearer {token}")),
        "{seen:?}"
    );
}

#[tokio::test]
async fn the_child_starts_from_a_cleared_environment() {
    let (report, _stub) = environment(&[]).await;
    let env = report["env"].as_object().unwrap();
    // The system may add its own `__`-prefixed variable to a started process
    // (macOS adds `__CF_USER_TEXT_ENCODING`).
    let mut names: Vec<&str> = env
        .keys()
        .map(String::as_str)
        .filter(|name| !name.starts_with("__"))
        .collect();
    names.sort_unstable();
    // Only what the profile sets; nothing of the test process's.
    assert_eq!(
        names,
        [
            "HOME",
            "KUBECONFIG",
            "LANG",
            "PATH",
            "TALOSCONFIG",
            "TMPDIR",
            "XDG_CONFIG_HOME"
        ]
    );
    assert_eq!(report["kubeconfig_len"], 0);
    assert_eq!(report["talosconfig_len"], 0);
    assert_eq!(report["config_home_entries"], 0);
    assert_eq!(report["cwd_entries"], 0);
}

#[tokio::test]
async fn cancel_ends_a_process_prompt_within_500_ms() {
    let stub = stub().await;
    let (session, mut rx) = start_binary(&[], stub.endpoint()).await;
    session.prompt("slow").unwrap();
    let mut inv = Investigation::default();
    let mut cancelled_at = None;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(20), rx.recv())
            .await
            .expect("the agent ends")
            .unwrap();
        let ended = matches!(event, Event::Ended(_));
        let thought = matches!(event, Event::Thought(_));
        inv.record(event);
        if thought && cancelled_at.is_none() {
            cancelled_at = Some(Instant::now());
            session.cancel();
        }
        if ended {
            break;
        }
    }
    let latency = cancelled_at.expect("a thought streamed").elapsed();
    assert!(latency < Duration::from_millis(500), "took {latency:?}");
    assert_eq!(inv.outcome(), Outcome::Cancelled);
}

#[tokio::test]
async fn a_crash_mid_prompt_in_a_process_ends_as_failed() {
    let stub = stub().await;
    let (session, mut rx) = start_binary(&[], stub.endpoint()).await;
    session.prompt("crash").unwrap();
    let mut inv = Investigation::default();
    until_ended(&mut rx, &mut inv).await;
    assert!(
        matches!(inv.outcome(), Outcome::Failed(_)),
        "{:?}",
        inv.outcome()
    );
}

#[tokio::test]
async fn a_process_that_never_initializes_fails_and_is_ended() {
    let stub = stub().await;
    let (session, mut rx) = start_binary(&["--never-initialize"], stub.endpoint()).await;
    // Dropping the session while it waits ends the process; nothing hangs.
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(session);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("the channel closes")
            .is_none()
    );
}
