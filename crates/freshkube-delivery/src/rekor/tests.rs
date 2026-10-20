//! The log lookup against a canned log. The entry bodies follow Rekor's
//! documented `hashedrekord` and `intoto` shapes; no live log was read.

use std::cell::RefCell;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use super::*;
use crate::chains::parse_attestations;
use crate::digest::Digest;
use crate::verify::tests::{signed_envelope, signing_key_pem, statement_with, subject};
use crate::verify::{Trust, TrustedKey, Verification, verify};

const BUILT: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const LOG: &str = "https://rekor.example.com";

fn b64(bytes: impl AsRef<[u8]>) -> String {
    STANDARD.encode(bytes)
}

/// One verified attestation for the whole run: signatures are randomised, so
/// a fresh one is not the one an entry holds.
fn verified() -> Verified {
    static VERIFIED: std::sync::OnceLock<Verified> = std::sync::OnceLock::new();
    VERIFIED.get_or_init(made).clone()
}

fn made() -> Verified {
    let statement = statement_with(vec![subject(BUILT.trim_start_matches("sha256:"))]);
    let envelope = signed_envelope("application/vnd.in-toto+json", &statement);
    let key = "taskrun-7c1f0c52-0000-4000-8000-000000000003";
    let run = json!({"metadata": {"name": "checkout-build", "namespace": "acme-builds",
    "annotations": {
        format!("chains.tekton.dev/signature-{key}"): b64(&envelope),
        format!("chains.tekton.dev/payload-{key}"): b64(statement.to_string()),
    }}});
    let attestation = parse_attestations(&run).read.remove(0);
    let trust = Trust::Key(TrustedKey::from_pem(&signing_key_pem()).unwrap());
    match verify(&trust, &attestation, &Digest::parse(BUILT).unwrap()) {
        Verification::Verified(verified) => verified,
        other => panic!("the fixture must verify: {other}"),
    }
}

fn hashedrekord(sig: &[u8], pem: &str) -> Value {
    json!({"apiVersion": "0.0.1", "kind": "hashedrekord", "spec": {
        "signature": {"content": b64(sig), "publicKey": {"content": b64(pem)}}}})
}

/// Rekor's `intoto` 0.0.2 entry stores the envelope's `sig` (already
/// base64) as base64 once more.
fn intoto(sig: &[u8], pem: &str) -> Value {
    json!({"apiVersion": "0.0.2", "kind": "intoto", "spec": {"content": {"envelope": {
        "signatures": [{"sig": b64(b64(sig)), "publicKey": b64(pem)}]}}}})
}

fn answer(index: u64, body: &Value) -> Fetched {
    let entry = json!({"body": b64(body.to_string()), "logIndex": index});
    Fetched {
        status: 200,
        body: json!({"24296fb24b8ad77a": entry}).to_string().into_bytes(),
    }
}

struct Canned {
    reply: Result<Fetched, Failure>,
    asked: RefCell<Vec<String>>,
}

impl Canned {
    fn of(reply: Result<Fetched, Failure>) -> Self {
        Self {
            reply,
            asked: RefCell::default(),
        }
    }
}

impl Fetch for Canned {
    async fn get(&self, url: &str) -> Result<Fetched, Failure> {
        self.asked.borrow_mut().push(url.to_owned());
        match &self.reply {
            Ok(fetched) => Ok(Fetched {
                status: fetched.status,
                body: fetched.body.clone(),
            }),
            Err(failure) => Err(failure.clone()),
        }
    }
}

async fn outcome(reply: Result<Fetched, Failure>) -> Outcome {
    look_up(&Canned::of(reply), LOG, Some(42), &verified()).await
}

#[test]
fn only_the_digits_of_the_index_are_taken() {
    let index = |text| log_index(text);
    assert_eq!(
        index("https://rekor.example.com/api/v1/log/entries?logIndex=42"),
        Some(42)
    );
    assert_eq!(index("x/api/v1/log/entries?logIndex=7&more=1"), Some(7));
    for bad in [
        "",
        "logIndex=",
        "logIndex=-1",
        "logIndex=4x",
        "logIndex=4/../../x",
        "logIndex=99999999999999999999",
        "no index here",
    ] {
        assert_eq!(index(bad), None, "{bad}");
    }
}

#[tokio::test]
async fn an_entry_that_holds_the_signature_and_the_key_matches() {
    let verified = verified();
    for body in [
        hashedrekord(&verified.signature, &signing_key_pem()),
        intoto(&verified.signature, &signing_key_pem()),
    ] {
        let found = outcome(Ok(answer(42, &body))).await;
        assert_eq!(found, Outcome::Matches { index: 42 }, "{body}");
        assert!(found.matches());
        assert!(
            found
                .to_string()
                .contains("log's own signature was not checked")
        );
    }
}

