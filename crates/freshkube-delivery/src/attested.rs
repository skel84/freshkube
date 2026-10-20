//! The supply-chain link and the commit link when a run carries an
//! attestation: what confirms them, and the many things that must not. The
//! envelopes are signed here with a throwaway key; the cosign-made envelope
//! has its own tests beside the verifier.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use super::fixtures::*;
use super::join::{Confidence, Hop, Link, Trail};
use super::observation::Fact;
use super::provenance::stands_on;
use super::tests::{ENV, one, run_configured};
use super::verify::tests::{signed_envelope, signing_key_pem, statement_with, subject};

const RUN_UID: &str = "7c1f0c52-0000-4000-8000-000000000002";
const RUN_KEY: &str = "pipelinerun-7c1f0c52-0000-4000-8000-000000000002";
const PAYLOAD_TYPE: &str = "application/vnd.in-toto+json";
const COSIGN_OTHER_KEY: &str = include_str!("../testdata/cosign/other.pub");

fn b64(bytes: impl AsRef<[u8]>) -> String {
    STANDARD.encode(bytes)
}

fn hex_of(digest: &str) -> &str {
    digest.trim_start_matches("sha256:")
}

/// A provenance statement for `digest` that names `commit` among its
/// materials.
fn provenance(digest: &str, commit: &str) -> Value {
    let mut statement = statement_with(vec![subject(hex_of(digest))]);
    statement["predicate"] = json!({"materials": [
        {"uri": "git+https://git.example/acme/storefront@refs/heads/main", "digest": {"sha1": commit}}
    ]});
    statement
}

/// The annotations Chains' `tekton` storage leaves around `envelope`.
fn annotate(world: &mut World, envelope: &str, statement: &Value) {
    let mut run = pipeline_run(SHA, false, Some(NEW));
    run["metadata"]["uid"] = json!(RUN_UID);
    run["metadata"]["annotations"][format!("chains.tekton.dev/signature-{RUN_KEY}")] =
        json!(b64(envelope));
    run["metadata"]["annotations"][format!("chains.tekton.dev/payload-{RUN_KEY}")] =
        json!(b64(statement.to_string()));
    world.tekton = std::mem::take(&mut world.tekton).with("pipelineruns", vec![run]);
}

fn world_with(envelope: &str, statement: &Value) -> World {
    let mut world = healthy().with_meta();
    annotate(&mut world, envelope, statement);
    world
}

/// The trail of `world` with the key file holding `pem`, or with none named.
async fn trail_with(world: &World, pem: Option<&str>) -> Trail {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cosign.pub");
    if let Some(pem) = pem {
        std::fs::write(&path, pem).unwrap();
    }
    // A squash-merged pull request whose merge commit is the change, so the
    // pull request's links are in every trail.
    let github = FixtureGitHub::with(vec![pull_request(7, OTHER_SHA, Some(SHA))]);
    run_configured(world, &ENV, &github, Some(GH_REPO), |plan| {
        plan.public_key_file = pem.map(|_| path.clone())
    })
    .await
}

/// Signed for one statement and carrying another that still names the
/// build's digest: only the signature check can tell.
fn tampered() -> World {
    let statement = provenance(NEW, SHA);
    let mut envelope: Value =
        serde_json::from_str(&signed_envelope(PAYLOAD_TYPE, &statement)).unwrap();
    let mut other = provenance(NEW, SHA);
    other["predicate"]["builder"] = json!({"id": "https://example.com/other-builder"});
    envelope["payload"] = json!(b64(other.to_string()));
    world_with(&envelope.to_string(), &other)
}

fn signed(statement: &Value) -> World {
    world_with(&signed_envelope(PAYLOAD_TYPE, statement), statement)
}

/// A trail whose one build carries an attestation that verifies.
pub(super) async fn verified_trail() -> Trail {
    trail_with(&signed(&provenance(NEW, SHA)), Some(&signing_key_pem())).await
}

fn supply(trail: &Trail) -> &Link {
    one(trail, Hop::PipelineRun, Hop::SupplyChain)
}

fn never_confirmed(trail: &Trail, why: &str) -> Confidence {
    for link in trail
        .links
        .iter()
        .filter(|link| (link.from, link.to) == (Hop::PipelineRun, Hop::SupplyChain))
    {
        assert_ne!(link.confidence, Confidence::Confirmed, "{why}: {link:#?}");
    }
    supply(trail).confidence
}

