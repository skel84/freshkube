//! Chains' stored attestations and configuration, against invented
//! annotations. Nothing here is signed: the signature bytes are made up,
//! since reading checks no signature.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use super::config::{Backend, ChainsConfig, Enabled, read_config};
use super::*;
use crate::fixtures::{FixtureReader, pipeline_run};
use crate::tekton::{parse_pipeline_run, parse_task_run};
use freshkube_core::resources::FailureKind;

const UID: &str = "7c1f0c52-0000-4000-8000-000000000001";
const DIGEST: &str = "sha256:9e0a3c1d5b7f2a4c6e8091b3d5f70a2c4e6a8c0e1f3b5d7a9c1e3f5b7d9a1c3e";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const OTHER_COMMIT: &str = "fedcba9876543210fedcba9876543210fedcba98";

fn b64(bytes: impl AsRef<[u8]>) -> String {
    STANDARD.encode(bytes)
}

/// An in-toto statement for acme's `checkout` image, with SLSA v0.2
/// provenance that names one source commit.
fn statement() -> Value {
    json!({
        "_type": "https://in-toto.io/Statement/v0.1",
        "predicateType": "https://slsa.dev/provenance/v0.2",
        "subject": [{
            "name": "registry.example.com/acme/checkout",
            "digest": {"sha256": DIGEST.trim_start_matches("sha256:")}
        }],
        "predicate": {"materials": [{
            "uri": "git+https://git.example.com/acme/checkout.git",
            "digest": {"sha1": COMMIT}
        }]}
    })
}

fn envelope_of(payload_type: &str, statement: &Value) -> Value {
    json!({
        "payloadType": payload_type,
        "payload": b64(statement.to_string()),
        "signatures": [{"keyid": "SHA256:invented", "sig": b64("not a real signature")}]
    })
}

/// A TaskRun carrying the four annotations Chains writes for itself.
fn signed(statement: &Value) -> Value {
    let key = format!("taskrun-{UID}");
    json!({
        "metadata": {
            "name": "checkout-build", "namespace": "acme-builds", "uid": UID,
            "annotations": {
                "chains.tekton.dev/signed": "true",
                format!("chains.tekton.dev/payload-{key}"): b64(statement.to_string()),
                format!("chains.tekton.dev/signature-{key}"):
                    b64(envelope_of(IN_TOTO_PAYLOAD_TYPE, statement).to_string()),
                format!("chains.tekton.dev/cert-{key}"): "",
                format!("chains.tekton.dev/chain-{key}"): ""
            }
        }
    })
}

fn annotate(object: &mut Value, name: &str, value: Value) {
    object["metadata"]["annotations"][name] = value;
}

fn only(object: &Value) -> Attestation {
    let found = parse_attestations(object);
    assert_eq!(found.omitted, 0);
    assert_eq!(found.read.len(), 1, "{found:?}");
    found.read.into_iter().next().unwrap()
}

fn envelope(attestation: &Attestation) -> &Envelope {
    match &attestation.stored {
        Stored::Envelope(envelope) => envelope,
        other => panic!("expected an envelope, got {other:?}"),
    }
}

fn malformed(object: &Value) -> Malformed {
    match only(object).stored {
        Stored::Malformed(problem) => problem,
        other => panic!("expected a malformed attestation, got {other:?}"),
    }
}

#[test]
fn a_signed_task_run_yields_its_envelope_and_what_the_statement_claims() {
    let attestation = only(&signed(&statement()));
    assert_eq!(attestation.key, format!("taskrun-{UID}"));
    assert_eq!(attestation.form, Form::Run);
    assert!(!attestation.certificate, "an empty certificate is none");
    let envelope = envelope(&attestation);
    assert_eq!(envelope.payload_type, IN_TOTO_PAYLOAD_TYPE);
    assert_eq!(envelope.payload_annotation_agrees, Some(true));
    assert_eq!(envelope.signatures.len(), 1);
    assert_eq!(
        envelope.signatures[0].key_id.as_deref(),
        Some("SHA256:invented")
    );
    let claims = &envelope.claims;
    assert_eq!(
        claims.predicate_type.as_deref(),
        Some("https://slsa.dev/provenance/v0.2")
    );
    assert_eq!(claims.subjects.len(), 1);
    assert_eq!(claims.subjects[0].digest, Digest::parse(DIGEST));
    assert_eq!(
        claims.subjects[0].name.as_deref(),
        Some("registry.example.com/acme/checkout")
    );
    assert_eq!(claims.commit.as_deref(), Some(COMMIT));
}

