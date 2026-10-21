//! The invented change example mode follows: acme's `checkout`, Freight
//! `wonky-otter`, from pull request 418 to the pods that run it, in the acme
//! workspace of [`applications::example`](crate::applications::example).
//! Every name is invented, and every address is under `example.test`.
//!
//! It holds the cases the page must word: a pruned run whose link is only
//! claimed, a Stage waiting for promotion with pods that can't be read, an
//! approval by hand past the upstream, and a failed verification.
//!
//! Times count from one reference instant the caller captures once, so a
//! page shown again doesn't move them.

use chrono::{DateTime, Duration, Utc};
use freshkube_core::resources::{Failure, FailureKind};

use super::*;
use crate::applications::example::{CICD, CORE};
use crate::github::example::Trouble;
use crate::{argocd, deployments, kargo, words};
use freshkube_core::indicators::HealthIndicator::{self, *};

pub(crate) const PROJECT: &str = "checkout";
pub(crate) const FREIGHT: &str = "wonky-otter";
/// The git and registry hosts the build read from.
pub(crate) const GIT: &str = "git.example.test";
pub(crate) const REGISTRY: &str = "registry.example.test";
/// Where the invented Kargo and Argo CD serve their pages.
pub(crate) const KARGO_PAGE: &str = "https://kargo.example.test";
pub(crate) const ARGOCD_PAGE: &str = "https://argocd.example.test";

const DIGEST: &str = "sha256:9c4e…b21a";
/// The pod-template hash of checkout-api's ReplicaSet that runs this
/// change: the same template on dev and stage, so the same hash, which
/// Coroot's example names its revision by.
pub const POD_HASH: &str = "5d8f7c9b6";
/// The checkout-api Deployment each environment runs.
pub const DEPLOYMENT: &str = "checkout-api";
const COMMIT: &str = "a1f3c9e";
/// The pull request that made the change, and its title.
pub(crate) const PULL_REQUEST: u64 = 418;
pub const TITLE: &str = "Retry idempotency keys";
/// The Freight prod-ams runs, from before this change.
pub(crate) const PREVIOUS: &str = "brave-lynx";
/// The pull request's head commit, which its pull_request run built.
const HEAD: &str = "7d2e04b";
const VERSION: &str = "v1alpha1";

/// The four promotion steps every Stage of checkout runs.
const STEPS: [&str; 4] = [
    "git-clone",
    "kustomize-set-image",
    "git-push",
    "argocd-update",
];

/// checkout's Stages, in promotion order.
pub(crate) const STAGES: [&str; 4] = ["dev", "stage", "prod-ams", "prod-lon"];

/// The Freight a Stage of a project carries, when example mode has one:
/// every Stage of `checkout` leads to `wonky-otter`. Builds no change.
pub fn freight_of(project: &str, stage: &str) -> Option<&'static str> {
    (project == PROJECT && STAGES.contains(&stage)).then_some(FREIGHT)
}

/// The change of a Stage of a project, when example mode has one.
#[cfg(test)]
pub(crate) fn for_stage(project: &str, stage: &str, now: DateTime<Utc>) -> Option<Change> {
    freight_of(project, stage).map(|_| change(now))
}

/// A time of the invented morning: `minute` minutes after nine, when the
/// reference instant is ten past ten.
fn at(now: DateTime<Utc>, minute: i64) -> DateTime<Utc> {
    now - Duration::minutes(70) + Duration::minutes(minute)
}

fn text(label: &str, value: &str) -> Field {
    Field {
        label: label.into(),
        value: Value::Text(value.into()),
    }
}

fn mono(label: &str, value: &str) -> Field {
    Field {
        label: label.into(),
        value: Value::Mono(value.into()),
    }
}

fn when(label: &str, at: DateTime<Utc>) -> Field {
    Field {
        label: label.into(),
        value: Value::At(at),
    }
}

fn link(
    confidence: Confidence,
    by: &str,
    before: &str,
    before_says: &str,
    here_says: &str,
    why: &str,
) -> Option<LinkDetail> {
    Some(LinkDetail {
        confidence,
        by: by.into(),
        before: before.into(),
        key: match by {
            "digest" => Some(DIGEST.into()),
            "commit" => Some(COMMIT.into()),
            "provenance" => Some(HEAD.into()),
            _ => None,
        },
        before_says: before_says.into(),
        here_says: here_says.into(),
        why: why.into(),
    })
}

fn check(state: HealthIndicator, name: &str, found: &str, at: Option<DateTime<Utc>>) -> Check {
    Check {
        state,
        name: name.into(),
        found: found.into(),
        at,
    }
}

fn object(cluster: &str, group: &str, kind: &str, namespace: &str, name: &str) -> Object {
    let (version, plural) = match kind {
        "Deployment" => ("v1", "deployments"),
        "PipelineRun" => ("v1", "pipelineruns"),
        "TaskRun" => ("v1", "taskruns"),
        "Pod" => ("v1", "pods"),
        "Freight" => (VERSION, "freights"),
        "Stage" => (VERSION, "stages"),
        _ => (VERSION, "applications"),
    };
    Object {
        cluster: cluster.into(),
        group: group.into(),
        version: version.into(),
        kind: kind.into(),
        plural: plural.into(),
        namespace: namespace.into(),
        name: name.into(),
    }
}

/// checkout-api's Deployment on `cluster`.
fn deployment(cluster: &str) -> Object {
    object(
        cluster,
        deployments::GROUP,
        "Deployment",
        PROJECT,
        DEPLOYMENT,
    )
}

fn resource(what: &str, object: Object) -> Action {
    Action::new(Target::Resource {
        what: what.into(),
        object,
    })
}

/// `label`'s page, `segments` below `base`.
fn browser(label: &str, base: &str, segments: &[&str]) -> Action {
    let address = Address::parse(base).map(|base| base.join(segments.iter().copied()));
    Action::browser(label, address)
}

/// An Application's page in Argo CD.
fn argocd_page(app: &str) -> Action {
    browser(
        "Open in Argo CD",
        ARGOCD_PAGE,
        &["applications", "argocd", app],
    )
}

/// The Freight's page in Kargo.
pub(crate) fn kargo_page() -> Address {
    Address::parse(KARGO_PAGE)
        .expect("an address")
        .join(["project", PROJECT, "freight", FREIGHT])
}

/// Where the release pipeline checks policy: on Kargo's cluster, the one
/// the example opens, so its logs open there.
pub(crate) const RELEASE_NAMESPACE: &str = "checkout-release";
/// The release run's policy TaskRun.
pub(crate) const VERIFY_TASK: &str = "checkout-release-wonky-otter-verify";

/// The pod a TaskRun of the CI cluster ran its steps in.
fn step_logs(task: &str, run: &str) -> Action {
    Action::new(Target::Logs {
        what: task.into(),
        pod: object(CICD, "", "Pod", "checkout-ci", &format!("{run}-{task}-pod")),
        container: Some(format!("step-{task}")),
    })
}

/// The Stage itself, in Kargo's project namespace.
pub(crate) fn stage_object(stage: &str) -> Object {
    object(CICD, kargo::GROUP, "Stage", PROJECT, stage)
}

fn application_object(name: &str) -> Object {
    object(CORE, argocd::GROUP, "Application", "argocd", name)
}

