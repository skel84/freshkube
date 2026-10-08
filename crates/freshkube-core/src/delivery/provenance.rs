//! F01 provenance: what each link says it was read from. One test per rule.

use std::collections::BTreeSet;

use super::fixtures::*;
use super::join::{Confidence, Hop, Key, Link, Trail, render};
use super::observation::{Fact, Observation, role};
use super::source::Source;
use super::tests::{
    ENV, link, observed_at, one, promoted, run, run_configured, run_with, squash_world,
};
use crate::resources::{Failure, FailureKind};

/// The kinds of read object whose observations stand on a hop's side of a
/// link. The change itself is the Commit hop's only side.
fn kinds(hop: Hop) -> &'static [&'static str] {
    match hop {
        // A pull request joins the Freight through the build of its head.
        Hop::PullRequest => &["PullRequest", "PipelineRun"],
        // The change itself, or the Freight's commit it was joined to.
        Hop::Commit => &["Commit"],
        Hop::PipelineRun => &["PipelineRun", "TaskRun"],
        // Only an annotation of Chains' says a build was signed, and it is
        // declared; no read object reports the signature.
        Hop::SupplyChain => &[],
        Hop::Freight => &["Freight"],
        Hop::Promotion => &["Promotion"],
        // Kargo records the push on the Promotion, not on the Stage.
        Hop::Stage => &["Stage", "Promotion"],
        Hop::Application => &["Application"],
        // A Rollout reports no image; see `rollout_stands_on_its_pods`.
        Hop::Rollout => &["Rollout", "ReplicaSet"],
        Hop::Pod => &["Pod"],
    }
}

fn key_text(key: &Key) -> String {
    match key {
        Key::Sha(sha) => sha.clone(),
        Key::Digest(digest) => digest.to_string(),
        other => panic!("a confirmed link joins on a SHA or a digest, not {other}"),
    }
}

/// Whether the link has, on `hop`'s side, an observation of a read object
/// that is reported, never declared or derived, and carries the link's key.
/// A Rollout's side stands on its own pods.
fn stands_on(link: &Link, hop: Hop) -> bool {
    let key = key_text(&link.key);
    if hop == Hop::Rollout && rollout_stands_on_its_pods(link, &key) {
        return true;
    }
    link.evidence.iter().any(|seen| {
        kinds(hop).contains(&seen.object.kind.as_str())
            // Only what a controller reported stands for a side. A derived
            // observation does not; the change is the Commit side's one input.
            && (seen.fact == Fact::Reported
                || (hop == Hop::Commit && seen.cluster == role::CHANGE))
            && seen
                .value
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(&key))
    })
}

/// A Rollout reports no image, and its spec's pin is declared, so its side
/// stands on its own pods through a chain of reported values: the Rollout's
/// current pod hash; a ReplicaSet whose owner reference names the Rollout's
/// UID and whose label carries the hash; a pod whose owner reference names
/// that ReplicaSet's UID, whose label carries the hash, and that reports the
/// key.
fn rollout_stands_on_its_pods(link: &Link, key: &str) -> bool {
    let label = "/metadata/labels/rollouts-pod-template-hash";
    let reported = |kind: &'static str, field: &'static str| {
        link.evidence.iter().filter(move |seen| {
            seen.fact == Fact::Reported && seen.object.kind == kind && seen.field == field
        })
    };
    let on = |kind, field, object: &super::observation::ObjectRef, value: Option<&str>| {
        reported(kind, field).any(|seen| &seen.object == object && seen.value.as_deref() == value)
    };
    reported("Rollout", "/status/currentPodHash").any(|rollout| {
        let hash = rollout.value.as_deref();
        reported("ReplicaSet", "/metadata/ownerReferences")
            .filter(|set| set.value.is_some() && set.value == rollout.object.uid)
            .filter(|set| on("ReplicaSet", label, &set.object, hash))
            .any(|set| {
                reported("Pod", "/metadata/ownerReferences")
                    .filter(|pod| pod.value.is_some() && pod.value == set.object.uid)
                    .any(|pod| {
                        on("Pod", label, &pod.object, hash)
                            && reported("Pod", "/status/containerStatuses").any(|running| {
                                running.object == pod.object
                                    && running
                                        .value
                                        .as_deref()
                                        .is_some_and(|value| value.eq_ignore_ascii_case(key))
                            })
                    })
            })
    })
}

/// The sides of confirmed links that no read object stands on: `(from, to,
/// side)`, each a confirmed link whose evidence on that side is declared
/// only, or absent. #387 closed the last of them; a side that stands on
/// nothing reported is Claimed instead, so a new entry needs a reason.
const KNOWN_GAPS: &[(&str, &str, &str)] = &[];

fn github() -> FixtureGitHub {
    FixtureGitHub::with(vec![pull_request(7, OTHER_SHA, Some(SHA))])
}

/// Every kind of link confirmed at once: a squash-merged pull request whose
/// head build shipped, signed by Chains, promoted to a Stage whose
/// Application synced the pushed commit and whose image summary lists the
/// digest, pinned by its Rollout, running.
async fn confirmed() -> Trail {
    let mut world = promoted(NEW, "Succeeded", PUSHED).with_meta();
    let image = format!("{REPO}@{NEW}");
    let mut app = summarised(application(Some("https://env-a.example:6443")), &[&image]);
    app["status"]["sync"]["revision"] = serde_json::json!(PUSHED);
    world.argocd = world.argocd.with("applications", vec![app]);
    world.environment = world
        .environment
        .with("rollouts", vec![rollout(&image)])
        .with("replicasets", vec![replica_set("5d9c", &image, 1, 1)]);
    world.tekton = world.tekton.with(
        "pipelineruns",
        vec![
            pipeline_run(SHA, true, Some(NEW)),
            pr_pipeline_run(OTHER_SHA, 7, NEW),
        ],
    );
    world.tekton = world.tekton.with(
        "taskruns",
        vec![
            task_run(),
            clone_task_run("storefront-push-x", SHA),
            clone_task_run("storefront-pr-y", OTHER_SHA),
        ],
    );
    run_with(&world, &ENV, &github(), Some(GH_REPO)).await
}

