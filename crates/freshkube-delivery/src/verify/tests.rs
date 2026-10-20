//! The verifier against an attestation `cosign attest-blob` made (committed
//! as invented test data, see `testdata/cosign/README.md`), and against
//! variations of it, each of which must not confirm. Envelopes the tests
//! sign themselves use a fixed, throwaway key.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ring::digest::{SHA256, digest};
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};
use serde_json::{Value, json};

use super::dsse::pre_authentication_encoding;
use super::*;
use crate::chains::parse_attestations;

const BLOB: &[u8] = include_bytes!("../../testdata/cosign/checkout.blob");
const ENVELOPE: &str = include_str!("../../testdata/cosign/checkout.att.json");
const KEY: &str = include_str!("../../testdata/cosign/checkout.pub");
const OTHER_KEY: &str = include_str!("../../testdata/cosign/other.pub");
const ED25519: &str = include_str!("../../testdata/cosign/ed25519.pub");
const RSA: &str = include_str!("../../testdata/cosign/rsa.pub");
const P384: &str = include_str!("../../testdata/cosign/p384.pub");

const BUILT: &str = "sha256:f4f5ecadeff863529ffa5690e4f100ee235e9a2b9cd19733697b2106ebdc2aa6";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const UID: &str = "7c1f0c52-0000-4000-8000-000000000001";

fn built() -> Digest {
    Digest::parse(BUILT).unwrap()
}

fn trusted() -> Trust {
    Trust::Key(TrustedKey::from_pem(KEY).unwrap())
}

fn b64(bytes: impl AsRef<[u8]>) -> String {
    STANDARD.encode(bytes)
}

/// A TaskRun as Chains' `tekton` storage leaves it, around `envelope` (a
/// DSSE envelope's JSON).
fn task_run_with(envelope: &str) -> Value {
    let carried: Value = serde_json::from_str(envelope).unwrap();
    let statement = STANDARD
        .decode(carried["payload"].as_str().unwrap_or_default())
        .unwrap_or_default();
    let key = format!("taskrun-{UID}");
    json!({"metadata": {"name": "checkout-build", "namespace": "acme-builds", "annotations": {
        format!("chains.tekton.dev/signature-{key}"): b64(envelope),
        format!("chains.tekton.dev/payload-{key}"): b64(statement),
    }}})
}

fn attestation_of(task_run: &Value) -> Attestation {
    let found = parse_attestations(task_run);
    assert_eq!(found.read.len(), 1, "{found:?}");
    found.read.into_iter().next().unwrap()
}

fn cosign_made() -> Attestation {
    attestation_of(&task_run_with(ENVELOPE))
}

fn outcome(trust: &Trust, attestation: &Attestation) -> Verification {
    verify(trust, attestation, &built())
}

/// A throwaway key pair for the envelopes the tests sign themselves,
/// generated fresh on each run.
pub(crate) fn signing_key() -> &'static (EcdsaKeyPair, TrustedKey) {
    static KEY: std::sync::OnceLock<(EcdsaKeyPair, TrustedKey)> = std::sync::OnceLock::new();
    KEY.get_or_init(|| {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        let trusted =
            TrustedKey::from_pem(&TrustedKey::pem_of_point(pair.public_key().as_ref())).unwrap();
        (pair, trusted)
    })
}

pub(crate) fn signed_envelope(payload_type: &str, statement: &Value) -> String {
    let (pair, _) = signing_key();
    let body = statement.to_string();
    let signature = pair
        .sign(
            &SystemRandom::new(),
            &pre_authentication_encoding(payload_type, body.as_bytes()),
        )
        .unwrap();
    json!({
        "payloadType": payload_type,
        "payload": b64(body),
        "signatures": [{"keyid": "", "sig": b64(signature.as_ref())}]
    })
    .to_string()
}

pub(crate) fn statement_with(subjects: Vec<Value>) -> Value {
    json!({
        "_type": "https://in-toto.io/Statement/v0.1",
        "predicateType": "https://slsa.dev/provenance/v0.2",
        "subject": subjects,
        "predicate": {}
    })
}

