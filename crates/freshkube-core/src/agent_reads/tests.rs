use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::Utc;
use futures::future::BoxFuture;
use http::{Method, Request as HttpRequest, Response};
use kube::client::Body as HttpBody;
use serde_json::{Value, json};

use super::*;
use crate::cluster_source::{ClusterAccess, KubeClientSource};
use crate::constants::agent::MAX_READS_PER_SESSION;
use crate::failure::Failure;
use crate::fake_api::{self, Recorded, Recorder};
use crate::resources::ResourceKind;

const ENTRY: &str = "cluster-a";
const CONTEXT: &str = "ctx-a";

/// A shared client that counts the times it was asked for and dropped.
struct Source(kube::Client, AtomicUsize, AtomicUsize);

impl KubeClientSource for Source {
    fn client(&self) -> BoxFuture<'_, Result<kube::Client, Failure>> {
        self.1.fetch_add(1, Ordering::SeqCst);
        let client = self.0.clone();
        Box::pin(async move { Ok(client) })
    }

    fn forget(&self) {
        self.2.fetch_add(1, Ordering::SeqCst);
    }
}

fn target_on(client: kube::Client) -> (AgentTarget, Arc<Source>) {
    let source = Arc::new(Source(client, AtomicUsize::new(0), AtomicUsize::new(0)));
    let target = AgentTarget {
        entry: ENTRY.into(),
        context: CONTEXT.into(),
        source: super::Source::Live(ClusterAccess::Live(source.clone())),
    };
    (target, source)
}

fn granted() -> AgentAccess {
    let access = AgentAccess::new();
    access.allow(ENTRY, CONTEXT);
    access
}

fn request(tool: &str, arguments: Value) -> Request {
    Call::parse(tool, &arguments).expect("a valid call").request
}

fn live(
    route: impl Fn(&Recorded) -> Option<(u16, String)> + Send + Sync + 'static,
) -> (AgentTarget, Recorder, Arc<Source>) {
    let (client, recorder) = fake_api::client(route);
    let (target, source) = target_on(client);
    (target, recorder, source)
}

fn ok(body: Value) -> Option<(u16, String)> {
    Some(fake_api::json(200, body))
}

fn status(code: u16, message: &str) -> Option<(u16, String)> {
    Some(fake_api::json(
        code,
        json!({"kind":"Status","apiVersion":"v1","status":"Failure","code":code,
               "reason":"Failure","message":message}),
    ))
}

fn wire(answer: &Answer) -> String {
    answer.body.wire("r1").to_string()
}

fn pod_table(names: &[&str]) -> Value {
    json!({"kind":"Table","apiVersion":"meta.k8s.io/v1","metadata":{},
        "columnDefinitions":[{"name":"Name","type":"string"},{"name":"Status","type":"string"}],
        "rows": names.iter().map(|name| json!({
            "cells":[name,"Running"],
            "object":{"kind":"PartialObjectMetadata","metadata":{
                "name":name,"namespace":"shop","uid":format!("uid-{name}"),
                "creationTimestamp":"2026-10-22T09:00:00Z"}}})).collect::<Vec<_>>()})
}

fn pod(name: &str) -> Value {
    json!({"apiVersion":"v1","kind":"Pod",
        "metadata":{"name":name,"namespace":"shop","uid":"0b1c3f2a-aaaa-bbbb-cccc-000000000001",
            "annotations":{"kubectl.kubernetes.io/last-applied-configuration":"{\"big\":true}","team":"payments"},
            "managedFields":[{"manager":"kubectl","fieldsV1":{"f:spec":{}}}]},
        "spec":{"containers":[{"name":"app"}]},"status":{"phase":"Running"}})
}

fn event(reason: &str, minutes_ago: i64) -> Value {
    let at = (Utc::now() - chrono::Duration::minutes(minutes_ago)).to_rfc3339();
    json!({"metadata":{"name":format!("e-{reason}"),"namespace":"shop"},
        "type":"Warning","reason":reason,"message":format!("{reason} happened"),
        "count":2,"lastTimestamp":at,
        "involvedObject":{"kind":"Pod","name":"api-0","namespace":"shop","uid":"u-1"}})
}