#[tokio::test]
async fn a_signature_that_verifies_and_names_the_builds_digest_confirms() {
    let trail = verified_trail().await;
    let link = supply(&trail);
    assert_eq!(link.confidence, Confidence::Confirmed, "{}", link.reason);
    assert!(link.reason.contains("signature verifies with key SHA256:"));
    for hop in [Hop::PipelineRun, Hop::SupplyChain] {
        assert!(stands_on(link, hop), "{hop:?}: {link:#?}");
    }
    let attestation = link
        .evidence
        .iter()
        .find(|seen| seen.object.kind == "Attestation")
        .expect("the attestation the signature verified");
    assert_eq!(attestation.fact, Fact::Reported);
    assert_eq!(attestation.value.as_deref(), Some(NEW));
}

#[tokio::test]
async fn chains_annotation_alone_is_never_confirmed() {
    // No attestation was stored where it is read, a key is named.
    let world = healthy().with_meta();
    let trail = trail_with(&world, Some(&signing_key_pem())).await;
    assert_eq!(never_confirmed(&trail, "annotation"), Confidence::Claimed);
    assert!(supply(&trail).reason.contains("no attestation was found"));
}

#[tokio::test]
async fn a_conforma_pass_alone_is_never_confirmed() {
    let mut world = healthy().with_meta();
    let mut run = pipeline_run(SHA, false, Some(NEW));
    run["metadata"]["annotations"] = json!({});
    world.tekton = world.tekton.with("pipelineruns", vec![run]);
    let trail = trail_with(&world, Some(&signing_key_pem())).await;
    assert!(supply(&trail).reason.contains("Conforma SUCCESS"));
    assert_eq!(never_confirmed(&trail, "conforma"), Confidence::Unknown);
}

#[tokio::test]
async fn an_evidence_record_alone_is_never_confirmed() {
    let mut world = healthy().with_meta();
    world.tekton = world
        .tekton
        .with("taskruns", vec![evidence_task_run(SHA, NEW)]);
    let trail = run_configured(&world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.evidence_result = Some(evidence_result())
    })
    .await;
    assert!(supply(&trail).reason.contains("agrees with"));
    assert_eq!(never_confirmed(&trail, "evidence"), Confidence::Claimed);
}

#[tokio::test]
async fn no_key_named_is_never_confirmed() {
    let trail = trail_with(&signed(&provenance(NEW, SHA)), None).await;
    assert_eq!(never_confirmed(&trail, "no trust"), Confidence::Claimed);
    assert!(supply(&trail).reason.contains("no key is named"));
}

#[tokio::test]
async fn a_key_that_does_not_match_is_never_confirmed() {
    let world = signed(&provenance(NEW, SHA));
    let trail = trail_with(&world, Some(COSIGN_OTHER_KEY)).await;
    assert_eq!(never_confirmed(&trail, "other key"), Confidence::Claimed);
    assert!(supply(&trail).reason.contains("doesn't verify with key"));
}

#[tokio::test]
async fn a_key_that_cannot_be_used_is_never_confirmed() {
    let world = signed(&provenance(NEW, SHA));
    let trail = trail_with(&world, Some("not a key")).await;
    assert_eq!(never_confirmed(&trail, "bad key"), Confidence::Claimed);
}

#[tokio::test]
async fn a_signature_that_fails_is_a_claim_that_says_so() {
    let trail = trail_with(&tampered(), Some(&signing_key_pem())).await;
    // The tampered statement still names the build's digest, so only the
    // signature check can have refused it.
    assert_eq!(never_confirmed(&trail, "tampered"), Confidence::Claimed);
    assert!(
        supply(&trail)
            .reason
            .contains("the signature doesn't verify")
    );
    // What is declared stays declared.
    assert!(
        supply(&trail)
            .evidence
            .iter()
            .filter(|seen| seen.object.name == RUN_KEY || seen.field.contains(RUN_KEY))
            .all(|seen| seen.fact == Fact::Declared)
    );
}

#[tokio::test]
async fn a_subject_digest_that_differs_is_never_confirmed() {
    let world = signed(&provenance(OLD, SHA));
    let trail = trail_with(&world, Some(&signing_key_pem())).await;
    assert_eq!(never_confirmed(&trail, "subject"), Confidence::Claimed);
    assert!(supply(&trail).reason.contains("no subject is the build's"));
}

#[tokio::test]
async fn a_malformed_envelope_is_never_confirmed() {
    let world = world_with("not json", &provenance(NEW, SHA));
    let trail = trail_with(&world, Some(&signing_key_pem())).await;
    assert_eq!(never_confirmed(&trail, "malformed"), Confidence::Claimed);
    assert!(supply(&trail).reason.contains("can't be read"));
}

