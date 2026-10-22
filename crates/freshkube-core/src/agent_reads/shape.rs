//! Turns what a server (or example data) returned into an answer, with the
//! bounds, the Secret rule and the place. Live and example reads both come
//! through here.

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

use super::answer::{
    Answer, Body, Cut, EventRow, Item, Place, Tab, check_object, event_message, fit, log_lines,
};
use super::call::{Events, List, Logs};
use super::refusal::{Code, Refusal};
use crate::constants::agent::MAX_ITEMS;
use crate::resources::{LAST_APPLIED, ResourceKind};

/// The namespace a read applies to: only a namespaced kind has one.
pub(super) fn scope(kind: &ResourceKind, namespace: Option<&str>) -> Option<String> {
    namespace.filter(|_| kind.namespaced).map(str::to_owned)
}

fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

/// Drops what is noise or a copy of the object: `managedFields` and the
/// last-applied annotation.
pub(super) fn clean(object: &mut Value) {
    let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) else {
        return;
    };
    metadata.remove("managedFields");
    if let Some(annotations) = metadata
        .get_mut("annotations")
        .and_then(Value::as_object_mut)
    {
        annotations.remove(LAST_APPLIED);
        if annotations.is_empty() {
            metadata.remove("annotations");
        }
    }
}

/// A Secret is only its identity: the API version, the kind and the metadata.
/// Its values and its key names are never kept.
pub(super) fn secret_metadata(kind: &ResourceKind, object: &Value) -> Value {
    let metadata = object.get("metadata").cloned().unwrap_or(Value::Null);
    let mut kept = serde_json::json!({
        "apiVersion": kind.api_version(),
        "kind": kind.kind,
        "metadata": metadata,
    });
    clean(&mut kept);
    kept
}