#[test]
fn slsa_v1_provenance_names_its_commit_in_the_resolved_dependencies() {
    let mut v1 = statement();
    v1["predicateType"] = json!("https://slsa.dev/provenance/v1");
    v1["predicate"] = json!({"buildDefinition": {"resolvedDependencies": [
        {"uri": "git+https://git.example.com/acme/checkout.git", "digest": {"gitCommit": COMMIT}},
        {"uri": "oci://registry.example.com/acme/base", "digest": {"sha256": "ab"}}
    ]}});
    let attestation = only(&signed(&v1));
    assert_eq!(
        envelope(&attestation).claims.commit.as_deref(),
        Some(COMMIT)
    );
}

#[test]
fn provenance_naming_two_commits_or_none_names_no_commit() {
    let mut two = statement();
    two["predicate"]["materials"] = json!([
        {"uri": "git+https://git.example.com/acme/checkout.git", "digest": {"sha1": COMMIT}},
        {"uri": "git+https://git.example.com/acme/shared.git", "digest": {"sha1": OTHER_COMMIT}}
    ]);
    assert_eq!(envelope(&only(&signed(&two))).claims.commit, None);

    let mut short = statement();
    short["predicate"]["materials"][0]["digest"]["sha1"] = json!("0123456");
    assert_eq!(envelope(&only(&signed(&short))).claims.commit, None);

    let mut not_git = statement();
    not_git["predicate"]["materials"][0]["uri"] = json!("https://example.com/archive.tgz");
    assert_eq!(envelope(&only(&signed(&not_git))).claims.commit, None);
}

#[test]
fn a_payload_annotation_that_differs_from_the_signed_payload_is_marked() {
    let mut run = signed(&statement());
    let key = format!("chains.tekton.dev/payload-taskrun-{UID}");
    annotate(&mut run, &key, json!(b64("{\"subject\":[]}")));
    assert_eq!(envelope(&only(&run)).payload_annotation_agrees, Some(false));
    run["metadata"]["annotations"]
        .as_object_mut()
        .unwrap()
        .remove(&key);
    assert_eq!(envelope(&only(&run)).payload_annotation_agrees, None);
}

#[test]
fn a_subject_without_a_sha256_digest_keeps_its_name_and_no_digest() {
    let mut other = statement();
    other["subject"] = json!([{"name": "acme/checkout", "digest": {"sha512": "ab"}},
                              {"name": "acme/odd", "digest": {"sha256": "short"}}]);
    let attestation = only(&signed(&other));
    let subjects = &envelope(&attestation).claims.subjects;
    assert_eq!(subjects.len(), 2);
    assert!(subjects.iter().all(|subject| subject.digest.is_none()));
}

#[test]
fn a_pipeline_run_and_an_image_signature_are_told_apart_by_their_key() {
    let mut run = signed(&statement());
    annotate(
        &mut run,
        "chains.tekton.dev/payload-9e0a3c1d5b7f",
        json!(b64("{}")),
    );
    annotate(
        &mut run,
        "chains.tekton.dev/signature-9e0a3c1d5b7f",
        json!(b64("sig")),
    );
    annotate(
        &mut run,
        "chains.tekton.dev/cert-9e0a3c1d5b7f",
        json!(b64("pem")),
    );
    let found = parse_attestations(&run);
    assert_eq!(found.read.len(), 2);
    let image = found.read.iter().find(|a| a.form == Form::Image).unwrap();
    assert_eq!(image.key, "9e0a3c1d5b7f");
    assert_eq!(image.stored, Stored::Unread);
    assert!(image.certificate);

    let key = format!("pipelinerun-{UID}");
    let mut pipeline = json!({"metadata": {"annotations": {
        format!("chains.tekton.dev/signature-{key}"):
            b64(envelope_of(IN_TOTO_PAYLOAD_TYPE, &statement()).to_string())
    }}});
    assert_eq!(only(&pipeline).form, Form::Run);
    pipeline["metadata"]["annotations"] = json!({});
    assert_eq!(parse_attestations(&pipeline), Attestations::default());
}

