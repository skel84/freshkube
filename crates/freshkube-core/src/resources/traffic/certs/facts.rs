//! The fields of cert-manager's Certificate, CertificateRequest, Order and
//! Challenge, and of an Ingress's TLS, that the page reads, parsed from the
//! objects as the API returned them. A missing field reads as absent, never
//! as a failure. A Certificate's facts here come from its spec and status;
//! none of this reads a Secret.
use crate::resources::traffic::routes::{Cond, conditions};
use chrono::DateTime;
use serde_json::Value;

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

fn opt(value: &Value) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| opt(s))
        .collect()
}

/// An RFC 3339 time as Unix seconds.
fn time(value: &Value) -> Option<i64> {
    DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|t| t.timestamp())
}

/// The UIDs of an object's owners, which is how cert-manager chains its
/// objects: a request is owned by its Certificate, an Order by the request,
/// a Challenge by the Order.
fn owners(meta: &Value) -> Vec<String> {
    meta["ownerReferences"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|o| opt(&o["uid"]))
        .collect()
}

/// A reference to a Secret or an object by `namespace/name`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuerRef {
    pub kind: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateFacts {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub generation: i64,
    /// `spec.secretName`: the Secret cert-manager keeps this certificate in.
    pub secret_name: String,
    /// `spec.dnsNames` and the common name: what the Certificate asks for.
    pub names: Vec<String>,
    pub issuer: Option<IssuerRef>,
    pub ready: Option<Cond>,
    pub issuing: Option<Cond>,
    pub not_before: Option<i64>,
    pub not_after: Option<i64>,
    pub renewal_time: Option<i64>,
    /// `status.revision`: the revision last issued.
    pub revision: Option<i64>,
    pub last_failure: Option<i64>,
}

impl CertificateFacts {
    pub fn parse(object: &Value) -> Option<Self> {
        let meta = &object["metadata"];
        let spec = &object["spec"];
        let status = &object["status"];
        let conds = conditions(&status["conditions"]);
        let mut names = strings(&spec["dnsNames"]);
        if let Some(common) = opt(&spec["commonName"]).filter(|c| !names.contains(c)) {
            names.push(common);
        }
        Some(Self {
            namespace: text(&meta["namespace"]),
            name: opt(&meta["name"])?,
            uid: text(&meta["uid"]),
            generation: meta["generation"].as_i64().unwrap_or(0),
            secret_name: text(&spec["secretName"]),
            names,
            issuer: opt(&spec["issuerRef"]["name"]).map(|name| IssuerRef {
                kind: opt(&spec["issuerRef"]["kind"]).unwrap_or_else(|| "Issuer".into()),
                name,
            }),
            ready: conds.iter().find(|c| c.kind == "Ready").cloned(),
            issuing: conds.iter().find(|c| c.kind == "Issuing").cloned(),
            not_before: time(&status["notBefore"]),
            not_after: time(&status["notAfter"]),
            renewal_time: time(&status["renewalTime"]),
            revision: status["revision"].as_i64(),
            last_failure: time(&status["lastFailureTime"]),
        })
    }

    pub fn key(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestFacts {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub owners: Vec<String>,
    /// The `cert-manager.io/certificate-revision` annotation.
    pub revision: Option<i64>,
    pub conditions: Vec<Cond>,
    pub failure_time: Option<i64>,
}

impl RequestFacts {
    pub fn parse(object: &Value) -> Option<Self> {
        let meta = &object["metadata"];
        Some(Self {
            namespace: text(&meta["namespace"]),
            name: opt(&meta["name"])?,
            uid: text(&meta["uid"]),
            owners: owners(meta),
            revision: meta["annotations"]["cert-manager.io/certificate-revision"]
                .as_str()
                .and_then(|r| r.parse().ok()),
            conditions: conditions(&object["status"]["conditions"]),
            failure_time: time(&object["status"]["failureTime"]),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderFacts {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub owners: Vec<String>,
    /// `status.state`: pending, ready, processing, valid, invalid, expired
    /// or errored; empty before cert-manager writes one.
    pub state: String,
    pub reason: String,
    pub failure_time: Option<i64>,
}

impl OrderFacts {
    pub fn parse(object: &Value) -> Option<Self> {
        let meta = &object["metadata"];
        let status = &object["status"];
        Some(Self {
            namespace: text(&meta["namespace"]),
            name: opt(&meta["name"])?,
            uid: text(&meta["uid"]),
            owners: owners(meta),
            state: text(&status["state"]),
            reason: text(&status["reason"]),
            failure_time: time(&status["failureTime"]),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChallengeFacts {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub owners: Vec<String>,
    /// `HTTP-01` or `DNS-01`.
    pub kind: String,
    pub dns_name: String,
    pub state: String,
    pub reason: String,
    pub presented: bool,
}

impl ChallengeFacts {
    pub fn parse(object: &Value) -> Option<Self> {
        let meta = &object["metadata"];
        let (spec, status) = (&object["spec"], &object["status"]);
        Some(Self {
            namespace: text(&meta["namespace"]),
            name: opt(&meta["name"])?,
            uid: text(&meta["uid"]),
            owners: owners(meta),
            kind: text(&spec["type"]),
            dns_name: text(&spec["dnsName"]),
            state: text(&status["state"]),
            reason: text(&status["reason"]),
            presented: status["presented"].as_bool().unwrap_or(false),
        })
    }
}

/// One `spec.tls` entry of an Ingress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngressTls {
    pub hosts: Vec<String>,
    /// Empty when the entry names no Secret (the controller's default
    /// certificate answers).
    pub secret: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngressFacts {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub tls: Vec<IngressTls>,
    /// The hosts of `spec.rules`.
    pub hosts: Vec<String>,
}

impl IngressFacts {
    pub fn parse(object: &Value) -> Option<Self> {
        let meta = &object["metadata"];
        let spec = &object["spec"];
        Some(Self {
            namespace: text(&meta["namespace"]),
            name: opt(&meta["name"])?,
            uid: text(&meta["uid"]),
            tls: spec["tls"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|t| IngressTls {
                    hosts: strings(&t["hosts"]),
                    secret: opt(&t["secretName"]),
                })
                .collect(),
            hosts: spec["rules"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| opt(&r["host"]))
                .collect(),
        })
    }

    pub fn key(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}
