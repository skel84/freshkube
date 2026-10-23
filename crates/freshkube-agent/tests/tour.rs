//! The end-to-end path of the dock's fixture: the scripted agent, over ACP,
//! reads through the real endpoint to example objects, then answers.

mod common;
mod support;

use std::time::Duration;

use common::{ENTRY, granted, start};
use freshkube_agent::acp::{AcpSession, Endpoint};
use freshkube_agent::investigation::{Entry, Investigation, Outcome, StepState};
use freshkube_agent::scripted::{self, Script, TourRead};
use freshkube_agent::{Outcome as ReadOutcome, ReadEvent};
use serde_json::json;
use support::{notes, said, until_ended};

fn read(tool: &str, arguments: serde_json::Value) -> TourRead {
    TourRead {
        tool: tool.into(),
        arguments,
    }
}

fn script() -> Script {
    Script {
        tour: vec![
            read("list_objects", json!({"kind": "pods", "namespace": "shop"})),
            read(
                "get_object",
                json!({"kind": "pods", "namespace": "shop", "name": "api-0"}),
            ),
            read("pod_logs", json!({"namespace": "shop", "pod": "api-0"})),
            read("list_events", json!({"namespace": "shop"})),
        ],
        tour_shell: true,
        ..Script::default()
    }
}

#[tokio::test]
async fn the_scripted_agent_reads_through_the_real_endpoint_and_answers() {
    let access = granted();
    let (endpoint, mut reads) = start(&access).await;
    let (session, mut events) = AcpSession::start(
        scripted::in_process(script()),
        Endpoint::new(endpoint.url(), endpoint.token().reveal()),
    );
    session.prompt("why is api-0 restarting?").unwrap();
    let mut inv = Investigation::default();
    until_ended(&mut events, &mut inv).await;

    assert_eq!(inv.outcome(), Outcome::Answered);
    let steps: Vec<(String, StepState)> = inv
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            Entry::Step(step) => Some((step.tool.clone(), step.state)),
            _ => None,
        })
        .collect();
    let [list, get, logs, events, shell] = steps.as_slice() else {
        panic!("five steps: {steps:?}");
    };
    assert_eq!(list, &("freshkube_list_objects".into(), StepState::Done));
    assert_eq!(get, &("freshkube_get_object".into(), StepState::Done));
    assert_eq!(logs, &("freshkube_pod_logs".into(), StepState::Done));
    assert_eq!(events, &("freshkube_list_events".into(), StepState::Done));
    assert_eq!(shell.1, StepState::Failed, "a shell is refused");
    assert_eq!(notes(&inv), ["Refused: kubectl get pods -A"]);
    let answer = said(&inv);
    for id in ["[r1]", "[r2]", "[r3]", "[r4]"] {
        assert!(answer.contains(id), "{answer} cites {id}");
    }

    // The endpoint saw exactly those four reads, in order, all answered.
    let mut seen: Vec<ReadEvent> = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(200), reads.recv()).await
    {
        seen.push(event);
    }
    let tools: Vec<&str> = seen.iter().map(|read| read.tool).collect();
    assert_eq!(
        tools,
        ["list_objects", "get_object", "pod_logs", "list_events"]
    );
    assert!(
        seen.iter()
            .all(|read| matches!(read.outcome, ReadOutcome::Answered { .. })
                && read.cluster == ENTRY)
    );
}
