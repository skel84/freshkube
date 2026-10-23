//! Ending a session ends the child and everything it started: a process group
//! on Unix, a job object on Windows (where this is the one test of it that
//! runs; the rest of the process tests are Unix-only).

mod support;

use std::time::{Duration, Instant};

use freshkube_agent::investigation::Investigation;
use serde_json::Value;
use support::{alive, said, start_binary, stub, until_ended};

#[tokio::test]
async fn dropping_the_session_ends_the_child_and_its_group() {
    let stub = stub().await;
    let (session, mut rx) = start_binary(&[], stub.endpoint()).await;
    session.prompt("start a grandchild").unwrap();
    let mut inv = Investigation::default();
    until_ended(&mut rx, &mut inv).await;
    let pids: Value = serde_json::from_str(&said(&inv)).unwrap();
    let agent = pids["agent"].as_i64().unwrap() as i32;
    let grandchild = pids["grandchild"].as_i64().unwrap() as i32;
    assert!(alive(agent) && alive(grandchild));

    drop(session);
    let deadline = Instant::now() + Duration::from_secs(6);
    while (alive(agent) || alive(grandchild)) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!alive(agent), "the agent is still running");
    assert!(!alive(grandchild), "its grandchild is still running");
}
