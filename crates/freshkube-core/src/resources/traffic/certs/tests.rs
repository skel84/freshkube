use super::*;
use crate::failure::{Failure, FailureKind};
use crate::resources::traffic::routes::{Cond, Listing, Tri};
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;
use serde_json::{Value, json};
use std::collections::BTreeMap;

const DAY: i64 = 86_400;
/// A certificate for `shop.example.com`, `*.shop.example.com` and
/// `cart.example.com`, valid from 2026-11-09 for a hundred years. Invented
/// for these tests; its key was never kept.
const CERTIFICATE: &str = "-----BEGIN CERTIFICATE-----\nMIICADCCAaagAwIBAgIUOhnyXcm27sSCpL9xjX4bei+ekJwwCgYIKoZIzj0EAwIw\nMjEZMBcGA1UEAwwQc2hvcC5leGFtcGxlLmNvbTEVMBMGA1UECgwMRXhhbXBsZSBT\naG9wMCAXDTI2MTEwOTIxNDYxM1oYDzIxMjYxMDE2MjE0NjEzWjAyMRkwFwYDVQQD\nDBBzaG9wLmV4YW1wbGUuY29tMRUwEwYDVQQKDAxFeGFtcGxlIFNob3AwWTATBgcq\nhkjOPQIBBggqhkjOPQMBBwNCAARzsNDYOPxVRqkQ8N+gaIWCj3f/g2/Gh76g9/Z/\n0sagCMrqA4KVjvddDdFs8NoA1El6nyZEge84DhEpdTRHuzdoo4GXMIGUMB0GA1Ud\nDgQWBBSZ0XFpH+bNirc2Dv2yPTxABnqZJjAfBgNVHSMEGDAWgBSZ0XFpH+bNirc2\nDv2yPTxABnqZJjAPBgNVHRMBAf8EBTADAQH/MEEGA1UdEQQ6MDiCEHNob3AuZXhh\nbXBsZS5jb22CEiouc2hvcC5leGFtcGxlLmNvbYIQY2FydC5leGFtcGxlLmNvbTAK\nBggqhkjOPQQDAgNIADBFAiEA076TQu2nkuFo/l+xr9pCpOB+r60U8+0pXoEPtLy0\nC7YCIGDYixduyZHEIaskICDF5U1uPY46OeboiqCg1q/EHs8H\n-----END CERTIFICATE-----\n";
const NOT_AFTER: i64 = 4_947_860_773;
/// Stands in for a private key. If it shows up anywhere it leaked.
const KEY_MARKER: &str = "PRIVATE-KEY-MARKER-0f3a9c";

fn cond(kind: &str, status: Tri, reason: &str, message: &str) -> Cond {
    Cond {
        kind: kind.into(),
        status,
        reason: reason.into(),
        message: message.into(),
        generation: Some(1),
    }
}

fn certificate(issuing: bool, ready: Option<Tri>) -> CertificateFacts {
    CertificateFacts {
        namespace: "shop".into(),
        name: "shop".into(),
        uid: "cert-1".into(),
        generation: 1,
        secret_name: "shop-tls".into(),
        names: vec!["shop.example.com".into()],
        issuer: Some(IssuerRef {
            kind: "ClusterIssuer".into(),
            name: "example-ca".into(),
        }),
        ready: ready.map(|status| cond("Ready", status, "Reason", "")),
        issuing: issuing.then(|| cond("Issuing", Tri::True, "Renewing", "Issuing certificate")),
        not_before: None,
        not_after: None,
        renewal_time: None,
        revision: Some(0),
        last_failure: None,
    }
}

fn request(conditions: Vec<Cond>) -> RequestFacts {
    RequestFacts {
        namespace: "shop".into(),
        name: "shop-1".into(),
        uid: "req-1".into(),
        owners: vec!["cert-1".into()],
        revision: Some(1),
        conditions,
        failure_time: None,
    }
}

fn order(state: &str, reason: &str) -> OrderFacts {
    OrderFacts {
        namespace: "shop".into(),
        name: "shop-1-1".into(),
        uid: "order-1".into(),
        owners: vec!["req-1".into()],
        state: state.into(),
        reason: reason.into(),
        failure_time: None,
    }
}

fn challenge(state: &str, reason: &str) -> ChallengeFacts {
    ChallengeFacts {
        namespace: "shop".into(),
        name: "shop-1-1-0".into(),
        uid: "chal-1".into(),
        owners: vec!["order-1".into()],
        kind: "HTTP-01".into(),
        dns_name: "shop.example.com".into(),
        state: state.into(),
        reason: reason.into(),
        presented: true,
    }
}

fn reads(
    requests: Vec<RequestFacts>,
    orders: Vec<OrderFacts>,
    challenges: Vec<ChallengeFacts>,
) -> Issuance {
    Issuance {
        requests: Lookup::Read(Ok(Listing::new(requests, false))),
        orders: Lookup::Read(Ok(Listing::new(orders, false))),
        challenges: Lookup::Read(Ok(Listing::new(challenges, false))),
    }
}

fn approved() -> Cond {
    cond("Approved", Tri::True, "cert-manager.io", "approved")
}

fn verdicts(chain: &Chain) -> Vec<Verdict> {
    chain.steps.iter().map(|s| s.verdict).collect()
}