pub(crate) fn subject(digest: &str) -> Value {
    json!({"name": "registry.example.com/acme/checkout", "digest": {"sha256": digest}})
}

/// The key file the signing key's public half would be, as PEM.
pub(crate) fn signing_key_pem() -> String {
    let (pair, _) = signing_key();
    TrustedKey::pem_of_point(pair.public_key().as_ref())
}

fn short_hex() -> &'static str {
    BUILT.trim_start_matches("sha256:")
}

#[test]
fn the_fixture_is_what_its_readme_says() {
    assert_eq!(
        format!("sha256:{}", hex(digest(&SHA256, BLOB).as_ref())),
        BUILT,
        "the digest of checkout.blob"
    );
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn an_attestation_cosign_made_verifies_with_its_key_and_the_blobs_digest() {
    let outcome = outcome(&trusted(), &cosign_made());
    let Verification::Verified(verified) = &outcome else {
        panic!("expected Verified, got {outcome}");
    };
    assert!(outcome.confirms());
    assert_eq!(verified.subject, built());
    assert_eq!(verified.subject_name.as_deref(), Some("checkout.blob"));
    assert_eq!(
        verified.predicate_type.as_deref(),
        Some("https://slsa.dev/provenance/v0.2")
    );
    assert_eq!(verified.commit.as_deref(), Some(COMMIT));
    assert!(verified.key.starts_with("SHA256:"));
    assert_eq!(verified.key.len(), "SHA256:".len() + 43);
    assert_eq!(
        outcome.to_string(),
        format!(
            "signature verifies with key {}; subject sha256:f4f5ecadeff8 matches the build",
            verified.key
        )
    );
}

#[test]
fn the_fingerprint_is_the_sha256_of_the_der_key_and_differs_per_key() {
    let one = TrustedKey::from_pem(KEY).unwrap();
    let other = TrustedKey::from_pem(OTHER_KEY).unwrap();
    assert_ne!(one.fingerprint(), other.fingerprint());
    let der = STANDARD
        .decode(
            KEY.lines()
                .filter(|line| !line.starts_with("-----"))
                .collect::<String>(),
        )
        .unwrap();
    assert_eq!(
        one.fingerprint(),
        format!(
            "SHA256:{}",
            base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest(&SHA256, &der))
        )
    );
}

// One negative test per variant of `Verification`.

#[test]
fn no_trust_set_never_confirms() {
    let outcome = outcome(&Trust::Unset, &cosign_made());
    assert_eq!(outcome, Verification::NoTrust);
    assert!(!outcome.confirms());
}

#[test]
fn a_key_that_cannot_be_used_never_confirms() {
    let outcome = outcome(
        &Trust::Unusable(KeyProblem::Unsupported("RSA")),
        &cosign_made(),
    );
    assert_eq!(
        outcome,
        Verification::KeyUnusable(KeyProblem::Unsupported("RSA"))
    );
    assert!(!outcome.confirms());
}

#[test]
fn a_malformed_attestation_never_confirms_and_says_why() {
    let mut task_run = task_run_with(ENVELOPE);
    task_run["metadata"]["annotations"][format!("chains.tekton.dev/signature-taskrun-{UID}")] =
        json!(b64("not an envelope"));
    let outcome = outcome(&trusted(), &attestation_of(&task_run));
    assert!(matches!(outcome, Verification::Malformed(_)), "{outcome}");
    assert!(!outcome.confirms());
}

#[test]
fn a_payload_annotation_that_is_not_the_signed_statement_never_confirms() {
    let mut task_run = task_run_with(ENVELOPE);
    task_run["metadata"]["annotations"][format!("chains.tekton.dev/payload-taskrun-{UID}")] =
        json!(b64(statement_with(vec![subject(short_hex())]).to_string()));
    let outcome = outcome(&trusted(), &attestation_of(&task_run));
    assert_eq!(outcome, Verification::Malformed(Malformed::PayloadDiffers));
    assert!(!outcome.confirms());
}