/// Every world the file builds, and some where a side has little to show.
async fn every_trail() -> Vec<Trail> {
    // The commit joins a Freight with no build read at all.
    let mut alone = healthy();
    alone.tekton = alone.tekton.with("pipelineruns", vec![]);
    // The build reports no image digest, so only labels name the commit.
    let mut no_digest = healthy();
    no_digest.tekton = no_digest
        .tekton
        .with("pipelineruns", vec![pipeline_run(SHA, true, None)]);
    vec![
        confirmed().await,
        run(&healthy(), &ENV).await,
        run(&promoted(NEW, "Succeeded", PUSHED), &ENV).await,
        run_with(&squash_world(), &ENV, &github(), Some(GH_REPO)).await,
        run(&without_rollouts(), &ENV).await,
        run(&alone, &ENV).await,
        run(&no_digest, &ENV).await,
    ]
}

#[tokio::test]
async fn a_confirmed_link_stands_on_what_was_read_on_each_side() {
    let trails = every_trail().await;
    let mut checked = BTreeSet::new();
    let mut gaps = BTreeSet::new();
    for trail in &trails {
        for link in trail
            .links
            .iter()
            .filter(|link| link.confidence == Confidence::Confirmed)
        {
            checked.insert((link.from.word(), link.to.word()));
            for hop in [link.from, link.to] {
                if !stands_on(link, hop) {
                    gaps.insert((link.from.word(), link.to.word(), hop.word()));
                }
            }
        }
    }
    for pair in [
        (Hop::PullRequest, Hop::Commit),
        (Hop::PullRequest, Hop::PipelineRun),
        (Hop::PullRequest, Hop::Freight),
        (Hop::Commit, Hop::PipelineRun),
        (Hop::PipelineRun, Hop::Freight),
        (Hop::Commit, Hop::Freight),
        (Hop::Freight, Hop::Promotion),
        (Hop::Freight, Hop::Stage),
        (Hop::Stage, Hop::Application),
        (Hop::Application, Hop::Rollout),
        (Hop::Rollout, Hop::Pod),
    ] {
        assert!(
            checked.contains(&(pair.0.word(), pair.1.word())),
            "{pair:?} was not confirmed, so not checked: {checked:?}"
        );
    }
    // Exactly the known gaps: a new one is a bug here, and a closed one
    // should leave the list.
    let known: BTreeSet<_> = KNOWN_GAPS.iter().copied().collect();
    assert_eq!(gaps, known, "gaps found, then known");
}

#[tokio::test]
async fn a_result_that_names_the_commit_is_reported_evidence_for_the_run() {
    // The run's own witness is a task result, not a declared parameter.
    let mut world = healthy().with_meta();
    world.tekton = world
        .tekton
        .with("pipelineruns", vec![pipeline_run(SHA, false, Some(NEW))]);
    let mut task = task_run();
    task["status"]["results"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"name": "commit", "value": SHA}));
    world.tekton = world.tekton.with("taskruns", vec![task]);
    let trail = run(&world, &ENV).await;
    let commit = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(commit.confidence, Confidence::Confirmed);
    for hop in [Hop::Commit, Hop::PipelineRun] {
        assert!(stands_on(commit, hop), "{hop:?}: {commit:#?}");
    }
    let task_side = commit
        .evidence
        .iter()
        .find(|seen| seen.object.kind == "TaskRun")
        .expect("the task run that wrote the result");
    assert_eq!(task_side.fact, Fact::Reported);
    assert_eq!(task_side.field, "/status/results");
}

/// A world whose one build is labelled with `SHA`, with a revision parameter
/// naming `param` and a clone TaskRun reporting `reported`, when given.
fn build_world(param: Option<&str>, reported: Option<&str>) -> World {
    let mut world = healthy().with_meta();
    let mut run_value = pipeline_run(SHA, false, Some(NEW));
    if let Some(param) = param {
        run_value["spec"]["params"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"name": "revision", "value": param}));
    }
    world.tekton = world.tekton.with("pipelineruns", vec![run_value]);
    let tasks = reported.map_or_else(
        || vec![task_run()],
        |sha| vec![task_run(), clone_task_run("storefront-push-x", sha)],
    );
    world.tekton = world.tekton.with("taskruns", tasks);
    world
}

#[tokio::test]
async fn a_label_and_a_parameter_that_agree_are_a_claim_both_declared() {
    let trail = run(&build_world(Some(SHA), None), &ENV).await;
    let commit = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(commit.confidence, Confidence::Claimed);
    assert!(
        commit.reason.contains("agree, both declared"),
        "{}",
        commit.reason
    );
    assert!(
        commit.reason.contains("no result reports"),
        "{}",
        commit.reason
    );
    assert!(!stands_on(commit, Hop::PipelineRun));
}

#[tokio::test]
async fn a_task_result_naming_the_commit_confirms_beside_a_parameter() {
    let trail = run(&build_world(Some(SHA), Some(SHA)), &ENV).await;
    let commit = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(commit.confidence, Confidence::Confirmed);
    assert!(stands_on(commit, Hop::PipelineRun), "{commit:#?}");
    let rule = commit
        .evidence
        .iter()
        .find(|seen| seen.object.kind == "Join")
        .expect("the join's conclusion");
    assert_eq!(rule.object.name, "a task result == the change");
    assert!(
        !commit
            .evidence
            .iter()
            .any(|seen| seen.field == "/spec/params"),
        "the reported witness wins: {commit:#?}"
    );
}