#[tokio::test]
async fn the_log_asked_is_the_one_given_never_the_annotations() {
    let canned = Canned::of(Ok(Fetched {
        status: 404,
        body: Vec::new(),
    }));
    let found = look_up(&canned, "https://rekor.example.com/", Some(5), &verified()).await;
    assert_eq!(found, Outcome::NotInLog { index: 5 });
    assert_eq!(
        *canned.asked.borrow(),
        ["https://rekor.example.com/api/v1/log/entries?logIndex=5"]
    );
}

#[tokio::test]
async fn an_entry_of_another_signature_or_key_differs() {
    let verified = verified();
    let other_key = include_str!("../../testdata/cosign/other.pub");
    for (body, why) in [
        (
            hashedrekord(b"another signature", &signing_key_pem()),
            "another signature or key",
        ),
        (
            hashedrekord(&verified.signature, other_key),
            "another signature or key",
        ),
        (
            intoto(b"another signature", &signing_key_pem()),
            "another signature or key",
        ),
        (
            json!({"kind": "rfc3161", "spec": {}}),
            "a kind of entry this build does not read",
        ),
        (
            json!({"kind": "hashedrekord", "spec": {"signature": {}}}),
            "a kind of entry this build does not read",
        ),
    ] {
        let found = outcome(Ok(answer(42, &body))).await;
        assert!(!found.matches(), "{body}");
        assert!(found.to_string().contains(why), "{found}");
    }
}

#[tokio::test]
async fn an_answer_that_is_not_the_entry_asked_for_differs() {
    let verified = verified();
    let body = hashedrekord(&verified.signature, &signing_key_pem());
    // Another index than the one asked for.
    assert!(!outcome(Ok(answer(43, &body))).await.matches());
    // Two entries.
    let entry = json!({"body": b64(body.to_string()), "logIndex": 42});
    let two = Fetched {
        status: 200,
        body: json!({"a": entry, "b": entry}).to_string().into_bytes(),
    };
    assert!(!outcome(Ok(two)).await.matches());
    // Not JSON, an empty object, a body that is not base64.
    for bytes in [
        "not json",
        "{}",
        "[]",
        r#"{"a": {"body": "!!", "logIndex": 42}}"#,
    ] {
        let reply = Fetched {
            status: 200,
            body: bytes.as_bytes().to_vec(),
        };
        let found = outcome(Ok(reply)).await;
        assert!(matches!(found, Outcome::Differs { .. }), "{bytes}: {found}");
    }
}

#[tokio::test]
async fn an_answer_cut_at_the_cap_never_matches() {
    let reply = Fetched {
        status: 200,
        body: vec![b' '; MOST_ENTRY_BYTES],
    };
    assert!(matches!(outcome(Ok(reply)).await, Outcome::Differs { .. }));
}

#[tokio::test]
async fn a_missing_unreachable_or_refusing_log_is_each_its_own_state() {
    let status = |status| {
        Ok(Fetched {
            status,
            body: Vec::new(),
        })
    };
    assert_eq!(outcome(status(404)).await, Outcome::NotInLog { index: 42 });
    for code in [403, 500] {
        assert!(
            matches!(outcome(status(code)).await, Outcome::Refused(_)),
            "{code}"
        );
    }
    let failed = |kind| Err(Failure::new(kind, "connection refused"));
    assert!(matches!(
        outcome(failed(FailureKind::Unreachable)).await,
        Outcome::Unreachable(_)
    ));
    assert!(matches!(
        outcome(failed(FailureKind::Timeout)).await,
        Outcome::Unreachable(_)
    ));
    assert!(matches!(
        outcome(failed(FailureKind::Forbidden)).await,
        Outcome::Refused(_)
    ));
    for found in [
        Outcome::NotInLog { index: 1 },
        Outcome::Unreachable("x".into()),
        Outcome::Refused("x".into()),
        Outcome::NoIndex,
    ] {
        assert!(!found.matches(), "{found}");
    }
}

#[tokio::test]
async fn a_run_without_an_index_asks_nothing() {
    let canned = Canned::of(Ok(Fetched {
        status: 200,
        body: Vec::new(),
    }));
    assert_eq!(
        look_up(&canned, LOG, None, &verified()).await,
        Outcome::NoIndex
    );
    assert!(canned.asked.borrow().is_empty());
}
