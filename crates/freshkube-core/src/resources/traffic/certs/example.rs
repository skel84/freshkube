//! Invented certificates for the Certificates page (`FRESHKUBE_CERTS`). The
//! objects are cert-manager's and the Ingresses' as the API writes them and
//! go through the same parsers, chain and view as a cluster's, so the fixture
//! proves the interpretation as well as the page. The Gateways are the
//! Routes example's, whose listeners name `edge-tls`. Times are relative to
//! the `now` the caller reads at. All names are invented.
use super::chain::{Issuance, Lookup};
use super::facts::{CertificateFacts, ChallengeFacts, IngressFacts, OrderFacts, RequestFacts};
use super::read::{CertsRead, Manager};
use super::secret::{SecretOutcome, SecretRequest, TlsFacts};
use crate::failure::{Failure, FailureKind};
use crate::resources::ResourceKind;
use crate::resources::traffic::routes::{self, Listing};
use chrono::{DateTime, SecondsFormat};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    /// Every state at once, grouped.
    #[default]
    All,
    /// Every certificate valid.
    Healthy,
    /// Some expire in under 14 days, one in exactly 14.
    Expiring,
    /// A certificate that doesn't name a host served under it.
    WrongName,
    /// Issuance stopped at the Order.
    StuckOrder,
    /// Issuance stopped at a Challenge.
    StuckChallenge,
    /// No cert-manager in the cluster.
    NotInstalled,
    /// Secrets nobody has read yet, and no cert-manager Certificate.
    SecretUnread,
    /// The same, and reading a Secret is refused.
    SecretRefused,
    /// Nothing is served and cert-manager has no Certificate.
    Empty,
    /// The lists are refused.
    Refused,
    /// The routes are read but the Gateway list is refused, so no listener's
    /// certificate is known.
    GatewaysRefused,
}

impl Shape {
    /// Every word the knob takes: the one list `parse` looks words up in
    /// and `docs/DEBUG_KNOBS.md`'s test compares with the doc's shape lines.
    pub const SHAPES: &'static [(&'static str, Self)] = &[
        ("all", Self::All),
        ("healthy", Self::Healthy),
        ("expiring", Self::Expiring),
        ("wrong-name", Self::WrongName),
        ("stuck-order", Self::StuckOrder),
        ("stuck-challenge", Self::StuckChallenge),
        ("not-installed", Self::NotInstalled),
        ("secret-unread", Self::SecretUnread),
        ("secret-refused", Self::SecretRefused),
        ("empty", Self::Empty),
        ("refused", Self::Refused),
        ("gateways-refused", Self::GatewaysRefused),
    ];

    pub fn parse(name: &str) -> Option<Self> {
        Self::SHAPES
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, shape)| *shape)
    }
}

const DAY: i64 = 86_400;
const HOUR: i64 = 3_600;

