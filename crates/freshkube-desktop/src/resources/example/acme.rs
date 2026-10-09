//! The objects of acme's `core-fra` (core's `applications::example`) in the
//! example cluster: Kargo's Projects, Stages, Warehouses and the change
//! page's Freight, Argo CD's
//! Applications and ApplicationSet, and the `status/status-page`
//! Deployment. The Applications page maps `core-fra` to the open example
//! cluster, so a part it opens in Resources is listed here, the same object
//! the page read. An environment cluster's context also lists that
//! cluster's labelled Deployments (`environment_objects`), so a part opened
//! there after a switch is the one read too.
//!
//! Their positions start at [`FIRST`], clear of the example's own rows, so a
//! Deployment's UID never names one of `WORKLOADS`.
use serde_json::{Value, json};

use freshkube_core::applications::example::{CoreObject, core_objects, environment_objects};

use super::*;

/// The position, in a UID, of acme's first object.
const FIRST: usize = 0xac00;
/// The position of an environment cluster's first labelled Deployment,
/// after core-fra's objects.
const ENVIRONMENT: usize = FIRST + 0x80;

/// The API groups acme's custom kinds are in, as discovery lists them.
pub(super) const GROUPS: [(&str, &[&str]); 2] = [
    ("argoproj.io", &["v1alpha1"]),
    ("kargo.akuity.io", &["v1alpha1"]),
];

/// (group, version, kind, plural, namespaced) of acme's custom kinds.
pub(super) const KINDS: [(&str, &str, &str, &str, bool); 6] = [
    (
        "argoproj.io",
        "v1alpha1",
        "Application",
        "applications",
        true,
    ),
    (
        "argoproj.io",
        "v1alpha1",
        "ApplicationSet",
        "applicationsets",
        true,
    ),
    ("kargo.akuity.io", "v1alpha1", "Project", "projects", false),
    ("kargo.akuity.io", "v1alpha1", "Stage", "stages", true),
    (
        "kargo.akuity.io",
        "v1alpha1",
        "Warehouse",
        "warehouses",
        true,
    ),
    ("kargo.akuity.io", "v1alpha1", "Freight", "freights", true),
];

/// Whether a row of this kind, at this position in its UID, is one of
/// acme's objects. Other kinds' UIDs may count past [`FIRST`].
pub(super) fn holds(resource: &str, ix: usize) -> bool {
    ix >= FIRST && (resource == "deployments.apps" || is_kind(resource))
}

fn is_kind(key: &str) -> bool {
    KINDS
        .iter()
        .any(|(group, _, _, plural, _)| format!("{plural}.{group}") == key)
}

/// acme's objects of one kind in the example cluster `context`, each with
/// its position: core-fra's in every context, and an environment cluster's
/// labelled Deployments in its own.
fn of(context: &str, key: &str) -> Vec<(usize, CoreObject)> {
    let core = core_objects()
        .into_iter()
        .enumerate()
        .map(|(ix, object)| (FIRST + ix, object));
    let environment = environment_objects(context)
        .into_iter()
        .enumerate()
        .map(|(ix, object)| (ENVIRONMENT + ix, object));
    core.chain(environment)
        .filter(|(_, object)| object.key == key)
        .collect()
}

fn text<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value.pointer(pointer).and_then(Value::as_str).unwrap_or("")
}

fn created(now: i64, ix: usize) -> i64 {
    now - 60 * 86_400 + (ix - FIRST) as i64 * 3_600
}

fn columns() -> Vec<ResourceColumn> {
    vec![
        ResourceColumn::new("Name", ColumnKind::Text, false),
        ResourceColumn::new("Age", ColumnKind::Age, false),
    ]
}

/// A custom kind's rows, as the server prints a kind without printer
/// columns; `None` for a kind acme doesn't have.
pub(super) fn read(
    context: &str,
    connection: &str,
    key: &str,
    now: i64,
) -> Option<(Vec<ResourceColumn>, Vec<ResourceRow>)> {
    if !is_kind(key) {
        return None;
    }
    let rows = of(context, key)
        .into_iter()
        .map(|(ix, object)| {
            let name = text(&object.value, "/metadata/name");
            ResourceRow {
                identity: identity(
                    connection,
                    key,
                    text(&object.value, "/metadata/namespace"),
                    name,
                    ix,
                ),
                cells: vec![name.into(), String::new()],
                created: Some(created(now, ix)),
                terminating: false,
                resource_version: EXAMPLE_VERSION.into(),
                owner: None,
                generated: None,
                pod: None,
            }
        })
        .collect();
    Some((columns(), rows))
}

/// acme's Deployments, in the Deployments list's columns, after the
/// example's own.
pub(super) fn deployments(
    context: &str,
    connection: &str,
    now: i64,
) -> impl Iterator<Item = ResourceRow> {
    of(context, "deployments.apps")
        .into_iter()
        .map(move |(ix, object)| {
            let name = text(&object.value, "/metadata/name");
            ResourceRow {
                identity: identity(
                    connection,
                    "deployments.apps",
                    text(&object.value, "/metadata/namespace"),
                    name,
                    ix,
                ),
                cells: vec![
                    name.into(),
                    "1/1".into(),
                    "1".into(),
                    "1".into(),
                    String::new(),
                    name.into(),
                    image(name),
                    format!("app={name}"),
                ],
                created: Some(created(now, ix)),
                terminating: false,
                resource_version: EXAMPLE_VERSION.into(),
                owner: None,
                generated: None,
                pod: None,
            }
        })
}

fn image(name: &str) -> String {
    format!("ghcr.io/example/{name}:1.4.0")
}

/// The full object behind one of acme's rows, as core holds it with the
/// row's identity; a Deployment also gets the spec and status the list
/// shows.
pub(super) fn document(row: &ResourceRow, ix: usize) -> Option<String> {
    let context = row.identity.connection.strip_prefix("example:")?;
    let (_, object) = of(context, &row.identity.resource)
        .into_iter()
        .find(|(at, _)| *at == ix)?;
    let mut value = object.value;
    let metadata = value.get_mut("metadata")?.as_object_mut()?;
    metadata.insert("uid".into(), row.identity.uid.clone().into());
    metadata.insert(
        "resourceVersion".into(),
        row.resource_version.clone().into(),
    );
    metadata.insert(
        "creationTimestamp".into(),
        objects::timestamp(row.created.unwrap_or_default()).into(),
    );
    if object.key == "deployments.apps" {
        let name = row.identity.name.as_str();
        let fields = value.as_object_mut()?;
        fields.insert(
            "spec".into(),
            json!({
                "replicas": 1,
                "selector": {"matchLabels": {"app": name}},
                "template": {
                    "metadata": {"labels": {"app": name}},
                    "spec": {"containers": [{"name": name, "image": image(name)}]},
                },
            }),
        );
        fields.insert(
            "status".into(),
            json!({"replicas": 1, "readyReplicas": 1, "availableReplicas": 1}),
        );
    }
    serde_yaml::to_string(&value).ok()
}