#[tokio::test]
async fn a_cluster_without_agent_access_refuses_before_any_request() {
    let (target, recorder, source) = live(|_| ok(json!({})));
    let refusal = read(
        &AgentAccess::new(),
        &target,
        &request("list_objects", json!({"kind": "pods", "namespace": "shop"})),
    )
    .await
    .unwrap_err();
    assert_eq!(refusal.code, Code::NotEnabled);
    assert!(recorder.requests().is_empty());
    assert_eq!(source.1.load(Ordering::SeqCst), 0, "no client was built");
}

#[tokio::test]
async fn turning_access_off_ends_a_read_in_flight() {
    let held = fake_api::held();
    let (client, recorder) = (held.client.clone(), held.recorder.clone());
    let (target, _) = target_on(client);
    let access = granted();
    let asking = request(
        "get_object",
        json!({"kind": "pods", "namespace": "shop", "name": "api-0"}),
    );
    let read = tokio::spawn({
        let (access, target) = (access.clone(), target.clone());
        async move { read(&access, &target, &asking).await }
    });
    for _ in 0..500 {
        if !recorder.requests().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        recorder.requests().len(),
        1,
        "the read reached the held server"
    );
    access.revoke(ENTRY);
    let done = tokio::time::timeout(Duration::from_secs(5), read)
        .await
        .expect("the read ended within the bound")
        .unwrap();
    assert_eq!(done.unwrap_err().code, Code::NotEnabled);
    assert!(!access.is_on(ENTRY, CONTEXT));
}

#[tokio::test]
async fn a_grant_for_another_context_reads_as_off() {
    let (target, recorder, _) = live(|_| ok(json!({})));
    let access = AgentAccess::new();
    access.allow(ENTRY, "ctx-old");
    let refusal = read(
        &access,
        &target,
        &request("list_objects", json!({"kind": "pods"})),
    )
    .await
    .unwrap_err();
    assert_eq!(refusal.code, Code::NotEnabled);
    assert!(recorder.requests().is_empty());
    assert!(!access.is_on(ENTRY, CONTEXT));
}

fn everything(request: &Recorded) -> Option<(u16, String)> {
    let path = request.path.as_str();
    if path.ends_with("/log") {
        return Some((200, "one\ntwo\n".into()));
    }
    if path.ends_with("/events") {
        return ok(json!({"items":[event("BackOff", 5)],"metadata":{}}));
    }
    if path.ends_with("/secrets") && request.accept.contains("PartialObjectMetadataList") {
        return ok(json!({"kind":"PartialObjectMetadataList","metadata":{},
            "items":[{"metadata":{"name":"db","namespace":"shop","uid":"u-s"}}]}));
    }
    if path.ends_with("/secrets/db") {
        return ok(
            json!({"kind":"PartialObjectMetadata","apiVersion":"meta.k8s.io/v1",
            "metadata":{"name":"db","namespace":"shop","uid":"u-s"},
            "data":{"password":"aHVudGVyMg=="},"stringData":{"user":"admin"}}),
        );
    }
    if path.ends_with("/pods") {
        return ok(pod_table(&["api-0"]));
    }
    if path.ends_with("/pods/api-0") {
        return ok(pod("api-0"));
    }
    None
}

fn all_tools() -> Vec<Request> {
    vec![
        request("list_objects", json!({"kind": "pods", "namespace": "shop"})),
        request(
            "get_object",
            json!({"kind": "pods", "namespace": "shop", "name": "api-0"}),
        ),
        request(
            "pod_logs",
            json!({"namespace": "shop", "pod": "api-0", "tail_lines": 50, "since_seconds": 600}),
        ),
        request(
            "list_events",
            json!({"namespace": "shop", "kind": "Pod", "name": "api-0"}),
        ),
        request(
            "list_objects",
            json!({"kind": "secrets", "namespace": "shop"}),
        ),
        request(
            "get_object",
            json!({"kind": "secret", "namespace": "shop", "name": "db"}),
        ),
    ]
}

#[tokio::test]
async fn every_agent_read_is_a_get() {
    let (target, recorder, _) = live(everything);
    let access = granted();
    for asking in all_tools() {
        read(&access, &target, &asking).await.expect("answered");
    }
    let seen = recorder.requests();
    assert_eq!(seen.len(), 6);
    recorder.assert_all("agent reads only get", |r| r.method == Method::GET);
    recorder.assert_all("logs use only the log subresource", |r| {
        !r.path.contains("/pods/api-0/") || r.path.ends_with("/log")
    });
    assert!(
        !seen
            .iter()
            .any(|r| r.path.contains("/exec") || r.path.contains("/attach"))
    );
}

