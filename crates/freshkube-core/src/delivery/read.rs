//! The one door every delivery read goes through.
//!
//! [`ReadOnlyClient`] is the only type in this module tree that holds a
//! `kube::Client`, and its only requests are GETs: one object, one bounded
//! page of a scoped collection, and the discovery documents. Everything else
//! in `delivery` reads through the [`Reader`] trait, so fixtures stand in for
//! a cluster and a review needs one grep (`Client`, `Request::`) in this file.
//!
//! Rules enforced here, not by callers:
//! - Every namespace, name, group, version and plural is checked against
//!   Kubernetes' own syntax before it goes into a path, so none can add a
//!   path segment (`..`, `/`) or a query.
//! - Secrets are never read, in any spelling of the path.
//! - A collection is read only inside a namespace or with a label selector,
//!   a page at a time, and stops after [`MAX_PAGES`] pages, saying so.
//! - A refused, missing or unreachable read is an error, never an empty list.

use std::collections::BTreeMap;

use kube::Client;
use serde_json::Value;

use crate::resources::{Failure, FailureKind};

use super::source::Truncation;

/// Items asked for per page.
pub const PAGE_LIMIT: u32 = 200;
/// Pages read before a listing is cut off and marked truncated.
pub const MAX_PAGES: u32 = 5;

/// One kind on one API version, as a REST path is built from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    pub group: String,
    pub version: String,
    pub plural: String,
    pub namespaced: bool,
}

impl Resource {
    pub fn new(group: &str, version: &str, plural: &str, namespaced: bool) -> Self {
        Self {
            group: group.into(),
            version: version.into(),
            plural: plural.into(),
            namespaced,
        }
    }

    fn prefix(&self) -> String {
        if self.group.is_empty() {
            format!("/api/{}", self.version)
        } else {
            format!("/apis/{}/{}", self.group, self.version)
        }
    }

    fn is_secret(&self) -> bool {
        self.group.is_empty() && self.plural.eq_ignore_ascii_case("secrets")
    }

    fn collection(&self, namespace: Option<&str>) -> String {
        match namespace.filter(|_| self.namespaced) {
            Some(namespace) => format!("{}/namespaces/{namespace}/{}", self.prefix(), self.plural),
            None => format!("{}/{}", self.prefix(), self.plural),
        }
    }
}

/// What a listing is limited to. There is no unscoped variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Everything of the kind in one namespace.
    Namespace(String),
    /// Objects matching a label selector, in one namespace or across all.
    Labels {
        namespace: Option<String>,
        selector: String,
    },
}

/// A bounded page-by-page read of one scoped collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListRequest {
    pub resource: Resource,
    pub scope: Scope,
}

/// The items of a listing and whether it was cut off.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listing {
    pub items: Vec<Value>,
    pub truncated: Option<Truncation>,
}

impl Listing {
    /// The items `parse` understands, with where the listing stopped.
    pub fn parse<T>(&self, parse: fn(&Value) -> Option<T>) -> (Vec<T>, Option<Truncation>) {
        (
            self.items.iter().filter_map(parse).collect(),
            self.truncated,
        )
    }
}

/// An API group's served versions, from `/apis`, preferred version first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServedGroup {
    pub name: String,
    pub versions: Vec<String>,
}

/// What a read source offers. The only implementations are
/// [`ReadOnlyClient`] and the test fixtures.
pub trait Reader {
    /// One object by name.
    fn get(
        &self,
        resource: &Resource,
        namespace: Option<&str>,
        name: &str,
    ) -> impl Future<Output = Result<Value, Failure>>;
    /// One scoped collection, up to [`MAX_PAGES`] pages.
    fn list(&self, request: &ListRequest) -> impl Future<Output = Result<Listing, Failure>>;
    /// `/apis`: every group and its served versions.
    fn groups(&self) -> impl Future<Output = Result<Vec<ServedGroup>, Failure>>;
    /// `/apis/<group>/<version>`: the plurals that version serves.
    fn plurals(
        &self,
        group: &str,
        version: &str,
    ) -> impl Future<Output = Result<Vec<String>, Failure>>;
}

/// A client for one explicitly named context that can only read.
#[derive(Clone)]
pub struct ReadOnlyClient {
    client: Client,
}

impl ReadOnlyClient {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    async fn get_value(&self, path: String) -> Result<Value, Failure> {
        let request = http::Request::get(path)
            .header(http::header::ACCEPT, "application/json")
            .body(Vec::new())
            .map_err(|error| Failure::new(FailureKind::Other, error.to_string()))?;
        self.client
            .request::<Value>(request)
            .await
            .map_err(Failure::from_kube)
    }
}