#[test]
fn an_object_without_annotations_has_no_attestations() {
    assert_eq!(parse_attestations(&json!({})), Attestations::default());
    assert_eq!(
        parse_attestations(
            &json!({"metadata": {"annotations": {"chains.tekton.dev/signed": "true"}}})
        ),
        Attestations::default(),
        "the signed annotation alone is not an attestation"
    );
}

#[test]
fn the_log_entry_is_kept_as_written_and_never_followed() {
    let mut run = signed(&statement());
    annotate(
        &mut run,
        "chains.tekton.dev/transparency",
        json!("https://rekor.example.com/api/v1/log/entries?logIndex=41"),
    );
    assert_eq!(
        only(&run).log_entry.as_deref(),
        Some("https://rekor.example.com/api/v1/log/entries?logIndex=41")
    );
    assert_eq!(only(&signed(&statement())).log_entry, None);
}

#[test]
fn a_certificate_annotation_is_noted_and_not_kept() {
    let mut run = signed(&statement());
    annotate(
        &mut run,
        &format!("chains.tekton.dev/cert-taskrun-{UID}"),
        json!(b64("-----BEGIN CERTIFICATE-----")),
    );
    assert!(only(&run).certificate);
}

#[test]
fn each_way_an_attestation_is_unreadable_says_why() {
    let sig = format!("chains.tekton.dev/signature-taskrun-{UID}");
    let payload = format!("chains.tekton.dev/payload-taskrun-{UID}");

    let mut no_signature = signed(&statement());
    no_signature["metadata"]["annotations"]
        .as_object_mut()
        .unwrap()
        .remove(&sig);
    assert_eq!(malformed(&no_signature), Malformed::Missing("signature"));

    let mut not_base64 = signed(&statement());
    annotate(&mut not_base64, &sig, json!("not base64 !!"));
    assert_eq!(
        malformed(&not_base64),
        Malformed::NotBase64("the signature")
    );

    let mut bad_payload = signed(&statement());
    annotate(&mut bad_payload, &payload, json!("%%%"));
    assert_eq!(malformed(&bad_payload), Malformed::NotBase64("the payload"));

    let mut not_json = signed(&statement());
    annotate(&mut not_json, &sig, json!(b64("plain text")));
    assert!(matches!(malformed(&not_json), Malformed::NotAnEnvelope(_)));

    let mut no_type = signed(&statement());
    annotate(
        &mut no_type,
        &sig,
        json!(b64(json!({"payload": "e30="}).to_string())),
    );
    assert_eq!(
        malformed(&no_type).to_string(),
        "the envelope has no payloadType"
    );

    let mut no_body = signed(&statement());
    annotate(
        &mut no_body,
        &sig,
        json!(b64(json!({"payloadType": IN_TOTO_PAYLOAD_TYPE}).to_string())),
    );
    assert_eq!(
        malformed(&no_body).to_string(),
        "the envelope has no payload"
    );

    let mut body_not_base64 = signed(&statement());
    annotate(
        &mut body_not_base64,
        &sig,
        json!(b64(
            json!({"payloadType": IN_TOTO_PAYLOAD_TYPE, "payload": "?"}).to_string()
        )),
    );
    assert_eq!(
        malformed(&body_not_base64),
        Malformed::NotBase64("the envelope's payload")
    );

    let mut unsigned = signed(&statement());
    let mut envelope = envelope_of(IN_TOTO_PAYLOAD_TYPE, &statement());
    envelope["signatures"] = json!([]);
    annotate(&mut unsigned, &sig, json!(b64(envelope.to_string())));
    assert_eq!(
        malformed(&unsigned).to_string(),
        "the envelope has no signature"
    );

    let mut other_type = signed(&statement());
    annotate(
        &mut other_type,
        &sig,
        json!(b64(envelope_of("text/plain", &statement()).to_string())),
    );
    assert_eq!(
        malformed(&other_type).to_string(),
        "the envelope is not an in-toto statement"
    );

    let mut not_a_statement = signed(&statement());
    let mut envelope = envelope_of(IN_TOTO_PAYLOAD_TYPE, &statement());
    envelope["payload"] = json!(b64("not json"));
    annotate(&mut not_a_statement, &sig, json!(b64(envelope.to_string())));
    assert_eq!(
        malformed(&not_a_statement).to_string(),
        "the payload is not JSON"
    );
}