fn task_run(name: &str) -> Object {
    object(CICD, "tekton.dev", "TaskRun", "checkout-ci", name)
}

fn steps(now: DateTime<Utc>, state: HealthIndicator, minutes: Option<[i64; 4]>) -> Vec<Check> {
    STEPS
        .iter()
        .enumerate()
        .map(|(ix, step)| match minutes {
            Some(minutes) => check(state, step, "done", Some(at(now, minutes[ix]))),
            None => check(state, step, "not started", None),
        })
        .collect()
}

fn stages(now: DateTime<Utc>) -> Vec<Stage> {
    let approvers = "kargo-approver: jon, mira";
    let promoters = "kargo-promoter: mira, ops-oncall";
    let stage = |name: &str,
                 cluster: &str,
                 state,
                 words: &str,
                 eligible,
                 promotion,
                 steps,
                 verification| Stage {
        name: name.into(),
        cluster: Destination::Cluster(cluster.into()),
        state,
        words: words.into(),
        running: None,
        eligible,
        promotion,
        approvers: Some(approvers.into()),
        promoters: Some(promoters.into()),
        steps,
        verification,
        link: stage_link(name),
        reach: Reach::Linked(Confidence::Confirmed),
        object: stage_object(name),
    };
    vec![
        stage(
            "dev",
            "dev-fra",
            Healthy,
            "Verified",
            Eligible::Warehouse {
                at: Some(at(now, 19)),
            },
            Promotion::Automatic {
                at: Some(at(now, 20)),
            },
            steps(now, Healthy, Some([20, 20, 20, 21])),
            vec![
                check(Healthy, "checkout-smoke", "passed", Some(at(now, 27))),
                check(Healthy, "error-rate", "0.1 %, under 1 %", Some(at(now, 31))),
            ],
        ),
        stage(
            "stage",
            "stage-fra",
            Healthy,
            "Verified",
            Eligible::Verified {
                upstream: "dev".into(),
                at: Some(at(now, 31)),
                checks: Some("2 of 2 analyses".into()),
            },
            Promotion::Automatic {
                at: Some(at(now, 40)),
            },
            steps(now, Healthy, Some([40, 40, 41, 41])),
            vec![
                check(Healthy, "checkout-smoke", "passed", Some(at(now, 48))),
                check(Healthy, "error-rate", "0.2 %, under 1 %", Some(at(now, 58))),
                check(
                    Healthy,
                    "latency-p99",
                    "212 ms, under 400 ms",
                    Some(at(now, 62)),
                ),
            ],
        ),
        Stage {
            running: Some(PREVIOUS.into()),
            reach: Reach::NotReached,
            ..stage(
                "prod-ams",
                "prod-ams",
                Pending,
                "Waiting for promotion",
                Eligible::Verified {
                    upstream: "stage".into(),
                    at: Some(at(now, 62)),
                    checks: Some("3 of 3 analyses".into()),
                },
                Promotion::Waiting,
                steps(now, Pending, None),
                vec![
                    check(Pending, "checkout-smoke", "runs after promotion", None),
                    check(Pending, "error-rate", "runs after promotion", None),
                ],
            )
        },
        stage(
            "prod-lon",
            "prod-lon",
            Error,
            "Verification failed",
            Eligible::Approved {
                by: Some("jon".into()),
                at: Some(at(now, 50)),
                past: "stage".into(),
                upstream: Upstream::Verified {
                    at: Some(at(now, 62)),
                },
            },
            Promotion::Automatic {
                at: Some(at(now, 51)),
            },
            steps(now, Healthy, Some([51, 51, 52, 52])),
            vec![
                check(Healthy, "checkout-smoke", "passed", Some(at(now, 58))),
                check(
                    Error,
                    "error-rate",
                    "2.4 % of requests failed, above 1 %",
                    Some(at(now, 61)),
                ),
            ],
        ),
    ]
}

/// Why an example Stage follows its parent without a confirmed join key.
fn stage_link(stage: &str) -> Option<LinkDetail> {
    match stage {
        "dev" => link(
            Confidence::Unknown,
            "eligibility",
            &format!("Freight {FREIGHT}"),
            "the Freight is from checkout-images",
            "dev is eligible from the Warehouse",
            "The Freight is eligible for dev from checkout-images, but no join key between the Freight and dev was read.",
        ),
        "prod-ams" => link(
            Confidence::Unknown,
            "eligibility",
            "Stage stage",
            "stage passed its three analyses",
            "prod-ams is eligible and waiting for promotion",
            &format!(
                "Not reached yet: prod-ams still runs {PREVIOUS}; {FREIGHT} is eligible after stage's verification and waits for promotion."
            ),
        ),
        "prod-lon" => link(
            Confidence::Unknown,
            "eligibility",
            "Stage stage",
            "stage verified the Freight",
            "jon approved prod-lon past stage's verification",
            "Verification in stage and jon's approval make the Freight eligible for prod-lon, but the Stage-to-Stage join key wasn't read.",
        ),
        _ => None,
    }
}

/// A Stage's three gate rows, which show its gates in the Inspector.
fn gates(
    group: usize,
    stage: usize,
    name: &str,
    follows: &str,
    rows: [(HealthIndicator, &str, Option<DateTime<Utc>>); 3],
) -> Vec<Hop> {
    ["Eligible", "Promotion", "Verification"]
        .iter()
        .zip(rows)
        .map(|(gate, (state, detail, at))| Hop {
            key: format!("{name}-{}", gate.to_lowercase()),
            group,
            state,
            name: (*gate).into(),
            detail: detail.into(),
            from: CICD.into(),
            at,
            link: None,
            after: Some(follows.into()),
            shows: Shows::Stage(stage),
        })
        .collect()
}

