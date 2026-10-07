//! F01 provenance: what each link says it was read from. One test per rule.

use std::collections::BTreeSet;

use super::fixtures::*;
use super::join::{Confidence, Hop, Key, Link, Trail, render};
use super::observation::{Fact, Observation};
use super::tests::{ENV, link, observed_at, one, promoted, run, run_with};

/// The kinds of object whose observations stand on a hop's side of a link.
fn kinds(hop: Hop) -> &'static [&'static str] {
    match hop {
        Hop::PullRequest => &["PullRequest"],
        Hop::Commit => &["Commit"],
        // A commit that joined a Freight with no build read stands alone.
        Hop::PipelineRun => &["PipelineRun", "TaskRun", "Commit"],
        Hop::SupplyChain => &["Image"],
        Hop::Freight => &["Freight"],
        Hop::Promotion => &["Promotion"],
        Hop::Stage => &["Stage"],
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

/// Whether the link has, on `hop`'s side, an observation that is reported or
/// derived, never merely declared, and carries `key`.
fn stands_on(link: &Link, hop: Hop) -> bool {
    let key = key_text(&link.key);
    link.evidence.iter().any(|seen| {
        kinds(hop).contains(&seen.object.kind.as_str())
            && seen.fact != Fact::Declared
            && seen
                .value
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(&key))
    })
}

fn github() -> FixtureGitHub {
    FixtureGitHub::with(vec![pull_request(7, OTHER_SHA, Some(SHA))])
}

/// Every kind of link confirmed at once: a squash-merged pull request whose
/// head build shipped, signed by Chains, promoted to a Stage whose
/// Application synced the pushed commit, pinned by its Rollout, running.
async fn confirmed() -> Trail {
    let mut world = promoted(NEW, "Succeeded", PUSHED);
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

fn pairs(trail: &Trail) -> BTreeSet<(&'static str, &'static str)> {
    trail
        .links
        .iter()
        .filter(|link| link.confidence == Confidence::Confirmed)
        .map(|link| (link.from.word(), link.to.word()))
        .collect()
}

#[tokio::test]
async fn a_confirmed_link_stands_on_both_sides() {
    let mut trails = vec![confirmed().await];
    // Without Rollouts the pods are the Application's.
    trails.push(run(&without_rollouts(), &ENV).await);
    let mut seen_pairs = BTreeSet::new();
    for trail in &trails {
        for link in trail
            .links
            .iter()
            .filter(|link| link.confidence == Confidence::Confirmed)
        {
            for hop in [link.from, link.to] {
                assert!(
                    stands_on(link, hop),
                    "{:?} has no observation on the {} side: {link:#?}",
                    link.key,
                    hop.word()
                );
            }
        }
        seen_pairs.extend(pairs(trail));
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
            seen_pairs.contains(&(pair.0.word(), pair.1.word())),
            "{pair:?} was not confirmed, so not checked: {seen_pairs:?}"
        );
    }
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
            // The reason is what a source that couldn't be read says.
            assert!(link.reason.len() > "not readable".len(), "{link:#?}");
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
    let mut world = promoted(NEW, "Succeeded", PUSHED);
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