#[test]
fn another_key_does_not_verify() {
    let other = Trust::Key(TrustedKey::from_pem(OTHER_KEY).unwrap());
    let outcome = outcome(&other, &cosign_made());
    let Verification::SignatureFailed { key } = &outcome else {
        panic!("expected SignatureFailed, got {outcome}");
    };
    assert!(
        outcome
            .to_string()
            .contains(&format!("doesn't verify with key {key}"))
    );
    assert!(!outcome.confirms());
}

#[test]
fn a_changed_payload_does_not_verify() {
    let mut envelope: Value = serde_json::from_str(ENVELOPE).unwrap();
    let statement = STANDARD
        .decode(envelope["payload"].as_str().unwrap())
        .unwrap();
    let text = String::from_utf8(statement)
        .unwrap()
        .replace("checkout.blob", "evil.blob");
    envelope["payload"] = json!(b64(text));
    let outcome = outcome(
        &trusted(),
        &attestation_of(&task_run_with(&envelope.to_string())),
    );
    assert!(
        matches!(outcome, Verification::SignatureFailed { .. }),
        "{outcome}"
    );
}

#[test]
fn a_changed_subject_digest_with_the_original_signature_does_not_verify() {
    let mut envelope: Value = serde_json::from_str(ENVELOPE).unwrap();
    let statement = STANDARD
        .decode(envelope["payload"].as_str().unwrap())
        .unwrap();
    let text = String::from_utf8(statement)
        .unwrap()
        .replace("f4f5ecad", "f4f5ecae");
    envelope["payload"] = json!(b64(text));
    // The attacker's version names a digest the build did not report.
    let other = Digest::parse(&BUILT.replace("f4f5ecad", "f4f5ecae")).unwrap();
    let outcome = verify(
        &trusted(),
        &attestation_of(&task_run_with(&envelope.to_string())),
        &other,
    );
    assert!(
        matches!(outcome, Verification::SignatureFailed { .. }),
        "{outcome}"
    );
}

#[test]
fn a_signature_that_is_not_der_or_is_cut_does_not_verify() {
    for bad in [
        vec![0u8; 64],
        vec![],
        b"junk".to_vec(),
        vec![0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01],
    ] {
        let mut envelope: Value = serde_json::from_str(ENVELOPE).unwrap();
        envelope["signatures"][0]["sig"] = json!(b64(&bad));
        let outcome = outcome(
            &trusted(),
            &attestation_of(&task_run_with(&envelope.to_string())),
        );
        assert!(
            matches!(outcome, Verification::SignatureFailed { .. }),
            "{bad:?}: {outcome}"
        );
    }
}

#[test]
fn a_signature_over_another_payload_type_does_not_verify() {
    // Signed correctly, but as text/plain; the envelope claims in-toto.
    let statement = statement_with(vec![subject(short_hex())]);
    let mut envelope: Value =
        serde_json::from_str(&signed_envelope("text/plain", &statement)).unwrap();
    envelope["payloadType"] = json!(IN_TOTO_PAYLOAD_TYPE);
    let key = signing_key().1.clone();
    let outcome = outcome(
        &Trust::Key(key),
        &attestation_of(&task_run_with(&envelope.to_string())),
    );
    assert!(
        matches!(outcome, Verification::SignatureFailed { .. }),
        "{outcome}"
    );
}

#[test]
fn a_verified_signature_over_a_digest_the_build_did_not_report_differs() {
    let other = Digest::parse(&format!("sha256:{}", "ab".repeat(32))).unwrap();
    let outcome = verify(&trusted(), &cosign_made(), &other);
    let Verification::SubjectDiffers {
        built, subjects, ..
    } = &outcome
    else {
        panic!("expected SubjectDiffers, got {outcome}");
    };
    assert_eq!(built, &other);
    assert_eq!(subjects, &vec![self::built()]);
    assert!(
        outcome
            .to_string()
            .contains("no subject is the build's sha256:abababababab")
    );
    assert!(!outcome.confirms());
}