/// Argo CD's Application, its Deployment and the pods for a Stage that
/// runs this change, each Confirmed. As a live read has it, the Deployment
/// names Coroot's revision by its current ReplicaSet's hash.
fn deployed(
    group: usize,
    stage: &str,
    cluster: &str,
    revision: &str,
    synced: DateTime<Utc>,
    pods: &str,
) -> Vec<Hop> {
    let app = format!("{PROJECT}-{stage}");
    let mut hops = vec![
        Hop {
            key: format!("{stage}-argocd"),
            group,
            state: Healthy,
            name: app.clone(),
            detail: "Synced · Healthy".into(),
            from: CORE.into(),
            at: Some(synced),
            link: Some(Confidence::Confirmed),
            after: Some(format!("{stage}-eligible")),
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "Argo CD Application".into(),
                title: app.clone(),
                state: "Synced · Healthy".into(),
                notice: None,
                fields: vec![
                    text("Project", PROJECT),
                    mono("Destination", cluster),
                    mono("Namespace", PROJECT),
                    mono("Synced revision", revision),
                    text("Source", "acme/checkout-deploy, env branch"),
                ],
                link: link(
                    Confidence::Confirmed,
                    "revision",
                    "Promotion",
                    "git-push wrote this revision to the env branch",
                    "Argo CD synced this revision",
                    "The revision matches on both sides.",
                ),
                unlinked: None,
                actions: vec![
                    argocd_page(&app),
                    resource("the Application", application_object(&app)),
                ],
            })),
        },
        Hop {
            key: format!("{stage}-deployment"),
            group,
            state: Healthy,
            name: format!("Deployment {DEPLOYMENT}"),
            detail: "Pins the digest · 3 of 3 replicas updated".into(),
            from: cluster.into(),
            at: Some(synced),
            link: Some(Confidence::Confirmed),
            after: Some(format!("{stage}-argocd")),
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "Deployment".into(),
                title: DEPLOYMENT.into(),
                state: "Available".into(),
                notice: None,
                fields: vec![
                    mono("Namespace", PROJECT),
                    mono("Name", DEPLOYMENT),
                    mono("Pod template hash", POD_HASH),
                    mono("Read from", cluster),
                ],
                link: link(
                    Confidence::Confirmed,
                    "digest",
                    "Freight wonky-otter",
                    DIGEST,
                    "its spec pins it",
                    "Managed by the Application, and its pod template pins the Freight's digest.",
                ),
                unlinked: None,
                actions: vec![
                    resource("the Deployment", deployment(cluster)),
                    Action::revision(deployment(cluster), Ok(POD_HASH.into())),
                ],
            })),
        },
        Hop {
            key: format!("{stage}-pods"),
            group,
            state: Healthy,
            name: "Pods".into(),
            detail: pods.into(),
            from: cluster.into(),
            at: Some(synced),
            link: Some(Confidence::Confirmed),
            after: Some(format!("{stage}-deployment")),
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "Pods".into(),
                title: "checkout-api".into(),
                state: "Running".into(),
                notice: None,
                fields: vec![
                    mono("Cluster", cluster),
                    mono("Namespace", PROJECT),
                    mono("Owner", "deploy/checkout-api"),
                    text("Running", pods),
                    mono("Image", "checkout-api:1.43.0 (tag, shown only)"),
                    mono("Image ID", DIGEST),
                ],
                link: link(
                    Confidence::Confirmed,
                    "digest",
                    "Freight wonky-otter",
                    DIGEST,
                    "every pod's imageID reports it",
                    "What runs is read from the pods, not from the plan, and the digest matches.",
                ),
                unlinked: None,
                actions: vec![resource("the Deployment", deployment(cluster))],
            })),
        },
    ];
    // The Application joins the Promotion's revision.
    if let Shows::Hop(detail) = &mut hops[0].shows
        && let Some(link) = &mut detail.link
    {
        link.key = Some(revision.into());
    }
    hops
}

fn build(now: DateTime<Utc>) -> Vec<Hop> {
    let pull_request = Hop {
        key: "pr".into(),
        group: 0,
        state: Healthy,
        name: "PR #418".into(),
        detail: "Retry idempotency keys · merged by mira".into(),
        from: GIT.into(),
        at: Some(at(now, 12)),
        link: None,
        after: None,
        shows: Shows::Hop(Box::new(HopDetail {
            kind: "Pull request".into(),
            title: "#418".into(),
            state: "Merged".into(),
            notice: None,
            fields: vec![
                text("Title", "Retry idempotency keys"),
                mono("Repository", &format!("{GIT}/acme/checkout")),
                when("Merged", at(now, 12)),
                text("Merged by", "mira, into main"),
                text("Checks", "6 of 6 passed"),
                text("Reviews", "2 approved"),
                mono("Head commit", HEAD),
                mono("Merge commit", COMMIT),
            ],
            link: None,
            unlinked: Some(
                "Where the change starts: every later hop joins back to its merge commit.".into(),
            ),
            actions: vec![browser(
                "Open the pull request",
                &format!("https://{GIT}"),
                &["acme", "checkout", "pull", "418"],
            )],
        })),
    };
    let pruned = Hop {
        key: "pr-run".into(),
        group: 0,
        state: Unknown,
        name: "Run checkout-pr-418-m2q8".into(),
        detail: "Run pruned, logs gone".into(),
        from: CICD.into(),
        at: Some(at(now, 5)),
        link: Some(Confidence::Claimed),
        after: Some("pr".into()),
        shows: Shows::Hop(Box::new(HopDetail {
            kind: "PipelineRun".into(),
            title: "checkout-pr-418-m2q8".into(),
            state: "Pruned".into(),
            notice: Some(Notice {
                state: Warning,
                lead: "Run pruned, logs gone.".into(),
                body: "Tekton pruned this run and Tekton Results isn't installed, so its \
                       tasks and logs can't be read."
                    .into(),
            }),
            fields: vec![
                text("Repository", "acme-checkout (Pipelines as Code)"),
                text("Event", "pull_request #418"),
                mono("Commit", HEAD),
                text("Tasks", "Not readable: the run is pruned"),
            ],
            link: link(
                Confidence::Claimed,
                "provenance",
                "PR #418",
                "head commit 7d2e04b",
                "Tekton Chains' SLSA provenance names this run and 7d2e04b",
                "Only the provenance says so. The run itself is pruned, so its labels and \
                 results can't be compared with the pull request.",
            ),
            unlinked: None,
            actions: vec![
                step_logs("build", "checkout-pr-418-m2q8").disabled("Run pruned, logs gone"),
            ],
        })),
    };
    let push = Hop {
        key: "push-run".into(),
        group: 0,
        state: Healthy,
        name: "Run checkout-push-x7k2".into(),
        detail: "push to main · 7 of 7 tasks · 6 m 12 s".into(),
        from: CICD.into(),
        at: Some(at(now, 18)),
        link: Some(Confidence::Confirmed),
        after: Some("pr".into()),
        shows: Shows::Hop(Box::new(HopDetail {
            kind: "PipelineRun".into(),
            title: "checkout-push-x7k2".into(),
            state: "Succeeded".into(),
            notice: None,
            fields: vec![
                text("Repository", "acme-checkout (Pipelines as Code)"),
                text("Event", "push to main"),
                text(
                    "Tasks",
                    "clone, lint, test, build, sbom, push, verify: 7 of 7",
                ),
                text("Took", "6 m 12 s"),
                mono("Result IMAGE_DIGEST", DIGEST),
            ],
            link: link(
                Confidence::Confirmed,
                "commit",
                "PR #418",
                "merge commit a1f3c9e",
                "label pipelinesascode.tekton.dev/sha = a1f3c9e, and its image digest matches \
                 the registry's",
                "The commit matches, and the digest the run reports is the one downstream.",
            ),
            unlinked: None,
            actions: vec![
                step_logs("build", "checkout-push-x7k2"),
                resource(
                    "the PipelineRun",
                    object(
                        CICD,
                        "tekton.dev",
                        "PipelineRun",
                        "checkout-ci",
                        "checkout-push-x7k2",
                    ),
                ),
            ],
        })),
    };
    let signature = Hop {
        key: "signature".into(),
        group: 0,
        state: Healthy,
        name: "Signature".into(),
        detail: "Tekton Chains, keyless · SLSA v1".into(),
        from: CICD.into(),
        at: Some(at(now, 18)),
        // Only Chains' annotation says it signed: no key or transparency
        // log is read, so nothing here verifies the signature (ROADMAP
        // step 5).
        link: Some(Confidence::Claimed),
        after: Some("push-run".into()),
        shows: Shows::Hop(Box::new(HopDetail {
            kind: "Tekton Chains".into(),
            title: "checkout-push-x7k2-build".into(),
            state: "Signed".into(),
            notice: None,
            fields: vec![
                text("Signature", "cosign, keyless"),
                text("Provenance", "SLSA v1"),
                mono("Subject", DIGEST),
            ],
            link: link(
                Confidence::Claimed,
                "digest",
                "Run checkout-push-x7k2",
                DIGEST,
                "Chains' annotation says it signed this digest",
                "Only Chains' annotation says signed: no key or transparency log was read, \
                 so the signature isn't verified.",
            ),
            unlinked: None,
            actions: vec![resource(
                "the TaskRun",
                task_run("checkout-push-x7k2-build"),
            )],
        })),
    };
    let policy = Hop {
        key: "policy".into(),
        group: 0,
        state: Healthy,
        name: "Policy acme-prod".into(),
        detail: "Conforma: 41 of 41 rules passed".into(),
        from: CICD.into(),
        at: Some(at(now, 18)),
        link: Some(Confidence::Confirmed),
        after: Some("signature".into()),
        shows: Shows::Hop(Box::new(HopDetail {
            kind: "Conforma".into(),
            title: "acme-prod".into(),
            state: "Passed".into(),
            notice: None,
            fields: vec![
                text("Rules", "41 of 41 passed"),
                mono("Task", VERIFY_TASK),
                mono("Image", DIGEST),
            ],
            link: link(
                Confidence::Confirmed,
                "digest",
                "Signature",
                DIGEST,
                "Conforma checked the same digest",
                "The digest matches on both sides.",
            ),
            unlinked: None,
            actions: vec![
                Action::new(Target::Logs {
                    what: "verify".into(),
                    pod: object(
                        CICD,
                        "",
                        "Pod",
                        RELEASE_NAMESPACE,
                        &format!("{VERIFY_TASK}-pod"),
                    ),
                    container: Some("step-validate".into()),
                }),
                resource(
                    "the TaskRun",
                    object(
                        CICD,
                        "tekton.dev",
                        "TaskRun",
                        RELEASE_NAMESPACE,
                        VERIFY_TASK,
                    ),
                ),
            ],
        })),
    };
    let image = Hop {
        key: "image".into(),
        group: 0,
        state: Warning,
        name: "Image checkout-api".into(),
        detail: "0 critical, 3 high (2 fixable) · SBOM · immutable".into(),
        from: REGISTRY.into(),
        at: Some(at(now, 18)),
        link: Some(Confidence::Confirmed),
        after: Some("push-run".into()),
        shows: Shows::Hop(Box::new(HopDetail {
            kind: "Registry artifact".into(),
            title: "shop/checkout-api".into(),
            state: "3 high findings".into(),
            notice: None,
            fields: vec![
                text("Tag", "1.43.0 (shown, never joined on)"),
                mono("Digest", DIGEST),
                text("Scan", "Trivy: 0 critical, 3 high, 2 of them fixable"),
                text("Accessories", "signature, SBOM, provenance"),
                when("Pushed", at(now, 18)),
            ],
            link: link(
                Confidence::Confirmed,
                "digest",
                "Run checkout-push-x7k2",
                DIGEST,
                "the registry holds the artifact by the same digest",
                "The digest matches on both sides.",
            ),
            unlinked: None,
            actions: vec![Action::browser(
                "Open in the registry",
                Err("The registry's page isn't recorded in the cluster".into()),
            )],
        })),
    };
    let freight = Hop {
        key: "freight".into(),
        group: 1,
        state: Healthy,
        name: format!("Freight {FREIGHT}"),
        detail: "Found by Warehouse checkout-images · image + checkout-deploy".into(),
        from: CICD.into(),
        at: Some(at(now, 19)),
        link: Some(Confidence::Confirmed),
        after: Some("image".into()),
        shows: Shows::Hop(Box::new(HopDetail {
            kind: "Kargo Freight".into(),
            title: FREIGHT.into(),
            state: "Found".into(),
            notice: None,
            fields: vec![
                text("Project", PROJECT),
                mono("Warehouse", "checkout-images"),
                mono("Image", DIGEST),
                mono("Commit", COMMIT),
                when("Found", at(now, 19)),
            ],
            link: link(
                Confidence::Confirmed,
                "digest",
                "Image checkout-api",
                DIGEST,
                "the Freight names the same digest, and commit a1f3c9e",
                "The digest matches on both sides.",
            ),
            unlinked: None,
            actions: vec![
                resource(
                    "the Freight",
                    object(CICD, kargo::GROUP, "Freight", PROJECT, FREIGHT),
                ),
                Action::browser("Open in Kargo", Ok(kargo_page())),
            ],
        })),
    };
    vec![
        pull_request,
        pruned,
        push,
        signature,
        policy,
        image,
        freight,
    ]
}