fn stamp(at: i64) -> String {
    DateTime::from_timestamp(at, 0)
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn cond(kind: &str, status: &str, reason: &str, message: &str) -> Value {
    json!({"type": kind, "status": status, "reason": reason, "message": message,
        "observedGeneration": 1, "lastTransitionTime": "2026-10-20T09:00:00Z"})
}

/// How a Certificate stands.
#[derive(Clone, Copy)]
enum Standing {
    /// Issued, valid until `now + left`.
    Ready(i64),
    /// Being issued, and not issued before.
    Issuing,
}

fn certificate(
    namespace: &str,
    name: &str,
    secret: &str,
    names: &[&str],
    standing: Standing,
    now: i64,
) -> Value {
    let mut status = match standing {
        Standing::Ready(left) => json!({
            "conditions": [cond("Ready", "True", "Ready", "Certificate is up to date and has not expired")],
            "notBefore": stamp(now - 30 * DAY),
            "notAfter": stamp(now + left),
            "renewalTime": stamp(now + left - 30 * DAY),
            "revision": 1,
        }),
        Standing::Issuing => json!({
            "conditions": [
                cond("Ready", "False", "DoesNotExist", "Issuing certificate as Secret does not exist"),
                cond("Issuing", "True", "DoesNotExist", "Issuing certificate as Secret does not exist")],
            "revision": 0,
        }),
    };
    status["observedGeneration"] = json!(1);
    json!({"apiVersion": "cert-manager.io/v1", "kind": "Certificate",
        "metadata": {"name": name, "namespace": namespace, "generation": 1,
            "uid": format!("cert-{namespace}-{name}")},
        "spec": {"secretName": secret, "dnsNames": names,
            "issuerRef": {"name": "example-ca", "kind": "ClusterIssuer", "group": "cert-manager.io"}},
        "status": status})
}

fn request(namespace: &str, name: &str, owner: &str, conditions: Vec<Value>) -> Value {
    json!({"apiVersion": "cert-manager.io/v1", "kind": "CertificateRequest",
        "metadata": {"name": name, "namespace": namespace, "uid": format!("req-{namespace}-{name}"),
            "annotations": {"cert-manager.io/certificate-revision": "1"},
            "ownerReferences": [{"uid": format!("cert-{namespace}-{owner}"), "kind": "Certificate", "name": owner}]},
        "status": {"conditions": conditions}})
}

fn order(namespace: &str, name: &str, owner: &str, state: &str, reason: &str) -> Value {
    json!({"apiVersion": "acme.cert-manager.io/v1", "kind": "Order",
        "metadata": {"name": name, "namespace": namespace, "uid": format!("order-{namespace}-{name}"),
            "ownerReferences": [{"uid": format!("req-{namespace}-{owner}"), "kind": "CertificateRequest", "name": owner}]},
        "status": {"state": state, "reason": reason}})
}

fn challenge(
    namespace: &str,
    name: &str,
    owner: &str,
    host: &str,
    state: &str,
    reason: &str,
) -> Value {
    json!({"apiVersion": "acme.cert-manager.io/v1", "kind": "Challenge",
        "metadata": {"name": name, "namespace": namespace, "uid": format!("chal-{namespace}-{name}"),
            "ownerReferences": [{"uid": format!("order-{namespace}-{owner}"), "kind": "Order", "name": owner}]},
        "spec": {"type": "HTTP-01", "dnsName": host},
        "status": {"state": state, "reason": reason, "presented": true, "processing": true}})
}

fn ingress(namespace: &str, name: &str, hosts: &[&str], secret: &str) -> Value {
    json!({"apiVersion": "networking.k8s.io/v1", "kind": "Ingress",
        "metadata": {"name": name, "namespace": namespace, "uid": format!("ing-{namespace}-{name}")},
        "spec": {"tls": [{"hosts": hosts, "secretName": secret}],
            "rules": hosts.iter().map(|h| json!({"host": h})).collect::<Vec<_>>()}})
}

struct World {
    certificates: Vec<Value>,
    requests: Vec<Value>,
    orders: Vec<Value>,
    challenges: Vec<Value>,
    ingresses: Vec<Value>,
}

const STUCK_REASON: &str =
    "Waiting for HTTP-01 challenge propagation: wrong status code '404', expected '200'";

fn world(shape: Shape, now: i64) -> World {
    let mut w = World {
        certificates: Vec::new(),
        requests: Vec::new(),
        orders: Vec::new(),
        challenges: Vec::new(),
        ingresses: Vec::new(),
    };
    if matches!(
        shape,
        Shape::NotInstalled | Shape::SecretUnread | Shape::SecretRefused | Shape::Empty
    ) {
        if shape != Shape::Empty {
            w.ingresses = vec![
                ingress(
                    "shop",
                    "web",
                    &["shop.example.com", "www.shop.example.com"],
                    "shop-tls",
                ),
                ingress("payments", "pay", &["pay.example.com"], "pay-tls"),
                ingress("prod-eu", "legacy", &["legacy.example.com"], "legacy-tls"),
                ingress(
                    "prod-eu",
                    "internal",
                    &["internal.example.com"],
                    "internal-tls",
                ),
            ];
        }
        return w;
    }
    // The edge certificate: the Gateway listeners serve it.
    let edge_names: &[&str] = if shape == Shape::WrongName {
        &[
            "shop.example.com",
            "cart.example.com",
            "pay.example.com",
            "checkout.example.com",
            "empty.example.com",
            "ghost.example.com",
            "beta.example.com",
        ]
    } else {
        &["*.example.com"]
    };
    let soon = match shape {
        Shape::Healthy => 60 * DAY,
        _ => 6 * DAY + 12 * HOUR,
    };
    w.certificates.push(certificate(
        "infra",
        "edge",
        "edge-tls",
        edge_names,
        Standing::Ready(60 * DAY),
        now,
    ));
    w.certificates.push(certificate(
        "staging",
        "edge",
        "edge-tls",
        &["*.example.com"],
        Standing::Ready(soon),
        now,
    ));
    let shop_names: &[&str] = if matches!(shape, Shape::Healthy | Shape::Expiring) {
        &["shop.example.com", "www.shop.example.com"]
    } else {
        &["shop.example.com"]
    };
    w.certificates.push(certificate(
        "shop",
        "shop",
        "shop-tls",
        shop_names,
        Standing::Ready(45 * DAY),
        now,
    ));
    let pay_stuck = matches!(shape, Shape::All | Shape::StuckOrder);
    let cart_stuck = matches!(shape, Shape::All | Shape::StuckChallenge);
    let pay_left = if shape == Shape::Expiring {
        14 * DAY + HOUR
    } else {
        30 * DAY
    };
    let cart_left = if shape == Shape::Expiring {
        13 * DAY + 12 * HOUR
    } else {
        30 * DAY
    };
    w.certificates.push(certificate(
        "payments",
        "pay",
        "pay-tls",
        &["pay.example.com"],
        if pay_stuck {
            Standing::Issuing
        } else {
            Standing::Ready(pay_left)
        },
        now,
    ));
    w.certificates.push(certificate(
        "cart",
        "cart",
        "cart-tls",
        &["cart.example.com"],
        if cart_stuck {
            Standing::Issuing
        } else {
            Standing::Ready(cart_left)
        },
        now,
    ));
    w.certificates.push(certificate(
        "prod-eu",
        "unused",
        "unused-tls",
        &["unused.example.com"],
        Standing::Ready(80 * DAY),
        now,
    ));
    w.ingresses = vec![
        ingress(
            "shop",
            "web",
            &["shop.example.com", "www.shop.example.com"],
            "shop-tls",
        ),
        ingress("payments", "pay", &["pay.example.com"], "pay-tls"),
        ingress("cart", "cart", &["cart.example.com"], "cart-tls"),
    ];
    if shape == Shape::All {
        w.ingresses.extend([
            ingress("prod-eu", "legacy", &["legacy.example.com"], "legacy-tls"),
            ingress(
                "prod-eu",
                "internal",
                &["internal.example.com"],
                "internal-tls",
            ),
        ]);
    }
    if pay_stuck {
        let pending = cond(
            "Ready",
            "False",
            "Pending",
            "Waiting on certificate issuance from order payments/pay-1: \"invalid\"",
        );
        w.requests.push(request(
            "payments",
            "pay-1",
            "pay",
            vec![
                cond(
                    "Approved",
                    "True",
                    "cert-manager.io",
                    "Certificate request has been approved by cert-manager.io",
                ),
                pending,
            ],
        ));
        w.orders.push(order(
            "payments",
            "pay-1-4021",
            "pay-1",
            "invalid",
            "Failed to finalize order: the CA rejected the domain pay.example.com",
        ));
    }
    if cart_stuck {
        w.requests.push(request(
            "cart",
            "cart-1",
            "cart",
            vec![
                cond(
                    "Approved",
                    "True",
                    "cert-manager.io",
                    "Certificate request has been approved by cert-manager.io",
                ),
                cond(
                    "Ready",
                    "False",
                    "Pending",
                    "Waiting on certificate issuance from order cart/cart-1-7710: \"pending\"",
                ),
            ],
        ));
        w.orders
            .push(order("cart", "cart-1-7710", "cart-1", "pending", ""));
        w.challenges.push(challenge(
            "cart",
            "cart-1-7710-0",
            "cart-1-7710",
            "cart.example.com",
            "pending",
            STUCK_REASON,
        ));
    }
    w
}

fn listing<T>(objects: &[Value], parse: fn(&Value) -> Option<T>) -> Listing<T> {
    Listing::new(objects.iter().filter_map(parse).collect(), false)
}

fn forbidden(what: &str) -> Failure {
    Failure::new(
        FailureKind::Forbidden,
        format!("{what} is forbidden for the invented viewer"),
    )
}

/// What a read of the invented cluster finds under `shape`, at `now`
/// (Unix seconds).
pub fn read(shape: Shape, now: i64) -> CertsRead {
    let gateway = if shape == Shape::Refused {
        Err(forbidden("gateways.gateway.networking.k8s.io"))
    } else {
        routes::example::read(match shape {
            Shape::Empty => routes::example::Shape::Empty,
            Shape::GatewaysRefused => routes::example::Shape::GatewaysRefused,
            _ => routes::example::Shape::All,
        })
    };
    if shape == Shape::NotInstalled {
        let w = world(shape, now);
        return CertsRead::new(Manager::NotInstalled)
            .with_gateway(gateway)
            .with_ingresses(Ok(listing(&w.ingresses, IngressFacts::parse)));
    }
    if shape == Shape::Refused {
        return CertsRead::new(Manager::Ready { acme: true })
            .with_gateway(gateway)
            .with_ingresses(Err(forbidden("ingresses.networking.k8s.io")))
            .with_certificates(
                Err(forbidden("certificates.cert-manager.io")),
                Issuance::skipped(),
            )
            .with_notes(vec![
                "Certificates could not be read (Forbidden: certificates.cert-manager.io is forbidden for the invented viewer)".into(),
            ]);
    }
    let w = world(shape, now);
    let certificates = listing(&w.certificates, CertificateFacts::parse);
    let busy = !w.requests.is_empty();
    let issuance = if busy {
        Issuance {
            requests: Lookup::Read(Ok(listing(&w.requests, RequestFacts::parse))),
            orders: Lookup::Read(Ok(listing(&w.orders, OrderFacts::parse))),
            challenges: Lookup::Read(Ok(listing(&w.challenges, ChallengeFacts::parse))),
        }
    } else {
        Issuance::skipped()
    };
    CertsRead::new(Manager::Ready { acme: true })
        .with_gateway(gateway)
        .with_ingresses(Ok(listing(&w.ingresses, IngressFacts::parse)))
        .with_certificates(Ok(certificates), issuance)
        .with_certificate_kind(ResourceKind::new(
            "cert-manager.io",
            "v1",
            "Certificate",
            "certificates",
            true,
        ))
}

/// What reading an invented Secret answers: the parsed facts of its
/// certificate, refused under `secret-refused`.
pub fn read_secret(shape: Shape, request: &SecretRequest, now: i64) -> SecretOutcome {
    if shape == Shape::SecretRefused {
        return SecretOutcome::Refused(forbidden("secrets"));
    }
    let facts = |subject: &str, names: &[&str], left: i64| {
        SecretOutcome::Read(TlsFacts {
            subject: format!("CN={subject}"),
            names: names.iter().map(|n| (*n).to_owned()).collect(),
            issuer: "CN=Example Internal CA".into(),
            not_before: now - 100 * DAY,
            not_after: now + left,
            resource_version: Some("48213".into()),
        })
    };
    match (request.namespace(), request.name()) {
        ("infra", "edge-tls") => facts("*.example.com", &["*.example.com"], 60 * DAY),
        ("staging", "edge-tls") => facts("*.example.com", &["*.example.com"], 6 * DAY + 12 * HOUR),
        ("shop", "shop-tls") => facts("shop.example.com", &["shop.example.com"], 45 * DAY),
        ("payments", "pay-tls") => facts("pay.example.com", &["pay.example.com"], 30 * DAY),
        ("prod-eu", "legacy-tls") => {
            facts("legacy.example.com", &["legacy.example.com"], 200 * DAY)
        }
        ("prod-eu", "internal-tls") => {
            facts("internal.example.com", &["internal.example.com"], -3 * DAY)
        }
        _ => SecretOutcome::NotFound,
    }
}

#[cfg(test)]
mod parse_pins {
    use super::*;

    /// Every word the parser takes, with what it means; written down before
    /// the parser looked its words up in a table.
    #[test]
    fn parse_takes_exactly_these_words() {
        let pinned = [
            ("all", Shape::All),
            ("healthy", Shape::Healthy),
            ("expiring", Shape::Expiring),
            ("wrong-name", Shape::WrongName),
            ("stuck-order", Shape::StuckOrder),
            ("stuck-challenge", Shape::StuckChallenge),
            ("not-installed", Shape::NotInstalled),
            ("secret-unread", Shape::SecretUnread),
            ("secret-refused", Shape::SecretRefused),
            ("empty", Shape::Empty),
            ("refused", Shape::Refused),
            ("gateways-refused", Shape::GatewaysRefused),
        ];
        for (word, shape) in pinned {
            assert!(
                matches!(Shape::parse(word), Some(found) if found == shape),
                "{word}"
            );
        }
        for unknown in ["", " ", "nope", "Firing", "all ", " all", "ALL"] {
            assert!(Shape::parse(unknown).is_none(), "{unknown:?}");
        }
    }
}
