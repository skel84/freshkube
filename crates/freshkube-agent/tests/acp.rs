//! The ACP client against the scripted agent in process and as a process.

mod support;

use std::time::{Duration, Instant};

use freshkube_agent::acp::{AcpSession, Endpoint, Models};
use freshkube_agent::investigation::{Entry, Event, Investigation, Outcome, StepState};
use freshkube_agent::scripted::{self, Script};
use support::{notes, said, start_in_process, stub, until_ended};

fn offline() -> Endpoint {
    Endpoint::new("http://127.0.0.1:1/mcp", "unused")
}

fn steps(inv: &Investigation) -> Vec<(String, StepState)> {
    inv.entries()
        .iter()
        .filter_map(|entry| match entry {
            Entry::Step(step) => Some((step.id.clone(), step.state)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn only_freshkube_tools_are_allowed() {
    let (session, mut rx) = start_in_process(offline());
    session.prompt("try the tools").unwrap();
    let mut inv = Investigation::default();
    until_ended(&mut rx, &mut inv).await;

    assert_eq!(inv.outcome(), Outcome::Answered);
    // Our get_object is allowed; nothing else is.
    assert_eq!(said(&inv), "allowed=ours");
    let states = steps(&inv);
    assert_eq!(states[0], ("ours".into(), StepState::Done));
    assert!(
        states[1..]
            .iter()
            .all(|(_, state)| *state == StepState::Failed)
    );
    assert_eq!(states.len(), 8);
    // Each refusal says what was refused.
    assert_eq!(
        notes(&inv),
        [
            "Refused: kubectl delete pod checkout-7c9d8f6b4-q8zxk",
            "Refused: ~/.kube/config",
            "Refused: /etc/hosts",
            "Refused: https://example.com/payload",
            "Refused: other_get_object",
            "Refused: freshkube_get_object_extra",
            "Refused: curl http://169.254.169.254/",
            "Refused: never-announced",
        ]
    );
}

#[tokio::test]
async fn the_agent_reads_through_the_endpoint_and_a_shell_is_refused() {
    let stub = stub().await;
    let (session, mut rx) = start_in_process(stub.endpoint());
    session.prompt("checkout is degraded").unwrap();
    let mut inv = Investigation::default();
    until_ended(&mut rx, &mut inv).await;
    assert_eq!(inv.outcome(), Outcome::Answered);
    assert_eq!(
        steps(&inv),
        [
            ("t1".into(), StepState::Done),
            ("t2".into(), StepState::Failed)
        ]
    );
    assert_eq!(
        notes(&inv),
        ["Refused: kubectl delete pod checkout-7c9d8f6b4-q8zxk"]
    );
    assert!(said(&inv).contains("Cause: scripted."));
    // The read carried the session's token, and nothing else did.
    let seen = stub.seen.lock().unwrap();
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|h| h == &format!("Bearer {}", stub.token)));
}

#[tokio::test]
async fn several_prompts_run_in_one_session_one_after_another() {
    let (session, mut rx) = start_in_process(offline());
    session.prompt("model").unwrap();
    session.prompt("model").unwrap();
    let mut inv = Investigation::default();
    until_ended(&mut rx, &mut inv).await;
    assert_eq!(inv.outcome(), Outcome::Answered);
    until_ended(&mut rx, &mut inv).await;
    let prompts = inv
        .entries()
        .iter()
        .filter(|e| matches!(e, Entry::Asked(_)))
        .count();
    assert_eq!(prompts, 2);
    assert_eq!(said(&inv), "model=hosted/largemodel=hosted/large");
}

#[tokio::test]
async fn cancel_ends_the_prompt_within_500_ms() {
    let (session, mut rx) = start_in_process(offline());
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
async fn a_cancel_drops_the_prompts_waiting_and_a_new_one_still_runs() {
    let (session, mut rx) = start_in_process(offline());
    session.prompt("slow").unwrap();
    session.prompt("never run").unwrap();
    let mut inv = Investigation::default();
    loop {
        let event = rx.recv().await.unwrap();
        let thought = matches!(event, Event::Thought(_));
        inv.record(event);
        if thought {
            session.cancel();
            break;
        }
    }
    until_ended(&mut rx, &mut inv).await;
    assert_eq!(inv.outcome(), Outcome::Cancelled);
    session.prompt("model").unwrap();
    until_ended(&mut rx, &mut inv).await;
    assert_eq!(inv.outcome(), Outcome::Answered);
    let asked: Vec<_> = inv
        .entries()
        .iter()
        .filter_map(|e| match e {
            Entry::Asked(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(asked, ["slow", "model"]);
}

#[tokio::test(start_paused = true)]
async fn an_agent_that_never_initializes_fails_within_the_bound() {
    let link = scripted::in_process(Script {
        never_initialize: true,
        ..Script::default()
    });
    let started = tokio::time::Instant::now();
    let (_session, mut rx) = AcpSession::start(link, offline());
    let event = rx.recv().await.expect("the session reports its failure");
    let waited = started.elapsed();
    assert_eq!(
        event,
        Event::Ended(Outcome::Failed("the scripted agent didn't start".into()))
    );
    assert!(
        waited >= freshkube_agent::acp::STARTUP_TIMEOUT,
        "{waited:?}"
    );
    assert!(waited < freshkube_agent::acp::STARTUP_TIMEOUT + Duration::from_secs(1));
}

#[tokio::test]
async fn the_model_list_and_choice_reach_the_agent() {
    let (session, mut rx) = start_in_process(offline());
    let mut models = session.models();
    let offered: Models = models
        .wait_for(|m| !m.choices.is_empty())
        .await
        .unwrap()
        .clone();
    let ids: Vec<_> = offered
        .choices
        .iter()
        .map(|m| (m.id.as_str(), m.local))
        .collect();
    assert_eq!(ids, [("hosted/large", false), ("ollama/small", true)]);
    assert_eq!(offered.current.as_deref(), Some("hosted/large"));

    // A model the agent didn't offer isn't sent.
    assert!(session.set_model("hosted/other").is_err());
    session.set_model("ollama/small").unwrap();
    models
        .wait_for(|m| m.current.as_deref() == Some("ollama/small"))
        .await
        .unwrap();
    session.prompt("which model").unwrap();
    let mut inv = Investigation::default();
    until_ended(&mut rx, &mut inv).await;
    assert_eq!(said(&inv), "model=ollama/small");
}

#[tokio::test]
async fn a_crash_mid_prompt_ends_as_failed() {
    let (session, mut rx) = start_in_process(offline());
    session.prompt("crash").unwrap();
    let mut inv = Investigation::default();
    until_ended(&mut rx, &mut inv).await;
    assert!(
        matches!(inv.outcome(), Outcome::Failed(reason) if reason.starts_with("the scripted agent stopped")),
        "{:?}",
        inv.outcome()
    );
    // The session is over: nothing more is reported, and nothing more is accepted.
    assert!(rx.recv().await.is_none());
    assert!(session.prompt("again").is_err());
}

#[tokio::test]
async fn a_flood_of_output_leaves_a_bounded_transcript() {
    let (session, mut rx) = start_in_process(offline());
    session.prompt("flood").unwrap();
    let mut inv = Investigation::default();
    until_ended(&mut rx, &mut inv).await;
    assert_eq!(inv.outcome(), Outcome::Answered);
    assert!(inv.trimmed());
    assert!(inv.text_bytes() <= freshkube_agent::investigation::MAX_TRANSCRIPT_BYTES);
    assert_eq!(said(&inv), "done flooding");
}

#[tokio::test]
async fn the_endpoint_never_shows_its_token_in_debug() {
    let endpoint = Endpoint::new("http://127.0.0.1:9/mcp", "very-secret-token");
    let shown = format!("{endpoint:?}");
    assert!(!shown.contains("very-secret-token"), "{shown}");
}