/// A DNS-1123 label, as a namespace or a plural is written.
fn is_label(text: &str) -> bool {
    let bytes = text.as_bytes();
    (1..=63).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && bytes[0] != b'-'
        && bytes[bytes.len() - 1] != b'-'
}

/// A DNS-1123 subdomain, as an object name or an API group is written.
fn is_subdomain(text: &str) -> bool {
    text.len() <= 253 && text.split('.').all(is_label)
}

/// An API version: `v1`, `v1beta1`.
fn is_version(text: &str) -> bool {
    (1..=63).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn check(what: &str, text: &str, valid: fn(&str) -> bool) -> Result<(), Failure> {
    if valid(text) {
        Ok(())
    } else {
        Err(Failure::new(
            FailureKind::Other,
            format!("{what} {text:?} is not valid Kubernetes syntax"),
        ))
    }
}

fn check_resource(resource: &Resource) -> Result<(), Failure> {
    refuse_secret(resource)?;
    if !resource.group.is_empty() {
        check("API group", &resource.group, is_subdomain)?;
    }
    check("API version", &resource.version, is_version)?;
    check("resource", &resource.plural, is_label)
}

fn check_namespace(namespace: Option<&str>) -> Result<(), Failure> {
    namespace.map_or(Ok(()), |namespace| check("namespace", namespace, is_label))
}

/// The path of one object, after every part is checked.
fn object_path(
    resource: &Resource,
    namespace: Option<&str>,
    name: &str,
) -> Result<String, Failure> {
    check_resource(resource)?;
    check_namespace(namespace)?;
    check("name", name, is_subdomain)?;
    Ok(format!("{}/{name}", resource.collection(namespace)))
}

/// The path of a group version's discovery document, after both are checked.
fn discovery_path(group: &str, version: &str) -> Result<String, Failure> {
    check("API group", group, is_subdomain)?;
    check("API version", version, is_version)?;
    Ok(format!("/apis/{group}/{version}"))
}

fn refuse_secret(resource: &Resource) -> Result<(), Failure> {
    if resource.is_secret() {
        return Err(Failure::new(
            FailureKind::Other,
            "Secrets are never read by the delivery spike",
        ));
    }
    Ok(())
}

fn page_path(request: &ListRequest, continue_token: Option<&str>) -> Result<String, Failure> {
    let (namespace, selector) = match &request.scope {
        Scope::Namespace(namespace) => (Some(namespace.as_str()), None),
        Scope::Labels {
            namespace,
            selector,
        } => (namespace.as_deref(), Some(selector.as_str())),
    };
    check_resource(&request.resource)?;
    check_namespace(namespace)?;
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("limit", &PAGE_LIMIT.to_string());
    if let Some(selector) = selector {
        query.append_pair("labelSelector", selector);
    }
    if let Some(token) = continue_token {
        query.append_pair("continue", token);
    }
    Ok(format!(
        "{}?{}",
        request.resource.collection(namespace),
        query.finish()
    ))
}

impl Reader for ReadOnlyClient {
    async fn get(
        &self,
        resource: &Resource,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<Value, Failure> {
        self.get_value(object_path(resource, namespace, name)?)
            .await
    }

    async fn list(&self, request: &ListRequest) -> Result<Listing, Failure> {
        let mut listing = Listing::default();
        let mut token: Option<String> = None;
        for page in 1..=MAX_PAGES {
            let mut body = self
                .get_value(page_path(request, token.as_deref())?)
                .await?;
            if let Some(items) = body.get_mut("items").and_then(Value::as_array_mut) {
                listing.items.append(items);
            }
            token = body
                .pointer("/metadata/continue")
                .and_then(Value::as_str)
                .filter(|token| !token.is_empty())
                .map(str::to_owned);
            if token.is_none() {
                return Ok(listing);
            }
            if page == MAX_PAGES {
                listing.truncated = Some(Truncation {
                    read: listing.items.len(),
                });
            }
        }
        Ok(listing)
    }

    async fn groups(&self) -> Result<Vec<ServedGroup>, Failure> {
        let body = self.get_value("/apis".into()).await?;
        Ok(parse_groups(&body))
    }

    async fn plurals(&self, group: &str, version: &str) -> Result<Vec<String>, Failure> {
        let body = self.get_value(discovery_path(group, version)?).await?;
        Ok(parse_plurals(&body))
    }
}

pub(super) fn parse_groups(body: &Value) -> Vec<ServedGroup> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for group in body
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = group.get("name").and_then(Value::as_str) else {
            continue;
        };
        let preferred = group
            .pointer("/preferredVersion/version")
            .and_then(Value::as_str);
        let mut versions: Vec<String> = group
            .get("versions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get("version").and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
        if let Some(preferred) = preferred
            && let Some(at) = versions.iter().position(|version| version == preferred)
        {
            let first = versions.remove(at);
            versions.insert(0, first);
        }
        groups.insert(name.to_owned(), versions);
    }
    groups
        .into_iter()
        .map(|(name, versions)| ServedGroup { name, versions })
        .collect()
}

pub(super) fn parse_plurals(body: &Value) -> Vec<String> {
    body.get("resources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("name").and_then(Value::as_str))
        .filter(|name| !name.contains('/'))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pods() -> Resource {
        Resource::new("", "v1", "pods", true)
    }

    #[test]
    fn secrets_are_refused_by_every_spelling_of_the_group() {
        assert!(refuse_secret(&Resource::new("", "v1", "secrets", true)).is_err());
        assert!(refuse_secret(&Resource::new("", "v1", "Secrets", true)).is_err());
        assert!(refuse_secret(&pods()).is_ok());
        // A custom kind that happens to be called secrets in another group
        // is not a core Secret.
        assert!(refuse_secret(&Resource::new("example.dev", "v1", "secrets", true)).is_ok());
    }

    #[test]
    fn a_page_is_always_limited_and_scoped() {
        let request = ListRequest {
            resource: pods(),
            scope: Scope::Labels {
                namespace: Some("app-a".into()),
                selector: "rollouts-pod-template-hash in (abc,def)".into(),
            },
        };
        let path = page_path(&request, Some("next")).unwrap();
        assert!(path.starts_with("/api/v1/namespaces/app-a/pods?limit=200&labelSelector="));
        assert!(path.contains("rollouts-pod-template-hash+in+%28abc%2Cdef%29"));
        assert!(path.ends_with("&continue=next"));
        let namespaced = ListRequest {
            resource: Resource::new("argoproj.io", "v1alpha1", "applications", true),
            scope: Scope::Namespace("argocd".into()),
        };
        assert_eq!(
            page_path(&namespaced, None).unwrap(),
            "/apis/argoproj.io/v1alpha1/namespaces/argocd/applications?limit=200"
        );
    }

    #[test]
    fn no_part_of_a_path_can_climb_out_of_it() {
        let pods = pods();
        for bad in ["../kube-system", "a/b", "..", "A", "-a", "a b", "a?x=1", ""] {
            assert!(
                object_path(&pods, Some(bad), "x").is_err(),
                "namespace {bad:?}"
            );
            assert!(
                object_path(&pods, Some("shop"), bad).is_err(),
                "name {bad:?}"
            );
            let request = ListRequest {
                resource: pods.clone(),
                scope: Scope::Namespace(bad.into()),
            };
            assert!(page_path(&request, None).is_err(), "namespace {bad:?}");
            assert!(discovery_path(bad, "v1").is_err(), "group {bad:?}");
            assert!(
                discovery_path("argoproj.io", bad).is_err(),
                "version {bad:?}"
            );
        }
        // Secrets can't be reached through another segment either.
        for plural in ["pods/../secrets", "../secrets", "secrets/x"] {
            let request = ListRequest {
                resource: Resource::new("", "v1", plural, true),
                scope: Scope::Namespace("shop".into()),
            };
            assert!(page_path(&request, None).is_err(), "{plural}");
        }
        assert!(object_path(&pods, Some("shop"), "../secrets/token").is_err());
        assert!(
            object_path(
                &Resource::new("../api", "v1", "secrets", true),
                Some("shop"),
                "x"
            )
            .is_err()
        );
        assert_eq!(
            object_path(&pods, Some("shop"), "storefront-5d9c-x.a1").unwrap(),
            "/api/v1/namespaces/shop/pods/storefront-5d9c-x.a1"
        );
        assert_eq!(
            discovery_path("kargo.akuity.io", "v1alpha1").unwrap(),
            "/apis/kargo.akuity.io/v1alpha1"
        );
    }

    #[test]
    fn discovery_documents_keep_the_preferred_version_first() {
        let groups = serde_json::json!({"groups": [
            {"name": "kargo.example", "versions": [{"version": "v1alpha1"}, {"version": "v1"}],
             "preferredVersion": {"version": "v1"}},
            {"name": "broken"}
        ]});
        let parsed = parse_groups(&groups);
        assert_eq!(parsed[1].name, "kargo.example");
        assert_eq!(parsed[1].versions, ["v1", "v1alpha1"]);
        assert!(parsed[0].versions.is_empty());
        let plurals = serde_json::json!({"resources": [
            {"name": "stages"}, {"name": "stages/status"}
        ]});
        assert_eq!(parse_plurals(&plurals), ["stages"]);
    }
}
