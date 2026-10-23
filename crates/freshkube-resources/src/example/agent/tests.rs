use freshkube_core::agent_reads::{
    AgentAccess, AgentTarget, Body, Call, Source, read as agent_read,
};
use serde_json::json;

use super::agent_objects;
use crate::example::{self, read};
use crate::live;

fn target() -> (AgentAccess, AgentTarget) {
    let access = AgentAccess::new();
    access.allow("prod-fra", "prod-fra");
    let target = AgentTarget {
        entry: "prod-fra".into(),
        context: "prod-fra".into(),
        source: Source::Example(agent_objects("prod-fra")),
    };
    (access, target)
}

async fn answer(tool: &str, arguments: serde_json::Value) -> Body {
    let (access, target) = target();
    let call = Call::parse(tool, &arguments).unwrap();
    agent_read(&access, &target, &call.request)
        .await
        .unwrap_or_else(|refusal| panic!("{tool}: {refusal:?}"))
        .body
}

/// A running pod of the example cluster and its namespace.
fn pod() -> (String, String) {
    let (_, rows) = read("prod-fra", "pods", None, live::now()).unwrap();
    let row = rows
        .into_iter()
        .find(|row| row.cells[2] == "Running")
        .expect("a running pod");
    (row.identity.namespace, row.identity.name)
}

#[tokio::test]
async fn the_four_reads_answer_from_the_resources_example_objects() {
    let (namespace, name) = pod();

    let Body::List { items, .. } = answer(
        "list_objects",
        json!({"kind": "pods", "namespace": namespace}),
    )
    .await
    else {
        panic!("a list");
    };
    assert!(items.iter().any(|item| item.name == name));

    let Body::Get { object, uid, .. } = answer(
        "get_object",
        json!({"kind": "pods", "namespace": namespace, "name": name}),
    )
    .await
    else {
        panic!("an object");
    };
    assert_eq!(object["metadata"]["name"], name.as_str());
    assert!(object["metadata"].get("managedFields").is_none());
    assert!(!uid.is_empty());

    let Body::Logs { lines, .. } =
        answer("pod_logs", json!({"namespace": namespace, "pod": name})).await
    else {
        panic!("logs");
    };
    assert!(!lines.is_empty(), "the pod's default container has lines");

    let Body::Events { events, .. } = answer(
        "list_events",
        json!({"namespace": namespace, "since_seconds": 86400}),
    )
    .await
    else {
        panic!("events");
    };
    assert!(
        events.iter().any(|event| event.object.ends_with(&name)),
        "{events:?}"
    );
}

#[tokio::test]
async fn a_secret_is_listed_and_read_as_metadata_only() {
    let (_, rows) = read("prod-fra", "secrets", None, live::now()).unwrap();
    let identity = rows[0].identity.clone();
    let Body::Get { object, .. } = answer(
        "get_object",
        json!({"kind": "secrets", "namespace": identity.namespace, "name": identity.name}),
    )
    .await
    else {
        panic!("an object");
    };
    assert!(object.get("data").is_none(), "{object}");
    let _ = example::connection("prod-fra");
}