#[test]
fn an_attestation_chains_stores_elsewhere_never_confirms() {
    let outcome = Verification::not_stored_here(&[Backend::Oci, Backend::Other("docdb".into())]);
    assert!(!outcome.confirms());
    assert!(outcome.to_string().contains("not read in this step"));
}

#[test]
fn an_image_signature_is_not_read_and_never_confirms() {
    let task_run = json!({"metadata": {"annotations": {
        "chains.tekton.dev/signature-9e0a3c1d5b7f": b64("sig"),
        "chains.tekton.dev/payload-9e0a3c1d5b7f": b64("{}"),
    }}});
    let outcome = outcome(&trusted(), &attestation_of(&task_run));
    assert_eq!(outcome, Verification::NotRead(NotRead::ImageSignature));
    assert!(!outcome.confirms());
}

#[test]
fn a_run_attestation_left_unread_is_not_a_verification() {
    let mut attestation = cosign_made();
    attestation.stored = Stored::Unread;
    let outcome = outcome(&trusted(), &attestation);
    assert_eq!(outcome, Verification::NotRead(NotRead::Unread));
    assert!(!outcome.confirms());
}

#[test]
fn subjects_past_the_cap_are_not_read_so_no_difference_is_claimed() {
    // 70 subjects, none the build's, correctly signed: only 64 are kept.
    let subjects = (0..70)
        .map(|index| subject(&format!("{index:064x}")))
        .collect();
    let envelope = signed_envelope(IN_TOTO_PAYLOAD_TYPE, &statement_with(subjects));
    let key = signing_key().1.clone();
    let outcome = outcome(&Trust::Key(key), &attestation_of(&task_run_with(&envelope)));
    assert_eq!(outcome, Verification::NotRead(NotRead::SubjectsCutAtCap));
    assert!(!outcome.confirms());
}

#[test]
fn the_build_digest_among_the_kept_subjects_confirms_even_with_more_past_the_cap() {
    let mut subjects: Vec<Value> = (0..70)
        .map(|index| subject(&format!("{index:064x}")))
        .collect();
    subjects[3] = subject(short_hex());
    let envelope = signed_envelope(IN_TOTO_PAYLOAD_TYPE, &statement_with(subjects));
    let key = signing_key().1.clone();
    assert!(outcome(&Trust::Key(key), &attestation_of(&task_run_with(&envelope))).confirms());
}

#[test]
fn a_payload_that_is_not_an_in_toto_statement_does_not_confirm_even_when_signed() {
    let mut statement = statement_with(vec![subject(short_hex())]);
    statement["_type"] = json!("https://example.com/not-in-toto");
    let envelope = signed_envelope(IN_TOTO_PAYLOAD_TYPE, &statement);
    let key = signing_key().1.clone();
    let outcome = outcome(&Trust::Key(key), &attestation_of(&task_run_with(&envelope)));
    assert!(
        matches!(
            outcome,
            Verification::Malformed(Malformed::NotAStatement(_))
        ),
        "{outcome}"
    );
}

#[test]
fn a_signed_statement_the_tests_make_confirms_so_the_negatives_are_not_vacuous() {
    let envelope = signed_envelope(
        IN_TOTO_PAYLOAD_TYPE,
        &statement_with(vec![subject(short_hex())]),
    );
    let key = signing_key().1.clone();
    assert!(outcome(&Trust::Key(key), &attestation_of(&task_run_with(&envelope))).confirms());
}

#[test]
fn any_one_of_several_signatures_may_verify_and_a_bad_one_beside_it_does_not_hurt() {
    let mut envelope: Value = serde_json::from_str(ENVELOPE).unwrap();
    envelope["signatures"] = json!([
        {"keyid": "", "sig": b64("junk")},
        envelope["signatures"][0].clone()
    ]);
    assert!(
        outcome(
            &trusted(),
            &attestation_of(&task_run_with(&envelope.to_string()))
        )
        .confirms()
    );
}