#[tokio::test]
async fn a_pipeline_that_clones_two_repositories_confirms_in_either_order() {
    let source = || clone_task_run("storefront-push-x", SHA);
    let config = || clone_task_run_named("storefront-push-x-clone-config", OTHER_SHA);
    for tasks in [vec![config(), source()], vec![source(), config()]] {
        let mut world = build_world(Some(SHA), None);
        world.tekton = world.tekton.with("taskruns", tasks);
        let trail = run(&world, &ENV).await;
        let commit = one(&trail, Hop::Commit, Hop::PipelineRun);
        assert_eq!(
            commit.confidence,
            Confidence::Confirmed,
            "{}",
            commit.reason
        );
        assert!(stands_on(commit, Hop::PipelineRun), "{commit:#?}");
    }
    // With neither equal, every distinct commit is named, once.
    let mut world = build_world(Some(SHA), None);
    let third = "c".repeat(40);
    world.tekton = world.tekton.with(
        "taskruns",
        vec![
            clone_task_run_named("storefront-push-x-clone-config", OTHER_SHA),
            clone_task_run_named("storefront-push-x-clone-lib", &third),
            clone_task_run_named("storefront-push-x-clone-dup", OTHER_SHA),
        ],
    );
    let trail = run(&world, &ENV).await;
    let commit = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(commit.confidence, Confidence::Claimed);
    assert!(
        commit.reason.contains(&OTHER_SHA[..12]),
        "{}",
        commit.reason
    );
    assert!(commit.reason.contains(&third[..12]), "{}", commit.reason);
    assert_eq!(commit.reason.matches(&OTHER_SHA[..12]).count(), 1);
}

#[tokio::test]
async fn a_configured_result_name_confirms_through_the_commit_link() {
    let mut world = build_world(None, None);
    let mut task = clone_task_run("storefront-push-x", SHA);
    task["status"]["results"][0]["name"] = serde_json::json!("made-up-result");
    world.tekton = world.tekton.with("taskruns", vec![task]);
    let plain = run(&world, &ENV).await;
    assert_eq!(
        one(&plain, Hop::Commit, Hop::PipelineRun).confidence,
        Confidence::Claimed
    );
    let configured = run_configured(&world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.commit_names.results = vec!["made-up-result".into()]
    })
    .await;
    let commit = one(&configured, Hop::Commit, Hop::PipelineRun);
    assert_eq!(
        commit.confidence,
        Confidence::Confirmed,
        "{}",
        commit.reason
    );
    assert!(stands_on(commit, Hop::PipelineRun), "{commit:#?}");
}

#[tokio::test]
async fn a_result_naming_another_commit_is_a_claim_whatever_the_declared_fields_say() {
    let trail = run(&build_world(Some(SHA), Some(OTHER_SHA)), &ENV).await;
    let commit = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(commit.confidence, Confidence::Claimed);
    assert!(
        commit.reason.contains("report other commits"),
        "{}",
        commit.reason
    );
    assert!(
        commit.reason.contains(&OTHER_SHA[..12]),
        "{}",
        commit.reason
    );
    // The reported mismatch is in the evidence, and the join concluded nothing.
    assert!(
        commit.evidence.iter().any(|seen| {
            seen.fact == Fact::Reported
                && seen.field == "/status/results"
                && seen.value.as_deref() == Some(OTHER_SHA)
        }),
        "{commit:#?}"
    );
    assert!(
        commit
            .evidence
            .iter()
            .all(|seen| seen.object.kind != "Join"),
        "{commit:#?}"
    );
}

/// A build of `SHA` that reports no image, so the Freight joins on the commit.
/// Besides the label it has a revision parameter naming `SHA` when `param`,
/// and a clone TaskRun reporting `reported`, when given.
fn sha_joined(param: bool, reported: Option<&str>) -> World {
    let mut world = build_world(None, reported);
    world.tekton = world
        .tekton
        .with("pipelineruns", vec![pipeline_run(SHA, param, None)]);
    world
}

#[tokio::test]
async fn a_sha_joined_freight_with_no_build_joins_the_commit_itself() {
    let mut world = healthy().with_meta();
    world.tekton = world.tekton.with("pipelineruns", vec![]);
    let trail = run(&world, &ENV).await;
    assert!(link(&trail, Hop::PipelineRun, Hop::Freight).is_empty());
    let freight = one(&trail, Hop::Commit, Hop::Freight);
    assert_eq!(freight.confidence, Confidence::Confirmed);
    assert_eq!(freight.key, Key::Sha(SHA.into()));
    for hop in [Hop::Commit, Hop::Freight] {
        assert!(stands_on(freight, hop), "{hop:?}: {freight:#?}");
    }
    // No PipelineRun is named: none was seen.
    assert!(
        freight
            .evidence
            .iter()
            .all(|seen| seen.object.kind != "PipelineRun"),
        "{freight:#?}"
    );
    // The chain still reaches the pods, so the summary reads as before.
    assert!(
        trail
            .summary()
            .starts_with("running, confirmed: pods run the digest; "),
        "{}",
        trail.summary()
    );
    assert_eq!(trail.running(), Some(Confidence::Confirmed));
}

#[tokio::test]
async fn a_sha_joined_freight_whose_build_is_tied_by_declared_fields_is_a_claim() {
    // A label alone, and a label with a revision parameter.
    for param in [false, true] {
        let trail = run(&sha_joined(param, None), &ENV).await;
        let freight = one(&trail, Hop::PipelineRun, Hop::Freight);
        assert_eq!(freight.key, Key::Sha(SHA.into()));
        assert_eq!(freight.confidence, Confidence::Claimed, "{param}");
        assert!(
            freight.reason.contains(if param {
                "the build's tie to it is the PaC label and the revision parameter, both declared"
            } else {
                "the build's tie to it is its declared label"
            }),
            "{}",
            freight.reason
        );
        assert!(!stands_on(freight, Hop::PipelineRun));
        assert!(stands_on(freight, Hop::Freight));
    }
}