fn pending() -> Cond {
    cond("Ready", Tri::False, "Pending", "waiting on the order")
}

// ---- names ----

#[test]
fn a_wildcard_covers_exactly_one_label() {
    for (name, host, covered) in [
        ("shop.example.com", "shop.example.com", true),
        ("Shop.Example.com", "shop.example.COM", true),
        ("*.example.com", "shop.example.com", true),
        ("*.example.com", "example.com", false),
        ("*.example.com", "a.shop.example.com", false),
        ("*.example.com", ".example.com", false),
        ("*.example.com", "*.example.com", true),
        ("shop.example.com", "*.example.com", false),
        ("*.example.com", "shop.example.org", false),
        ("example.com", "shop.example.com", false),
        ("*.example.com.", "shop.example.com", true),
    ] {
        assert_eq!(covers(name, host), covered, "{name} vs {host}");
    }
}

#[test]
fn uncovered_lists_each_missing_host_once_in_order() {
    let names = vec![
        "shop.example.com".to_owned(),
        "*.cart.example.com".to_owned(),
    ];
    let hosts: Vec<String> = [
        "b.example.com",
        "shop.example.com",
        "a.cart.example.com",
        "b.example.com",
        "a.example.com",
    ]
    .map(String::from)
    .into();
    assert_eq!(
        uncovered(&names, &hosts),
        ["b.example.com", "a.example.com"]
    );
}

// ---- the chain ----

#[test]
fn an_issued_certificate_has_nothing_in_flight() {
    let chain = issuance_chain(&certificate(false, Some(Tri::True)), &Issuance::skipped());
    assert_eq!(
        verdicts(&chain),
        [
            Verdict::Done,
            Verdict::NotUsed,
            Verdict::NotUsed,
            Verdict::NotUsed
        ]
    );
    assert!(chain.is_finished());
    assert_eq!(chain.line(), "issued");
}

#[test]
fn a_failed_certificate_stops_at_itself_and_the_rest_is_not_reached() {
    let chain = issuance_chain(
        &certificate(false, Some(Tri::False)),
        &reads(vec![], vec![], vec![]),
    );
    assert_eq!(
        verdicts(&chain),
        [
            Verdict::Broken,
            Verdict::NotReached,
            Verdict::NotReached,
            Verdict::NotReached
        ]
    );
    assert_eq!(chain.stopped_at().unwrap().kind, StepKind::Certificate);
}

#[test]
fn no_request_yet_is_in_progress_at_the_request() {
    let chain = issuance_chain(
        &certificate(true, Some(Tri::False)),
        &reads(vec![], vec![], vec![]),
    );
    assert_eq!(chain.steps[1].verdict, Verdict::Waiting);
    assert!(
        chain.steps[1]
            .says
            .contains("has not made a CertificateRequest")
    );
    assert_eq!(chain.stopped_at().unwrap().kind, StepKind::Request);
}

#[test]
fn a_denied_request_breaks_there_and_order_and_challenge_are_not_reached() {
    let denied = cond("Denied", Tri::True, "Policy", "denied by the approver");
    let chain = issuance_chain(
        &certificate(true, Some(Tri::False)),
        &reads(
            vec![request(vec![denied])],
            vec![order("pending", "")],
            vec![],
        ),
    );
    assert_eq!(
        verdicts(&chain),
        [
            Verdict::Done,
            Verdict::Broken,
            Verdict::NotReached,
            Verdict::NotReached
        ]
    );
    assert!(chain.steps[1].says.contains("denied by the approver"));
}

#[test]
fn an_invalid_order_breaks_at_the_order_and_the_challenge_is_not_reached() {
    let chain = issuance_chain(
        &certificate(true, Some(Tri::False)),
        &reads(
            vec![request(vec![approved(), pending()])],
            vec![order("invalid", "the CA rejected the domain")],
            vec![challenge("pending", "")],
        ),
    );
    assert_eq!(
        verdicts(&chain),
        [
            Verdict::Done,
            Verdict::Waiting,
            Verdict::Broken,
            Verdict::NotReached
        ]
    );
    let at = chain.stopped_at().unwrap();
    assert_eq!(at.kind, StepKind::Order);
    assert!(at.says.contains("the CA rejected the domain"));
    assert_eq!(
        chain.line(),
        "stopped at Order — Order is invalid: the CA rejected the domain"
    );
}

#[test]
fn a_pending_challenge_with_a_reason_is_where_it_stopped() {
    let chain = issuance_chain(
        &certificate(true, Some(Tri::False)),
        &reads(
            vec![request(vec![approved(), pending()])],
            vec![order("pending", "")],
            vec![challenge("pending", "wrong status code '404'")],
        ),
    );
    assert_eq!(
        verdicts(&chain),
        [
            Verdict::Done,
            Verdict::Waiting,
            Verdict::Waiting,
            Verdict::Broken
        ]
    );
    let at = chain.stopped_at().unwrap();
    assert_eq!(at.kind, StepKind::Challenge);
    assert!(
        at.says
            .contains("HTTP-01 challenge for shop.example.com is pending: wrong status code '404'")
    );
}