#[test]
fn a_payload_at_the_cap_decodes_and_one_byte_more_does_not() {
    let at_cap = vec![b'x'; MOST_DECODED_BYTES];
    assert_eq!(
        decode(&b64(&at_cap), "the payload").unwrap().len(),
        MOST_DECODED_BYTES
    );
    let over = vec![b'x'; MOST_DECODED_BYTES + 1];
    assert_eq!(
        decode(&b64(&over), "the payload"),
        Err(Malformed::TooLarge("the payload"))
    );
    // Far past the cap, it is refused before it is decoded.
    let huge = "A".repeat(MOST_DECODED_BYTES * 2);
    assert_eq!(
        decode(&huge, "the payload"),
        Err(Malformed::TooLarge("the payload"))
    );
}

#[test]
fn a_signature_annotation_over_the_cap_is_too_large_not_truncated() {
    let mut run = signed(&statement());
    annotate(
        &mut run,
        &format!("chains.tekton.dev/signature-taskrun-{UID}"),
        json!(b64(vec![b'x'; MOST_DECODED_BYTES + 1])),
    );
    let problem = malformed(&run);
    assert_eq!(problem, Malformed::TooLarge("the signature"));
    assert_eq!(problem.to_string(), "the signature is larger than 256 KiB");
}

#[test]
fn only_the_first_sixteen_attestations_are_read_and_the_rest_are_counted() {
    let mut annotations = serde_json::Map::new();
    for index in 0..(MOST_ATTESTATIONS + 3) {
        annotations.insert(
            format!("chains.tekton.dev/signature-taskrun-{index:02}"),
            json!(b64(
                envelope_of(IN_TOTO_PAYLOAD_TYPE, &statement()).to_string()
            )),
        );
    }
    let found = parse_attestations(&json!({"metadata": {"annotations": annotations}}));
    assert_eq!(found.read.len(), MOST_ATTESTATIONS);
    assert_eq!(found.omitted, 3);
    assert_eq!(found.read[0].key, "taskrun-00", "in key order");
}

#[test]
fn only_the_first_sixty_four_subjects_are_kept_and_the_rest_are_counted() {
    let mut many = statement();
    many["subject"] = Value::Array(
        (0..MOST_SUBJECTS + 5)
            .map(|index| json!({"name": format!("acme/image-{index}"), "digest": {"sha256": "ab"}}))
            .collect(),
    );
    let attestation = only(&signed(&many));
    let claims = &envelope(&attestation).claims;
    assert_eq!(claims.subjects.len(), MOST_SUBJECTS);
    assert_eq!(claims.subjects_omitted, 5);
}

#[test]
fn an_attestation_is_a_declared_observation_of_the_run_it_is_an_annotation_of() {
    let task_run = parse_task_run(&signed(&statement())).unwrap();
    let attestation = &task_run.attestations.read[0];
    let seen = attestation.observation(task_run.object_ref());
    assert_eq!(seen.fact, Fact::Declared);
    assert_eq!(
        seen.field,
        format!("/metadata/annotations/chains.tekton.dev~1signature-taskrun-{UID}")
    );
    assert_eq!(seen.value.as_deref(), Some(DIGEST));
    assert!(seen.describe().contains("declared"));
}

#[test]
fn the_run_parsers_carry_what_chains_stored() {
    let mut value = pipeline_run(COMMIT, true, Some(DIGEST));
    let key = format!("pipelinerun-{UID}");
    annotate(
        &mut value,
        &format!("chains.tekton.dev/signature-{key}"),
        json!(b64(
            envelope_of(IN_TOTO_PAYLOAD_TYPE, &statement()).to_string()
        )),
    );
    let run = parse_pipeline_run(&value).unwrap();
    assert_eq!(run.attestations.read.len(), 1);
    assert_eq!(run.chains_state.as_deref(), Some("true"));

    let plain = parse_pipeline_run(&pipeline_run(COMMIT, true, Some(DIGEST))).unwrap();
    assert_eq!(plain.attestations, Attestations::default());
    assert_eq!(
        parse_task_run(&signed(&statement()))
            .unwrap()
            .attestations
            .read
            .len(),
        1
    );
}