#[tokio::test]
async fn a_sha_joined_freight_whose_build_reports_another_commit_says_so() {
    let trail = run(&sha_joined(true, Some(OTHER_SHA)), &ENV).await;
    let freight = one(&trail, Hop::PipelineRun, Hop::Freight);
    assert_eq!(freight.confidence, Confidence::Claimed);
    assert!(
        freight
            .reason
            .contains("the build's tie to it is that a result of the build reports another commit"),
        "{}",
        freight.reason
    );
}

#[tokio::test]
async fn a_sha_joined_freight_whose_build_reports_the_commit_stays_confirmed() {
    let trail = run(&sha_joined(true, Some(SHA)), &ENV).await;
    let freight = one(&trail, Hop::PipelineRun, Hop::Freight);
    assert_eq!(freight.confidence, Confidence::Confirmed);
    for hop in [Hop::PipelineRun, Hop::Freight] {
        assert!(stands_on(freight, hop), "{hop:?}: {freight:#?}");
    }
    // Only reported observations stand on either side.
    assert!(
        freight
            .evidence
            .iter()
            .all(|seen| seen.fact == Fact::Reported),
        "{freight:#?}"
    );
}

#[tokio::test]
async fn chains_signed_annotation_is_a_claim_never_a_confirmation() {
    let signed = |annotation: Option<&str>| {
        let mut world = healthy().with_meta();
        let mut run_value = pipeline_run(SHA, true, Some(NEW));
        match annotation {
            Some(state) => {
                run_value["metadata"]["annotations"]["chains.tekton.dev/signed"] =
                    serde_json::json!(state)
            }
            None => {
                run_value["metadata"]["annotations"] = serde_json::json!({});
            }
        }
        world.tekton = world.tekton.with("pipelineruns", vec![run_value]);
        world
    };
    let trail = run(&signed(Some("true")), &ENV).await;
    let chain = one(&trail, Hop::PipelineRun, Hop::SupplyChain);
    assert_eq!(chain.confidence, Confidence::Claimed);
    assert!(
        chain
            .reason
            .contains("Chains' annotation says signed; the signature was not verified"),
        "{}",
        chain.reason
    );
    // The Conforma verdict stays in the reason.
    assert!(
        chain.reason.contains("Conforma SUCCESS"),
        "{}",
        chain.reason
    );
    for (annotation, words) in [
        (Some("failed"), "signed=failed"),
        (None, "no Chains annotation"),
    ] {
        let trail = run(&signed(annotation), &ENV).await;
        let chain = one(&trail, Hop::PipelineRun, Hop::SupplyChain);
        assert_eq!(chain.confidence, Confidence::Unknown, "{annotation:?}");
        assert!(chain.reason.contains(words), "{}", chain.reason);
    }
}

#[tokio::test]
async fn a_conclusion_is_the_joins_and_never_an_objects() {
    let trail = confirmed().await;
    let concluded: Vec<&Observation> = trail
        .links
        .iter()
        .flat_map(|link| &link.evidence)
        .filter(|seen| seen.fact == Fact::Derived && seen.cluster != role::CHANGE)
        .collect();
    assert!(!concluded.is_empty());
    for seen in concluded {
        assert_eq!(seen.cluster, role::JOIN, "{seen:#?}");
        assert_eq!(seen.object.kind, "Join", "{seen:#?}");
    }
    // A run whose own revision differs from its label is not "concluded"
    // to agree with it.
    let mut world = healthy().with_meta();
    let mut differing = pipeline_run(SHA, true, Some(NEW));
    differing["spec"]["params"] = serde_json::json!([{"name": "revision", "value": OTHER_SHA}]);
    world.tekton = world.tekton.with("pipelineruns", vec![differing]);
    let trail = run(&world, &ENV).await;
    let commit = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(commit.confidence, Confidence::Claimed);
    assert!(
        commit
            .evidence
            .iter()
            .all(|seen| seen.fact != Fact::Derived || seen.cluster == role::CHANGE),
        "{commit:#?}"
    );
}

/// What `why_not_read` says of the failures the test's readers answer with.
fn unread_reasons() -> Vec<String> {
    [
        Failure::new(FailureKind::Forbidden, "forbidden by RBAC"),
        Failure::new(FailureKind::Forbidden, "gh: Forbidden (HTTP 403)"),
        Failure::new(FailureKind::Unreachable, "connection refused"),
    ]
    .into_iter()
    .filter_map(|failure| Source::<()>::from_result(Err(failure)).why_not_read())
    .collect()
}

#[tokio::test]
async fn a_link_from_an_unread_source_carries_no_observations() {
    // Builds and pull requests unread in one world, pods in another, and
    // Applications in a third: each ends the chain, or hides what follows.
    let mut builds = healthy();
    builds.tekton = builds.tekton.refusing("pipelineruns");
    let mut pods = healthy();
    pods.environment = pods.environment.refusing("pods");
    let mut applications = healthy();
    applications.argocd = applications.argocd.unreachable("applications");
    let trails = [
        run_with(&builds, &ENV, &FixtureGitHub::refusing(), Some(GH_REPO)).await,
        run(&pods, &ENV).await,
        run(&applications, &ENV).await,
    ];
    let mut hops = BTreeSet::new();
    for trail in &trails {
        for link in trail.links.iter().filter(|link| {
            link.reason.starts_with("not readable") || link.reason.starts_with("not installed")
        }) {
            assert_eq!(link.confidence, Confidence::Unknown);
            assert!(link.evidence.is_empty(), "{link:#?}");
            // The reason is what the source that couldn't be read says.
            assert!(unread_reasons().contains(&link.reason), "{link:#?}");
            hops.insert((link.from.word(), link.to.word()));
        }
    }
    for expected in [
        (Hop::PullRequest, Hop::Commit),
        (Hop::Commit, Hop::PipelineRun),
        (Hop::Stage, Hop::Application),
        (Hop::Rollout, Hop::Pod),
    ] {
        assert!(
            hops.contains(&(expected.0.word(), expected.1.word())),
            "{expected:?} was not unread: {hops:?}"
        );
    }
}

