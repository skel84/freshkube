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
//!   a page at a time, and stops after [`MAX_PAGES`] pages, saying so. A
//!   selector's values are checked as label values ([`label_equals`]).
//! - A response body is read up to [`MAX_BODY_BYTES`]; a longer one fails.
//! - A refused, missing or unreachable read is an error, never an empty list.

use std::collections::BTreeMap;

use kube::Client;
use serde_json::Value;

use crate::resources::{Failure, FailureKind};

use super::source::{Truncation, cap_message, redact_body};

/// Items asked for per page.
pub const PAGE_LIMIT: u32 = 200;
/// Pages read before a listing is cut off and marked truncated.
pub const MAX_PAGES: u32 = 5;
/// The longest response body read. A page of [`PAGE_LIMIT`] large objects
/// fits; a longer answer fails rather than fill memory.
pub const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

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
            .body(kube::client::Body::empty())
            .map_err(|error| Failure::new(FailureKind::Other, error.to_string()))?;
        let response = self
            .client
            .send(request)
            .await
            .map_err(Failure::from_kube)?;
        let status = response.status();
        let body = read_body(response.into_body(), MAX_BODY_BYTES).await?;
        decode(status, &body)
    }
}

/// A response body, read to its end unless it passes `limit` bytes.
async fn read_body(body: kube::client::Body, limit: usize) -> Result<Vec<u8>, Failure> {
    use http_body_util::BodyExt;
    match http_body_util::Limited::new(body, limit).collect().await {
        Ok(collected) => Ok(collected.to_bytes().to_vec()),
        Err(error) if error.is::<http_body_util::LengthLimitError>() => Err(Failure::new(
            FailureKind::Other,
            format!("the response is longer than {limit} bytes and was not read"),
        )),
        Err(error) => match error.downcast::<kube::Error>() {
            Ok(error) => Err(Failure::from_kube(*error)),
            Err(error) => Err(Failure::new(FailureKind::Unreachable, error.to_string())),
        },
    }
}

/// The JSON of a successful answer, or the failure an error status reports,
/// classified as kube classifies it. A body that isn't the API server's
/// `Status`, such as a proxy's page or its own JSON, loses every place it
/// names, bare host names included, and is cut short ([`redact_body`]); its
/// final message, which quoting can lengthen, is cut short again
/// ([`cap_message`]). The server's own message is kept whole here: it is
/// redacted and cut only when it is printed ([`super::source::printable`]),
/// since a cut before redaction could split a credential from its `@`.
fn decode(status: http::StatusCode, body: &[u8]) -> Result<Value, Failure> {
    if status.is_client_error() || status.is_server_error() {
        let text = String::from_utf8_lossy(body);
        if let Some(response) = api_status(&text) {
            return Err(Failure::from_kube(kube::Error::Api(response)));
        }
        let response = kube::core::ErrorResponse {
            status: status.to_string(),
            code: status.as_u16(),
            message: format!("{:?}", redact_body(&text)),
            reason: "Failed to parse error data".into(),
        };
        let mut failure = Failure::from_kube(kube::Error::Api(response));
        failure.message = cap_message(failure.message);
        return Err(failure);
    }
    serde_json::from_slice(body).map_err(|error| {
        Failure::new(
            FailureKind::Other,
            format!("the answer is not the JSON expected: {error}"),
        )
    })
}

/// The API server's own error: a `Status` object. A proxy's JSON with
/// `status`, `code` and `message` fields reads as an `ErrorResponse` too, so
/// it counts only with `kind: Status`.
fn api_status(text: &str) -> Option<kube::core::ErrorResponse> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("kind").and_then(Value::as_str) != Some("Status") {
        return None;
    }
    serde_json::from_value(value).ok()
}

