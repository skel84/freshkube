//! A read on a cluster: `get` and `list` requests and the `log` subresource,
//! with the person's own access. Every request here is built by `Request::get`
//! (or the kube client's `get_metadata`), so nothing in this module can write.

use http::Request as HttpRequest;
use kube::Client;
use serde_json::Value;

use super::answer::{Answer, Item};
use super::call::{Events, Get, List, Logs, Request};
use super::refusal::Refusal;
use super::shape::{
    events_answer, get_answer, list_answer, logs_answer, missing, needs_namespace, scope,
};
use crate::cluster_source::ClusterAccess;
use crate::constants::agent::{MAX_EVENT_PAGES, MAX_ITEMS, MAX_LOG_BYTES};
use crate::failure::{Failure, FailureKind};
use crate::resources::{
    JSON, ResourceKind, TABLE_ACCEPT, Table, get_metadata, json_get, list_metadata_page,
    query_value, resolve_kind_name, row_tone,
};

pub(super) async fn run(
    cluster: &ClusterAccess,
    entry: &str,
    request: &Request,
) -> Result<Answer, Refusal> {
    // The gate has been passed; this is the first place a client is built.
    let client = cluster.client().await.map_err(|failure| {
        cluster.forget();
        Refusal::from_failure(&failure)
    })?;
    let answer = match request {
        Request::List(list) => list_objects(&client, entry, list).await,
        Request::Get(get) => get_object(&client, entry, get).await,
        Request::Logs(logs) => pod_logs(&client, entry, logs).await,
        Request::Events(events) => list_events(&client, entry, events).await,
    };
    if let Err(refusal) = &answer
        && refusal.drops_client()
    {
        cluster.forget();
    }
    answer
}

async fn resolve(client: &Client, asked: &str) -> Result<ResourceKind, Refusal> {
    match resolve_kind_name(client, asked).await {
        Ok(Some(kind)) => Ok(kind),
        Ok(None) => Err(Refusal::not_served()),
        Err(failure) => Err(Refusal::from_kind_failure(&failure)),
    }
}

/// A failed read of a kind: a 404 there means the API is not served.
fn kind_failure(failure: &Failure) -> Refusal {
    Refusal::from_kind_failure(failure)
}

async fn list_objects(client: &Client, entry: &str, request: &List) -> Result<Answer, Refusal> {
    let kind = resolve(client, request.kind.as_str()).await?;
    let namespace = scope(&kind, request.namespace.as_ref().map(|n| n.as_str()));
    let limit = request.limit.min(MAX_ITEMS);
    let (items, capped) = if kind.is_secret() {
        // Names and metadata only: the server is asked for no values.
        let page = list_metadata_page(client, &kind, namespace.as_deref(), limit)
            .await
            .map_err(|failure| kind_failure(&failure))?;
        let items = page
            .objects
            .into_iter()
            .map(|meta| Item {
                namespace: meta.namespace,
                name: meta.name.unwrap_or_default(),
                uid: meta.uid.unwrap_or_default(),
                status: String::new(),
                created: meta.creation_timestamp.map(|time| time.0.to_rfc3339()),
            })
            .collect();
        (items, page.capped)
    } else {
        let path = format!(
            "{}?includeObject=Metadata&limit={limit}",
            kind.collection_path(namespace.as_deref())
        );
        let table: Table = client
            .request(json_get(path, TABLE_ACCEPT).map_err(|failure| kind_failure(&failure))?)
            .await
            .map_err(|error| kind_failure(&Failure::from_kube(error)))?;
        let columns: Vec<String> = table
            .column_definitions
            .iter()
            .map(|column| column.name.clone())
            .collect();
        let items = table
            .rows
            .iter()
            .filter_map(|row| {
                let metadata = row.metadata()?;
                let cells: Vec<String> = row
                    .cells
                    .iter()
                    .map(|cell| match cell {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    })
                    .collect();
                Some(Item {
                    namespace: metadata.namespace.clone(),
                    name: metadata.name.clone(),
                    uid: metadata.uid.clone(),
                    status: row_tone(&columns, &cells)
                        .map(|(_, status)| status)
                        .unwrap_or_default(),
                    created: metadata.creation_timestamp.clone(),
                })
            })
            .collect();
        (items, !table.metadata.continue_token.is_empty())
    };
    list_answer(entry, &kind, request, items, capped)
}