#[tokio::test]
async fn a_payload_cut_at_the_cap_is_never_confirmed() {
    let mut statement = provenance(NEW, SHA);
    statement["predicate"]["padding"] = json!("x".repeat(300 * 1024));
    let trail = trail_with(&signed(&statement), Some(&signing_key_pem())).await;
    assert_eq!(never_confirmed(&trail, "too large"), Confidence::Claimed);
    assert!(supply(&trail).reason.contains("larger than"));
}

#[tokio::test]
async fn an_attestation_stored_under_an_image_key_is_never_confirmed() {
    // A validly signed in-toto envelope for the build's digest, kept under
    // an image's 12-digit key: not an attestation of the run.
    let statement = provenance(NEW, SHA);
    let mut world = healthy().with_meta();
    let mut run = pipeline_run(SHA, false, Some(NEW));
    run["metadata"]["uid"] = json!(RUN_UID);
    run["metadata"]["annotations"]["chains.tekton.dev/signature-111111111111"] =
        json!(b64(signed_envelope(PAYLOAD_TYPE, &statement)));
    run["metadata"]["annotations"]["chains.tekton.dev/payload-111111111111"] =
        json!(b64(statement.to_string()));
    world.tekton = world.tekton.with("pipelineruns", vec![run]);
    let trail = trail_with(&world, Some(&signing_key_pem())).await;
    assert_eq!(never_confirmed(&trail, "image key"), Confidence::Claimed);
    assert_eq!(
        one(&trail, Hop::Commit, Hop::PipelineRun).confidence,
        Confidence::Claimed
    );
}

#[tokio::test]
async fn an_attestation_copied_from_another_object_is_never_confirmed() {
    // The annotations of a signed run, on a run with another uid.
    let statement = provenance(NEW, SHA);
    let mut world = healthy().with_meta();
    let mut run = pipeline_run(SHA, false, Some(NEW));
    run["metadata"]["uid"] = json!("7c1f0c52-0000-4000-8000-0000000000ff");
    run["metadata"]["annotations"][format!("chains.tekton.dev/signature-{RUN_KEY}")] =
        json!(b64(signed_envelope(PAYLOAD_TYPE, &statement)));
    run["metadata"]["annotations"][format!("chains.tekton.dev/payload-{RUN_KEY}")] =
        json!(b64(statement.to_string()));
    world.tekton = world.tekton.with("pipelineruns", vec![run]);
    let trail = trail_with(&world, Some(&signing_key_pem())).await;
    assert_eq!(never_confirmed(&trail, "copied"), Confidence::Claimed);
    assert!(supply(&trail).reason.contains("belongs to another object"));
    assert!(supply(&trail).signature_warn);
    assert_eq!(
        one(&trail, Hop::Commit, Hop::PipelineRun).confidence,
        Confidence::Claimed
    );
}

#[tokio::test]
async fn a_verified_provenance_that_names_the_commit_is_a_reported_witness() {
    let trail = verified_trail().await;
    let commit = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(
        commit.confidence,
        Confidence::Confirmed,
        "{}",
        commit.reason
    );
    for hop in [Hop::Commit, Hop::PipelineRun] {
        assert!(stands_on(commit, hop), "{hop:?}: {commit:#?}");
    }
    // Without a key, or naming another commit, the label and the revision
    // parameter are all there is.
    let world = signed(&provenance(NEW, SHA));
    let unkeyed = trail_with(&world, None).await;
    assert_eq!(
        one(&unkeyed, Hop::Commit, Hop::PipelineRun).confidence,
        Confidence::Claimed
    );
    let other = trail_with(
        &signed(&provenance(NEW, OTHER_SHA)),
        Some(&signing_key_pem()),
    )
    .await;
    assert_eq!(
        one(&other, Hop::Commit, Hop::PipelineRun).confidence,
        Confidence::Claimed
    );
    // The digest still confirms the supply chain, whatever commit it names.
    assert_eq!(supply(&other).confidence, Confidence::Confirmed);
}

#[tokio::test]
async fn a_provenance_whose_subject_differs_is_not_a_witness() {
    let statement = provenance(OLD, SHA);
    let trail = trail_with(&signed(&statement), Some(&signing_key_pem())).await;
    assert_eq!(
        one(&trail, Hop::Commit, Hop::PipelineRun).confidence,
        Confidence::Claimed
    );
}

#[tokio::test]
async fn a_provenance_that_does_not_verify_is_not_a_witness() {
    // Naming the commit and the build's digest, and signed by no key the
    // person named, or tampered after signing.
    for (world, pem) in [
        (
            signed(&provenance(NEW, SHA)),
            include_str!("../testdata/cosign/other.pub"),
        ),
        (tampered(), &signing_key_pem()),
    ] {
        let trail = trail_with(&world, Some(pem)).await;
        assert_eq!(
            one(&trail, Hop::Commit, Hop::PipelineRun).confidence,
            Confidence::Claimed
        );
    }
}