/// prod-ams: verified upstream, waiting for promotion, still running
/// brave-lynx, and its pods forbidden to the reader.
fn waiting(group: usize, now: DateTime<Utc>) -> Vec<Hop> {
    let app = "checkout-prod-ams";
    vec![
        Hop {
            key: "prod-ams-argocd".into(),
            group,
            state: Healthy,
            name: app.into(),
            detail: "Synced · Healthy · runs brave-lynx".into(),
            from: CORE.into(),
            at: Some(at(now, -48)),
            link: None,
            after: Some("prod-ams-eligible".into()),
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "Argo CD Application".into(),
                title: app.into(),
                state: "Synced · Healthy".into(),
                notice: None,
                fields: vec![
                    text("Project", PROJECT),
                    mono("Destination", "prod-ams"),
                    mono("Namespace", PROJECT),
                    mono("Synced revision", "5d20b1a (brave-lynx)"),
                ],
                link: None,
                unlinked: Some(
                    "Not reached yet: prod-ams runs brave-lynx until wonky-otter is promoted \
                     there."
                        .into(),
                ),
                actions: vec![
                    argocd_page(app),
                    resource("the Application", application_object(app)),
                ],
            })),
        },
        Hop {
            key: "prod-ams-pods".into(),
            group,
            state: Unknown,
            name: "Pods".into(),
            detail: "Not readable: pods is forbidden on prod-ams".into(),
            from: "prod-ams".into(),
            at: None,
            link: Some(Confidence::Unknown),
            after: Some("prod-ams-argocd".into()),
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "Pods".into(),
                title: "checkout-api".into(),
                state: "Not readable".into(),
                notice: Some(Notice {
                    state: Warning,
                    lead: "Not readable.".into(),
                    body: "pods is forbidden on prod-ams in namespace checkout for mira. This \
                           says nothing about what runs there."
                        .into(),
                }),
                fields: vec![
                    mono("Cluster", "prod-ams"),
                    mono("Namespace", PROJECT),
                    text("Read as", "mira"),
                ],
                link: link(
                    Confidence::Unknown,
                    "digest",
                    "Freight wonky-otter",
                    DIGEST,
                    "can't be read",
                    "A side can't be read, so the link is Unknown. An unreadable list isn't an \
                     empty one: nothing is shown as missing.",
                ),
                unlinked: None,
                actions: vec![resource(
                    "the Deployment",
                    object(
                        "prod-ams",
                        deployments::GROUP,
                        "Deployment",
                        PROJECT,
                        "checkout-api",
                    ),
                )],
            })),
        },
    ]
}

