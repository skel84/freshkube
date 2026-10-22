//! A target on example objects and a fake agent's client, shared by the
//! endpoint tests.
#![allow(dead_code)]

use std::sync::Arc;

use freshkube_agent::{Endpoint, ReadEvent, SessionId, Targets};
use freshkube_core::agent_reads::{
    AgentAccess, AgentTarget, EntryId, ExampleObjects, Source, example_objects,
};
use freshkube_core::resources::ResourceKind;
use rmcp::ServiceExt;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

pub mod capture;

pub const ENTRY: &str = "cluster-a";
pub const OTHER: &str = "cluster-b";
pub const CONTEXT: &str = "ctx-a";
pub const POD_UID: &str = "0b1c3f2a-1111-4222-8333-000000000001";

pub struct Objects;

fn pod(name: &str) -> Value {
    json!({"apiVersion":"v1","kind":"Pod",
        "metadata":{"name":name,"namespace":"shop","uid":POD_UID,
            "managedFields":[{"manager":"kubectl"}]},
        "spec":{"containers":[{"name":"app"}]},"status":{"phase":"Running"}})
}

impl ExampleObjects for Objects {
    fn kinds(&self) -> Vec<ResourceKind> {
        Vec::new()
    }

    fn objects(&self, kind: &ResourceKind, _: Option<&str>) -> Vec<Value> {
        if kind.plural == "pods" {
            vec![pod("api-0"), pod("api-1")]
        } else {
            Vec::new()
        }
    }

    fn log_lines(&self, _: &str, pod: &str, _: Option<&str>, _: bool) -> Option<Vec<String>> {
        (pod == "api-0").then(|| vec!["starting".into(), "listening".into()])
    }

    fn events(&self, _: Option<&str>) -> Vec<Value> {
        vec![
            json!({"metadata":{"name":"e1","namespace":"shop"},"type":"Warning",
            "reason":"BackOff","message":"Back-off restarting failed container","count":3,
            "lastTimestamp": chrono::Utc::now().to_rfc3339(),
            "involvedObject":{"kind":"Pod","name":"api-0","namespace":"shop","uid":POD_UID}}),
        ]
    }
}

/// Resolves the session's entry and `cluster-b`; anything else is unknown.
pub struct Example;

impl Targets for Example {
    fn resolve(&self, entry: Option<&EntryId>) -> Option<AgentTarget> {
        let entry = entry.map_or(ENTRY, |id| id.as_str());
        matches!(entry, ENTRY | OTHER).then(|| AgentTarget {
            entry: entry.to_owned(),
            context: format!("ctx-{}", &entry[entry.len() - 1..]),
            source: Source::Example(example_objects(Objects)),
        })
    }
}

pub fn granted() -> AgentAccess {
    let access = AgentAccess::new();
    access.allow(ENTRY, CONTEXT);
    access
}

pub async fn start(access: &AgentAccess) -> (Endpoint, UnboundedReceiver<ReadEvent>) {
    let (events, received) = unbounded_channel();
    let endpoint = Endpoint::start(SessionId::new(), ENTRY, access, Arc::new(Example), events)
        .await
        .expect("the endpoint starts");
    (endpoint, received)
}

pub async fn client(endpoint: &Endpoint) -> RunningService<RoleClient, ()> {
    ().serve(StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(endpoint.url())
            .auth_header(endpoint.token().reveal().to_owned()),
    ))
    .await
    .expect("the client connects")
}
