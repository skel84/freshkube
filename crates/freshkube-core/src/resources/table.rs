//! Server-side printing: the API server renders the same columns `kubectl
//! get` shows for any kind, custom resources included, as a `meta.k8s.io`
//! `Table`.

use std::collections::BTreeMap;

use futures::Stream;
use kube::Client;
use kube::core::WatchEvent;
use serde::Deserialize;

use super::failure::Failure;
use super::kinds::ResourceKind;
use super::object::json_get;
use super::pod_row::{self, PodFacts, PodSpec, PodStatus, lenient};

/// Asks for a table, falling back to plain JSON on servers without one.
/// `includeObject=Metadata` adds each row's identity; pods include the whole
/// object, whose status and resources their rows show.
const TABLE_ACCEPT: &str = "application/json;as=Table;v=v1;g=meta.k8s.io,application/json;as=Table;v=v1beta1;g=meta.k8s.io,application/json";
const PAGE_SIZE: usize = 500;
const WATCH_SECONDS: u32 = 290;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    #[serde(default, deserialize_with = "nullable")]
    pub column_definitions: Vec<TableColumn>,
    #[serde(default, deserialize_with = "nullable")]
    pub rows: Vec<TableRow>,
    #[serde(default, deserialize_with = "nullable")]
    pub metadata: ListMetadata,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListMetadata {
    #[serde(default, deserialize_with = "nullable")]
    pub resource_version: String,
    #[serde(default, rename = "continue", deserialize_with = "nullable")]
    pub continue_token: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableColumn {
    pub name: String,
    /// `string`, `integer`, `number`, `boolean` or `date`.
    #[serde(rename = "type")]
    pub column_type: String,
    #[serde(default, deserialize_with = "nullable")]
    pub format: String,
    #[serde(default, deserialize_with = "nullable")]
    pub description: String,
    /// 0 for columns kubectl shows by default; higher only with `-o wide`.
    #[serde(default, deserialize_with = "nullable")]
    pub priority: i32,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct TableRow {
    #[serde(default, deserialize_with = "nullable")]
    pub cells: Vec<serde_json::Value>,
    #[serde(default)]
    pub object: Option<RowObject>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct RowObject {
    #[serde(default, deserialize_with = "nullable")]
    pub metadata: RowMetadata,
    /// Present only for pods, whose rows include the whole object.
    #[serde(default, deserialize_with = "lenient")]
    spec: Option<PodSpec>,
    #[serde(default, deserialize_with = "lenient")]
    status: Option<PodStatus>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowMetadata {
    #[serde(default, deserialize_with = "nullable")]
    pub name: String,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    pub uid: String,
    #[serde(default, deserialize_with = "nullable")]
    pub resource_version: String,
    #[serde(default)]
    pub creation_timestamp: Option<String>,
    #[serde(default)]
    pub deletion_timestamp: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    pub labels: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "nullable")]
    pub owner_references: Vec<OwnerReference>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerReference {
    #[serde(default, deserialize_with = "nullable")]
    pub api_version: String,
    #[serde(default, deserialize_with = "nullable")]
    pub kind: String,
    #[serde(default, deserialize_with = "nullable")]
    pub name: String,
    #[serde(default, deserialize_with = "nullable")]
    pub uid: String,
    #[serde(default)]
    pub controller: Option<bool>,
}

impl RowMetadata {
    /// The reference that manages the object, such as a pod's ReplicaSet.
    pub fn controller(&self) -> Option<&OwnerReference> {
        self.owner_references
            .iter()
            .find(|owner| owner.controller == Some(true))
    }
}

/// Go encodes an empty list or map as `null`; take that as empty.
pub(crate) fn nullable<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}

impl TableRow {
    pub fn metadata(&self) -> Option<&RowMetadata> {
        self.object.as_ref().map(|object| &object.metadata)
    }

    /// A pod's facts, when the row includes the whole pod.
    pub fn pod(&self) -> Option<PodFacts> {
        let object = self.object.as_ref()?;
        object
            .spec
            .as_ref()
            .map(|spec| pod_row::facts(spec, object.status.as_ref()))
    }
}

/// Lists every page of a collection as one table.
pub async fn list_table(
    client: &Client,
    kind: &ResourceKind,
    namespace: Option<&str>,
    field_selector: Option<&str>,
) -> Result<Table, Failure> {
    let base = kind.collection_path(namespace);
    let mut table = Table::default();
    let mut continue_token = String::new();
    loop {
        let mut path = format!(
            "{base}?includeObject={}&limit={PAGE_SIZE}",
            include_object(kind)
        );
        if let Some(selector) = field_selector {
            path.push_str("&fieldSelector=");
            path.push_str(&query_value(selector));
        }
        if !continue_token.is_empty() {
            path.push_str("&continue=");
            path.push_str(&query_value(&continue_token));
        }
        let page: Table = client
            .request(json_get(path, TABLE_ACCEPT)?)
            .await
            .map_err(Failure::from_kube)?;
        if table.column_definitions.is_empty() {
            table.column_definitions = page.column_definitions;
        }
        table.rows.extend(page.rows);
        table.metadata.resource_version = page.metadata.resource_version;
        continue_token = page.metadata.continue_token;
        if continue_token.is_empty() {
            return Ok(table);
        }
    }
}

/// Watches a collection from a resource version. The server ends the stream
/// after about five minutes; callers resume from the last version they saw.
pub(crate) async fn watch_table(
    client: &Client,
    kind: &ResourceKind,
    namespace: Option<&str>,
    field_selector: Option<&str>,
    resource_version: &str,
) -> Result<impl Stream<Item = kube::Result<WatchEvent<Table>>> + use<>, Failure> {
    let mut path = format!(
        "{}?watch=1&includeObject={}&resourceVersion={}&timeoutSeconds={WATCH_SECONDS}",
        kind.collection_path(namespace),
        include_object(kind),
        query_value(resource_version)
    );
    if let Some(selector) = field_selector {
        path.push_str("&fieldSelector=");
        path.push_str(&query_value(selector));
    }
    client
        .request_events::<Table>(json_get(path, TABLE_ACCEPT)?)
        .await
        .map_err(Failure::from_kube)
}

/// What each row carries of its object: a pod's row needs its containers'
/// states and resources, any other only its identity.
fn include_object(kind: &ResourceKind) -> &'static str {
    if kind.is_pod() { "Object" } else { "Metadata" }
}

/// Percent-encodes a query value. Continue tokens are opaque and may carry
/// characters a query string can't hold as they are.
fn query_value(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_rows_parse_cells_and_metadata() {
        let table: Table = serde_json::from_str(
            r#"{"kind":"Table","apiVersion":"meta.k8s.io/v1",
                "metadata":{"resourceVersion":"42","continue":"next"},
                "columnDefinitions":[
                  {"name":"Name","type":"string","format":"name","description":"","priority":0},
                  {"name":"Restarts","type":"string","format":"","description":"","priority":0},
                  {"name":"IP","type":"string","format":"","description":"","priority":1}],
                "rows":[{"cells":["api-0","2 (5m ago)","10.0.0.1"],
                  "object":{"kind":"PartialObjectMetadata","metadata":{
                    "name":"api-0","namespace":"default","uid":"u-1","resourceVersion":"41",
                    "creationTimestamp":"2026-09-30T10:00:00Z","labels":{"app":"api"}}}}]}"#,
        )
        .unwrap();
        assert_eq!(table.metadata.resource_version, "42");
        assert_eq!(table.metadata.continue_token, "next");
        assert_eq!(table.column_definitions[2].priority, 1);
        let metadata = table.rows[0].metadata().unwrap();
        assert_eq!(metadata.uid, "u-1");
        assert_eq!(metadata.namespace.as_deref(), Some("default"));
        assert_eq!(metadata.labels["app"], "api");
        assert_eq!(table.rows[0].cells[1], "2 (5m ago)");
    }

    #[test]
    fn null_lists_and_maps_read_as_empty() {
        // Watch events after the first carry no column definitions.
        let table: Table = serde_json::from_str(
            r#"{"columnDefinitions":null,"rows":[{"cells":null,"object":{"metadata":{
                "name":"a","labels":null,"resourceVersion":null}}}],"metadata":null}"#,
        )
        .unwrap();
        assert!(table.column_definitions.is_empty());
        assert!(table.rows[0].cells.is_empty());
        assert!(table.rows[0].metadata().unwrap().labels.is_empty());
        assert_eq!(table.rows[0].metadata().unwrap().name, "a");
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(query_value("eyJ2Ijo-x_1.~"), "eyJ2Ijo-x_1.~");
        assert_eq!(query_value("a+b/c=d&e f"), "a%2Bb%2Fc%3Dd%26e%20f");
    }
}