#[tokio::test]
async fn secrets_are_read_metadata_only() {
    let (target, recorder, _) = live(everything);
    let access = granted();
    let list = read(
        &access,
        &target,
        &request(
            "list_objects",
            json!({"kind": "secrets", "namespace": "shop"}),
        ),
    )
    .await
    .unwrap();
    let get = read(
        &access,
        &target,
        &request(
            "get_object",
            json!({"kind": "secrets", "namespace": "shop", "name": "db"}),
        ),
    )
    .await
    .unwrap();
    for seen in recorder.requests() {
        assert!(
            seen.accept.contains("PartialObjectMetadata"),
            "{} asked {}",
            seen.line(),
            seen.accept
        );
    }
    for answer in [&list, &get] {
        let text = wire(answer);
        for forbidden in [
            "\"data\"",
            "\"stringData\"",
            "password",
            "aHVudGVyMg",
            "admin",
            "\"user\"",
        ] {
            assert!(!text.contains(forbidden), "{forbidden} in {text}");
        }
    }
    assert!(wire(&get).contains("\"name\":\"db\""));
}

#[tokio::test]
async fn managed_fields_and_last_applied_never_reach_the_answer() {
    let (target, _, _) = live(everything);
    let answer = read(
        &granted(),
        &target,
        &request(
            "get_object",
            json!({"kind": "pods", "namespace": "shop", "name": "api-0"}),
        ),
    )
    .await
    .unwrap();
    let text = wire(&answer);
    assert!(!text.contains("managedFields"));
    assert!(!text.contains("last-applied"));
    assert!(text.contains("payments"), "other annotations stay");
    assert_eq!(
        answer.place.uid.as_deref(),
        Some("0b1c3f2a-aaaa-bbbb-cccc-000000000001")
    );
}

#[tokio::test]
async fn log_reads_are_bounded() {
    let long = "x".repeat(5000);
    let text: String = (0..600).map(|n| format!("line {n} {long}\n")).collect();
    let (target, recorder, _) =
        live(move |r| r.path.ends_with("/log").then(|| (200, text.clone())));
    let answer = read(
        &granted(),
        &target,
        &request(
            "pod_logs",
            json!({"namespace": "shop", "pod": "api-0", "tail_lines": 500, "since_seconds": 86400}),
        ),
    )
    .await
    .unwrap();
    let query = recorder.requests()[0].query.clone();
    assert!(query.contains("tailLines=500"), "{query}");
    assert!(query.contains("sinceSeconds=86400"), "{query}");
    assert!(query.contains("limitBytes=65536"), "{query}");
    let Body::Logs { lines, cut, .. } = &answer.body else {
        panic!("not logs")
    };
    assert!(cut.is_some(), "the answer says it is cut");
    assert!(lines.len() <= 500);
    assert!(lines.iter().all(|line| line.len() <= 2048));
    assert!(lines.iter().map(String::len).sum::<usize>() <= 64 * 1024);
    assert!(
        lines.last().unwrap().starts_with("line 599"),
        "the newest lines are kept"
    );
    assert_eq!(answer.place.tab, Tab::Logs);
}

#[tokio::test]
async fn a_long_line_alone_is_cut_at_two_kib() {
    let (target, _, _) = live(|r| r.path.ends_with("/log").then(|| (200, "y".repeat(3000))));
    let answer = read(
        &granted(),
        &target,
        &request("pod_logs", json!({"namespace": "shop", "pod": "api-0"})),
    )
    .await
    .unwrap();
    let Body::Logs { lines, cut, .. } = &answer.body else {
        panic!("not logs")
    };
    assert_eq!(lines[0].len(), 2048);
    assert_eq!(*cut, Some(Cut::bytes(2048)));
}

#[tokio::test]
async fn lists_and_events_are_cut_at_their_limit() {
    let (target, recorder, _) = live(|r| {
        if r.path.ends_with("/events") {
            let events: Vec<Value> = (0..5).map(|n| event(&format!("R{n}"), n + 1)).collect();
            ok(json!({"items": events, "metadata": {}}))
        } else {
            ok(pod_table(&["a", "b", "c"]))
        }
    });
    let access = granted();
    let list = read(
        &access,
        &target,
        &request(
            "list_objects",
            json!({"kind": "pods", "namespace": "shop", "limit": 2}),
        ),
    )
    .await
    .unwrap();
    let Body::List { items, cut, .. } = &list.body else {
        panic!("not a list")
    };
    assert_eq!(items.len(), 2);
    assert_eq!(*cut, Some(Cut::items(2)));
    assert!(recorder.requests()[0].query.contains("limit=2"));

    let events = read(
        &access,
        &target,
        &request("list_events", json!({"namespace": "shop", "limit": 2})),
    )
    .await
    .unwrap();
    let Body::Events { events, cut, .. } = &events.body else {
        panic!("not events")
    };
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].reason, "R0", "the newest first");
    assert_eq!(*cut, Some(Cut::items(2)));
}