#[tokio::test]
async fn two_attestations_one_verified_for_another_commit_and_one_unverified_for_this_one() {
    let key = signing_key_pem();
    let mut world = signed(&provenance(NEW, OTHER_SHA));
    // A task of the run carries a tampered provenance that names SHA.
    let tampered_task = {
        let statement = provenance(NEW, SHA);
        let mut envelope: Value =
            serde_json::from_str(&signed_envelope(PAYLOAD_TYPE, &statement)).unwrap();
        let mut other = provenance(NEW, SHA);
        other["predicate"]["builder"] = json!({"id": "https://example.com/other-builder"});
        envelope["payload"] = json!(b64(other.to_string()));
        let task_key = "taskrun-7c1f0c52-0000-4000-8000-000000000003";
        let mut task = task_run();
        task["metadata"]["uid"] = json!("7c1f0c52-0000-4000-8000-000000000003");
        task["metadata"]["annotations"] = json!({
            format!("chains.tekton.dev/signature-{task_key}"): b64(envelope.to_string()),
            format!("chains.tekton.dev/payload-{task_key}"): b64(other.to_string()),
        });
        task
    };
    world.tekton = world.tekton.with("taskruns", vec![tampered_task]);
    let trail = trail_with(&world, Some(&key)).await;
    assert_eq!(
        one(&trail, Hop::Commit, Hop::PipelineRun).confidence,
        Confidence::Claimed
    );
    // The run's own attestation still verifies for the digest.
    assert_eq!(supply(&trail).confidence, Confidence::Confirmed);
}

#[tokio::test]
async fn a_verified_provenance_does_not_raise_the_pull_requests_links() {
    let trail = verified_trail().await;
    assert_eq!(
        one(&trail, Hop::Commit, Hop::PipelineRun).confidence,
        Confidence::Confirmed
    );
    // The pull request's merge commit is the change, and no result of the
    // build reports it: the links through the build stay what the labels
    // alone make them.
    let pr_builds = super::tests::link(&trail, Hop::PullRequest, Hop::PipelineRun);
    assert!(!pr_builds.is_empty());
    for pr_build in pr_builds {
        assert_ne!(
            pr_build.confidence,
            Confidence::Confirmed,
            "{}",
            pr_build.reason
        );
    }
    for link in trail.links.iter().filter(|l| l.from == Hop::PullRequest) {
        assert!(
            link.evidence
                .iter()
                .all(|seen| seen.object.kind != "Attestation"),
            "{link:#?}"
        );
    }
}

#[tokio::test]
async fn the_signature_is_marked_failing_for_exactly_the_four_warn_rows() {
    let statement = provenance(NEW, SHA);
    let key = signing_key_pem();

    let warns = [
        ("key unusable", signed(&statement), Some("not a key")),
        ("signature fails", tampered(), Some(key.as_str())),
        (
            "malformed",
            world_with("not json", &statement),
            Some(key.as_str()),
        ),
        (
            "subject differs",
            signed(&provenance(OLD, SHA)),
            Some(key.as_str()),
        ),
    ];
    for (row, world, pem) in warns {
        let trail = trail_with(&world, pem).await;
        assert!(
            supply(&trail).signature_warn,
            "{row}: {}",
            supply(&trail).reason
        );
        assert_eq!(supply(&trail).confidence, Confidence::Claimed, "{row}");
    }

    // The key can't be used and nothing was stored where it is read: still
    // the key-unusable row, as long as Chains says it signed.
    let unattested = trail_with(&healthy().with_meta(), Some("not a key")).await;
    assert!(supply(&unattested).signature_warn);

    let quiet = [
        ("verified", signed(&statement), Some(key.as_str())),
        ("no key set", signed(&statement), None),
        ("nothing stored", healthy().with_meta(), Some(key.as_str())),
    ];
    for (row, world, pem) in quiet {
        let trail = trail_with(&world, pem).await;
        assert!(!supply(&trail).signature_warn, "{row}");
    }
    // Unknown: no annotation, no attestation.
    let mut bare = healthy().with_meta();
    let mut run = pipeline_run(SHA, false, Some(NEW));
    run["metadata"]["annotations"] = json!({});
    bare.tekton = bare.tekton.with("pipelineruns", vec![run]);
    let unknown = trail_with(&bare, Some("not a key")).await;
    assert_eq!(supply(&unknown).confidence, Confidence::Unknown);
    assert!(!supply(&unknown).signature_warn);
    // No other link carries it.
    let verified = verified_trail().await;
    assert!(verified.links.iter().all(|link| !link.signature_warn));
}
