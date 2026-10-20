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
        // A read kind that lacks its fields holds nothing.
        (
            json!({"apiVersion": "0.0.1", "kind": "hashedrekord", "spec": {"signature": {}}}),
            "another signature or key",
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

#[tokio::test]
async fn an_entry_of_a_kind_not_read_is_unread_never_a_difference() {
    for (body, named) in [
        (json!({"apiVersion": "0.0.1", "kind": "rfc3161", "spec": {}}), "rfc3161 0.0.1"),
        // A known kind at a version this build does not read.
        (json!({"apiVersion": "0.0.1", "kind": "intoto", "spec": {}}), "intoto 0.0.1"),
        (json!({"apiVersion": "0.0.2", "kind": "hashedrekord", "spec": {}}), "hashedrekord 0.0.2"),
        (json!({"spec": {}}), "unnamed"),
        // The log's text is repeated only as plain characters.
        (json!({"kind": "a\nb<c>"}), "a?b?c?"),
    ] {
        let found = outcome(Ok(answer(42, &body))).await;
        assert_eq!(
            found,
            Outcome::Unread {
                index: 42,
                kind: named.to_owned()
            },
            "{body}"
        );
        assert!(!found.matches());
        assert_eq!(
            found.to_string(),
            format!(
                "the transparency log's entry 42 is a {named} entry, which this version doesn't read"
            )
        );
        assert!(!found.to_string().contains("differs"));
    }
}

mod served {
    //! The client against a fake log over https on loopback.

    use std::time::Duration;

    use crate::rekor::fake::{CERT, Log, Reply};
    use super::*;

    async fn within<T>(work: impl Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(30), work)
            .await
            .expect("the log client did not answer within 30 seconds")
    }

    fn ok(body: Vec<u8>) -> Reply {
        Reply {
            status: 200,
            headers: String::new(),
            body,
        }
    }

    async fn ask(log: &Log, index: Option<u64>) -> Outcome {
        let client = Client::trusting(CERT).unwrap();
        within(look_up(&client, &log.address, index, &verified())).await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_lookup_is_one_get_with_no_authorization() {
        let verified = verified();
        let body = hashedrekord(&verified.signature, &signing_key_pem());
        let log = Log::new(ok(answer(42, &body).body));
        assert_eq!(ask(&log, Some(42)).await, Outcome::Matches { index: 42 });
        let requests = log.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let request = requests[0].to_ascii_lowercase();
        assert!(request.starts_with("get /api/v1/log/entries?logindex=42 http/1.1"));
        // No token or credential of any kind reaches the log.
        for header in ["authorization", "proxy-authorization", "cookie"] {
            assert!(!request.contains(header), "{request}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_redirect_is_an_answer_and_is_not_followed() {
        let elsewhere = Log::new(ok(Vec::new()));
        let log = Log::new(Reply {
            status: 301,
            headers: format!("location: {}/api/v1/log/entries?logIndex=42\r\n", elsewhere.address),
            body: Vec::new(),
        });
        assert!(matches!(ask(&log, Some(42)).await, Outcome::Refused(_)));
        assert!(elsewhere.requests.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_answer_past_the_cap_is_cut_and_never_matches() {
        let log = Log::new(ok(vec![b' '; MOST_ENTRY_BYTES * 2]));
        assert!(matches!(ask(&log, Some(42)).await, Outcome::Differs { .. }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_log_that_is_not_trusted_is_unreachable() {
        // The system roots do not hold the fake's certificate.
        let log = Log::new(ok(Vec::new()));
        let client = Client::new().unwrap();
        let found = within(look_up(&client, &log.address, Some(42), &verified())).await;
        assert!(matches!(found, Outcome::Unreachable(_)), "{found}");
    }

    #[tokio::test]
    async fn only_an_https_address_without_credentials_is_asked() {
        let client = Client::trusting(CERT).unwrap();
        for address in [
            "http://127.0.0.1:1",
            "ftp://rekor.example.com",
            "https://user:pass@rekor.example.com",
            "not a url",
        ] {
            let found = within(look_up(&client, address, Some(42), &verified())).await;
            assert!(matches!(found, Outcome::Unreachable(_)), "{address}: {found}");
        }
    }
}

mod read {
    //! Where the log is.

    use super::super::read::{Address, resolve};
    use crate::chains::config::ChainsConfig;

    fn config(data: serde_json::Value) -> ChainsConfig {
        ChainsConfig::parse(&serde_json::json!({"data": data}))
    }

    #[test]
    fn the_setting_wins_and_chains_names_it_otherwise() {
        let set = Address::Set("https://rekor.example.com".into());
        assert_eq!(resolve(set).unwrap(), "https://rekor.example.com");
        let named = config(serde_json::json!({"transparency.url": "https://log.example.com"}));
        assert_eq!(
            resolve(Address::FromChains(Ok(named))).unwrap(),
            "https://log.example.com"
        );
    }

    #[test]
    fn a_config_that_names_none_or_can_not_be_read_never_defaults_to_the_public_log() {
        let silent = resolve(Address::FromChains(Ok(config(serde_json::json!({})))));
        assert!(silent.unwrap_err().contains("chains-config"));
        let failure = freshkube_core::resources::Failure::new(
            freshkube_core::resources::FailureKind::Forbidden,
            "configmaps is forbidden",
        );
        let refused = resolve(Address::FromChains(Err(failure))).unwrap_err();
        assert!(refused.starts_with("chains-config"), "{refused}");
        assert!(!refused.contains("sigstore"));
    }
}