/// The whole invented change, its times counted back from `now`.
pub(crate) fn change(now: DateTime<Utc>) -> Change {
    let stages = stages(now);
    let mut groups = vec![
        Group {
            phase: Phase::Build,
            detail: format!("{GIT} · {CICD} · {REGISTRY}"),
        },
        Group {
            phase: Phase::Freight,
            detail: format!("Kargo on {CICD}"),
        },
    ];
    groups.extend(stages.iter().enumerate().map(|(ix, stage)| Group {
        phase: Phase::Stage(ix),
        detail: stage.cluster.words(),
    }));
    let mut hops = build(now);
    let (dev, stage, ams, lon) = (2, 3, 4, 5);
    hops.extend(gates(
        dev,
        0,
        "dev",
        "freight",
        [
            (Healthy, "From the Warehouse", Some(at(now, 19))),
            (Healthy, "Automatic · 4 of 4 steps", Some(at(now, 20))),
            (Healthy, "2 of 2 analyses passed", Some(at(now, 31))),
        ],
    ));
    hops.extend(deployed(
        dev,
        "dev",
        "dev-fra",
        "3b07e9d",
        at(now, 22),
        "3 of 3 run the digest",
    ));
    hops.extend(gates(
        stage,
        1,
        "stage",
        "dev-eligible",
        [
            (Healthy, "Verified in dev", Some(at(now, 31))),
            (Healthy, "Automatic · 4 of 4 steps", Some(at(now, 41))),
            (Healthy, "3 of 3 analyses passed", Some(at(now, 62))),
        ],
    ));
    hops.extend(deployed(
        stage,
        "stage",
        "stage-fra",
        "81c4a2f",
        at(now, 43),
        "3 of 3 run the digest",
    ));
    hops.extend(gates(
        ams,
        2,
        "prod-ams",
        "stage-eligible",
        [
            (Healthy, "Verified in stage", Some(at(now, 62))),
            (Pending, "Waiting for promotion: auto-promotion off", None),
            (Pending, "Runs after promotion", None),
        ],
    ));
    hops.extend(waiting(ams, now));
    hops.extend(gates(
        lon,
        3,
        "prod-lon",
        "stage-eligible",
        [
            (Info, "Approved by hand, past stage", Some(at(now, 50))),
            (Healthy, "Automatic · 4 of 4 steps", Some(at(now, 52))),
            (
                Error,
                "Failed: error rate 2.4 %, above 1 %",
                Some(at(now, 61)),
            ),
        ],
    ));
    hops.extend(deployed(
        lon,
        "prod-lon",
        "prod-lon",
        "e4a9c1b",
        at(now, 53),
        "6 of 6 run the digest",
    ));
    Change {
        project: PROJECT.into(),
        freight: FREIGHT.into(),
        kargo_cluster: CICD.into(),
        observed_at: now,
        page: Ok(kargo_page()),
        origin: Origin {
            pull_request: Some(PULL_REQUEST),
            commit: Some(COMMIT.into()),
            title: Some(TITLE.into()),
            pull: crate::github::example::pull(PULL_REQUEST),
        },
        missing: Vec::new(),
        groups,
        stages,
        hops,
    }
}

/// What the example couldn't read, as the acme workspace's entries are
/// reshaped for captures and tests (`FRESHKUBE_APPLICATIONS` in debug
/// builds): each in the words a live read across the workspace uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    /// Everything read.
    #[default]
    Whole,
    /// Argo CD installed in five namespaces, two past the read cap.
    Spread,
    /// `cicd-fra`, where Kargo and the builds are read, couldn't be
    /// reached: no Freight answers.
    KargoUnreachable,
    /// Argo CD's Applications refused on `core-fra`.
    ArgoRefused,
    /// `dev-fra`, the `dev` Stage's destination, couldn't be reached.
    DestinationUnreachable,
    /// The Freight names an image and no commit, so the build is joined to
    /// it by the image digest a result of the build reports.
    ImageOnly,
    /// GitHub's rate limit is reached: the pull requests read before are
    /// kept, marked stale.
    GitHubLimited,
    /// GitHub refused the token: no pull request was read.
    GitHubRefused,
    /// GitHub didn't answer: the pull requests read before are kept,
    /// marked stale.
    GitHubOffline,
}

impl Shape {
    /// What is wrong with the example's GitHub in this shape.
    fn github(self) -> Option<Trouble> {
        match self {
            Self::GitHubLimited => Some(Trouble::Limited),
            Self::GitHubRefused => Some(Trouble::Refused),
            Self::GitHubOffline => Some(Trouble::Offline),
            _ => None,
        }
    }
}

/// Why the example's unreachable entry wasn't read.
const NO_ANSWER: &str = "no answer within 5 s; a credential plugin may be waiting for a sign-in";
/// The Stage whose destination [`Shape::DestinationUnreachable`] can't
/// reach, and its cluster.
const UNREACHED: (&str, &str) = ("dev", "dev-fra");

impl Shape {
    /// The words of what wasn't read, as a live read says it.
    fn words(self) -> Option<String> {
        match self {
            Self::Whole
            | Self::Spread
            | Self::ImageOnly
            | Self::GitHubLimited
            | Self::GitHubRefused
            | Self::GitHubOffline => None,
            Self::KargoUnreachable => Some(words::unreachable(CICD, NO_ANSWER)),
            Self::ArgoRefused => Some(format!(
                "Argo CD Applications were refused on {CORE}: applications.argoproj.io is \
                 forbidden for this identity"
            )),
            Self::DestinationUnreachable => Some(words::unreachable(
                UNREACHED.1,
                &format!("connection refused by {}.example.test:6443", UNREACHED.1),
            )),
        }
    }

    /// Why `project`'s Freight couldn't be read, when its entry wasn't.
    pub fn unreached(self) -> Option<String> {
        (self == Self::KargoUnreachable)
            .then(|| self.words())
            .flatten()
    }

