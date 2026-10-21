use super::argocd::{
    DestinationMatch, Destinations, StageNaming, match_destination, parse_application,
};
use super::collect::{Clusters, Plan, collect, commit_of_pull_request};
use super::fixtures::*;
use super::join::{self, Confidence, Hop, Key, Trail, join};
use super::kargo::{parse_freight, parse_stage};
use super::source::Source;
use super::tekton::{built_images, conforma, read_builds};

use super::{kargo, observation, pods, rollouts, tekton};

mod builds;
mod deployments;
mod destinations;
mod evidence;
mod joins;
mod naming;
mod parsers;
mod promotions;
mod pull_requests;
mod reads;

/// The caller's clock, fixed: nothing in core reads the time.
pub(super) fn observed_at() -> chrono::DateTime<chrono::Utc> {
    "2026-10-06T12:00:00Z".parse().expect("a fixed time")
}

pub(super) fn plan(contexts: &[(&str, &str)]) -> Plan {
    Plan {
        sha: SHA.into(),
        digests: Vec::new(),
        followed: None,
        kargo_project: "storefront".into(),
        argocd_namespace: "argocd".into(),
        build_namespace: Ok(Some("acme-builds".into())),
        github_repo: None,
        environment: "env-a".into(),
        argocd: "core".into(),
        cluster_names: Vec::new(),
        evidence_result: None,
        public_key_file: None,
        transparency_log: false,
        transparency_url: None,
        stage_naming: None,
        map_hint: Default::default(),
        names: Default::default(),
        commit_names: Default::default(),
        contexts: contexts
            .iter()
            .map(|(name, server)| ((*name).into(), (*server).into()))
            .collect(),
    }
}

pub(super) async fn run(world: &World, contexts: &[(&str, &str)]) -> Trail {
    run_with(world, contexts, &FixtureGitHub::default(), None).await
}

pub(super) async fn run_with(
    world: &World,
    contexts: &[(&str, &str)],
    github: &FixtureGitHub,
    repo: Option<&str>,
) -> Trail {
    run_configured(world, contexts, github, repo, |_| {}).await
}

/// An invented naming a setup without the authorized-stage annotation might
/// configure.
pub(super) fn naming() -> StageNaming {
    StageNaming {
        project_key: Some("example.test/project".into()),
        name_template: "app-{project}-in-{stage}".into(),
    }
}

pub(super) async fn run_configured(
    world: &World,
    contexts: &[(&str, &str)],
    github: &FixtureGitHub,
    repo: Option<&str>,
    configure: impl FnOnce(&mut Plan),
) -> Trail {
    let clusters = Clusters {
        kargo: &world.kargo,
        argocd: &world.argocd,
        tekton: &world.tekton,
        environment: &world.environment,
        github,
    };
    let mut plan = plan(contexts);
    plan.github_repo = repo.map(str::to_owned);
    configure(&mut plan);
    join(&collect(&clusters, &plan, observed_at()).await)
}

pub(super) fn link(trail: &Trail, from: Hop, to: Hop) -> Vec<&super::join::Link> {
    trail
        .links
        .iter()
        .filter(|l| l.from == from && l.to == to)
        .collect()
}

pub(super) fn one(trail: &Trail, from: Hop, to: Hop) -> &super::join::Link {
    let found = link(trail, from, to);
    assert_eq!(found.len(), 1, "{from:?}->{to:?} in {trail:#?}");
    found[0]
}

// ---- GitHub ------------------------------------------------------------

/// A squash merge: the pull request's head commit is not the commit on main.
/// Main's build is what shipped; the head's build produced another digest,
/// which Kargo also holds as older freight.
pub(super) fn squash_world() -> World {
    let mut world = healthy();
    world.tekton = world.tekton.with(
        "pipelineruns",
        vec![
            pipeline_run(SHA, true, Some(NEW)),
            pr_pipeline_run(OTHER_SHA, 7, OLD),
        ],
    );
    world
}

/// The healthy world, with the Promotion and the Stage recording what Kargo
/// writes, and the Application synced at `revision`.
pub(super) fn promoted(digest: &str, phase: &str, revision: &str) -> World {
    let mut world = healthy();
    world.kargo = world
        .kargo
        .with("stages", vec![stage_holding("f-new", digest)])
        .with(
            "promotions",
            vec![promotion_pushing("f-new", digest, phase, PUSHED)],
        );
    let mut app = application(Some("https://env-a.example:6443"));
    app["status"]["sync"]["revision"] = serde_json::json!(revision);
    world.argocd = world.argocd.with("applications", vec![app]);
    world
}

pub(super) const ENV: [(&str, &str); 1] = [("env-a", "https://env-a.example:6443")];