#[test]
fn a_pending_challenge_with_no_reason_is_only_in_progress() {
    let chain = issuance_chain(
        &certificate(true, Some(Tri::False)),
        &reads(
            vec![request(vec![approved(), pending()])],
            vec![order("pending", "")],
            vec![challenge("pending", "")],
        ),
    );
    assert!(!chain.is_broken());
    assert_eq!(chain.stopped_at().unwrap().kind, StepKind::Challenge);
    assert!(chain.line().starts_with("in progress at Challenge"));
}

#[test]
fn an_order_with_no_challenge_yet_waits_and_a_valid_one_needs_none() {
    let waiting = issuance_chain(
        &certificate(true, Some(Tri::False)),
        &reads(
            vec![request(vec![approved(), pending()])],
            vec![order("pending", "")],
            vec![],
        ),
    );
    assert_eq!(waiting.steps[3].verdict, Verdict::Waiting);
    let valid = issuance_chain(
        &certificate(true, Some(Tri::False)),
        &reads(
            vec![request(vec![approved(), pending()])],
            vec![order("valid", "")],
            vec![],
        ),
    );
    assert_eq!(valid.steps[2].verdict, Verdict::Done);
    assert_eq!(valid.steps[3].verdict, Verdict::NotUsed);
}

#[test]
fn a_signed_request_ends_the_chain_without_an_order() {
    let ready = cond(
        "Ready",
        Tri::True,
        "Issued",
        "Certificate fetched from issuer",
    );
    let chain = issuance_chain(
        &certificate(true, Some(Tri::False)),
        &reads(vec![request(vec![approved(), ready])], vec![], vec![]),
    );
    assert_eq!(
        verdicts(&chain),
        [
            Verdict::Done,
            Verdict::Done,
            Verdict::NotUsed,
            Verdict::NotUsed
        ]
    );
    assert!(chain.is_finished());
}

#[test]
fn an_issuer_that_is_not_acme_makes_no_order() {
    let not_acme = Issuance {
        requests: Lookup::Read(Ok(Listing::new(
            vec![request(vec![approved(), pending()])],
            false,
        ))),
        orders: Lookup::NotServed,
        challenges: Lookup::NotServed,
    };
    let chain = issuance_chain(&certificate(true, Some(Tri::False)), &not_acme);
    assert_eq!(
        verdicts(&chain),
        [
            Verdict::Done,
            Verdict::Waiting,
            Verdict::NotUsed,
            Verdict::NotUsed
        ]
    );
    assert_eq!(chain.stopped_at().unwrap().kind, StepKind::Request);
}

#[test]
fn a_refused_or_cut_list_is_unknown_and_never_an_empty_answer() {
    let refused = Failure::new(FailureKind::Forbidden, "orders is forbidden");
    let chain = issuance_chain(
        &certificate(true, Some(Tri::False)),
        &Issuance {
            requests: Lookup::Read(Ok(Listing::new(
                vec![request(vec![approved(), pending()])],
                false,
            ))),
            orders: Lookup::Read(Err(refused.clone())),
            challenges: Lookup::Read(Ok(Listing::new(vec![], false))),
        },
    );
    assert_eq!(chain.steps[2].verdict, Verdict::Unknown);
    assert_eq!(chain.steps[3].verdict, Verdict::NotReached);
    assert!(chain.steps[2].says.contains("Forbidden"));
    let cut = issuance_chain(
        &certificate(true, Some(Tri::False)),
        &Issuance {
            requests: Lookup::Read(Ok(Listing::new(vec![], true))),
            orders: Lookup::Skipped,
            challenges: Lookup::Skipped,
        },
    );
    assert_eq!(cut.steps[1].verdict, Verdict::Unknown);
    assert!(cut.steps[1].says.contains("cut"));
}

#[test]
fn only_the_requests_of_this_certificate_and_its_latest_revision_count() {
    let mut other = request(vec![cond("Denied", Tri::True, "Policy", "not mine")]);
    other.owners = vec!["cert-2".into()];
    let mut old = request(vec![cond("Denied", Tri::True, "Policy", "an old one")]);
    old.revision = Some(1);
    old.name = "shop-1".into();
    let mut new = request(vec![approved(), pending()]);
    new.revision = Some(2);
    new.name = "shop-2".into();
    new.uid = "req-2".into();
    let mut o = order("pending", "");
    o.owners = vec!["req-2".into()];
    let chain = issuance_chain(
        &certificate(true, Some(Tri::False)),
        &reads(vec![other, old, new], vec![o], vec![]),
    );
    assert_eq!(chain.steps[1].object.as_deref(), Some("shop-2"));
    assert_eq!(chain.steps[1].verdict, Verdict::Waiting);
    assert_eq!(chain.steps[2].verdict, Verdict::Waiting);
}

// ---- entries ----

fn entry(names: Option<&[&str]>, hosts: &[&str], not_after: Option<i64>, now: i64) -> CertEntry {
    let mut entry = CertEntry {
        namespace: "shop".into(),
        secret: "shop-tls".into(),
        servings: vec![Serving {
            via: Via::Ingress {
                ingress: "shop/web".into(),
            },
            hosts: hosts.iter().map(|h| (*h).to_owned()).collect(),
        }],
        managed: None,
        chain: None,
        secret_state: SecretState::NotRead,
        names: None,
        subject: None,
        issuer: None,
        not_before: None,
        not_after: None,
        wrong: vec![],
        state: CertState::Unknown,
        reason: String::new(),
        days_left: None,
    };
    if let Some(names) = names {
        let mut cert = certificate(false, Some(Tri::True));
        cert.names = names.iter().map(|n| (*n).to_owned()).collect();
        cert.not_after = not_after;
        entry.chain = Some(issuance_chain(&cert, &Issuance::skipped()));
        entry.managed = Some(cert);
    }
    entry.assess(now);
    entry
}