/// One line of status, for an object the API did not print a row for.
pub(super) fn status_line(object: &Value) -> String {
    let status = object.get("status");
    if let Some(phase) = text(status.and_then(|status| status.get("phase"))) {
        return phase;
    }
    let count = |value: Option<&Value>| value.and_then(Value::as_u64);
    if let Some(want) = count(object.get("spec").and_then(|spec| spec.get("replicas"))) {
        let ready = count(status.and_then(|status| status.get("readyReplicas"))).unwrap_or(0);
        return format!("{ready}/{want} ready");
    }
    status
        .and_then(|status| status.get("conditions"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|condition| {
            matches!(
                text(condition.get("type")).as_deref(),
                Some("Ready" | "Available" | "Established")
            )
        })
        .map(|condition| {
            let kind = text(condition.get("type")).unwrap_or_default();
            match text(condition.get("status")).as_deref() {
                Some("True") => kind,
                _ => format!("Not {kind}"),
            }
        })
        .unwrap_or_default()
}

/// A row of `list_objects` from a raw object.
pub(super) fn item(kind: &ResourceKind, object: &Value) -> Item {
    let metadata = object.get("metadata");
    let field = |key: &str| text(metadata.and_then(|metadata| metadata.get(key)));
    Item {
        namespace: field("namespace"),
        name: field("name").unwrap_or_default(),
        uid: field("uid").unwrap_or_default(),
        status: if kind.is_secret() {
            String::new()
        } else {
            status_line(object)
        },
        created: field("creationTimestamp"),
    }
}

pub(super) fn place_of_kind(kind: &ResourceKind, namespace: Option<String>) -> Place {
    Place {
        kind: kind.key(),
        namespace,
        name: None,
        uid: None,
        tab: Tab::Details,
    }
}

/// `list_objects`: at most the request's limit, `cut` when more were there.
pub(super) fn list_answer(
    cluster: &str,
    kind: &ResourceKind,
    request: &List,
    mut items: Vec<Item>,
    server_cut: bool,
) -> Result<Answer, Refusal> {
    let limit = request.limit.min(MAX_ITEMS);
    let cut = (server_cut || items.len() > limit).then(|| Cut::items(limit));
    items.truncate(limit);
    let place = place_of_kind(
        kind,
        scope(kind, request.namespace.as_ref().map(|n| n.as_str())),
    );
    let body = fit(Body::List {
        cluster: cluster.to_owned(),
        kind: kind.key(),
        items,
        cut,
    })?;
    Ok(Answer { body, place })
}

/// `get_object`: the object without its noise; a Secret as metadata alone.
pub(super) fn get_answer(
    cluster: &str,
    kind: &ResourceKind,
    namespace: Option<String>,
    name: &str,
    mut object: Value,
) -> Result<Answer, Refusal> {
    if kind.is_secret() {
        object = secret_metadata(kind, &object);
    } else {
        clean(&mut object);
    }
    check_object(&object)?;
    let uid = text(
        object
            .get("metadata")
            .and_then(|metadata| metadata.get("uid")),
    )
    .unwrap_or_default();
    let place = Place {
        kind: kind.key(),
        namespace: namespace.clone(),
        name: Some(name.to_owned()),
        uid: (!uid.is_empty()).then(|| uid.clone()),
        tab: Tab::Details,
    };
    let body = fit(Body::Get {
        cluster: cluster.to_owned(),
        kind: kind.key(),
        namespace,
        name: name.to_owned(),
        uid,
        object,
    })?;
    Ok(Answer { body, place })
}

/// `pod_logs`: the latest lines, bounded in count, bytes and line length.
pub(super) fn logs_answer(
    cluster: &str,
    request: &Logs,
    text: &str,
    server_cut: bool,
) -> Result<Answer, Refusal> {
    let (lines, cut) = log_lines(text, request.tail_lines, server_cut);
    let place = Place {
        kind: "pods".into(),
        namespace: Some(request.namespace.as_str().to_owned()),
        name: Some(request.pod.as_str().to_owned()),
        uid: None,
        tab: Tab::Logs,
    };
    let body = fit(Body::Logs {
        cluster: cluster.to_owned(),
        namespace: request.namespace.as_str().to_owned(),
        pod: request.pod.as_str().to_owned(),
        container: request.container.as_ref().map(|c| c.as_str().to_owned()),
        previous: request.previous,
        lines,
        cut,
    })?;
    Ok(Answer { body, place })
}

/// One event, read out of the API's object.
struct Event {
    row: EventRow,
    last: Option<DateTime<Utc>>,
    kind: String,
    name: String,
    namespace: Option<String>,
    uid: Option<String>,
    cut: bool,
}

fn time(value: Option<&Value>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.and_then(Value::as_str)?)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

fn event(raw: &Value) -> Event {
    let involved = raw.get("involvedObject");
    let involved_text = |key: &str| text(involved.and_then(|involved| involved.get(key)));
    let series = raw.get("series");
    let last = time(series.and_then(|series| series.get("lastObservedTime")))
        .or_else(|| time(raw.get("lastTimestamp")))
        .or_else(|| time(raw.get("eventTime")))
        .or_else(|| time(raw.get("firstTimestamp")))
        .or_else(|| time(raw.get("metadata").and_then(|m| m.get("creationTimestamp"))));
    let count = series
        .and_then(|series| series.get("count"))
        .and_then(Value::as_u64)
        .or_else(|| raw.get("count").and_then(Value::as_u64))
        .map_or(1, |count| count.clamp(1, u64::from(u32::MAX)) as u32);
    let (message, cut) = event_message(&text(raw.get("message")).unwrap_or_default());
    let kind = involved_text("kind").unwrap_or_default();
    let name = involved_text("name").unwrap_or_default();
    Event {
        row: EventRow {
            event_type: text(raw.get("type")).unwrap_or_default(),
            reason: text(raw.get("reason")).unwrap_or_default(),
            object: format!("{kind}/{name}"),
            message,
            count,
            last: last.map(|time| time.to_rfc3339()),
        },
        last,
        kind,
        name,
        namespace: text(raw.get("metadata").and_then(|m| m.get("namespace"))),
        uid: involved_text("uid"),
        cut,
    }
}

/// `list_events`: the newest events that match, at most the request's limit.
/// `looked_at` is how many events were read when the server had more.
pub(super) fn events_answer(
    cluster: &str,
    kind: Option<&ResourceKind>,
    request: &Events,
    raw: &[Value],
    looked_at: Option<usize>,
    now: DateTime<Utc>,
) -> Result<Answer, Refusal> {
    let since = now - Duration::seconds(request.since_seconds as i64);
    let namespace = request.namespace.as_ref().map(|n| n.as_str());
    let name = request.name.as_ref().map(|n| n.as_str());
    let mut events: Vec<Event> = raw
        .iter()
        .map(event)
        .filter(|event| kind.is_none_or(|kind| event.kind == kind.kind))
        .filter(|event| name.is_none_or(|name| event.name == name))
        .filter(|event| {
            namespace.is_none_or(|namespace| {
                event
                    .namespace
                    .as_deref()
                    .is_none_or(|have| have == namespace)
            })
        })
        .filter(|event| event.last.is_none_or(|last| last >= since))
        .collect();
    events.sort_by_key(|event| std::cmp::Reverse(event.last));
    let limit = request.limit.min(MAX_ITEMS);
    // `looked_at` is set when the server had more events than were read: the
    // newest of them may be among those not read, so the answer says it is cut.
    let mut cut = if events.len() > limit {
        Some(Cut::items(limit))
    } else {
        looked_at.map(Cut::items)
    };
    events.truncate(limit);
    if events.iter().any(|event| event.cut) {
        cut.get_or_insert(Cut::bytes(crate::constants::agent::MAX_EVENT_MESSAGE_BYTES));
    }
    let place = match (kind, name) {
        (Some(kind), Some(name)) => Place {
            kind: kind.key(),
            namespace: scope(kind, namespace),
            name: Some(name.to_owned()),
            uid: events.iter().find_map(|event| event.uid.clone()),
            tab: Tab::Events,
        },
        _ => Place {
            kind: "events".into(),
            namespace: namespace.map(str::to_owned),
            name: None,
            uid: None,
            tab: Tab::Details,
        },
    };
    let body = fit(Body::Events {
        cluster: cluster.to_owned(),
        events: events.into_iter().map(|event| event.row).collect(),
        cut,
    })?;
    Ok(Answer { body, place })
}

/// A name the cluster does not serve, or that is a Secret list asked for in a
/// way core does not allow.
pub(super) fn needs_namespace(
    kind: &ResourceKind,
    namespace: &Option<String>,
) -> Result<(), Refusal> {
    if kind.namespaced && namespace.is_none() {
        return Err(Refusal::invalid(
            "namespace",
            "is required for a namespaced kind",
        ));
    }
    Ok(())
}

/// The refusal for an object missing from example data.
pub(super) fn missing() -> Refusal {
    Refusal::new(Code::NotFound, "no such object")
}