#[test]
fn the_keyid_is_a_hint_and_not_a_match() {
    let mut envelope: Value = serde_json::from_str(ENVELOPE).unwrap();
    envelope["signatures"][0]["keyid"] = json!("SHA256:claimed");
    assert!(
        outcome(
            &trusted(),
            &attestation_of(&task_run_with(&envelope.to_string()))
        )
        .confirms()
    );
    // And naming the right key's id does not make a wrong signature verify.
    let other = Trust::Key(TrustedKey::from_pem(OTHER_KEY).unwrap());
    let Trust::Key(good) = trusted() else {
        unreachable!()
    };
    envelope["signatures"][0]["keyid"] = json!(good.fingerprint());
    let outcome = outcome(
        &other,
        &attestation_of(&task_run_with(&envelope.to_string())),
    );
    assert!(!outcome.confirms());
}

#[test]
fn a_certificate_the_run_carries_is_not_used_to_verify() {
    let mut task_run = task_run_with(ENVELOPE);
    task_run["metadata"]["annotations"][format!("chains.tekton.dev/cert-taskrun-{UID}")] = json!(
        b64("-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----")
    );
    let other = Trust::Key(TrustedKey::from_pem(OTHER_KEY).unwrap());
    assert!(!outcome(&other, &attestation_of(&task_run)).confirms());
}

// The key file.

#[test]
fn a_private_key_is_refused_unread() {
    for label in [
        "PRIVATE KEY",
        "EC PRIVATE KEY",
        "ENCRYPTED SIGSTORE PRIVATE KEY",
    ] {
        let text = format!("-----BEGIN {label}-----\nAAAA\n-----END {label}-----\n");
        assert_eq!(
            TrustedKey::from_pem(&text),
            Err(KeyProblem::PrivateKey),
            "{label}"
        );
    }
    // A public key followed by a private one is still refused.
    let both = format!("{KEY}-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n");
    assert_eq!(TrustedKey::from_pem(&both), Err(KeyProblem::PrivateKey));
}

#[test]
fn a_file_that_is_not_exactly_one_public_key_is_refused() {
    assert_eq!(TrustedKey::from_pem(""), Err(KeyProblem::NotOnePublicKey));
    assert_eq!(
        TrustedKey::from_pem("-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n"),
        Err(KeyProblem::NotOnePublicKey)
    );
    assert_eq!(
        TrustedKey::from_pem(&format!("{KEY}{OTHER_KEY}")),
        Err(KeyProblem::NotOnePublicKey),
        "two keys are not one trusted key"
    );
}

#[test]
fn a_public_key_block_that_does_not_decode_is_malformed() {
    assert_eq!(
        TrustedKey::from_pem("-----BEGIN PUBLIC KEY-----\n!!!\n-----END PUBLIC KEY-----\n"),
        Err(KeyProblem::Malformed)
    );
    assert_eq!(
        TrustedKey::from_pem("-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----\n"),
        Err(KeyProblem::Malformed)
    );
}

#[test]
fn a_p256_key_of_another_shape_is_malformed_and_a_point_off_the_curve_verifies_nothing() {
    // A compressed point (0x02 prefix) is not the shape that is checked.
    let mut compressed = vec![
        0x30, 0x39, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x22, 0x00, 0x02,
    ];
    compressed.extend_from_slice(&[7u8; 32]);
    let pem = format!(
        "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
        STANDARD.encode(compressed)
    );
    assert_eq!(TrustedKey::from_pem(&pem), Err(KeyProblem::Malformed));
    // 0x04 followed by 64 bytes that are no point on P-256: accepted as a
    // key (ring checks the point when it verifies) and verifies nothing.
    let mut off_curve = vec![0x04];
    off_curve.extend_from_slice(&[1u8; 64]);
    let key = TrustedKey::from_pem(&TrustedKey::pem_of_point(&off_curve)).unwrap();
    let statement = statement_with(vec![subject(short_hex())]);
    let envelope = signed_envelope(IN_TOTO_PAYLOAD_TYPE, &statement);
    let outcome = outcome(&Trust::Key(key), &attestation_of(&task_run_with(&envelope)));
    assert!(
        matches!(outcome, Verification::SignatureFailed { .. }),
        "{outcome}"
    );
}