#[test]
fn a_host_the_certificate_does_not_name_is_a_wrong_name() {
    let now = 1_000_000;
    let e = entry(
        Some(&["shop.example.com"]),
        &["shop.example.com", "www.shop.example.com"],
        Some(now + 40 * DAY),
        now,
    );
    assert_eq!(e.state, CertState::WrongName);
    assert_eq!(e.wrong, ["www.shop.example.com"]);
    assert_eq!(e.reason, "does not name www.shop.example.com");
    let fine = entry(
        Some(&["*.example.com"]),
        &["shop.example.com", "www.example.com"],
        Some(now + 40 * DAY),
        now,
    );
    assert_eq!(fine.state, CertState::Healthy);
}

#[test]
fn expiry_under_fourteen_days_reads_expiring_and_fourteen_does_not() {
    let now = 1_000_000;
    let at = |days: i64, hours: i64| {
        entry(
            Some(&["a.example.com"]),
            &["a.example.com"],
            Some(now + days * DAY + hours * 3600),
            now,
        )
    };
    assert_eq!(at(13, 23).state, CertState::Expiring);
    assert_eq!(at(13, 23).days_left, Some(13));
    assert_eq!(at(14, 0).state, CertState::Healthy);
    assert_eq!(at(0, 5).state, CertState::Expiring);
    assert_eq!(at(0, 5).reason, "expires in 0 days");
    assert_eq!(at(1, 1).reason, "expires in 1 day");
    let expired = entry(
        Some(&["a.example.com"]),
        &["a.example.com"],
        Some(now - 1),
        now,
    );
    assert_eq!(expired.state, CertState::Broken);
    assert_eq!(expired.reason, "expired");
    // A wrong name outranks an expiring certificate, and expired outranks both.
    let both = entry(
        Some(&["a.example.com"]),
        &["b.example.com"],
        Some(now + DAY),
        now,
    );
    assert_eq!(both.state, CertState::WrongName);
}

#[test]
fn nothing_is_judged_without_the_names() {
    let now = 1_000_000;
    let e = entry(None, &["shop.example.com"], None, now);
    assert_eq!(e.state, CertState::Unknown);
    assert!(e.wrong.is_empty());
    assert!(e.reason.contains("Secret not read"));
    assert!(e.can_read_secret());
    // An Ingress that names no host cannot be wrong.
    let silent = entry(Some(&["a.example.com"]), &[], Some(now + 40 * DAY), now);
    assert_eq!(silent.state, CertState::Healthy);
}

#[test]
fn a_secret_read_fills_the_names_and_the_state_follows() {
    let now = 1_000_000;
    let mut e = entry(None, &["shop.example.com", "cart.example.com"], None, now);
    let outcome = SecretOutcome::Read(TlsFacts {
        subject: "CN=shop.example.com".into(),
        names: vec!["shop.example.com".into()],
        issuer: "CN=Example CA".into(),
        not_before: now - DAY,
        not_after: now + 90 * DAY,
    });
    let mut view = CertsView::new(&CertsRead::new(Manager::NotInstalled), now);
    view.entries.push(e.clone());
    view.apply_secret("shop/shop-tls", outcome, now);
    e = view.entries.remove(0);
    assert_eq!(e.state, CertState::WrongName);
    assert_eq!(e.wrong, ["cart.example.com"]);
    assert_eq!(e.subject.as_deref(), Some("CN=shop.example.com"));
    assert_eq!(e.source(), "the Secret's certificate, read on request");
    assert!(!e.can_read_secret(), "it was read once already");
}

#[test]
fn each_secret_outcome_has_its_own_words_and_a_refusal_is_unknown() {
    let now = 1_000_000;
    let mut view = CertsView::new(&CertsRead::new(Manager::NotInstalled), now);
    for (outcome, state, words) in [
        (SecretOutcome::NotFound, CertState::Broken, "does not exist"),
        (
            SecretOutcome::NoCertificate,
            CertState::Broken,
            "holds no tls.crt",
        ),
        (
            SecretOutcome::NotTls {
                kind: "Opaque".into(),
            },
            CertState::Broken,
            "of type Opaque",
        ),
        (
            SecretOutcome::Unparseable("bad".into()),
            CertState::Broken,
            "not a certificate: bad",
        ),
        (
            SecretOutcome::Refused(Failure::new(FailureKind::Forbidden, "secrets is forbidden")),
            CertState::Unknown,
            "Forbidden: secrets is forbidden",
        ),
    ] {
        view.entries = vec![entry(None, &["shop.example.com"], None, now)];
        view.apply_secret("shop/shop-tls", outcome, now);
        assert_eq!(view.entries[0].state, state, "{words}");
        assert!(
            view.entries[0].reason.contains(words),
            "{}",
            view.entries[0].reason
        );
    }
}