/// A label value: at most 63 characters, alphanumeric at both ends, with
/// `-`, `_` and `.` between.
fn is_label_value(text: &str) -> bool {
    let bytes = text.as_bytes();
    (1..=63).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// The selector `key=value`, once the value is checked as a label value, so
/// a value read from an object can't add a term to the selector.
pub fn label_equals(key: &str, value: &str) -> Result<String, Failure> {
    check("label value", value, is_label_value)?;
    Ok(format!("{key}={value}"))
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
    fn a_selector_value_cannot_add_a_term() {
        assert_eq!(
            label_equals("rollouts-pod-template-hash", "5d8f7c9b4").unwrap(),
            "rollouts-pod-template-hash=5d8f7c9b4"
        );
        for bad in [
            "a,b=c",
            "a b",
            "",
            "-a",
            "a.",
            "a=b",
            "a!=b",
            &"a".repeat(64),
        ] {
            assert!(label_equals("key", bad).is_err(), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn a_body_longer_than_the_limit_is_not_read() {
        let body = |len: usize| kube::client::Body::from(vec![b' '; len]);
        let failure = read_body(body(11), 10).await.unwrap_err();
        assert!(
            failure.message.contains("longer than 10 bytes"),
            "{failure}"
        );
        assert_eq!(read_body(body(10), 10).await.unwrap().len(), 10);
    }

    #[test]
    fn an_error_status_is_classified_as_kube_would() {
        let status = br#"{"kind":"Status","status":"Failure","message":"pods is forbidden","reason":"Forbidden","code":403}"#;
        let refused = decode(http::StatusCode::FORBIDDEN, status).unwrap_err();
        assert_eq!(refused.kind, FailureKind::Forbidden);
        assert_eq!(refused.message, "pods is forbidden");
        let missing = decode(http::StatusCode::NOT_FOUND, b"404 page not found\n").unwrap_err();
        assert_eq!(missing.kind, FailureKind::NotFound);
        assert_eq!(missing.message, "404 page not found");
        assert_eq!(
            decode(http::StatusCode::OK, br#"{"items":[]}"#).unwrap(),
            serde_json::json!({"items": []})
        );
        assert!(decode(http::StatusCode::OK, b"<html>").is_err());
    }

    #[test]
    fn a_body_that_is_not_json_names_no_host() {
        let page = b"<html><body>502: upstream gw.example.net (198.51.100.7) refused; try api.example.com:6443</body></html>\n";
        let failure = decode(http::StatusCode::BAD_GATEWAY, page).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Other);
        assert_eq!(
            failure.message,
            "<html><body>502: upstream <address> (<address>) refused; try <address></body></html>"
        );
        // The API server's own message keeps the API group it names.
        let status = br#"{"kind":"Status","status":"Failure","message":"pipelineruns.tekton.dev is forbidden","reason":"Forbidden","code":403}"#;
        let refused = decode(http::StatusCode::FORBIDDEN, status).unwrap_err();
        assert_eq!(refused.message, "pipelineruns.tekton.dev is forbidden");
    }

    #[test]
    fn json_without_kind_status_is_a_body_not_the_servers_error() {
        // A proxy's JSON has the fields of a Status but not its kind.
        let proxy = br#"{"status":"Failure","code":403,"message":"denied by gw.example.net for jane@example.com","reason":"Forbidden"}"#;
        let failure = decode(http::StatusCode::BAD_GATEWAY, proxy).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Other);
        for word in ["gw.example", "jane"] {
            assert!(!failure.message.contains(word), "{}", failure.message);
        }
        assert!(
            failure
                .message
                .contains("denied by <address> for <address>"),
            "{}",
            failure.message
        );
        // The API server's own Status is trusted as before.
        let status = br#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"pods is forbidden","reason":"Forbidden","code":403}"#;
        let refused = decode(http::StatusCode::FORBIDDEN, status).unwrap_err();
        assert_eq!(refused.kind, FailureKind::Forbidden);
        assert_eq!(refused.message, "pods is forbidden");
    }

    #[test]
    fn a_huge_error_body_is_cut_short() {
        let page = "<p>upstream gw.example.net refused</p> ".repeat(1_000_000);
        let failure = decode(http::StatusCode::BAD_GATEWAY, page.as_bytes()).unwrap_err();
        let most = super::super::source::MAX_BODY_MESSAGE_BYTES + '…'.len_utf8();
        assert!(failure.message.len() <= most, "{}", failure.message.len());
        assert!(failure.message.ends_with('…'), "{}", failure.message);
        assert!(!failure.message.contains("gw.example"));
    }

    #[test]
    fn the_cap_holds_on_the_final_message_after_quoting() {
        // `{:?}` writes each control character as `\u{1}`, five bytes, which
        // no JSON parser reads back, so the quoted text stays and is longer
        // than what redact_body kept.
        let page = "\u{1}\u{1} gw.example.net\n".repeat(10_000);
        let failure = decode(http::StatusCode::BAD_GATEWAY, page.as_bytes()).unwrap_err();
        let most = super::super::source::MAX_BODY_MESSAGE_BYTES;
        assert!(failure.message.len() <= most, "{}", failure.message.len());
        assert!(failure.message.ends_with('…'), "{}", failure.message);
        assert!(failure.message.contains(r"\u{1}"), "{}", failure.message);
        assert!(!failure.message.contains("gw.example"));
    }

    #[test]
    fn a_servers_long_message_is_redacted_before_it_is_cut() {
        // No space within 256 bytes of the cut: a cut before redaction would
        // keep `user:sec` and drop the `@` that marks it as a credential.
        let message = format!("denied: {},user:secret@db.example.com", "a".repeat(4_076));
        let status = serde_json::json!({
            "kind": "Status",
            "apiVersion": "v1",
            "status": "Failure",
            "message": message,
            "reason": "Forbidden",
            "code": 403,
        });
        let body = serde_json::to_vec(&status).unwrap();
        let failure = decode(http::StatusCode::FORBIDDEN, &body).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Forbidden);
        assert_eq!(failure.message, message);
        let printed = super::super::source::printable(&failure);
        assert_eq!(printed, format!("denied: {},<address>", "a".repeat(4_076)));
    }

    #[test]
    fn a_webhooks_failure_is_printed_without_its_name_or_the_host_it_looked_up() {
        let message = r#"Internal error occurred: failed calling webhook "validate.example.com": failed to call webhook: Post "https://hooks.shop.svc:443/validate?timeout=10s": dial tcp: lookup hooks.shop.svc on 10.96.0.10:53: no such host"#;
        let status = serde_json::json!({
            "kind": "Status",
            "apiVersion": "v1",
            "status": "Failure",
            "message": message,
            "reason": "InternalError",
            "code": 500,
        });
        let body = serde_json::to_vec(&status).unwrap();
        let failure = decode(http::StatusCode::INTERNAL_SERVER_ERROR, &body).unwrap_err();
        assert_eq!(failure.message, message);
        // The webhook's quoted name, a domain often its owner's, goes too.
        assert_eq!(
            super::super::source::printable(&failure),
            r#"Internal error occurred: failed calling webhook "<redacted>": failed to call webhook: Post "<url>": dial tcp: lookup <address> on <address>: no such host"#
        );
    }

    #[test]
    fn secrets_are_refused_whatever_the_case_of_the_plural() {
        assert!(refuse_secret(&Resource::new("", "v1", "secrets", true)).is_err());
        assert!(refuse_secret(&Resource::new("", "v1", "Secrets", true)).is_err());
        assert!(refuse_secret(&pods()).is_ok());
        // A custom kind that happens to be called secrets in another group
        // is not a core Secret.
        assert!(refuse_secret(&Resource::new("example.test", "v1", "secrets", true)).is_ok());
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
