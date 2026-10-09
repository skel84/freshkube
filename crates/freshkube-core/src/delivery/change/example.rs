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

use super::*;
use crate::applications::example::{CICD, CORE};
use crate::delivery::{argocd, deployments, kargo};
use crate::indicators::HealthIndicator::{self, *};

pub const PROJECT: &str = "checkout";
pub const FREIGHT: &str = "wonky-otter";
/// The git and registry hosts the build read from.
pub const GIT: &str = "git.example.test";
pub const REGISTRY: &str = "registry.example.test";

const DIGEST: &str = "sha256:9c4e…b21a";
const COMMIT: &str = "a1f3c9e";
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
pub const STAGES: [&str; 4] = ["dev", "stage", "prod-ams", "prod-lon"];

/// The Freight a Stage of a project carries, when example mode has one:
/// every Stage of `checkout` leads to `wonky-otter`. Builds no change.
pub fn freight_of(project: &str, stage: &str) -> Option<&'static str> {
    (project == PROJECT && STAGES.contains(&stage)).then_some(FREIGHT)
}

/// The change of a Stage of a project, when example mode has one.
pub fn for_stage(project: &str, stage: &str, now: DateTime<Utc>) -> Option<Change> {
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

fn resource(what: &str, object: Object) -> Action {
    Action::new(Target::Resource {
        what: what.into(),
        object,
    })
}

fn browser(label: &str) -> Action {
    Action::new(Target::Browser {
        label: label.into(),
    })
}

/// The Stage itself, in Kargo's project namespace.
pub fn stage_object(stage: &str) -> Object {
    object(CORE, kargo::GROUP, "Stage", PROJECT, stage)
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
        cluster: cluster.into(),
        state,
        words: words.into(),
        running: None,
        eligible,
        promotion,
        approvers: Some(approvers.into()),
        promoters: Some(promoters.into()),
        steps,
        verification,
        link: None,
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
            running: Some("brave-lynx".into()),
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

/// A Stage's three gate rows, which show its gates in the Inspector.
fn gates(
    group: usize,
    stage: usize,
    name: &str,
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
            from: CORE.into(),
            at,
            link: None,
            shows: Shows::Stage(stage),
        })
        .collect()
}

/// Argo CD's Application and the pods for a Stage that runs this change,
/// both Confirmed.
fn deployed(
    group: usize,
    stage: &str,
    cluster: &str,
    revision: &str,
    synced: DateTime<Utc>,
    pods: &str,
) -> Vec<Hop> {
    let app = format!("{PROJECT}-{stage}");
    vec![
        Hop {
            key: format!("{stage}-argocd"),
            group,
            state: Healthy,
            name: app.clone(),
            detail: "Synced · Healthy".into(),
            from: CORE.into(),
            at: Some(synced),
            link: Some(Confidence::Confirmed),
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
                    browser("Open diff in Argo CD"),
                    resource("the Application", application_object(&app)),
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
                actions: vec![resource(
                    "the Deployment",
                    object(
                        cluster,
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
            actions: vec![browser("Open the pull request")],
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
                Action::new(Target::Logs {
                    cluster: CICD.into(),
                })
                .disabled("Run pruned, logs gone"),
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
                Action::new(Target::Logs {
                    cluster: CICD.into(),
                }),
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
        shows: Shows::Hop(Box::new(HopDetail {
            kind: "Conforma".into(),
            title: "acme-prod".into(),
            state: "Passed".into(),
            notice: None,
            fields: vec![
                text("Rules", "41 of 41 passed"),
                mono("Task", "checkout-push-x7k2-verify"),
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
            actions: vec![resource(
                "the TaskRun",
                task_run("checkout-push-x7k2-verify"),
            )],
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
            actions: vec![browser("Open in the registry")],
        })),
    };
    let freight = Hop {
        key: "freight".into(),
        group: 1,
        state: Healthy,
        name: format!("Freight {FREIGHT}"),
        detail: "Found by Warehouse checkout-images · image + checkout-deploy".into(),
        from: CORE.into(),
        at: Some(at(now, 19)),
        link: Some(Confidence::Confirmed),
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
                    object(CORE, kargo::GROUP, "Freight", PROJECT, FREIGHT),
                ),
                browser("Open in Kargo"),
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
                    browser("Open diff in Argo CD"),
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
pub fn change(now: DateTime<Utc>) -> Change {
    let stages = stages(now);
    let mut groups = vec![
        Group {
            phase: Phase::Build,
            detail: format!("{GIT} · {CICD} · {REGISTRY}"),
        },
        Group {
            phase: Phase::Freight,
            detail: format!("Kargo on {CORE}"),
        },
    ];
    groups.extend(stages.iter().enumerate().map(|(ix, stage)| Group {
        phase: Phase::Stage(ix),
        detail: format!("deploys to {}", stage.cluster),
    }));
    let mut hops = build(now);
    let (dev, stage, ams, lon) = (2, 3, 4, 5);
    hops.extend(gates(
        dev,
        0,
        "dev",
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
        kargo_cluster: CORE.into(),
        observed_at: now,
        groups,
        stages,
        hops,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_760_000_000, 0).unwrap()
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
        let change = change(now());
        let text = format!("{change:?}");
        // API groups end in `.io`; hosts are all under example.test.
        for host in ["github", "harbor", ".com", "://"] {
            assert!(!text.contains(host), "{host}");
        }
        assert!(GIT.ends_with(".example.test") && REGISTRY.ends_with(".example.test"));
    }
}