// ---- reading a Secret ----

fn tls_secret(crt: &str, key: &str) -> Secret {
    let mut data = BTreeMap::new();
    data.insert("tls.crt".to_owned(), ByteString(crt.as_bytes().to_vec()));
    data.insert("tls.key".to_owned(), ByteString(key.as_bytes().to_vec()));
    data.insert("ca.crt".to_owned(), ByteString(b"another value".to_vec()));
    Secret {
        type_: Some("kubernetes.io/tls".into()),
        data: Some(data),
        ..Secret::default()
    }
}

#[test]
fn the_certificates_facts_are_parsed_and_only_they_leave() {
    let outcome = facts_from_secret(tls_secret(CERTIFICATE, KEY_MARKER));
    let SecretOutcome::Read(facts) = &outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(facts.subject, "CN=shop.example.com, O=Example Shop");
    assert_eq!(facts.issuer, "CN=shop.example.com, O=Example Shop");
    assert_eq!(
        facts.names,
        ["shop.example.com", "*.shop.example.com", "cart.example.com"]
    );
    assert_eq!(facts.not_after, NOT_AFTER);
    assert!(facts.not_before < facts.not_after);
}

#[test]
fn safety_g21_tls_key_never_leaves_core() {
    let outcome = facts_from_secret(tls_secret(CERTIFICATE, KEY_MARKER));
    let SecretOutcome::Read(_) = &outcome else {
        panic!("{outcome:?}");
    };
    // The key, the other keys and the raw PEM are in no word of the result.
    let shown = format!("{outcome:?}");
    assert!(!shown.contains(KEY_MARKER), "{shown}");
    assert!(!shown.contains("another value"), "{shown}");
    assert!(!shown.contains("BEGIN CERTIFICATE"), "{shown}");
    assert!(!shown.contains("MIIB"), "{shown}");
    // Nor in a view that holds it, or in what a row would say.
    let mut view = CertsView::new(&CertsRead::new(Manager::NotInstalled), 0);
    view.entries = vec![entry(None, &["shop.example.com"], None, 0)];
    view.apply_secret("shop/shop-tls", outcome, 0);
    let shown = format!("{view:?}");
    assert!(!shown.contains(KEY_MARKER), "{shown}");
    assert!(!shown.contains("another value"), "{shown}");
}

#[test]
fn a_key_that_is_not_a_certificate_is_unparseable_and_never_quoted() {
    for crt in [
        "",
        "not pem",
        "-----BEGIN PRIVATE KEY-----\nQUJD\n-----END PRIVATE KEY-----\n",
    ] {
        let outcome = facts_from_secret(tls_secret(crt, KEY_MARKER));
        let SecretOutcome::Unparseable(why) = &outcome else {
            panic!("{outcome:?}");
        };
        assert!(!why.contains("QUJD") && !why.contains(KEY_MARKER), "{why}");
    }
    let plain = Secret {
        type_: Some("Opaque".into()),
        ..Secret::default()
    };
    assert_eq!(
        facts_from_secret(plain),
        SecretOutcome::NotTls {
            kind: "Opaque".into()
        }
    );
    let mut empty = tls_secret(CERTIFICATE, KEY_MARKER);
    empty.data.as_mut().unwrap().remove("tls.crt");
    assert_eq!(facts_from_secret(empty), SecretOutcome::NoCertificate);
}

// ---- the reads ----

const GROUPS: &str = "/apis";

fn groups(names: &[&str]) -> (u16, String) {
    (
        200,
        json!({"groups": names.iter().map(|name| json!({"name": name,
            "versions": [{"groupVersion": format!("{name}/v1"), "version": "v1"}],
            "preferredVersion": {"groupVersion": format!("{name}/v1"), "version": "v1"}})).collect::<Vec<_>>()})
        .to_string(),
    )
}

fn resources(kinds: &[(&str, &str)]) -> (u16, String) {
    (
        200,
        json!({"resources": kinds.iter().map(|(name, kind)| json!({
            "name": name, "kind": kind, "namespaced": true,
            "verbs": ["get", "list", "watch"]})).collect::<Vec<_>>()})
        .to_string(),
    )
}

fn list(items: Vec<Value>) -> (u16, String) {
    (200, json!({"metadata": {}, "items": items}).to_string())
}

fn ready_certificate(secret: &str) -> Value {
    json!({"metadata": {"name": "shop", "namespace": "shop", "uid": "cert-1"},
        "spec": {"secretName": secret, "dnsNames": ["shop.example.com"]},
        "status": {"conditions": [{"type": "Ready", "status": "True"}],
            "notBefore": "2026-10-01T00:00:00Z", "notAfter": "2027-01-01T00:00:00Z"}})
}

fn ingress(namespace: &str, name: &str, secret: &str) -> Value {
    json!({"metadata": {"name": name, "namespace": namespace},
        "spec": {"tls": [{"hosts": ["shop.example.com"], "secretName": secret}]}})
}