async fn get_object(client: &Client, entry: &str, request: &Get) -> Result<Answer, Refusal> {
    let kind = resolve(client, request.kind.as_str()).await?;
    let namespace = scope(&kind, request.namespace.as_ref().map(|n| n.as_str()));
    needs_namespace(&kind, &namespace)?;
    let name = request.name.as_str();
    let object = if kind.is_secret() {
        // The metadata read: the server answers a PartialObjectMetadata, so a
        // value or a key name is never on the wire.
        let meta = get_metadata(client, &kind, namespace.as_deref(), name)
            .await
            .map_err(|failure| Refusal::from_failure(&failure))?;
        serde_json::json!({
            "apiVersion": kind.api_version(),
            "kind": kind.kind,
            "metadata": serde_json::to_value(meta).unwrap_or(Value::Null),
        })
    } else {
        let path = kind.object_path(namespace.as_deref(), name);
        client
            .request::<Value>(
                json_get(path, JSON).map_err(|failure| Refusal::from_failure(&failure))?,
            )
            .await
            .map_err(|error| Refusal::from_failure(&Failure::from_kube(error)))?
    };
    if object.is_null() {
        return Err(missing());
    }
    get_answer(entry, &kind, namespace, name, object)
}

async fn pod_logs(client: &Client, entry: &str, request: &Logs) -> Result<Answer, Refusal> {
    // The `log` subresource, with every bound asked of the server.
    let mut path = format!(
        "/api/v1/namespaces/{}/pods/{}/log?tailLines={}&limitBytes={MAX_LOG_BYTES}",
        request.namespace.as_str(),
        request.pod.as_str(),
        request.tail_lines,
    );
    if let Some(since) = request.since_seconds {
        path.push_str(&format!("&sinceSeconds={since}"));
    }
    if let Some(container) = &request.container {
        path.push_str(&format!("&container={}", container.as_str()));
    }
    if request.previous {
        path.push_str("&previous=true");
    }
    let http: HttpRequest<Vec<u8>> = HttpRequest::get(path)
        .header(http::header::ACCEPT, "text/plain")
        .body(Vec::new())
        .map_err(|error| {
            Refusal::from_failure(&Failure::new(FailureKind::Other, error.to_string()))
        })?;
    let text = client
        .request_text(http)
        .await
        .map_err(|error| Refusal::from_failure(&Failure::from_kube(error)))?;
    logs_answer(entry, request, &text, text.len() >= MAX_LOG_BYTES)
}

async fn list_events(client: &Client, entry: &str, request: &Events) -> Result<Answer, Refusal> {
    let kind = match &request.kind {
        Some(asked) => Some(resolve(client, asked.as_str()).await?),
        None => None,
    };
    let base = match &request.namespace {
        Some(namespace) => format!("/api/v1/namespaces/{}/events", namespace.as_str()),
        None => "/api/v1/events".to_owned(),
    };
    let mut selectors = Vec::new();
    if let Some(kind) = &kind {
        selectors.push(format!("involvedObject.kind={}", kind.kind));
    }
    if let Some(name) = &request.name {
        selectors.push(format!("involvedObject.name={}", name.as_str()));
    }
    let selector = if selectors.is_empty() {
        String::new()
    } else {
        format!("&fieldSelector={}", query_value(&selectors.join(",")))
    };
    // The API lists events in no order of time, so the newest are found by
    // reading the pages, up to a bound, and sorting here.
    let mut items: Vec<Value> = Vec::new();
    let mut next = String::new();
    let mut more = false;
    for page in 0..MAX_EVENT_PAGES {
        let path = format!("{base}?limit={MAX_ITEMS}{selector}{next}");
        let list: Value = client
            .request(json_get(path, JSON).map_err(|failure| Refusal::from_failure(&failure))?)
            .await
            .map_err(|error| Refusal::from_failure(&Failure::from_kube(error)))?;
        items.extend(
            list.get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .cloned(),
        );
        let token = list
            .pointer("/metadata/continue")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if token.is_empty() {
            break;
        }
        more = page + 1 == MAX_EVENT_PAGES;
        next = format!("&continue={}", query_value(token));
    }
    let looked_at = more.then_some(items.len());
    events_answer(
        entry,
        kind.as_ref(),
        request,
        &items,
        looked_at,
        chrono::Utc::now(),
    )
}