#[tokio::test]
async fn a_capped_source_never_yields_none() {
    // The only Freight read doesn't match, and the listing stopped at the cap.
    let mut world = healthy();
    world.kargo = world
        .kargo
        .with("freights", vec![freight("f-old", OLD, OTHER_SHA)])
        .capped("freights");
    let trail = run(&world, &ENV).await;
    let freight = one(&trail, Hop::PipelineRun, Hop::Freight);
    assert_eq!(freight.confidence, Confidence::Unknown);
    assert!(freight.reason.contains("page cap"), "{}", freight.reason);

    // Pods of another digest, from a capped listing: the link keeps what was
    // read and still says the right pod may be among those not read.
    let mut world = healthy();
    world.environment = world
        .environment
        .with(
            "pods",
            vec![pod(
                "storefront-5d9c-x",
                &format!("{REPO}:v1.4.0"),
                &format!("docker-pullable://{REPO}@{OLD}"),
            )],
        )
        .capped("pods");
    let trail = run(&world, &ENV).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Claimed);
    assert!(pods.reason.contains("page cap"), "{}", pods.reason);
    assert!(!pods.evidence.is_empty());
    assert!(
        pods.evidence
            .iter()
            .filter(|seen| seen.object.kind == "Pod")
            .all(|seen| seen.value.as_deref() == Some(OLD)),
        "{pods:#?}"
    );
}

#[tokio::test]
async fn an_observation_never_holds_a_server_an_identity_or_a_secret() {
    let mut world = promoted(NEW, "Succeeded", PUSHED).with_meta();
    world.environment = world
        .environment
        .with("rollouts", vec![rollout(&format!("{REPO}@{NEW}"))]);
    let mut promotion = promotion_pushing("f-new", NEW, "Succeeded", PUSHED);
    promotion["metadata"]["annotations"] =
        serde_json::json!({"kargo.akuity.io/create-actor": "email:alice@example.test"});
    promotion["status"]["message"] = serde_json::json!("pushed by alice@example.test");
    world.kargo = world.kargo.with("promotions", vec![promotion]);
    // A pod that carries a Secret's value in its environment.
    let mut leaky = pod(
        "storefront-5d9c-x",
        &format!("{REPO}:v1.4.0"),
        &format!("docker-pullable://{REPO}@{NEW}"),
    );
    leaky["spec"] = serde_json::json!({"containers": [
        {"name": "app", "env": [{"name": "TOKEN", "value": "s3cr3t-value"}]}]});
    world.environment = world.environment.with("pods", vec![leaky]);
    world.tekton = world.tekton.with(
        "taskruns",
        vec![task_run(), clone_task_run("storefront-push-x", SHA)],
    );
    let trail = run_with(&world, &ENV, &github(), Some(GH_REPO)).await;
    // A Freight joined on the commit with no build, and one with a label-only build.
    let mut alone = healthy().with_meta();
    alone.tekton = alone.tekton.with("pipelineruns", vec![]);
    let alone = run(&alone, &ENV).await;
    let label_only = run(&sha_joined(false, None), &ENV).await;

    let observations: Vec<&Observation> = [&trail, &alone, &label_only]
        .into_iter()
        .flat_map(|trail| &trail.links)
        .flat_map(|link| &link.evidence)
        .collect();
    assert!(observations.len() > 10, "{}", observations.len());
    let rendered = [&trail, &alone, &label_only]
        .into_iter()
        .map(render)
        .collect::<String>();
    let forbidden = [
        "env-a.example",
        "://",
        "alice",
        "email:",
        "s3cr3t-value",
        "git.example",
        "registry.example",
    ];
    for seen in observations {
        let text = format!("{seen:?}");
        for word in forbidden {
            assert!(!text.contains(word), "{word} in {seen:#?}");
        }
    }
    for line in rendered
        .lines()
        .filter(|l| l.trim_start().starts_with("from "))
    {
        for word in forbidden {
            assert!(!line.contains(word), "{word} in {line}");
        }
    }
}

#[tokio::test]
async fn every_cluster_observation_names_its_object_and_version() {
    let trail = confirmed().await;
    let cluster = ["kargo", "argocd", "tekton", "environment"];
    let mut count = 0;
    for seen in trail.links.iter().flat_map(|link| &link.evidence) {
        // An image is known by its digest alone.
        if cluster.contains(&seen.cluster.as_str()) && seen.object.kind != "Image" {
            count += 1;
            assert!(seen.object.uid.is_some(), "{seen:#?}");
            assert!(seen.object.resource_version.is_some(), "{seen:#?}");
            assert!(seen.field.starts_with('/'), "{seen:#?}");
        }
    }
    assert!(count > 10, "{count}");
    // GitHub's answer has neither, and says nothing it hasn't.
    assert!(trail.links.iter().flat_map(|l| &l.evidence).any(|seen| {
        seen.cluster == "github" && seen.object.uid.is_none() && seen.fact == Fact::Reported
    }));
}

#[tokio::test]
async fn the_trail_carries_the_callers_clock_and_renders_its_sources() {
    let trail = confirmed().await;
    assert_eq!(trail.observed_at, observed_at());
    let rendered = render(&trail);
    assert!(
        rendered.contains("(read 2026-10-06T12:00:00Z)"),
        "{rendered}"
    );
    let froms: Vec<&str> = rendered
        .lines()
        .filter(|line| line.starts_with("      from "))
        .collect();
    assert!(froms.len() > 10, "{rendered}");
    assert!(
        froms
            .iter()
            .any(|line| line.contains("Rollout.argoproj.io shop/storefront uid ")),
        "{rendered}"
    );
    // The summary is as it was.
    assert!(
        trail
            .summary()
            .starts_with("running, confirmed: pods run the digest; "),
        "{}",
        trail.summary()
    );
}