fn cluster(request: &crate::fake_api::Recorded) -> Option<(u16, String)> {
    match request.path.as_str() {
        GROUPS => Some(groups(&["cert-manager.io", "acme.cert-manager.io"])),
        "/apis/cert-manager.io/v1" => Some(resources(&[
            ("certificates", "Certificate"),
            ("certificaterequests", "CertificateRequest"),
        ])),
        "/apis/acme.cert-manager.io/v1" => Some(resources(&[
            ("orders", "Order"),
            ("challenges", "Challenge"),
        ])),
        "/apis/cert-manager.io/v1/certificates" => Some(list(vec![ready_certificate("shop-tls")])),
        "/apis/networking.k8s.io/v1/ingresses" => Some(list(vec![
            ingress("shop", "web", "shop-tls"),
            ingress("shop", "old", "legacy-tls"),
        ])),
        "/api/v1/namespaces/shop/secrets/legacy-tls" => Some((
            200,
            json!({"metadata": {"name": "legacy-tls", "namespace": "shop"},
                "type": "kubernetes.io/tls",
                "data": {"tls.crt": base64_of(CERTIFICATE), "tls.key": base64_of(KEY_MARKER)}})
            .to_string(),
        )),
        _ => None,
    }
}

fn base64_of(text: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(text)
}

#[tokio::test]
async fn safety_g21_the_page_read_asks_for_no_secret_and_a_managed_secret_is_never_requested() {
    let (client, recorder) = crate::fake_api::client(cluster);
    let read = read_certificates(&client).await;
    assert_eq!(read.manager, Manager::Ready { acme: true });
    let view = CertsView::new(&read, 1_800_000_000);
    let lines = recorder.lines();
    // Lists and gets only, and nothing that names a Secret.
    recorder.assert_all("the certificates read only reads", |r| {
        r.method == http::Method::GET
    });
    assert!(!lines.iter().any(|l| l.contains("secrets")), "{lines:?}");
    // The Secret a Certificate answers for has its facts from the Certificate
    // and offers no read at all.
    let managed = view.entry("shop/shop-tls").unwrap();
    assert!(managed.managed.is_some());
    assert!(managed.secret_request().is_none());
    assert!(!managed.can_read_secret());
    assert!(managed.not_after.is_some() && managed.names.is_some());
    assert_eq!(managed.source(), "cert-manager Certificate shop/shop");
    // The other one is not read until the person asks, and may be.
    let other = view.entry("shop/legacy-tls").unwrap();
    assert_eq!(other.state, CertState::Unknown);
    assert!(other.can_read_secret());
    assert!(other.secret_request().is_some());
    // A read applied to a managed Secret changes nothing.
    let mut view = view;
    view.apply_secret("shop/shop-tls", SecretOutcome::NotFound, 1_800_000_000);
    assert_eq!(
        view.entry("shop/shop-tls").unwrap().secret_state,
        SecretState::NotRead
    );
}

#[tokio::test]
async fn safety_g21_a_secret_is_read_only_by_name_and_with_one_get() {
    let (client, recorder) = crate::fake_api::client(cluster);
    let read = read_certificates(&client).await;
    let view = CertsView::new(&read, 1_800_000_000);
    let before = recorder.lines().len();
    let request = view
        .entry("shop/legacy-tls")
        .unwrap()
        .secret_request()
        .unwrap();
    let outcome = read_tls_secret(&client, &request).await;
    let lines = recorder.lines();
    assert_eq!(
        &lines[before..],
        ["GET /api/v1/namespaces/shop/secrets/legacy-tls"]
    );
    let SecretOutcome::Read(facts) = &outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(facts.not_after, NOT_AFTER);
    assert!(!format!("{outcome:?}").contains(KEY_MARKER));
}

#[tokio::test]
async fn safety_g21_a_refused_or_missing_secret_is_unknown_or_not_found_never_wrong() {
    let (client, _) = crate::fake_api::client(|request| match request.path.as_str() {
        "/api/v1/namespaces/shop/secrets/hidden" => Some((
            403,
            json!({"kind": "Status", "apiVersion": "v1", "status": "Failure", "code": 403,
                "reason": "Forbidden", "message": "secrets \"hidden\" is forbidden"})
            .to_string(),
        )),
        "/api/v1/namespaces/shop/secrets/gone" => Some((
            404,
            json!({"kind": "Status", "apiVersion": "v1", "status": "Failure", "code": 404,
                "reason": "NotFound", "message": "secrets \"gone\" not found"})
            .to_string(),
        )),
        _ => None,
    });
    let ask = |name: &str| {
        let mut e = entry(None, &["shop.example.com"], None, 0);
        e.secret = name.into();
        e.secret_request().unwrap()
    };
    let refused = read_tls_secret(&client, &ask("hidden")).await;
    assert!(
        matches!(&refused, SecretOutcome::Refused(f) if f.kind == FailureKind::Forbidden),
        "{refused:?}"
    );
    let mut view = CertsView::new(&CertsRead::new(Manager::NotInstalled), 0);
    view.entries = vec![entry(None, &["shop.example.com"], None, 0)];
    view.apply_secret("shop/shop-tls", refused, 0);
    assert_eq!(view.entries[0].state, CertState::Unknown);
    assert_eq!(
        read_tls_secret(&client, &ask("gone")).await,
        SecretOutcome::NotFound
    );
}