fn config_map(data: Value) -> Value {
    json!({"metadata": {"name": "chains-config", "namespace": "tekton-chains"}, "data": data})
}

#[test]
fn an_empty_config_has_chains_documented_defaults_and_says_it_was_not_set() {
    let config = ChainsConfig::parse(&config_map(json!({})));
    assert_eq!(config.task_runs.backends, vec![Backend::Tekton]);
    assert!(!config.task_runs.storage_set);
    assert_eq!(config.task_runs.signer, "x509");
    assert_eq!(config.pipeline_runs.format, "in-toto");
    assert!(config.task_runs.all_stored_here());
    assert_eq!(config.transparency.enabled, Enabled::Off);
    assert_eq!(config.transparency.url, "https://rekor.sigstore.dev");
    assert!(!config.transparency.url_set);
    assert_eq!(ChainsConfig::parse(&json!({})), config, "no data at all");
}

#[test]
fn a_config_names_where_chains_stores_and_the_log_it_uploads_to() {
    let config = ChainsConfig::parse(&config_map(json!({
        "artifacts.taskrun.storage": "tekton, oci",
        "artifacts.taskrun.format": "slsa/v2alpha3",
        "artifacts.pipelinerun.storage": "oci,docdb",
        "artifacts.pipelinerun.signer": "kms",
        "transparency.enabled": "manual",
        "transparency.url": "https://rekor.example.com"
    })));
    assert_eq!(
        config.task_runs.backends,
        vec![Backend::Tekton, Backend::Oci]
    );
    assert!(config.task_runs.storage_set);
    assert_eq!(config.task_runs.format, "slsa/v2alpha3");
    assert!(
        !config.task_runs.all_stored_here(),
        "the registry holds a copy"
    );
    assert_eq!(
        config.pipeline_runs.backends,
        vec![Backend::Oci, Backend::Other("docdb".into())]
    );
    assert_eq!(config.pipeline_runs.signer, "kms");
    assert_eq!(config.transparency.enabled, Enabled::Manual);
    assert_eq!(config.transparency.url, "https://rekor.example.com");
    assert!(config.transparency.url_set);
}

#[test]
fn an_empty_storage_disables_it_and_an_unknown_switch_is_kept_as_written() {
    let config = ChainsConfig::parse(&config_map(json!({
        "artifacts.taskrun.storage": "",
        "transparency.enabled": "sometimes"
    })));
    assert!(config.task_runs.backends.is_empty());
    assert!(config.task_runs.storage_set);
    assert_eq!(
        config.transparency.enabled,
        Enabled::Other("sometimes".into())
    );
    assert_eq!(
        ChainsConfig::parse(&config_map(json!({"transparency.enabled": "true"})))
            .transparency
            .enabled,
        Enabled::On
    );
}

#[tokio::test]
async fn the_config_is_one_get_of_the_chains_config_map() {
    let reader = FixtureReader::default().with(
        "configmaps",
        vec![config_map(json!({"artifacts.taskrun.storage": "oci"}))],
    );
    let config = read_config(&reader, "tekton-chains").await.unwrap();
    assert_eq!(config.task_runs.backends, vec![Backend::Oci]);
    assert_eq!(
        *reader.requests.borrow(),
        vec!["GET /configmaps Some(\"tekton-chains\") chains-config".to_string()]
    );
}

#[tokio::test]
async fn a_config_that_cannot_be_read_fails_with_why() {
    let missing = read_config(&FixtureReader::default(), "tekton-chains").await;
    assert_eq!(missing.unwrap_err().kind, FailureKind::NotFound);
    let refused = read_config(
        &FixtureReader::default().refusing("configmaps"),
        "tekton-chains",
    )
    .await;
    assert_eq!(refused.unwrap_err().kind, FailureKind::Forbidden);
}