#[tokio::test]
async fn an_object_over_the_limit_is_too_large() {
    let (target, _, _) = live(|_| {
        let mut big = pod("api-0");
        big["spec"]["blob"] = json!("z".repeat(70 * 1024));
        ok(big)
    });
    let refusal = read(
        &granted(),
        &target,
        &request(
            "get_object",
            json!({"kind": "pods", "namespace": "shop", "name": "api-0"}),
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(refusal.code, Code::TooLarge);
}

#[test]
fn call_parse_refuses_what_it_cannot_bound() {
    let name_254 = "a".repeat(254);
    let cases: Vec<(&str, Value, &str)> = vec![
        ("restart_pod", json!({}), "tool"),
        (
            "list_objects",
            json!({"kind": "pods", "selector": "app=api"}),
            "selector",
        ),
        (
            "pod_logs",
            json!({"namespace": "shop", "pod": "api-0", "tail_lines": 501}),
            "tail_lines",
        ),
        (
            "pod_logs",
            json!({"namespace": "shop", "pod": "api-0", "since_seconds": 86401}),
            "since_seconds",
        ),
        (
            "pod_logs",
            json!({"namespace": "shop", "pod": "api-0", "tail_lines": 0}),
            "tail_lines",
        ),
        (
            "get_object",
            json!({"kind": "pods", "namespace": "shop", "name": name_254}),
            "name",
        ),
        (
            "get_object",
            json!({"kind": "pods", "namespace": "Shop", "name": "x"}),
            "namespace",
        ),
        ("list_objects", json!({"kind": 7}), "kind"),
        (
            "pod_logs",
            json!({"namespace": "shop", "pod": "api-0", "previous": "yes"}),
            "previous",
        ),
        (
            "list_objects",
            json!({"kind": "pods", "limit": "5"}),
            "limit",
        ),
        (
            "list_objects",
            json!({"kind": "pods", "limit": 201}),
            "limit",
        ),
        (
            "list_objects",
            json!({"kind": "pods", "cluster": "a b"}),
            "cluster",
        ),
        (
            "list_events",
            json!({"since_seconds": 1.5}),
            "since_seconds",
        ),
        ("list_objects", json!([1]), "arguments"),
    ];
    for (tool, arguments, field) in cases {
        let refusal = Call::parse(tool, &arguments).expect_err(&format!("{tool} {arguments}"));
        assert_eq!(refusal.code, Code::Invalid, "{tool} {arguments}");
        assert!(
            refusal.message().contains(field),
            "{} names {field}",
            refusal.message()
        );
        let value_text = arguments.to_string();
        for word in ["restart_pod", "app=api", "Shop", "a b", "yes"] {
            if value_text.contains(word) {
                assert!(
                    !refusal.message().contains(word),
                    "{} repeats {word}",
                    refusal.message()
                );
            }
        }
        assert!(
            !refusal.message().contains(&"a".repeat(20)),
            "no long value"
        );
    }
    let twice = Call::parse_pairs(
        "list_objects",
        [
            ("kind".to_owned(), json!("pods")),
            ("kind".to_owned(), json!("nodes")),
        ],
    )
    .unwrap_err();
    assert_eq!(twice.code, Code::Invalid);
    assert!(twice.message().contains("kind"));
    let odd = Call::parse(
        "list_objects",
        &json!({"kind": "pods", "tok\u{1b}en=hunter2": 1}),
    )
    .unwrap_err();
    assert!(!odd.message().contains("hunter2"));
}

#[test]
fn call_parse_fills_the_defaults_and_keeps_the_cluster() {
    let call = Call::parse(
        "pod_logs",
        &json!({"cluster": "cluster-b", "namespace": "shop", "pod": "api-0"}),
    )
    .unwrap();
    assert_eq!(
        call.cluster.as_ref().map(EntryId::as_str),
        Some("cluster-b")
    );
    let Request::Logs(logs) = call.request else {
        panic!()
    };
    assert_eq!(
        (logs.tail_lines, logs.since_seconds, logs.previous),
        (200, None, false)
    );
    let Request::Events(events) = Call::parse("list_events", &Value::Null).unwrap().request else {
        panic!()
    };
    assert_eq!((events.since_seconds, events.limit), (3600, 100));
    assert_eq!(
        TOOLS,
        ["list_objects", "get_object", "pod_logs", "list_events"]
    );
    assert_eq!(SERVER_NAME, "freshkube");
}

#[tokio::test]
async fn failures_map_to_their_codes() {
    let asking = request(
        "get_object",
        json!({"kind": "pods", "namespace": "shop", "name": "api-0"}),
    );
    for (code, expected) in [
        (401, Code::Refused),
        (403, Code::Refused),
        (404, Code::NotFound),
        (504, Code::TimedOut),
        (500, Code::Failed),
    ] {
        let (target, _, _) = live(move |_| status(code, "no"));
        let refusal = read(&granted(), &target, &asking).await.unwrap_err();
        assert_eq!(refusal.code, expected, "HTTP {code}");
    }

    let (target, _, _) = live(|r| {
        (r.path == "/apis")
            .then(|| fake_api::json(200, r#"{"groups":[]}"#))
            .or_else(|| ok(json!({})))
    });
    let unresolved = read(
        &granted(),
        &target,
        &request("list_objects", json!({"kind": "widgets"})),
    )
    .await
    .unwrap_err();
    assert_eq!(unresolved.code, Code::NotServed);

    let down = tower::service_fn(|_: HttpRequest<HttpBody>| async {
        Err::<Response<HttpBody>, _>(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "connection refused",
        ))
    });
    let (target, _) = target_on(kube::Client::new(down, "default"));
    let unreachable = read(&granted(), &target, &asking).await.unwrap_err();
    assert_eq!(unreachable.code, Code::Unreachable);

    let (target, _, _) = live(|_| {
        status(
            500,
            "upstream said Bearer sk-live-0123456789abcdef0123456789abcdef and token=hunter2",
        )
    });
    let scrubbed = read(&granted(), &target, &asking).await.unwrap_err();
    assert!(
        !scrubbed.message().contains("sk-live"),
        "{}",
        scrubbed.message()
    );
    assert!(
        !scrubbed.message().contains("hunter2"),
        "{}",
        scrubbed.message()
    );
}

#[tokio::test(start_paused = true)]
async fn a_read_past_its_deadline_times_out() {
    let client = fake_api::held().client;
    let (target, _) = target_on(client);
    let refusal = read(
        &granted(),
        &target,
        &request(
            "get_object",
            json!({"kind": "pods", "namespace": "shop", "name": "api-0"}),
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(refusal.code, Code::TimedOut);
}

struct Example;

impl ExampleObjects for Example {
    fn kinds(&self) -> Vec<ResourceKind> {
        vec![ResourceKind::new(
            "cert-manager.io",
            "v1",
            "Certificate",
            "certificates",
            true,
        )]
    }

    fn objects(&self, kind: &ResourceKind, _namespace: Option<&str>) -> Vec<Value> {
        match kind.plural.as_str() {
            "pods" => (0..250)
                .map(|n| {
                    let mut p = pod(&format!("api-{n}"));
                    p["metadata"]["uid"] = json!(format!("uid-{n}"));
                    p
                })
                .collect(),
            "secrets" => vec![json!({"apiVersion":"v1","kind":"Secret",
                "metadata":{"name":"db","namespace":"shop","uid":"u-s","managedFields":[{}]},
                "data":{"password":"aHVudGVyMg=="},"stringData":{"user":"admin"}})],
            "certificates" => vec![
                json!({"apiVersion":"cert-manager.io/v1","kind":"Certificate",
                "metadata":{"name":"web","namespace":"shop","uid":"u-c"},
                "status":{"conditions":[{"type":"Ready","status":"True"}]}}),
            ],
            "deployments" => vec![json!({"apiVersion":"apps/v1","kind":"Deployment",
                "metadata":{"name":"api","namespace":"shop","uid":"u-d"},
                "spec":{"replicas":3},"status":{"readyReplicas":2}})],
            _ => vec![],
        }
    }

    fn log_lines(&self, _: &str, pod: &str, _: Option<&str>, _: bool) -> Option<Vec<String>> {
        (pod == "api-0").then(|| {
            (0..900)
                .map(|n| format!("line {n} {}", "w".repeat(3000)))
                .collect()
        })
    }

    fn events(&self, _: Option<&str>) -> Vec<Value> {
        (0..300)
            .map(|n| event(&format!("R{n}"), 1 + n % 50))
            .collect()
    }
}

fn example_target() -> AgentTarget {
    AgentTarget {
        entry: ENTRY.into(),
        context: CONTEXT.into(),
        source: super::Source::Example(example_objects(Example)),
    }
}

#[tokio::test]
async fn example_objects_get_the_same_bounds_and_secret_rule() {
    let target = example_target();
    let access = granted();

    let list = read(
        &access,
        &target,
        &request(
            "list_objects",
            json!({"kind": "po", "namespace": "shop", "limit": 200}),
        ),
    )
    .await
    .unwrap();
    let Body::List { items, cut, .. } = &list.body else {
        panic!()
    };
    assert_eq!((items.len(), *cut), (200, Some(Cut::items(200))));
    assert_eq!(items[0].status, "Running");

    let secret = read(
        &access,
        &target,
        &request(
            "get_object",
            json!({"kind": "Secret", "namespace": "shop", "name": "db"}),
        ),
    )
    .await
    .unwrap();
    let text = wire(&secret);
    for forbidden in [
        "\"data\"",
        "\"stringData\"",
        "password",
        "aHVudGVyMg",
        "admin",
        "managedFields",
    ] {
        assert!(!text.contains(forbidden), "{forbidden} in {text}");
    }
    let secrets = read(
        &access,
        &target,
        &request(
            "list_objects",
            json!({"kind": "secrets", "namespace": "shop"}),
        ),
    )
    .await
    .unwrap();
    assert!(!wire(&secrets).contains("password"));

    let managed = read(
        &access,
        &target,
        &request(
            "get_object",
            json!({"kind": "pods", "namespace": "shop", "name": "api-3"}),
        ),
    )
    .await
    .unwrap();
    assert!(!wire(&managed).contains("managedFields"));
    assert_eq!(managed.place.uid.as_deref(), Some("uid-3"));

    let logs = read(
        &access,
        &target,
        &request(
            "pod_logs",
            json!({"namespace": "shop", "pod": "api-0", "tail_lines": 500}),
        ),
    )
    .await
    .unwrap();
    let Body::Logs { lines, cut, .. } = &logs.body else {
        panic!()
    };
    assert!(cut.is_some() && lines.len() <= 500 && lines.iter().all(|l| l.len() <= 2048));

    let events = read(
        &access,
        &target,
        &request("list_events", json!({"namespace": "shop"})),
    )
    .await
    .unwrap();
    let Body::Events { events, cut, .. } = &events.body else {
        panic!()
    };
    assert_eq!((events.len(), *cut), (100, Some(Cut::items(100))));

    let missing = read(
        &access,
        &target,
        &request(
            "get_object",
            json!({"kind": "pods", "namespace": "shop", "name": "nope"}),
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(missing.code, Code::NotFound);
}

#[tokio::test]
async fn kinds_resolve_by_what_kubectl_takes_and_places_name_the_key() {
    let target = example_target();
    let access = granted();
    for asked in [
        "deployments",
        "Deployment",
        "deployment.apps",
        "deploy",
        "deployments.apps",
    ] {
        let answer = read(
            &access,
            &target,
            &request("list_objects", json!({"kind": asked, "namespace": "shop"})),
        )
        .await
        .unwrap();
        assert_eq!(answer.place.kind, "deployments.apps", "{asked}");
        let Body::List { items, .. } = &answer.body else {
            panic!()
        };
        assert_eq!(items[0].status, "2/3 ready");
    }
    let custom = read(
        &access,
        &target,
        &request(
            "list_objects",
            json!({"kind": "certificate", "namespace": "shop"}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(custom.place.kind, "certificates.cert-manager.io");
    let unknown = read(
        &access,
        &target,
        &request("list_objects", json!({"kind": "widgets"})),
    )
    .await
    .unwrap_err();
    assert_eq!(unknown.code, Code::NotServed);
    let needs = read(
        &access,
        &target,
        &request("get_object", json!({"kind": "pods", "name": "api-0"})),
    )
    .await
    .unwrap_err();
    assert_eq!(needs.code, Code::Invalid);
}

#[tokio::test]
async fn the_five_hundred_and_first_read_is_limit_reached() {
    let target = example_target();
    let session = granted().for_session();
    let asking = request(
        "list_objects",
        json!({"kind": "deployments", "namespace": "shop"}),
    );
    for n in 0..MAX_READS_PER_SESSION {
        read(&session, &target, &asking)
            .await
            .unwrap_or_else(|e| panic!("read {n}: {e}"));
    }
    let refusal = read(&session, &target, &asking).await.unwrap_err();
    assert_eq!(refusal.code, Code::LimitReached);
    // Another session shares the grants and has its own reads.
    assert!(
        read(&session.clone().for_session(), &target, &asking)
            .await
            .is_ok()
    );
    // A read with no grant spends nothing.
    let off = AgentAccess::new().for_session();
    for _ in 0..3 {
        assert_eq!(
            read(&off, &target, &asking).await.unwrap_err().code,
            Code::NotEnabled
        );
    }
}

#[test]
fn every_failure_kind_has_a_code_and_codes_have_their_words() {
    use crate::failure::FailureKind::*;
    for kind in [
        Config,
        Unreachable,
        Timeout,
        Unauthorized,
        Forbidden,
        NotFound,
        Other,
    ] {
        let code = Refusal::from_failure(&Failure::new(kind, "x")).code;
        assert!(Code::ALL.contains(&code));
    }
    let words: Vec<_> = Code::ALL.iter().map(|code| code.as_str()).collect();
    assert_eq!(
        words,
        [
            "invalid",
            "unknown_cluster",
            "not_enabled",
            "refused",
            "not_found",
            "not_served",
            "too_large",
            "unreachable",
            "timed_out",
            "failed",
            "limit_reached"
        ]
    );
}

#[test]
fn the_wire_body_carries_its_read_id_and_the_design_shapes() {
    let body = Body::Events {
        cluster: ENTRY.into(),
        events: vec![],
        cut: None,
    };
    assert_eq!(
        body.wire("r7"),
        json!({"read": "r7", "cluster": ENTRY, "events": [], "cut": null})
    );
}

#[tokio::test]
async fn a_403_keeps_the_shared_client_and_a_401_or_a_dead_server_drops_it() {
    let asking = request(
        "get_object",
        json!({"kind": "pods", "namespace": "shop", "name": "api-0"}),
    );
    let (target, _, source) = live(|_| status(403, "forbidden"));
    for _ in 0..3 {
        let refusal = read(&granted(), &target, &asking).await.unwrap_err();
        assert_eq!(refusal.code, Code::Refused);
    }
    assert_eq!(
        source.2.load(Ordering::SeqCst),
        0,
        "a 403 leaves the client cached"
    );

    let (target, _, source) = live(|_| status(401, "unauthorized"));
    read(&granted(), &target, &asking).await.unwrap_err();
    assert_eq!(source.2.load(Ordering::SeqCst), 1, "a 401 drops it");

    let down = tower::service_fn(|_: HttpRequest<HttpBody>| async {
        Err::<Response<HttpBody>, _>(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "refused",
        ))
    });
    let (target, source) = target_on(kube::Client::new(down, "default"));
    read(&granted(), &target, &asking).await.unwrap_err();
    assert_eq!(
        source.2.load(Ordering::SeqCst),
        1,
        "an unreachable server drops it"
    );

    let (target, _, source) = live(|_| status(404, "not found"));
    read(&granted(), &target, &asking).await.unwrap_err();
    assert_eq!(source.2.load(Ordering::SeqCst), 0, "a 404 keeps it");
}

#[tokio::test]
async fn events_are_read_page_by_page_to_find_the_newest() {
    let (target, recorder, _) = live(|r| {
        let page: Value = if r.query.contains("continue=page-two") {
            json!({"items": [event("Newest", 1)], "metadata": {}})
        } else {
            json!({"items": [event("Older", 30), event("Oldest", 50)], "metadata": {"continue": "page-two"}})
        };
        ok(page)
    });
    let answer = read(
        &granted(),
        &target,
        &request("list_events", json!({"namespace": "shop", "limit": 2})),
    )
    .await
    .unwrap();
    let Body::Events { events, cut, .. } = &answer.body else {
        panic!()
    };
    assert_eq!(
        events.iter().map(|e| e.reason.as_str()).collect::<Vec<_>>(),
        ["Newest", "Older"]
    );
    assert_eq!(*cut, Some(Cut::items(2)), "a third event did not fit");
    assert_eq!(recorder.requests().len(), 2);
    assert!(recorder.requests()[1].query.contains("continue=page-two"));
}

#[tokio::test]
async fn events_the_pages_did_not_reach_make_the_answer_cut() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let (target, _, _) = live(move |_| {
        let n = counted.fetch_add(1, Ordering::SeqCst);
        ok(
            json!({"items": [event(&format!("R{n}"), 1 + n as i64)], "metadata": {"continue": "more"}}),
        )
    });
    let answer = read(
        &granted(),
        &target,
        &request("list_events", json!({"namespace": "shop"})),
    )
    .await
    .unwrap();
    let Body::Events { events, cut, .. } = &answer.body else {
        panic!()
    };
    assert_eq!(
        calls.load(Ordering::SeqCst),
        crate::constants::agent::MAX_EVENT_PAGES
    );
    assert_eq!(events.len(), crate::constants::agent::MAX_EVENT_PAGES);
    assert_eq!(
        *cut,
        Some(Cut::items(crate::constants::agent::MAX_EVENT_PAGES)),
        "incomplete is said"
    );
}

#[tokio::test]
async fn a_secrets_last_applied_copy_never_reaches_the_answer() {
    let copy = r#"{"apiVersion":"v1","data":{"password":"aHVudGVyMg=="},"kind":"Secret"}"#;
    let annotated = json!({"kind":"PartialObjectMetadata","apiVersion":"meta.k8s.io/v1",
        "metadata":{"name":"db","namespace":"shop","uid":"u-s",
            "annotations":{"kubectl.kubernetes.io/last-applied-configuration": copy, "team": "payments"},
            "managedFields":[{"manager":"kubectl"}]},
        "data":{"password":"aHVudGVyMg=="}});
    let (target, _, _) = live(move |_| ok(annotated.clone()));
    let asking = request(
        "get_object",
        json!({"kind": "secrets", "namespace": "shop", "name": "db"}),
    );
    let answer = read(&granted(), &target, &asking).await.unwrap();
    let text = wire(&answer);
    for forbidden in [
        "last-applied",
        "aHVudGVyMg",
        "password",
        "managedFields",
        "\"data\"",
    ] {
        assert!(!text.contains(forbidden), "{forbidden} in {text}");
    }
    assert!(text.contains("payments"), "other annotations stay");

    // Example data takes the same rule.
    struct Annotated;
    impl ExampleObjects for Annotated {
        fn objects(&self, _: &ResourceKind, _: Option<&str>) -> Vec<Value> {
            vec![
                json!({"apiVersion":"v1","kind":"Secret","data":{"password":"aHVudGVyMg=="},
                "metadata":{"name":"db","namespace":"shop","uid":"u-s",
                    "annotations":{"kubectl.kubernetes.io/last-applied-configuration": r#"{"data":{"password":"aHVudGVyMg=="}}"#}}}),
            ]
        }
        fn log_lines(&self, _: &str, _: &str, _: Option<&str>, _: bool) -> Option<Vec<String>> {
            None
        }
        fn events(&self, _: Option<&str>) -> Vec<Value> {
            Vec::new()
        }
    }
    let example = AgentTarget {
        entry: ENTRY.into(),
        context: CONTEXT.into(),
        source: super::Source::Example(example_objects(Annotated)),
    };
    let answer = read(&granted(), &example, &asking).await.unwrap();
    let text = wire(&answer);
    assert!(
        !text.contains("last-applied") && !text.contains("aHVudGVyMg"),
        "{text}"
    );
}

#[tokio::test]
async fn a_loose_kind_name_resolves_through_discovery_for_an_agent() {
    let (target, _, _) = live(|r| match r.path.as_str() {
        "/apis" => ok(
            json!({"groups":[{"name":"cert-manager.io","versions":[{"version":"v1"}],
            "preferredVersion":{"version":"v1"}}]}),
        ),
        "/apis/cert-manager.io/v1" => ok(
            json!({"resources":[{"name":"certificates","kind":"Certificate",
            "namespaced":true,"verbs":["list","watch"]}]}),
        ),
        p if p.ends_with("/certificates") => {
            ok(json!({"kind":"Table","metadata":{},"columnDefinitions":[],"rows":[]}))
        }
        _ => None,
    });
    let answer = read(
        &granted(),
        &target,
        &request(
            "list_objects",
            json!({"kind": "Certificate", "namespace": "shop"}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(answer.place.kind, "certificates.cert-manager.io");
}
