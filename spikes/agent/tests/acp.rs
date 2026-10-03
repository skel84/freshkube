//! The ACP backend against the scripted agent in `src/bin/fake_acp_agent.rs`,
//! which reads through our MCP endpoint as Claude Code and Codex do.

use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_client_protocol::AcpAgentConfig;
use freshkube_agent_spike::acp::{AcpSession, AgentKind};
use freshkube_agent_spike::investigation::{
    DWELL, Entry, Event, Investigation, Outcome, StepState,
};
use freshkube_agent_spike::mcp;
use freshkube_agent_spike::scenario::{Cluster, ObjectRef};
use freshkube_agent_spike::tools::{Place, Tab};

fn fake() -> AgentKind {
    AgentKind::Command(AcpAgentConfig::new(env!("CARGO_BIN_EXE_fake_acp_agent")))
}

fn start(prompt: &str) -> (AcpSession, tokio::sync::mpsc::UnboundedReceiver<Event>) {
    AcpSession::start(fake(), Arc::new(Cluster::checkout_crash()), prompt)
}

#[tokio::test]
async fn an_acp_agent_reads_through_mcp_and_the_view_follows() {
    let (_session, mut rx) = start("checkout is degraded");
    let mut inv = Investigation::default();
    let mut moves = Vec::new();
    let mut now = Instant::now();
    while let Some(event) = tokio::time::timeout(Duration::from_secs(20), rx.recv())
        .await
        .expect("the agent answers")
    {
        let ended = matches!(event, Event::Ended(_));
        now += DWELL * 2;
        moves.extend(inv.record(event, now));
        if ended {
            break;
        }
    }

    assert_eq!(inv.outcome, Outcome::Answered);
    assert_eq!(
        moves,
        vec![Place::Object {
            object: ObjectRef {
                kind: "Deployment",
                namespace: "shop",
                name: "checkout"
            },
            tab: Tab::Overview,
        }]
    );
    let steps: Vec<_> = inv
        .entries
        .iter()
        .filter_map(|e| match e {
            Entry::Step {
                id, state, place, ..
            } => Some((id.as_str(), *state, place.is_some())),
            _ => None,
        })
        .collect();
    // Our read is allowed, lands on its step and moves the view; the shell
    // command is refused before it runs.
    assert_eq!(
        steps,
        vec![
            ("t1", StepState::Done, true),
            ("t2", StepState::Failed, false)
        ]
    );
    assert_eq!(inv.unmatched_reads, 0);
    assert!(inv.entries.contains(&Entry::Note(
        "Refused: kubectl delete pod checkout-7c9d8f6b4-q8zxk".into()
    )));
    assert!(
        inv.entries
            .contains(&Entry::Thought("Reading the Deployment.".into()))
    );
    assert!(matches!(inv.entries.last(), Some(Entry::Said(s)) if s == "Cause: scripted."));
}

#[tokio::test]
async fn cancelling_an_acp_agent_ends_its_prompt_promptly() {
    let (session, mut rx) = start("slow");
    let mut inv = Investigation::default();
    let mut cancelled_at = None;
    while let Some(event) = tokio::time::timeout(Duration::from_secs(20), rx.recv())
        .await
        .expect("the agent ends")
    {
        let ended = matches!(event, Event::Ended(_));
        let thought = matches!(event, Event::Thought(_));
        inv.record(event, Instant::now());
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
    assert_eq!(inv.outcome, Outcome::Cancelled);
}

#[tokio::test]
async fn the_mcp_endpoint_refuses_requests_without_the_session_token() {
    let server = mcp::serve(Arc::new(Cluster::checkout_crash()), Arc::new(|_, _| {}))
        .await
        .unwrap();
    let initialize = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}
    });
    let post = |token: Option<&str>| {
        let mut request = reqwest::Client::new()
            .post(&server.url)
            .header("Accept", "application/json, text/event-stream")
            .json(&initialize);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send()
    };
    assert_eq!(post(None).await.unwrap().status(), 401);
    assert_eq!(post(Some("guess")).await.unwrap().status(), 401);
    assert_eq!(post(Some(&server.token)).await.unwrap().status(), 200);
}