#[tokio::test]
async fn cert_manager_not_installed_is_said_and_never_an_empty_list() {
    let (client, recorder) = crate::fake_api::client(|request| match request.path.as_str() {
        GROUPS => Some(groups(&[])),
        "/apis/networking.k8s.io/v1/ingresses" => {
            Some(list(vec![ingress("shop", "web", "shop-tls")]))
        }
        _ => None,
    });
    let read = read_certificates(&client).await;
    assert_eq!(read.manager, Manager::NotInstalled);
    assert!(
        read.manager
            .problem()
            .unwrap()
            .contains("cert-manager is not installed")
    );
    let view = CertsView::new(&read, 0);
    // The Ingress's Secret is still a row, unread.
    assert_eq!(view.entries.len(), 1);
    assert_eq!(view.entries[0].state, CertState::Unknown);
    assert!(!recorder.lines().iter().any(|l| l.contains("secrets")));
}

#[tokio::test]
async fn a_refused_group_discovery_is_not_read_as_not_installed() {
    let (client, _) = crate::fake_api::client(|request| match request.path.as_str() {
        GROUPS => Some((
            403,
            json!({"kind": "Status", "apiVersion": "v1", "status": "Failure", "code": 403,
                "reason": "Forbidden", "message": "forbidden"})
            .to_string(),
        )),
        "/apis/networking.k8s.io/v1/ingresses" => Some(list(vec![])),
        _ => None,
    });
    let read = read_certificates(&client).await;
    assert!(
        matches!(read.manager, Manager::Unknown(_)),
        "{:?}",
        read.manager
    );
    assert!(read.gateway.is_err());
    let view = CertsView::new(&read, 0);
    assert!(
        view.notes
            .iter()
            .any(|n| n.contains("Gateways could not be read")),
        "{:?}",
        view.notes
    );
}

#[tokio::test]
async fn the_chain_lists_are_read_only_when_a_certificate_is_not_issued() {
    let (client, recorder) = crate::fake_api::client(cluster);
    read_certificates(&client).await;
    assert!(
        !recorder
            .lines()
            .iter()
            .any(|l| l.contains("certificaterequests")
                || l.contains("orders")
                || l.contains("challenges"))
    );
    let busy = |request: &crate::fake_api::Recorded| match request.path.as_str() {
        "/apis/cert-manager.io/v1/certificates" => {
            let mut c = ready_certificate("shop-tls");
            c["status"]["conditions"] = json!([{"type": "Ready", "status": "False"}, {"type": "Issuing", "status": "True"}]);
            Some(list(vec![c]))
        }
        "/apis/cert-manager.io/v1/certificaterequests" => Some(list(vec![json!({
            "metadata": {"name": "shop-1", "namespace": "shop", "uid": "req-1",
                "ownerReferences": [{"uid": "cert-1"}]},
            "status": {"conditions": [{"type": "Denied", "status": "True", "reason": "Policy", "message": "no"}]}})])),
        "/apis/acme.cert-manager.io/v1/orders" | "/apis/acme.cert-manager.io/v1/challenges" => {
            Some(list(vec![]))
        }
        _ => cluster(request),
    };
    let (client, recorder) = crate::fake_api::client(busy);
    let read = read_certificates(&client).await;
    let view = CertsView::new(&read, 1_800_000_000);
    let entry = view.entry("shop/shop-tls").unwrap();
    assert_eq!(entry.state, CertState::Broken);
    assert!(
        entry.reason.starts_with("stopped at CertificateRequest"),
        "{}",
        entry.reason
    );
    recorder.assert_all("only reads", |r| r.method == http::Method::GET);
}

// ---- the example ----

fn example(shape: example::Shape) -> CertsView {
    let now = 1_800_000_000;
    CertsView::new(&example::read(shape, now), now)
}

fn states(view: &CertsView) -> Vec<(String, CertState)> {
    view.entries.iter().map(|e| (e.key(), e.state)).collect()
}

#[test]
fn the_example_shows_every_state_from_the_same_interpretation() {
    let view = example(example::Shape::All);
    let by = |key: &str| view.entry(key).unwrap();
    assert_eq!(by("infra/edge-tls").state, CertState::Healthy);
    assert_eq!(by("staging/edge-tls").state, CertState::Expiring);
    assert_eq!(by("shop/shop-tls").state, CertState::WrongName);
    assert_eq!(by("shop/shop-tls").wrong, ["www.shop.example.com"]);
    assert_eq!(by("payments/pay-tls").state, CertState::Broken);
    assert!(
        by("payments/pay-tls")
            .reason
            .starts_with("stopped at Order")
    );
    assert_eq!(by("cart/cart-tls").state, CertState::Broken);
    assert!(
        by("cart/cart-tls")
            .reason
            .starts_with("stopped at Challenge")
    );
    assert_eq!(by("prod-eu/legacy-tls").state, CertState::Unknown);
    assert_eq!(by("prod-eu/unused-tls").state, CertState::Healthy);
    assert!(by("prod-eu/unused-tls").servings.is_empty());
}

