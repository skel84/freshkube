//! A read on example objects. The objects are raw, as a server would return
//! them; everything after them is the live path's code.

use std::sync::Arc;

use super::answer::Answer;
use super::call::{Events, Get, List, Logs, Request};
use super::refusal::Refusal;
use super::shape::{
    events_answer, get_answer, item, list_answer, logs_answer, missing, needs_namespace, scope,
};
use super::target::ExampleObjects;
use crate::resources::{ResourceKind, find_named};

pub(super) fn run(
    objects: &dyn ExampleObjects,
    entry: &str,
    request: &Request,
) -> Result<Answer, Refusal> {
    match request {
        Request::List(list) => list_objects(objects, entry, list),
        Request::Get(get) => get_object(objects, entry, get),
        Request::Logs(logs) => pod_logs(objects, entry, logs),
        Request::Events(events) => list_events(objects, entry, events),
    }
}

fn resolve(objects: &dyn ExampleObjects, asked: &str) -> Result<ResourceKind, Refusal> {
    if let Some(kind) = find_named(asked) {
        return Ok(kind);
    }
    let (name, group) = crate::resources::split_name(asked);
    let mut found = objects.kinds().into_iter().filter(|kind| {
        kind.key() == asked.to_ascii_lowercase()
            || (group.as_deref().is_none_or(|group| kind.group == group)
                && crate::resources::names_kind(kind, &name, false))
    });
    match (found.next(), found.next()) {
        (Some(kind), None) => Ok(kind),
        _ => Err(Refusal::not_served()),
    }
}

fn list_objects(
    objects: &dyn ExampleObjects,
    entry: &str,
    request: &List,
) -> Result<Answer, Refusal> {
    let kind = resolve(objects, request.kind.as_str())?;
    let namespace = scope(&kind, request.namespace.as_ref().map(|n| n.as_str()));
    let items = objects
        .objects(&kind, namespace.as_deref())
        .iter()
        .map(|object| item(&kind, object))
        .collect();
    list_answer(entry, &kind, request, items, false)
}

fn get_object(objects: &dyn ExampleObjects, entry: &str, request: &Get) -> Result<Answer, Refusal> {
    let kind = resolve(objects, request.kind.as_str())?;
    let namespace = scope(&kind, request.namespace.as_ref().map(|n| n.as_str()));
    needs_namespace(&kind, &namespace)?;
    let name = request.name.as_str();
    let object = objects
        .objects(&kind, namespace.as_deref())
        .into_iter()
        .find(|object| object.pointer("/metadata/name").and_then(|n| n.as_str()) == Some(name))
        .ok_or_else(missing)?;
    get_answer(entry, &kind, namespace, name, object)
}

fn pod_logs(objects: &dyn ExampleObjects, entry: &str, request: &Logs) -> Result<Answer, Refusal> {
    let lines = objects
        .log_lines(
            request.namespace.as_str(),
            request.pod.as_str(),
            request.container.as_ref().map(|c| c.as_str()),
            request.previous,
        )
        .ok_or_else(missing)?;
    logs_answer(entry, request, &lines.join("\n"), false)
}

fn list_events(
    objects: &dyn ExampleObjects,
    entry: &str,
    request: &Events,
) -> Result<Answer, Refusal> {
    let kind = request
        .kind
        .as_ref()
        .map(|asked| resolve(objects, asked.as_str()))
        .transpose()?;
    let raw = objects.events(request.namespace.as_ref().map(|n| n.as_str()));
    events_answer(
        entry,
        kind.as_ref(),
        request,
        &raw,
        None,
        chrono::Utc::now(),
    )
}

/// Lets a caller hold example objects behind an `Arc`.
pub fn shared(objects: impl ExampleObjects + 'static) -> Arc<dyn ExampleObjects> {
    Arc::new(objects)
}
