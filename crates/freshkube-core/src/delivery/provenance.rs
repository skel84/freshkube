//! F01 provenance: what each link says it was read from. One test per rule.

use std::collections::BTreeSet;

use super::fixtures::*;
use super::join::{Confidence, Hop, Key, Link, Trail, render};
use super::observation::{Fact, Observation, role};
use super::source::Source;
use super::tests::{ENV, link, observed_at, one, promoted, run, run_with};
use crate::resources::{Failure, FailureKind};

/// The kinds of read object whose observations stand on a hop's side of a
/// link. The change itself is the Commit hop's only side.
fn kinds(hop: Hop) -> &'static [&'static str] {
    match hop {
        // A pull request joins the Freight through the build of its head.
        Hop::PullRequest => &["PullRequest", "PipelineRun"],
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
fn stands_on(link: &Link, hop: Hop) -> bool {
    let key = key_text(&link.key);
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

/// The sides of confirmed links that no read object stands on: `(from, to,
/// side)`. Each is a confirmed link, on main, whose evidence on that side is
/// declared only, or absent. Provenance shows them as they are; whether they
/// stay confirmed is for the confidence slice (#387).
const KNOWN_GAPS: &[(&str, &str, &str)] = &[
    // The run's only witness is a declared revision parameter; a result
    // would be reported (see the test on a task result).
    ("commit", "PipelineRun", "PipelineRun"),
    ("pull request", "PipelineRun", "PipelineRun"),
    // Chains' `signed` annotation is all that says it, and it is declared.
    ("PipelineRun", "supply chain", "supply chain"),
    // The Rollout's spec pin is declared, and Argo CD's resource list names
    // the Rollout, not the digest.
    ("Application", "Rollout", "Application"),
    ("Application", "Rollout", "Rollout"),
    // The Rollout reports its pod-template hash, which the pods' label
    // matches; neither carries the digest.
    ("Rollout", "pods", "Rollout"),
];

fn github() -> FixtureGitHub {
    FixtureGitHub::with(vec![pull_request(7, OTHER_SHA, Some(SHA))])
}

/// Every kind of link confirmed at once: a squash-merged pull request whose
/// head build shipped, signed by Chains, promoted to a Stage whose
/// Application synced the pushed commit, pinned by its Rollout, running.
async fn confirmed() -> Trail {
    let mut world = promoted(NEW, "Succeeded", PUSHED).with_meta();
    world.environment = world
        .environment
        .with("rollouts", vec![rollout(&format!("{REPO}@{NEW}"))]);
    world.tekton = world.tekton.with(
        "pipelineruns",
        vec![
            pipeline_run(SHA, true, Some(NEW)),
            pr_pipeline_run(OTHER_SHA, 7, NEW),
        ],
    );
    run_with(&world, &ENV, &github(), Some(GH_REPO)).await
}

#[tokio::test]
async fn a_confirmed_link_stands_on_what_was_read_on_each_side() {
    let trails = [confirmed().await];
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
        (Hop::PipelineRun, Hop::SupplyChain),
        (Hop::PipelineRun, Hop::Freight),
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

#[tokio::test]
async fn a_conclusion_is_the_joins_and_names_no_field() {
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
    let trail = run_with(&world, &ENV, &github(), Some(GH_REPO)).await;

    let observations: Vec<&Observation> =
        trail.links.iter().flat_map(|link| &link.evidence).collect();
    assert!(observations.len() > 10, "{}", observations.len());
    let rendered = render(&trail);
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
    let squash = run_with(
        &super::tests::squash_world(),
        &ENV,
        &github(),
        Some(GH_REPO),
    )
    .await;
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

/// `[confirmed, claimed, unknown]` of each fixture, as the join gave them
/// before links carried observations.
const COUNTS: [[usize; 3]; 4] = [[4, 4, 0], [7, 1, 0], [12, 0, 0], [8, 4, 0]];

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