#[test]
fn the_example_shapes_each_make_their_state() {
    use example::Shape::*;
    let healthy = example(Healthy);
    assert!(
        healthy
            .entries
            .iter()
            .all(|e| e.state == CertState::Healthy),
        "{:?}",
        states(&healthy)
    );
    let expiring = example(Expiring);
    let s = states(&expiring);
    assert!(
        s.contains(&("staging/edge-tls".into(), CertState::Expiring)),
        "{s:?}"
    );
    assert!(
        s.contains(&("cart/cart-tls".into(), CertState::Expiring)),
        "{s:?}"
    );
    // Exactly fourteen days is not expiring.
    assert!(
        s.contains(&("payments/pay-tls".into(), CertState::Healthy)),
        "{s:?}"
    );
    let wrong = example(WrongName);
    assert_eq!(
        wrong.entry("infra/edge-tls").unwrap().state,
        CertState::WrongName
    );
    assert_eq!(
        wrong.entry("infra/edge-tls").unwrap().wrong,
        ["stale.example.com"]
    );
    let order = example(StuckOrder);
    assert!(
        order
            .entry("payments/pay-tls")
            .unwrap()
            .reason
            .starts_with("stopped at Order")
    );
    assert_eq!(
        order.entry("cart/cart-tls").unwrap().state,
        CertState::Healthy
    );
    let challenge = example(StuckChallenge);
    assert!(
        challenge
            .entry("cart/cart-tls")
            .unwrap()
            .reason
            .starts_with("stopped at Challenge")
    );
    let none = example(NotInstalled);
    assert_eq!(none.manager, Manager::NotInstalled);
    assert!(
        none.entries
            .iter()
            .all(|e| e.managed.is_none() && e.state == CertState::Unknown)
    );
    assert!(!none.entries.is_empty());
    let unread = example(SecretUnread);
    assert!(unread.entries.iter().all(|e| e.can_read_secret()));
    let empty = example(Empty);
    assert!(empty.entries.is_empty());
    assert!(empty.manager.problem().is_none());
    let refused = example(Refused);
    assert!(refused.entries.is_empty());
    assert_eq!(refused.notes.len(), 3, "{:?}", refused.notes);
}

#[test]
fn the_example_secrets_read_as_their_own_story_and_refusal_is_a_shape() {
    let now = 1_800_000_000;
    let mut view = example(example::Shape::All);
    let asks: Vec<_> = view
        .entries
        .iter()
        .filter_map(|e| Some((e.key(), e.secret_request()?)))
        .collect();
    // Only the Secrets no Certificate answers for can be asked.
    let keys: Vec<_> = asks.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["prod-eu/internal-tls", "prod-eu/legacy-tls"]);
    for (key, request) in asks {
        let outcome = example::read_secret(example::Shape::All, &request, now);
        view.apply_secret(&key, outcome, now);
    }
    assert_eq!(
        view.entry("prod-eu/legacy-tls").unwrap().state,
        CertState::Healthy
    );
    assert_eq!(
        view.entry("prod-eu/internal-tls").unwrap().state,
        CertState::Broken
    );
    let mut view = example(example::Shape::SecretRefused);
    let (key, request) = {
        let e = &view.entries[0];
        (e.key(), e.secret_request().unwrap())
    };
    view.apply_secret(
        &key,
        example::read_secret(example::Shape::SecretRefused, &request, now),
        now,
    );
    assert_eq!(view.entries[0].state, CertState::Unknown);
    assert!(view.entries[0].reason.contains("Forbidden"));
}

#[test]
fn listener_hosts_come_from_the_routes_attached_to_it() {
    let view = example(example::Shape::All);
    let edge = view.entry("infra/edge-tls").unwrap();
    let listener = edge
        .servings
        .iter()
        .find(|s| matches!(&s.via, Via::Listener { gateway, .. } if gateway == "infra/edge"))
        .unwrap();
    assert!(
        listener.hosts.contains(&"shop.example.com".to_owned()),
        "{:?}",
        listener.hosts
    );
    assert!(listener.hosts.contains(&"pay.example.com".to_owned()));
    // A route on another Gateway's listener is not served under this one.
    assert!(!listener.hosts.contains(&"beta.example.com".to_owned()));
    assert!(!listener.hosts.contains(&"docs.example.org".to_owned()));
    let keys: Vec<_> = view
        .serving_host("shop.example.com")
        .iter()
        .map(|e| e.key())
        .collect();
    assert_eq!(keys, ["infra/edge-tls", "shop/shop-tls"]);
    let keys: Vec<_> = view
        .for_listener("infra/edge", "https")
        .iter()
        .map(|e| e.key())
        .collect();
    assert_eq!(keys, ["infra/edge-tls"]);
    assert!(view.for_listener("infra/edge", "http").is_empty());
}

#[test]
fn a_routes_trace_names_the_secrets_of_the_listener_it_chose() {
    let read = routes_read();
    let certificates = |name: &str| {
        read.routes
            .iter()
            .find(|entry| entry.facts.name == name)
            .unwrap()
            .trace
            .parents[0]
            .certificates
            .clone()
    };
    // The HTTPS listener of infra/edge names edge-tls, in the Gateway's namespace.
    assert_eq!(
        certificates("web"),
        [("infra".to_owned(), "edge-tls".to_owned())]
    );
    // A plain HTTP listener has none, and neither has a Service parent.
    assert!(certificates("status").is_empty());
    assert!(certificates("mesh-cart").is_empty());
}

fn routes_read() -> crate::resources::traffic::routes::RoutesRead {
    crate::resources::traffic::routes::example::read(
        crate::resources::traffic::routes::example::Shape::All,
    )
    .unwrap()
}