    /// `change` as this shape reads it: an error when its Freight couldn't
    /// be read at all.
    pub fn apply(self, mut change: Change) -> Result<Change, Failure> {
        if self == Self::Spread {
            spread(&mut change);
            return Ok(change);
        }
        if self == Self::ImageOnly {
            image_only(&mut change);
            return Ok(change);
        }
        if let Some(trouble) = self.github() {
            github(&mut change, trouble);
            return Ok(change);
        }
        let Some(why) = self.words() else {
            return Ok(change);
        };
        match self {
            Self::Whole
            | Self::Spread
            | Self::ImageOnly
            | Self::GitHubLimited
            | Self::GitHubRefused
            | Self::GitHubOffline => {}
            Self::KargoUnreachable => {
                return Err(Failure::new(
                    FailureKind::Unreachable,
                    format!(
                        "Freight {} of {} couldn't be read: {why}",
                        change.freight, change.project
                    ),
                ));
            }
            Self::ArgoRefused => {
                for ix in 0..change.stages.len() {
                    let Some(group) = stage_group(&change, ix) else {
                        continue;
                    };
                    // The supplied Application is the evidence for a placeholder;
                    // a Stage alone does not imply an Application exists.
                    if !change.hops_of(group).any(|hop| {
                        matches!(&hop.shows,
                        Shows::Hop(detail) if detail.kind == "Argo CD Application")
                    }) {
                        continue;
                    }
                    let stage = &mut change.stages[ix];
                    if stage.reach != Reach::NotReached {
                        stage.reach = Reach::Refused("Argo CD Applications".into());
                    }
                    stage.cluster =
                        Destination::Unknown(format!("its Application couldn't be read: {why}"));
                    change.groups[group].detail = stage.cluster.words();
                    let name = stage.name.clone();
                    let at = change
                        .hops
                        .iter()
                        .position(|hop| hop.group == group && !is_gate(hop))
                        .unwrap_or(change.hops.len());
                    change.hops.retain(|hop| hop.group != group || is_gate(hop));
                    let at = at.min(change.hops.len());
                    change
                        .hops
                        .insert(at, refused_application(&change.project, &name, group, &why));
                }
            }
            Self::DestinationUnreachable => {
                let (name, cluster) = UNREACHED;
                if let Some(ix) = change.stages.iter().position(|stage| stage.name == name)
                    && let Some(group) = stage_group(&change, ix)
                {
                    let stage = &mut change.stages[ix];
                    // A side couldn't be read: Unknown, unless it was less.
                    let unread = Reach::Linked(Confidence::Unknown);
                    if stage.reach.rank() > unread.rank() {
                        stage.reach = unread;
                    }
                    stage.cluster = Destination::Entry {
                        entry: cluster.into(),
                        via: format!("server https://{cluster}.example.test:6443"),
                    };
                    change.groups[group].detail = stage.cluster.words();
                    let read_there =
                        |hop: &Hop| hop.group == group && hop.from == cluster && !is_gate(hop);
                    if let Some(at) = change.hops.iter().position(read_there) {
                        change.hops.retain(|hop| !read_there(hop));
                        let workload = reported(name, cluster, group, &why);
                        let pods = super::unread::pods(&workload, cluster, &why);
                        change.hops.splice(at..at, [workload, pods]);
                    }
                }
            }
        }
        let gap = match self {
            Self::ArgoRefused => Gap::Refused,
            _ => Gap::Unreadable,
        };
        change.missing.push(Missing::new(gap, why));
        Ok(change)
    }
}

/// The change as GitHub leaves it in `trouble`: its pull request kept and
/// stale, or none, and what the read couldn't do among what may be missing.
fn github(change: &mut Change, trouble: Trouble) {
    change.origin.pull = trouble.leaves(change.origin.pull.take());
    let (gap, words) = match (
        trouble,
        change.origin.pull.as_ref().and_then(|p| p.stale.as_ref()),
    ) {
        (Trouble::Refused, _) => (Gap::Refused, trouble.words().to_owned()),
        (_, Some(stale)) => (
            Gap::Unreadable,
            format!(
                "{}; showing what was read at {}",
                stale.why,
                stale.read_at.format("%H:%M UTC")
            ),
        ),
        (_, None) => (Gap::Unreadable, trouble.words().to_owned()),
    };
    change.missing.push(Missing::new(gap, words));
}

/// The example's Freight as a Warehouse that subscribes to an image alone
/// makes it: no commit, so no pull request or commit hop, and the build is
/// joined to the Freight on the image digest.
fn image_only(change: &mut Change) {
    change.origin = Origin {
        pull_request: None,
        commit: None,
        title: None,
        pull: None,
    };
    change
        .hops
        .retain(|hop| !matches!(hop.key.as_str(), "pr" | "pr-run"));
    for hop in &mut change.hops {
        let Shows::Hop(detail) = &mut hop.shows else {
            continue;
        };
        match hop.key.as_str() {
            "push-run" => {
                hop.after = None;
                detail.link = link(
                    Confidence::Confirmed,
                    "digest",
                    "Freight image",
                    DIGEST,
                    "a result of the run, IMAGE_DIGEST, reports the Freight's image digest",
                    "The Freight names no commit, so the build is found by the digest its \
                     results report. A digest names one image, whichever commit built it.",
                );
            }
            "freight" => {
                detail.fields.retain(|field| field.label != "Commit");
                detail.link = link(
                    Confidence::Confirmed,
                    "digest",
                    "Image checkout-api",
                    DIGEST,
                    "the Freight names the same digest",
                    "The digest matches on both sides.",
                );
            }
            _ => {}
        }
    }
}

/// Where Argo CD is installed in the spread example, in discovery order.
pub const SPREAD_NAMESPACES: [&str; 5] = ["delivery", "gitops", "platform", "sandbox", "tenants"];

/// A spread Application's namespace, shared by the Applications inputs
/// and their change trails. The dev and stage Applications exercise the
/// second and third discovered namespaces.
pub fn spread_namespace(application: &str) -> &'static str {
    match application
        .strip_prefix("checkout-")
        .or_else(|| application.strip_prefix("cart-"))
    {
        Some("dev") => SPREAD_NAMESPACES[1],
        Some("stage") => SPREAD_NAMESPACES[2],
        _ => SPREAD_NAMESPACES[0],
    }
}

/// The same namespace scope and omissions as the live read, with the
/// Application's namespace visible in its row, Inspector and actions.
fn spread(change: &mut Change) {
    for hop in &mut change.hops {
        let Shows::Hop(detail) = &mut hop.shows else {
            continue;
        };
        if detail.kind != "Argo CD Application" {
            continue;
        }
        let namespace = spread_namespace(&detail.title);
        hop.detail = format!("{} · {namespace}", hop.detail);
        detail.fields.push(mono("Application namespace", namespace));
        for action in &mut detail.actions {
            match &mut action.target {
                Target::Resource { object, .. } if object.kind == "Application" => {
                    object.namespace = namespace.into();
                }
                Target::Browser { label, address } if label == "Open in Argo CD" => {
                    *address = Address::parse(ARGOCD_PAGE)
                        .map(|base| base.join(["applications", namespace, detail.title.as_str()]))
                        .ok();
                }
                _ => {}
            }
        }
    }
    let cap = crate::applications::read::MAX_ARGO_NAMESPACES;
    let namespaces = SPREAD_NAMESPACES[..cap]
        .iter()
        .map(|ns| (*ns).into())
        .collect::<Vec<_>>();
    let found = crate::applications::ArgoFound::Labelled {
        skipped: SPREAD_NAMESPACES[cap..]
            .iter()
            .map(|ns| (*ns).into())
            .collect(),
    };
    if let Some(note) = crate::applications::read::argo_namespace_note(CORE, &namespaces, &found) {
        change.missing.push(Missing::new(Gap::Unreadable, note));
    }
}

/// The group of the `ix`th Stage.
fn stage_group(change: &Change, ix: usize) -> Option<usize> {
    change
        .groups
        .iter()
        .position(|group| group.phase == Phase::Stage(ix))
}

/// A Stage's gate, which Kargo answers, not Argo CD.
fn is_gate(hop: &Hop) -> bool {
    matches!(hop.shows, Shows::Stage(_))
}