#[tokio::test]
async fn the_existing_fixtures_keep_their_confidence_counts() {
    let count = |trail: &Trail, confidence| {
        trail
            .links
            .iter()
            .filter(|link| link.confidence == confidence)
            .count()
    };
    let healthy = run(&healthy(), &ENV).await;
    let promoted = run(&promoted(NEW, "Succeeded", PUSHED), &ENV).await;
    let confirmed = confirmed().await;
    let squash = run_with(&squash_world(), &ENV, &github(), Some(GH_REPO)).await;
    let got: Vec<[usize; 3]> = [&healthy, &promoted, &confirmed, &squash]
        .iter()
        .map(|trail| {
            [
                count(trail, Confidence::Confirmed),
                count(trail, Confidence::Claimed),
                count(trail, Confidence::Unknown),
            ]
        })
        .collect();
    assert_eq!(got, COUNTS, "{got:?}");
    assert!(link(&healthy, Hop::Freight, Hop::Stage).len() == 1);
}

/// `[confirmed, claimed, unknown]` of each fixture.
const COUNTS: [[usize; 3]; 4] = [[2, 6, 0], [5, 3, 0], [11, 1, 0], [3, 9, 0]];
// healthy: Commit -> PipelineRun (declared label and parameter) and PipelineRun -> supply chain.
// promoted: the same two links.
// confirmed: only PipelineRun -> supply chain; the clone TaskRun's `commit` result keeps
// Commit -> PipelineRun and PullRequest -> PipelineRun Confirmed.
// squash: Commit -> PipelineRun, both PullRequest -> PipelineRun links, PipelineRun ->
// supply chain, and PullRequest -> Freight (the head build is tied to the head commit by
// declared fields only; `storefront-pr-y` has no clone TaskRun there).
// #387 moved none: Rollout -> pods stays Confirmed in every fixture on its current pods'
// owner chain to the Rollout, which the fixtures serve as a cluster does; Application ->
// Rollout stays Confirmed in `confirmed`, whose Application's image summary lists the
// digest its pods run.

#[test]
fn the_rule_fails_on_a_side_with_nothing_read() {
    use super::join::{Hop, Link};
    use super::observation::{Meta, ObjectRef};
    let digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    let object =
        |kind: &str| ObjectRef::new("argoproj.io", kind, Some("shop"), "api", &Meta::default());
    let key = Key::Digest(super::Digest::parse(digest).unwrap());
    let link = |evidence: Vec<Observation>| Link {
        from: Hop::Application,
        to: Hop::Rollout,
        subject: "shop/api".into(),
        key: key.clone(),
        confidence: Confidence::Confirmed,
        reason: String::new(),
        evidence,
    };
    let declared =
        Observation::declared(role::ENVIRONMENT, object("Rollout"), "/spec", Some(digest));
    let concluded = Observation::derived(
        role::JOIN,
        ObjectRef::new("", "Join", None, "rule", &Meta::default()),
        "/key",
        Some(digest),
    );
    let on_rollout =
        Observation::derived(role::ENVIRONMENT, object("Rollout"), "/spec", Some(digest));
    let reported = Observation::reported(
        role::ENVIRONMENT,
        object("Rollout"),
        "/status",
        Some(digest),
    );
    // Declared, a join's conclusion, or a derived copy of a read field: none
    // is a side that was read.
    for evidence in [vec![declared.clone()], vec![concluded], vec![on_rollout]] {
        assert!(!stands_on(&link(evidence), Hop::Rollout));
    }
    // A value that isn't the key is not it either.
    let other = Observation::reported(role::ENVIRONMENT, object("Rollout"), "/status", Some("x"));
    assert!(!stands_on(&link(vec![other]), Hop::Rollout));
    // What the pods or another hop report is not this side.
    assert!(!stands_on(&link(vec![reported.clone()]), Hop::Application));
    assert!(stands_on(&link(vec![declared, reported]), Hop::Rollout));
}

#[test]
fn each_value_keeps_the_field_it_was_read_from() {
    use super::kargo::{parse_freight, parse_promotion, parse_stage};
    use super::observation::pointer_segment;
    let digest = |hex: char| format!("sha256:{}", hex.to_string().repeat(64));
    let (x, y) = (digest('1'), digest('2'));
    let parsed = |d: &str| super::Digest::parse(d).unwrap();
    // Older Kargo: the current Freight A, and the last Promotion's B.
    let stage = parse_stage(&serde_json::json!({
        "metadata": {"namespace": "storefront", "name": "dev"},
        "status": {
            "currentFreight": {"name": "a", "images": [{"digest": x}]},
            "lastPromotion": {"name": "p", "freight": {"name": "b", "images": [{"digest": y}]}},
        }
    }))
    .unwrap();
    assert_eq!(stage.freight_pointer("a"), "/status/currentFreight/name");
    assert_eq!(
        stage.freight_pointer("b"),
        "/status/lastPromotion/freight/name"
    );
    assert_eq!(
        stage.digest_pointer(&parsed(&x)),
        "/status/currentFreight/images"
    );
    assert_eq!(
        stage.digest_pointer(&parsed(&y)),
        "/status/lastPromotion/freight/images"
    );
    // A Promotion that holds one digest in each place.
    let promotion = parse_promotion(&serde_json::json!({
        "metadata": {"namespace": "storefront", "name": "p"},
        "status": {
            "freight": {"name": "a", "images": [{"digest": x}]},
            "freightCollection": {"items": {"k": {"images": [{"digest": y}]}}},
        }
    }))
    .unwrap();
    assert_eq!(
        promotion.digest_pointer(&parsed(&x)),
        "/status/freight/images"
    );
    assert_eq!(
        promotion.digest_pointer(&parsed(&y)),
        "/status/freightCollection/items"
    );
    // Older Kargo keeps a Freight's contents under `spec`.
    let old = parse_freight(&serde_json::json!({
        "metadata": {"namespace": "storefront", "name": "f"},
        "spec": {"images": [{"repoURL": "registry.example/acme/app", "digest": x}]},
    }))
    .unwrap();
    assert_eq!(old.pointer("images"), "/spec/images");
    let new = parse_freight(&serde_json::json!({
        "metadata": {"namespace": "storefront", "name": "f"},
        "images": [{"repoURL": "registry.example/acme/app", "digest": x}],
    }))
    .unwrap();
    assert_eq!(new.pointer("images"), "/images");
    assert_eq!(pointer_segment("a/b"), "a~1b");
}