#[test]
fn text_after_the_key_block_is_refused() {
    assert_eq!(
        TrustedKey::from_pem(&format!("{KEY}trailing")),
        Err(KeyProblem::NotOnePublicKey)
    );
    assert!(TrustedKey::from_pem(&format!("{KEY}\n\n")).is_ok());
}

#[test]
fn other_key_types_are_named_and_refused() {
    assert_eq!(
        TrustedKey::from_pem(ED25519),
        Err(KeyProblem::Unsupported("Ed25519"))
    );
    assert_eq!(
        TrustedKey::from_pem(RSA),
        Err(KeyProblem::Unsupported("RSA"))
    );
    assert_eq!(
        TrustedKey::from_pem(P384),
        Err(KeyProblem::Unsupported("ECDSA P-384"))
    );
    assert_eq!(
        KeyProblem::Unsupported("RSA").to_string(),
        "RSA keys are not supported yet; use ECDSA P-256"
    );
}

#[test]
fn a_key_file_is_read_from_a_regular_file_of_bounded_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("checkout.pub");
    std::fs::write(&path, KEY).unwrap();
    assert!(matches!(Trust::from_key_file(&path), Trust::Key(_)));

    let missing = Trust::from_key_file(&dir.path().join("absent.pub"));
    assert!(
        matches!(missing, Trust::Unusable(KeyProblem::Unreadable(_))),
        "{missing:?}"
    );

    let folder = Trust::from_key_file(dir.path());
    assert_eq!(
        folder,
        Trust::Unusable(KeyProblem::Unreadable("not a regular file".into()))
    );

    let big = dir.path().join("big.pub");
    std::fs::write(&big, format!("{KEY}{}", "#".repeat(8 * 1024))).unwrap();
    assert_eq!(
        Trust::from_key_file(&big),
        Trust::Unusable(KeyProblem::Unreadable("larger than 8 KiB".into()))
    );
}

#[test]
fn an_unusable_key_file_yields_a_verification_that_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("key.pem");
    std::fs::write(
        &path,
        "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n",
    )
    .unwrap();
    let outcome = outcome(&Trust::from_key_file(&path), &cosign_made());
    assert_eq!(outcome, Verification::KeyUnusable(KeyProblem::PrivateKey));
    assert_eq!(
        outcome.to_string(),
        "the key file holds a private key; name the public key"
    );
}

#[test]
fn only_verified_confirms_across_every_variant() {
    let verified = outcome(&trusted(), &cosign_made());
    let others = [
        Verification::NoTrust,
        Verification::KeyUnusable(KeyProblem::PrivateKey),
        Verification::Malformed(Malformed::PayloadDiffers),
        Verification::SignatureFailed {
            key: "SHA256:x".into(),
        },
        Verification::SubjectDiffers {
            key: "SHA256:x".into(),
            built: built(),
            subjects: vec![],
        },
        Verification::not_stored_here(&[Backend::Oci]),
        Verification::NotRead(NotRead::ImageSignature),
        Verification::NotRead(NotRead::Unread),
        Verification::NotRead(NotRead::SubjectsCutAtCap),
    ];
    assert!(verified.confirms());
    assert!(others.iter().all(|other| !other.confirms()));
    // Exhaustive on purpose: a new variant fails to compile here until it is
    // placed on one side of `confirms`.
    for variant in others.iter().chain([&verified]) {
        match variant {
            Verification::Verified(_) => assert!(variant.confirms()),
            Verification::NoTrust
            | Verification::KeyUnusable(_)
            | Verification::Malformed(_)
            | Verification::SignatureFailed { .. }
            | Verification::SubjectDiffers { .. }
            | Verification::NotStoredHere(_)
            | Verification::NotRead(_) => assert!(!variant.confirms()),
        }
    }
}