/// In place of what Argo CD's Applications would have said of a Stage.
fn refused_application(project: &str, stage: &str, group: usize, why: &str) -> Hop {
    Hop {
        key: format!("{stage}-argocd"),
        group,
        state: Unknown,
        name: "Application not read".into(),
        detail: why.into(),
        from: CORE.into(),
        at: None,
        link: Some(Confidence::Unknown),
        after: Some(format!("{stage}-eligible")),
        shows: Shows::Hop(Box::new(HopDetail {
            kind: "Argo CD Application".into(),
            title: "Application not read".into(),
            state: "Unknown".into(),
            notice: Some(Notice {
                state: Warning,
                lead: "Not readable.".into(),
                body: format!(
                    "{why}. This says nothing about what runs there: nothing past the Stage \
                     can be followed."
                ),
            }),
            fields: vec![
                mono("Kargo Stage", &format!("{project}/{stage}")),
                mono("Read from", CORE),
            ],
            link: link(
                Confidence::Unknown,
                "nothing",
                "Stage",
                "not reported",
                "not read",
                why,
            ),
            unlinked: None,
            actions: Vec::new(),
        })),
    }
}

/// What Argo CD reports of a Stage's Deployment on an entry that couldn't
/// be reached, as a live read shows it.
fn reported(stage: &str, cluster: &str, group: usize, why: &str) -> Hop {
    let mut target = deployment(cluster);
    target.cluster = String::new();
    Hop {
        key: format!("{stage}-reported-{PROJECT}-{stage}-0"),
        group,
        state: Info,
        name: format!("Deployment {DEPLOYMENT}"),
        detail: format!("Argo CD reports it on {cluster}; not read there"),
        from: CORE.into(),
        at: None,
        link: Some(Confidence::Claimed),
        after: Some(format!("{stage}-argocd")),
        shows: Shows::Hop(Box::new(HopDetail {
            kind: "Deployment".into(),
            title: DEPLOYMENT.into(),
            state: "Argo CD reports Synced · Healthy".into(),
            notice: Some(Notice {
                state: Info,
                lead: "Not read.".into(),
                body: format!("{why}, so this is what Argo CD reports, not what was read there."),
            }),
            fields: vec![
                mono("Namespace", PROJECT),
                mono("Name", DEPLOYMENT),
                mono("Cluster", cluster),
                mono("Read from", &format!("Argo CD on {CORE}")),
            ],
            link: Some(LinkDetail {
                confidence: Confidence::Claimed,
                by: "name".into(),
                before: "Application".into(),
                key: None,
                before_says: format!("status.resources lists Deployment {PROJECT}/{DEPLOYMENT}"),
                here_says: "not read".into(),
                why: format!("Only Argo CD's status says so: {why}, so nothing there confirms it."),
            }),
            unlinked: None,
            actions: vec![Action::new(Target::OnEntry {
                what: "the Deployment".into(),
                entry: cluster.into(),
                object: target,
            })],
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_760_000_000, 0).unwrap()
    }

    fn assert_unread(hop: &Hop, after: &str, reason: &str) {
        assert_eq!(hop.state, Unknown);
        assert_eq!(hop.link, Some(Confidence::Unknown));
        assert_eq!(hop.after.as_deref(), Some(after));
        assert_eq!(hop.at, None);
        assert!(hop.detail.contains(reason), "{}", hop.detail);
        let Shows::Hop(detail) = &hop.shows else {
            panic!("placeholder");
        };
        let link = detail.link.as_ref().unwrap();
        assert_eq!(link.confidence, Confidence::Unknown);
        assert_eq!(link.key, None);
        assert!(link.why.contains(reason));
        assert!(detail.actions.is_empty());
    }

    #[test]
    fn argo_refused_keeps_each_known_application_without_guessing_its_name() {
        let original = change(now());
        let change = Shape::ArgoRefused.apply(original.clone()).unwrap();
        let reason = &change.missing.last().unwrap().text;
        assert_eq!(change.missing.last().unwrap().gap, Gap::Refused);
        for (ix, stage) in change.stages.iter().enumerate() {
            let group = stage_group(&change, ix).unwrap();
            let hops: Vec<_> = change.hops_of(group).collect();
            assert_eq!(
                hops.len(),
                4,
                "three gates and one Application: {}",
                stage.name
            );
            let app = change.hop(&format!("{}-argocd", stage.name)).unwrap();
            assert_unread(app, &format!("{}-eligible", stage.name), reason);
            assert_eq!(app.name, "Application not read");
            let Shows::Hop(detail) = &app.shows else {
                unreachable!()
            };
            assert_eq!(detail.title, "Application not read");
            assert_eq!(stage.link, original.stages[ix].link);
        }
        assert_eq!(
            change
                .hops
                .iter()
                .filter(|h| h.group < 2)
                .collect::<Vec<_>>(),
            original
                .hops
                .iter()
                .filter(|h| h.group < 2)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn argo_refused_does_not_invent_an_application_for_a_stage_without_one() {
        let mut original = change(now());
        let group = stage_group(&original, 1).unwrap();
        original
            .hops
            .retain(|hop| hop.group != group || is_gate(hop));
        let changed = Shape::ArgoRefused.apply(original.clone()).unwrap();
        assert_eq!(
            changed.hops_of(group).collect::<Vec<_>>(),
            original.hops_of(group).collect::<Vec<_>>()
        );
        assert_eq!(changed.stages[1], original.stages[1]);
    }

    #[test]
    fn an_image_only_freight_starts_at_the_build_the_digest_found() {
        let original = change(now());
        let image = Shape::ImageOnly.apply(original.clone()).unwrap();
        assert!(image.hop("pr").is_none() && image.hop("pr-run").is_none());
        assert_eq!(image.origin.commit, None);
        assert!(image.missing.is_empty());
        let build = image.hop("push-run").unwrap();
        assert_eq!(build.after, None);
        let Shows::Hop(detail) = &build.shows else {
            panic!("a hop's detail");
        };
        let joined = detail.link.as_ref().unwrap();
        assert_eq!(
            (joined.confidence, joined.by.as_str()),
            (Confidence::Confirmed, "digest")
        );
        assert_eq!(joined.key.as_deref(), Some(DIGEST));
        let Shows::Hop(freight) = &image.hop("freight").unwrap().shows else {
            panic!("a hop's detail");
        };
        assert!(freight.fields.iter().all(|field| field.label != "Commit"));
        // Every other hop is the example's own.
        assert_eq!(
            image.hops.iter().filter(|h| h.group > 1).count(),
            original.hops.iter().filter(|h| h.group > 1).count()
        );
        let mut keys: Vec<_> = image.hops.iter().map(|hop| &hop.key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), image.hops.len());
    }

    #[test]
    fn an_unreachable_destination_keeps_unknown_pods_after_its_reported_workload() {
        let original = change(now());
        let change = Shape::DestinationUnreachable
            .apply(original.clone())
            .unwrap();
        let workload = change.hop("dev-reported-checkout-dev-0").unwrap();
        assert_eq!(workload.after.as_deref(), Some("dev-argocd"));
        assert_eq!(workload.link, Some(Confidence::Claimed));
        let pods = change
            .hops
            .iter()
            .find(|hop| hop.after.as_deref() == Some(workload.key.as_str()))
            .unwrap();
        assert_unread(pods, &workload.key, &change.missing.last().unwrap().text);
        assert_eq!(pods.name, "Pods");
        assert_eq!(pods.from, "dev-fra");
        assert_eq!(change.stages[0].reach, Reach::Linked(Confidence::Unknown));
        assert_eq!(
            change
                .hops
                .iter()
                .filter(|h| h.group != workload.group)
                .collect::<Vec<_>>(),
            original
                .hops
                .iter()
                .filter(|h| h.group != workload.group)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_unreachable_destination_without_a_reported_workload_gets_no_pods() {
        let mut original = change(now());
        let group = stage_group(&original, 0).unwrap();
        original
            .hops
            .retain(|hop| hop.group != group || hop.from != UNREACHED.1);
        let changed = Shape::DestinationUnreachable
            .apply(original.clone())
            .unwrap();
        assert_eq!(
            changed.hops_of(group).collect::<Vec<_>>(),
            original.hops_of(group).collect::<Vec<_>>()
        );
    }

    #[test]
    fn stage_links_explain_their_parents_without_claiming_a_join() {
        let change = change(now());
        let Shows::Hop(freight) = &change.hop("freight").unwrap().shows else {
            panic!("Freight detail");
        };
        let warehouse = freight
            .fields
            .iter()
            .find(|field| field.label == "Warehouse")
            .unwrap();
        let Value::Mono(warehouse) = &warehouse.value else {
            panic!("Warehouse name");
        };
        for (name, parent, before, evidence, reach) in [
            (
                "dev",
                "freight",
                format!("Freight {FREIGHT}"),
                warehouse.as_str(),
                Reach::Linked(Confidence::Confirmed),
            ),
            (
                "prod-ams",
                "stage-eligible",
                "Stage stage".into(),
                PREVIOUS,
                Reach::NotReached,
            ),
            (
                "prod-lon",
                "stage-eligible",
                "Stage stage".into(),
                "jon's approval",
                Reach::Linked(Confidence::Confirmed),
            ),
        ] {
            let stage = change
                .stages
                .iter()
                .find(|stage| stage.name == name)
                .unwrap();
            let detail = stage.link.as_ref().expect("the example explains its link");
            assert_eq!(detail.confidence, Confidence::Unknown);
            assert_eq!(detail.key, None);
            assert_eq!(detail.before, before);
            assert!(!detail.before_says.is_empty() && !detail.here_says.is_empty());
            assert!(detail.why.contains(name) && detail.why.contains(evidence));
            assert!(detail.why.ends_with('.'));
            assert_eq!(stage.reach, reach, "a link reason doesn't change reach");
            let gate = change.hop(&format!("{name}-eligible")).unwrap();
            assert_eq!(gate.after.as_deref(), Some(parent));
            assert_eq!(gate.link, None, "a gate remains a gate, not a new join");
        }
    }

    #[test]
    fn every_hop_has_a_unique_key_and_a_group_that_exists() {
        let change = change(now());
        let mut keys: Vec<&str> = change.hops.iter().map(|hop| hop.key.as_str()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), change.hops.len());
        assert!(
            change
                .hops
                .iter()
                .all(|hop| hop.group < change.groups.len())
        );
        // Travel order: groups never go back.
        assert!(change.hops.windows(2).all(|w| w[0].group <= w[1].group));
    }

    #[test]
    fn every_hop_follows_a_hop_of_the_change_and_two_stages_share_their_stage_before() {
        let change = change(now());
        let keys: Vec<&str> = change.hops.iter().map(|hop| hop.key.as_str()).collect();
        for hop in &change.hops {
            if let Some(after) = hop.after.as_deref() {
                assert!(keys.contains(&after), "{} follows {after}", hop.key);
                assert_ne!(after, hop.key);
            }
        }
        let after = |key: &str| {
            change
                .hops
                .iter()
                .find(|hop| hop.key == key)
                .and_then(|hop| hop.after.as_deref())
        };
        // prod-ams and prod-lon both run the commit after stage.
        assert_eq!(after("prod-ams-eligible"), Some("stage-eligible"));
        assert_eq!(after("prod-lon-eligible"), Some("stage-eligible"));
        assert_eq!(after("dev-eligible"), Some("freight"));
    }

    #[test]
    fn a_link_names_the_value_it_joined_on() {
        let change = change(now());
        for hop in &change.hops {
            if let Shows::Hop(detail) = &hop.shows
                && let Some(link) = &detail.link
            {
                assert!(
                    link.key.as_deref().is_some_and(|key| !key.is_empty()),
                    "{} joined by {} says no key",
                    hop.key,
                    link.by
                );
            }
        }
    }

    #[test]
    fn each_stage_has_its_three_gates_apart_and_gates_have_no_link() {
        let change = change(now());
        for (ix, stage) in change.stages.iter().enumerate() {
            let gates: Vec<&Hop> = change
                .hops
                .iter()
                .filter(|hop| hop.shows == Shows::Stage(ix))
                .collect();
            let names: Vec<&str> = gates.iter().map(|hop| hop.name.as_str()).collect();
            assert_eq!(
                names,
                ["Eligible", "Promotion", "Verification"],
                "{}",
                stage.name
            );
            assert!(gates.iter().all(|hop| hop.link.is_none()));
        }
    }

    /// A hop's Link column and its Inspector's Link section say one
    /// confidence; a hop with no link in its Inspector has none in the
    /// column either.
    #[test]
    fn a_hops_link_is_said_once() {
        for hop in change(now()).hops {
            if let Shows::Hop(detail) = &hop.shows {
                assert_eq!(
                    hop.link,
                    detail.link.as_ref().map(|link| link.confidence),
                    "{}",
                    hop.key
                );
            }
        }
    }

    #[test]
    fn the_stages_are_acmes_checkout_stages_in_promotion_order() {
        let change = change(now());
        let names: Vec<&str> = change.stages.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, STAGES);
        assert_eq!(freight_of("checkout", "dev"), Some(FREIGHT));
        assert_eq!(freight_of("cart", "dev"), None);
        assert!(for_stage("checkout", "prod-ams", now()).is_some());
        assert!(for_stage("checkout", "qa", now()).is_none());
        assert!(for_stage("cart", "dev", now()).is_none());
    }

    #[test]
    fn missing_evidence_is_never_a_failure() {
        let change = change(now());
        let pruned = change.hop("pr-run").unwrap();
        assert_eq!(pruned.state, Unknown);
        assert_eq!(pruned.link, Some(Confidence::Claimed));
        let Shows::Hop(detail) = &pruned.shows else {
            panic!("a run shows itself")
        };
        assert!(detail.actions[0].disabled.is_some());
        let forbidden = change.hop("prod-ams-pods").unwrap();
        assert_eq!(forbidden.state, Unknown);
        assert_eq!(forbidden.link, Some(Confidence::Unknown));
    }

    #[test]
    fn times_count_from_the_reference_instant() {
        let change = change(now());
        assert_eq!(change.observed_at, now());
        assert!(
            change
                .hops
                .iter()
                .filter_map(|hop| hop.at)
                .all(|at| at <= now())
        );
    }

    #[test]
    fn addresses_are_invented() {
        let mut change = change(now());
        // The pull request's page and avatars are GitHub's own addresses,
        // with invented names in them.
        change.origin.pull = None;
        let text = format!("{change:?}");
        // API groups end in `.io`; hosts are all under example.test.
        for host in ["github", "harbor", ".com", "://"] {
            assert!(!text.contains(host), "{host}");
        }
        assert!(GIT.ends_with(".example.test") && REGISTRY.ends_with(".example.test"));
    }
}