/// A pod of the Rollout's current revision, `5d9c`, owned by its ReplicaSet,
/// with these containers: `(name, image, imageID, ready)`.
fn current_pod(name: &str, containers: &[(&str, &str, Option<&str>, bool)]) -> serde_json::Value {
    let mut pod = pod_of(name, "5d9c", "", "", true);
    pod["status"]["containerStatuses"] = containers
        .iter()
        .map(|(container, image, id, ready)| {
            serde_json::json!({"name": container, "image": image, "imageID": id, "ready": ready})
        })
        .collect();
    pod
}

async fn current_pods(pods: Vec<serde_json::Value>) -> Trail {
    let mut world = healthy();
    world.environment = world.environment.with("pods", pods);
    run(&world, &ENV).await
}

fn running(digest: &str) -> String {
    format!("docker-pullable://{REPO}@{digest}")
}

#[tokio::test]
async fn a_ready_current_pod_reporting_the_digest_confirms_through_owner_uids() {
    let tagged = format!("{REPO}:v1.4.0");
    let trail = current_pods(vec![current_pod(
        "storefront-5d9c-a",
        &[("app", &tagged, Some(&running(NEW)), true)],
    )])
    .await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Confirmed);
    for hop in [Hop::Rollout, Hop::Pod] {
        assert!(stands_on(pods, hop), "{hop:?}: {pods:#?}");
    }
    let rule = pods
        .evidence
        .iter()
        .find(|seen| seen.object.kind == "Join")
        .expect("the join's conclusion");
    assert_eq!(
        rule.object.name,
        "pod -> ReplicaSet -> Rollout by owner UID; pod-template-hash == currentPodHash"
    );
}

#[tokio::test]
async fn every_form_of_an_image_id_compares_by_its_digest() {
    let tagged = format!("{REPO}:v1.4.0");
    for id in [running(NEW), format!("{REPO}@{NEW}"), NEW.to_owned()] {
        let trail = current_pods(vec![current_pod(
            "storefront-5d9c-a",
            &[("app", &tagged, Some(&id), true)],
        )])
        .await;
        let pods = one(&trail, Hop::Rollout, Hop::Pod);
        assert_eq!(pods.confidence, Confidence::Confirmed, "{id}");
        assert!(
            matches!(&pods.key, Key::Digest(d) if d.as_str() == NEW),
            "{id}"
        );
    }
}

#[tokio::test]
async fn a_sidecar_of_another_image_does_not_count() {
    let tagged = format!("{REPO}:v1.4.0");
    let sidecar = format!("registry.example/acme/proxy@{OLD}");
    let trail = current_pods(vec![current_pod(
        "storefront-5d9c-a",
        &[
            ("app", &tagged, Some(&running(NEW)), true),
            ("proxy", &sidecar, Some(&sidecar), true),
        ],
    )])
    .await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Confirmed, "{}", pods.reason);
    assert!(
        pods.reason.contains(
            "1 container(s) run the Freight's digest, 1 ready of 1 pod container(s) read"
        ),
        "{}",
        pods.reason
    );
}

#[tokio::test]
async fn the_pins_container_counts_by_name_whatever_repository_it_reports() {
    // The runtime reports a mirror's name for the Rollout's `app` container.
    let mirrored = "mirror.example/acme/storefront:v1.4.0";
    for (id, confidence) in [
        (running(NEW), Confidence::Confirmed),
        (running(OLD), Confidence::Claimed),
    ] {
        let trail = current_pods(vec![current_pod(
            "storefront-5d9c-a",
            &[("app", mirrored, Some(&id), true)],
        )])
        .await;
        let pods = one(&trail, Hop::Rollout, Hop::Pod);
        assert_eq!(pods.confidence, confidence, "{}", pods.reason);
    }
}

#[tokio::test]
async fn a_current_pod_on_another_digest_keeps_the_link_claimed_and_names_it() {
    let tagged = format!("{REPO}:v1.4.0");
    let trail = current_pods(vec![
        current_pod(
            "storefront-5d9c-a",
            &[("app", &tagged, Some(&running(NEW)), true)],
        ),
        current_pod(
            "storefront-5d9c-b",
            &[("app", &tagged, Some(&running(OLD)), true)],
        ),
    ])
    .await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Claimed);
    let old = super::Digest::parse(OLD).unwrap().short();
    assert!(
        pods.reason.contains(&format!(
            "the current revision's pods run another digest ({old})"
        )),
        "{}",
        pods.reason
    );
    assert!(
        pods.evidence.iter().any(|seen| seen.object.kind == "Pod"
            && seen.fact == Fact::Reported
            && seen.value.as_deref() == Some(OLD)),
        "{pods:#?}"
    );
}

#[tokio::test]
async fn without_a_ready_current_pod_reporting_the_digest_the_link_only_claims() {
    let tagged = format!("{REPO}:v1.4.0");
    for (id, ready, words) in [
        (
            Some(running(NEW)),
            false,
            "1 container(s) of the current revision report the Freight's digest, none ready yet",
        ),
        (
            None,
            false,
            "no pod of the current revision reports a digest yet",
        ),
    ] {
        let trail = current_pods(vec![current_pod(
            "storefront-5d9c-a",
            &[("app", &tagged, id.as_deref(), ready)],
        )])
        .await;
        let pods = one(&trail, Hop::Rollout, Hop::Pod);
        assert_eq!(pods.confidence, Confidence::Claimed, "{}", pods.reason);
        assert!(pods.reason.contains(words), "{}", pods.reason);
    }
    let trail = current_pods(Vec::new()).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Claimed, "{}", pods.reason);
    assert!(
        pods.reason
            .contains("no pod of the current revision reports a digest yet"),
        "{}",
        pods.reason
    );
}

