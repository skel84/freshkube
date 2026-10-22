mod common;

use std::time::Duration;

use common::*;
use freshkube_agent::{Outcome, SessionId};
use freshkube_core::agent_reads::{Code, TOOLS};
use freshkube_nav::{
    KindKey, LinkPage, Name, Namespace, ObjectUid, ViewObject, ViewState, ViewTab,
};
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};

fn post(endpoint: &freshkube_agent::Endpoint) -> reqwest::RequestBuilder {
    reqwest::Client::new()
        .post(endpoint.url())
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#)
}

fn bearer(endpoint: &freshkube_agent::Endpoint) -> String {
    format!("Bearer {}", endpoint.token().reveal())
}

#[tokio::test]
async fn the_endpoint_listens_on_loopback_only() {
    let (endpoint, _) = start(&granted()).await;
    assert_eq!(endpoint.addr().ip().to_string(), "127.0.0.1");
    assert!(endpoint.url().starts_with("http://127.0.0.1:"));
    assert!(endpoint.url().ends_with("/mcp"));
}

#[tokio::test]
async fn the_endpoint_refuses_without_the_session_token() {
    let (endpoint, _) = start(&granted()).await;
    let (other, _) = start(&granted()).await;
    let status = |response: reqwest::Response| response.status().as_u16();

    assert_eq!(
        status(post(&endpoint).send().await.unwrap()),
        401,
        "no token"
    );
    let wrong = post(&endpoint)
        .header("authorization", "Bearer 0000")
        .send()
        .await
        .unwrap();
    assert_eq!(status(wrong), 401, "a wrong token");
    let foreign = post(&endpoint)
        .header("authorization", bearer(&other))
        .send()
        .await
        .unwrap();
    assert_eq!(status(foreign), 401, "another session's token");

    let with_origin = post(&endpoint)
        .header("authorization", bearer(&endpoint))
        .header("origin", "http://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(status(with_origin), 403, "an Origin header");
    let foreign_host = post(&endpoint)
        .header("authorization", bearer(&endpoint))
        .header("host", "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(status(foreign_host), 403, "a foreign Host");

    let big = reqwest::Client::new()
        .post(endpoint.url())
        .header("authorization", bearer(&endpoint))
        .header("content-type", "application/json")
        .body(vec![b' '; 65 * 1024])
        .send()
        .await
        .unwrap();
    assert_eq!(status(big), 413, "a 65 KiB body");

    let fine = post(&endpoint)
        .header("authorization", bearer(&endpoint))
        .send()
        .await
        .unwrap();
    assert_ne!(fine.status().as_u16(), 401);
    assert_ne!(fine.status().as_u16(), 403);
}

#[tokio::test]
async fn the_token_dies_with_the_session() {
    let (endpoint, _) = start(&granted()).await;
    let (addr, token) = (endpoint.addr(), endpoint.token().reveal().to_owned());
    drop(endpoint);
    let mut refused = false;
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_err() {
            refused = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(refused, "the port is closed after the drop");
    let (next, _) = start(&granted()).await;
    assert_ne!(next.token().reveal(), token);
}

fn view(tab: ViewTab, name: Option<(&str, Option<&str>)>) -> ViewState {
    let mut view = ViewState::page(LinkPage::Resources);
    view.kind = Some(KindKey::parse("pods").unwrap());
    view.ns = Some(Namespace::parse("shop").unwrap());
    view.object = name.map(|(name, uid)| ViewObject {
        name: Name::parse(name).unwrap(),
        uid: uid.map(|uid| ObjectUid::parse(uid).unwrap()),
    });
    view.tab = tab;
    view
}

fn call(tool: &'static str, arguments: Value) -> CallToolRequestParams {
    let Value::Object(arguments) = arguments else {
        panic!()
    };
    CallToolRequestParams::new(tool).with_arguments(arguments)
}

#[tokio::test]
async fn a_fake_agent_client_reads_through_the_endpoint() {
    let (endpoint, mut events) = start(&granted()).await;
    let client = client(&endpoint).await;

    let listed = client.list_tools(None).await.unwrap();
    let mut names: Vec<_> = listed
        .tools
        .iter()
        .map(|tool| tool.name.to_string())
        .collect();
    names.sort();
    let mut expected: Vec<_> = TOOLS.iter().map(|name| name.to_string()).collect();
    expected.sort();
    assert_eq!(names, expected, "exactly the four tools");

    let calls = [
        (
            call("list_objects", json!({"kind": "pods", "namespace": "shop"})),
            view(ViewTab::Details, None),
        ),
        (
            call(
                "get_object",
                json!({"kind": "pods", "namespace": "shop", "name": "api-0"}),
            ),
            view(ViewTab::Details, Some(("api-0", Some(POD_UID)))),
        ),
        (
            call("pod_logs", json!({"namespace": "shop", "pod": "api-0"})),
            view(ViewTab::Logs, Some(("api-0", None))),
        ),
        (
            call(
                "list_events",
                json!({"namespace": "shop", "kind": "Pod", "name": "api-0"}),
            ),
            view(ViewTab::Events, Some(("api-0", Some(POD_UID)))),
        ),
    ];
    for (n, (request, expected_view)) in calls.into_iter().enumerate() {
        let tool = request.name.to_string();
        let result = client.call_tool(request).await.unwrap();
        assert_eq!(result.is_error, Some(false), "{tool}");
        let structured = result
            .structured_content
            .clone()
            .expect("structuredContent");
        assert_eq!(structured["read"], format!("r{}", n + 1));
        assert_eq!(structured["cluster"], ENTRY);
        let text = match &result.content[0] {
            rmcp::model::ContentBlock::Text(text) => text.text.clone(),
            other => panic!("not text: {other:?}"),
        };
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap(),
            structured,
            "text and structured agree"
        );

        let event = events.recv().await.expect("one ReadEvent per call");
        assert_eq!(event.tool, tool);
        assert_eq!(event.read.to_string(), format!("r{}", n + 1));
        assert_eq!(event.session, endpoint.session());
        assert_eq!(event.cluster, ENTRY);
        assert_eq!(event.view.as_ref(), Some(&expected_view), "{tool}");
        assert_eq!(event.outcome, Outcome::Answered { cut: None });
    }
    assert!(events.try_recv().is_err(), "exactly one event per call");
}

#[tokio::test]
async fn a_refused_read_gets_a_read_id_and_no_view() {
    let (endpoint, mut events) = start(&granted()).await;
    let client = client(&endpoint).await;
    let cases = [
        (
            call("list_objects", json!({"kind": "pods", "cluster": OTHER})),
            Code::NotEnabled,
        ),
        (
            call("list_objects", json!({"kind": "pods", "limit": 9000})),
            Code::Invalid,
        ),
        (
            call(
                "list_objects",
                json!({"kind": "pods", "cluster": "cluster-z"}),
            ),
            Code::UnknownCluster,
        ),
        (
            call(
                "get_object",
                json!({"kind": "pods", "namespace": "shop", "name": "missing"}),
            ),
            Code::NotFound,
        ),
    ];
    for (n, (request, code)) in cases.into_iter().enumerate() {
        let result = client.call_tool(request).await.unwrap();
        assert_eq!(result.is_error, Some(true));
        let body = result.structured_content.unwrap();
        assert_eq!(body["read"], format!("r{}", n + 1));
        assert_eq!(body["error"]["code"], code.as_str());
        assert!(body["error"]["message"].is_string());
        let event = events.recv().await.unwrap();
        assert_eq!(event.view, None);
        assert_eq!(event.outcome, Outcome::Refused(code));
    }
}

#[tokio::test]
async fn no_view_state_goes_on_the_wire() {
    let (endpoint, _events) = start(&granted()).await;
    let client = client(&endpoint).await;
    for request in [
        call("list_objects", json!({"kind": "pods", "namespace": "shop"})),
        call(
            "get_object",
            json!({"kind": "pods", "namespace": "shop", "name": "api-0"}),
        ),
        call("pod_logs", json!({"namespace": "shop", "pod": "api-0"})),
        call("list_events", json!({"namespace": "shop"})),
        call("list_objects", json!({"kind": "pods", "cluster": OTHER})),
    ] {
        let result = client.call_tool(request).await.unwrap();
        let all = serde_json::to_string(&result).unwrap();
        for forbidden in [
            "freshkube://",
            "ViewState",
            "node_tab",
            "\"page\"",
            "LinkPage",
            "\"tab\"",
        ] {
            assert!(!all.contains(forbidden), "{forbidden} in {all}");
        }
    }
}

#[tokio::test]
async fn a_session_has_its_own_ids_and_budget() {
    let (a, mut ea) = start(&granted()).await;
    let (b, mut eb) = start(&granted()).await;
    assert_ne!(a.session(), b.session());
    let _ = SessionId::new();
    for endpoint in [&a, &b] {
        let client = client(endpoint).await;
        client
            .call_tool(call("list_objects", json!({"kind": "pods"})))
            .await
            .unwrap();
    }
    assert_eq!(ea.recv().await.unwrap().read.to_string(), "r1");
    assert_eq!(eb.recv().await.unwrap().read.to_string(), "r1");
}
