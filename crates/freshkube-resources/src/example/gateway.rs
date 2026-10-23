//! Invented Gateway API objects in the example cluster: the HTTPRoutes,
//! Gateways and GatewayClasses of core's routes example (always its `All`
//! shape, so every route a pane opens is in the answer the pane reads), so a
//! Routes row opens a real example object on its Trace tab.
use super::*;
use freshkube_core::resources::traffic::routes::example::{Shape, world};
use serde_json::Value;

/// The position, in a UID, of the first Gateway API object.
const FIRST: usize = 0xb000;

pub(super) const GROUPS: [(&str, &[&str]); 1] = [("gateway.networking.k8s.io", &["v1"])];

/// (group, version, kind, plural, namespaced).
pub(super) const KINDS: [(&str, &str, &str, &str, bool); 3] = [
    (
        "gateway.networking.k8s.io",
        "v1",
        "HTTPRoute",
        "httproutes",
        true,
    ),
    (
        "gateway.networking.k8s.io",
        "v1",
        "Gateway",
        "gateways",
        true,
    ),
    (
        "gateway.networking.k8s.io",
        "v1",
        "GatewayClass",
        "gatewayclasses",
        false,
    ),
];

fn key(plural: &str) -> String {
    format!("{plural}.gateway.networking.k8s.io")
}

fn objects(plural: &str) -> Vec<Value> {
    let world = world(Shape::All);
    match plural {
        "httproutes" => world.routes,
        "gateways" => world.gateways,
        _ => world.classes,
    }
}

fn plural_of(resource: &str) -> Option<&'static str> {
    KINDS
        .iter()
        .map(|(_, _, _, plural, _)| *plural)
        .find(|plural| key(plural) == resource)
}

/// Whether a row of this kind, at this position in its UID, is one of these.
pub(super) fn holds(resource: &str, ix: usize) -> bool {
    ix >= FIRST && plural_of(resource).is_some()
}

fn name_of(object: &Value) -> (&str, &str) {
    (
        object["metadata"]["namespace"].as_str().unwrap_or_default(),
        object["metadata"]["name"].as_str().unwrap_or_default(),
    )
}

pub(super) fn read(
    connection: &str,
    resource: &str,
    now: i64,
) -> Option<(Vec<ResourceColumn>, Vec<ResourceRow>)> {
    let plural = plural_of(resource)?;
    let rows = objects(plural)
        .iter()
        .enumerate()
        .map(|(ix, object)| {
            let (namespace, name) = name_of(object);
            ResourceRow {
                identity: identity(connection, resource, namespace, name, FIRST + ix),
                cells: vec![name.into(), String::new()],
                created: Some(now - 30 * 86_400 + ix as i64 * 3_600),
                terminating: false,
                resource_version: EXAMPLE_VERSION.into(),
                owner: None,
                generated: None,
                pod: None,
            }
        })
        .collect();
    Some((
        vec![
            ResourceColumn::new("Name", ColumnKind::Text, false),
            ResourceColumn::new("Age", ColumnKind::Age, false),
        ],
        rows,
    ))
}

/// The object behind a row, with the row's identity.
pub(super) fn document(row: &ResourceRow, ix: usize) -> Option<String> {
    let plural = plural_of(&row.identity.resource)?;
    let mut object = objects(plural).into_iter().nth(ix.checked_sub(FIRST)?)?;
    let metadata = object.get_mut("metadata")?.as_object_mut()?;
    metadata.insert("uid".into(), row.identity.uid.clone().into());
    metadata.insert(
        "resourceVersion".into(),
        row.resource_version.clone().into(),
    );
    metadata.insert(
        "creationTimestamp".into(),
        objects::timestamp(row.created.unwrap_or_default()).into(),
    );
    serde_yaml::to_string(&object).ok()
}