/// [`pinned`] with Argo CD's image summary listing `images`.
fn pinned_listing(images: &[&str]) -> World {
    let mut world = pinned();
    world.argocd = world.argocd.with(
        "applications",
        vec![summarised(
            application(Some("https://env-a.example:6443")),
            images,
        )],
    );
    world
}

#[tokio::test]
async fn argo_cds_summary_and_the_current_pods_confirm_a_pinned_rollout() {
    let trail = run(&pinned(), &ENV).await;
    let rollout = one(&trail, Hop::Application, Hop::Rollout);
    assert_eq!(
        rollout.confidence,
        Confidence::Confirmed,
        "{}",
        rollout.reason
    );
    for hop in [Hop::Application, Hop::Rollout] {
        assert!(stands_on(rollout, hop), "{hop:?}: {rollout:#?}");
    }
    let summary = rollout
        .evidence
        .iter()
        .find(|seen| seen.field == "/status/summary/images")
        .expect("Argo CD's summary");
    assert_eq!(summary.fact, Fact::Reported);
    assert_eq!(summary.value.as_deref(), Some(NEW));
    // The pin is still shown, as what it is.
    assert!(rollout.evidence.iter().any(|seen| {
        seen.object.kind == "Rollout"
            && seen.fact == Fact::Declared
            && seen.value.as_deref() == Some(NEW)
    }));
}

#[tokio::test]
async fn a_spec_pin_without_argo_cds_summary_only_claims() {
    let mut world = pinned();
    world.argocd = world.argocd.with(
        "applications",
        vec![application(Some("https://env-a.example:6443"))],
    );
    let trail = run(&world, &ENV).await;
    let rollout = one(&trail, Hop::Application, Hop::Rollout);
    assert_eq!(rollout.confidence, Confidence::Claimed);
    for words in [
        "the Rollout's spec pins the Freight's digest, declared only",
        "Argo CD reports no image summary for the Application",
    ] {
        assert!(
            rollout.reason.contains(words),
            "{words}: {}",
            rollout.reason
        );
    }
    assert!(!stands_on(rollout, Hop::Application));
    // The pods still confirm their own link.
    assert_eq!(
        one(&trail, Hop::Rollout, Hop::Pod).confidence,
        Confidence::Confirmed
    );
}

#[tokio::test]
async fn a_summary_that_lists_another_image_keeps_it_claimed_and_names_it() {
    let old = format!("{REPO}@{OLD}");
    let tagged = format!("{REPO}:v1.4.0");
    let other = format!("registry.example/acme/proxy@{NEW}");
    let short = super::Digest::parse(OLD).unwrap().short();
    for (images, words) in [
        (
            vec![old.as_str()],
            format!("Argo CD's image summary lists {short} instead"),
        ),
        (
            vec![tagged.as_str()],
            "Argo CD's image summary lists tag v1.4.0 instead".to_owned(),
        ),
        (
            vec![other.as_str()],
            "Argo CD's image summary does not list its image".to_owned(),
        ),
    ] {
        let trail = run(&pinned_listing(&images), &ENV).await;
        let rollout = one(&trail, Hop::Application, Hop::Rollout);
        assert_eq!(rollout.confidence, Confidence::Claimed, "{images:?}");
        assert!(
            rollout.reason.contains(&words),
            "{words}: {}",
            rollout.reason
        );
        assert!(!stands_on(rollout, Hop::Application), "{images:?}");
    }
    // What the summary lists of the image is kept, as reported.
    let trail = run(&pinned_listing(&[&old]), &ENV).await;
    let rollout = one(&trail, Hop::Application, Hop::Rollout);
    assert!(rollout.evidence.iter().any(|seen| {
        seen.field == "/status/summary/images"
            && seen.fact == Fact::Reported
            && seen.value.as_deref() == Some(old.as_str())
    }));
}

#[tokio::test]
async fn current_pods_on_another_digest_keep_a_pinned_rollout_claimed() {
    let mut world = pinned();
    world.environment = world.environment.with(
        "pods",
        vec![pod(
            "storefront-5d9c-x",
            &format!("{REPO}@{NEW}"),
            &format!("docker-pullable://{REPO}@{OLD}"),
        )],
    );
    let trail = run(&world, &ENV).await;
    let rollout = one(&trail, Hop::Application, Hop::Rollout);
    assert_eq!(rollout.confidence, Confidence::Claimed);
    let short = super::Digest::parse(OLD).unwrap().short();
    assert!(
        rollout
            .reason
            .contains(&format!("its current pods: the Stage claims this Freight but the current revision's pods run another digest ({short})")),
        "{}",
        rollout.reason
    );
    assert!(stands_on(rollout, Hop::Application));
    assert!(!stands_on(rollout, Hop::Rollout));
}

#[tokio::test]
async fn a_capped_pod_listing_only_claims_even_when_every_pod_read_matches() {
    let tagged = format!("{REPO}:v1.4.0");
    let mut world = healthy();
    world.environment = world
        .environment
        .with(
            "pods",
            vec![current_pod(
                "storefront-5d9c-a",
                &[("app", &tagged, Some(&running(NEW)), true)],
            )],
        )
        .capped("pods");
    let trail = run(&world, &ENV).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Claimed, "{}", pods.reason);
    for words in [
        "1 container(s) run the Freight's digest, 1 ready of 1 pod container(s) read",
        "the pod listing stopped at the page cap after 1 items, so a pod not read may run another digest",
    ] {
        assert!(pods.reason.contains(words), "{words}: {}", pods.reason);
    }
    // What was read is still shown, as reported.
    assert!(pods.evidence.iter().any(|seen| {
        seen.object.kind == "Pod"
            && seen.fact == Fact::Reported
            && seen.value.as_deref() == Some(NEW)
    }));
}
