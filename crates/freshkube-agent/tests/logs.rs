mod common;

use common::capture;

use common::*;
use freshkube_agent::{SessionId, Token};
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};

fn call(tool: &'static str, arguments: Value) -> CallToolRequestParams {
    let Value::Object(arguments) = arguments else {
        panic!()
    };
    CallToolRequestParams::new(tool).with_arguments(arguments)
}

#[tokio::test]
async fn the_token_never_reaches_debug_or_logs() {
    capture::install();
    let (endpoint, _events) = start(&granted()).await;
    let token = endpoint.token().reveal().to_owned();
    let client = client(&endpoint).await;
    client
        .call_tool(call(
            "get_object",
            json!({"kind": "pods", "namespace": "shop", "name": "api-0"}),
        ))
        .await
        .unwrap();
    // A refused call, with a value the agent typed that must not be logged.
    client
        .call_tool(call(
            "list_objects",
            json!({"kind": "pods", "limit": 12345, "namespace": "SecretShop"}),
        ))
        .await
        .unwrap();

    // A cluster the session cannot read, named by an id the agent typed.
    client
        .call_tool(call(
            "list_objects",
            json!({"kind": "pods", "cluster": "cluster-zzz"}),
        ))
        .await
        .unwrap();

    for printed in [
        format!("{endpoint:?}"),
        format!("{:?}", endpoint.token()),
        format!("{}", endpoint.token()),
        format!("{:?}", endpoint.session()),
        format!("{}", endpoint.session()),
        format!("{:?}", Token::generate().unwrap()),
    ] {
        assert!(!printed.contains(&token), "{printed}");
    }
    assert_eq!(
        format!("{}", endpoint.token()),
        "",
        "Display prints nothing"
    );

    let log = capture::captured();
    assert!(!log.contains(&token), "the token is in the log: {log}");
    assert!(!log.contains("Bearer"), "{log}");
    let lines: Vec<_> = log
        .lines()
        .filter(|line| line.contains("agent read"))
        .collect();
    assert_eq!(lines.len(), 3, "one line per read: {log}");
    let first = lines[0];
    for expected in [
        endpoint.session().short().as_str(),
        "read=r1",
        "entry=cluster-a",
        "tool=\"get_object\"",
        "code=\"ok\"",
        "pods shop/api-0",
    ] {
        assert!(first.contains(expected), "{expected} missing from {first}");
    }
    assert!(lines[1].contains("code=\"invalid\""), "{}", lines[1]);
    assert!(
        lines[2].contains("entry=unknown") && lines[2].contains("code=\"unknown_cluster\""),
        "{}",
        lines[2]
    );
    // The dependencies log at trace too, with ports and times; the rule is
    // about what this crate says.
    let ours: String = log
        .lines()
        .filter(|line| line.contains("freshkube"))
        .collect();
    for value in ["12345", "SecretShop", "cluster-zzz"] {
        assert!(!ours.contains(value), "{value} was logged: {ours}");
    }
    assert!(!ours.contains("Running"), "no answer data in the log");
    let _ = SessionId::new();
}
