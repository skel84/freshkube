//! The invented change the story follows: checkout's freight wonky-otter,
//! from pull request #418 to the pods that run it, in the acme workspace
//! of the platform mocks (docs/platform/). Every name here is invented.
//!
//! The data says what a reader of each tool would find, and how each hop
//! joins the one before it: the key both sides report and the link's
//! confidence (docs/PLATFORM.md, "How the hops join").

use freshkube_ui::ui::Tone;

/// How sure a link between two hops is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    /// The key matches on both sides.
    Confirmed,
    /// Only a label, an annotation or an attestation says so.
    Claimed,
    /// A side can't be read.
    Unknown,
}

impl Confidence {
    pub fn word(self) -> &'static str {
        match self {
            Confidence::Confirmed => "Confirmed",
            Confidence::Claimed => "Claimed",
            Confidence::Unknown => "Unknown",
        }
    }

    /// The glyph the link shows, if any. Confirmed is the usual case and
    /// shows none, so the other two stand out; Claimed is something to
    /// know, not a fault, and Unknown is the dashed ring of anything not
    /// known.
    pub fn tone(self) -> Option<Tone> {
        match self {
            Confidence::Confirmed => None,
            Confidence::Claimed => Some(Tone::Info),
            Confidence::Unknown => Some(Tone::Unknown),
        }
    }
}

/// How a hop joins the hop before it.
pub struct Link {
    pub confidence: Confidence,
    /// What joins them: `commit`, `digest`, `revision`.
    pub by: &'static str,
    /// The hop before, by name.
    pub before: &'static str,
    /// What the hop before reports.
    pub before_says: &'static str,
    /// What this hop reports.
    pub here_says: &'static str,
    /// Why the link has its confidence.
    pub why: &'static str,
}

/// Where an action leads. The story opens nothing; it says where it would.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opens {
    /// An object in Resources, on the cluster that holds it.
    Resources {
        what: &'static str,
        cluster: &'static str,
    },
    /// The tool's own page, in the browser.
    Browser { label: &'static str },
    /// A step container's log in the dock, from the cluster that ran it.
    Logs { cluster: &'static str },
}

impl Opens {
    /// The button's label. A link into Resources names its cluster.
    pub fn label(self) -> String {
        match self {
            Opens::Resources { what, cluster } => format!("Open {what} in Resources · {cluster}"),
            Opens::Browser { label } => format!("{label} ↗"),
            Opens::Logs { .. } => "Logs".into(),
        }
    }

    /// What the story says when it is pressed.
    pub fn would(self) -> String {
        match self {
            Opens::Resources { what, cluster } => {
                format!("Would open {what} in Resources on {cluster}.")
            }
            Opens::Browser { label } => format!("Would {}.", label.to_lowercase()),
            Opens::Logs { cluster } => {
                format!("Would open the step's log in the dock, from {cluster}.")
            }
        }
    }
}

pub struct Action {
    pub opens: Opens,
    /// Why it can't be pressed, when it can't.
    pub disabled: Option<&'static str>,
}

const fn open(opens: Opens) -> Action {
    Action {
        opens,
        disabled: None,
    }
}

/// A notice under a hop's heading: its tone, lead and body.
pub type Notice = (Tone, &'static str, &'static str);

/// What the Inspector shows for a hop.
pub struct HopDetail {
    /// Its kind, as the heading's caption.
    pub kind: &'static str,
    /// Its name, in monospace.
    pub title: &'static str,
    pub state: &'static str,
    pub notice: Option<Notice>,
    /// Label, value, and whether the value is monospace.
    pub fields: Vec<(&'static str, &'static str, bool)>,
    pub link: Option<Link>,
    /// Said in the link's place for a hop that has none.
    pub unlinked: Option<&'static str>,
    pub actions: Vec<Action>,
}

/// One check of a gate: its state, its name and what it found.
pub type Check = (Tone, &'static str, &'static str);

/// How the freight became eligible for a Stage.
pub enum Eligible {
    /// It passed verification in the upstream Stage.
    Verified {
        upstream: &'static str,
        at: &'static str,
        checks: &'static str,
    },
    /// Someone approved it for this Stage by hand, past its upstream.
    Approved {
        by: &'static str,
        at: &'static str,
        past: &'static str,
        /// What the upstream did after.
        upstream_then: &'static str,
    },
    /// The first Stage takes freight from the Warehouse.
    Warehouse { at: &'static str },
}

/// How the freight is promoted to a Stage.
pub enum Promotion {
    /// Auto-promotion is on: Kargo promoted it.
    Automatic { at: &'static str },
    /// Auto-promotion is off: it waits for someone who may promote.
    Waiting,
}

/// A Stage's three gates, kept apart: whether the freight is eligible,
/// how it is promoted, and the verification after.
pub struct Stage {
    pub name: &'static str,
    /// The environment cluster it deploys to.
    pub cluster: &'static str,
    pub tone: Tone,
    pub state: &'static str,
    /// What runs there now, when it isn't this change.
    pub running: Option<&'static str>,
    pub eligible: Eligible,
    pub promotion: Promotion,
    /// Who may approve freight for the Stage, past its upstream.
    pub approvers: &'static str,
    /// Who may promote freight to it.
    pub promoters: &'static str,
    /// The promotion's own steps.
    pub steps: Vec<Check>,
    /// Each AnalysisRun after the promotion.
    pub verification: Vec<Check>,
}

pub enum Shows {
    Hop(Box<HopDetail>),
    /// A gate row shows its Stage's three gates.
    Stage(usize),
}

/// One row of the trail.
pub struct Hop {
    /// Selects it; also its row's id.
    pub key: &'static str,
    pub group: usize,
    pub tone: Tone,
    pub name: &'static str,
    pub detail: &'static str,
    /// The cluster or service it was read from.
    pub from: &'static str,
    pub time: &'static str,
    /// How sure its link to the hop before is. Gates have none.
    pub link: Option<Confidence>,
    pub shows: Shows,
}

/// A group of the trail: a phase, or a Stage.
pub struct Group {
    pub label: &'static str,
    pub detail: &'static str,
    /// The Stage it shows, by index.
    pub stage: Option<usize>,
}

pub struct Trail {
    pub groups: Vec<Group>,
    pub stages: Vec<Stage>,
    pub hops: Vec<Hop>,
}

const DIGEST: &str = "sha256:9c4e…b21a";
const COMMIT: &str = "a1f3c9e";
/// The pull request's head commit, which its pull_request run built.
const HEAD: &str = "7d2e04b";

/// The four promotion steps every Stage of checkout runs.
const STEPS: [&str; 4] = [
    "git-clone",
    "kustomize-set-image",
    "git-push",
    "argocd-update",
];

fn steps(tone: Tone, details: [&'static str; 4]) -> Vec<Check> {
    STEPS
        .iter()
        .zip(details)
        .map(|(step, detail)| (tone, *step, detail))
        .collect()
}

fn stages() -> Vec<Stage> {
    vec![
        Stage {
            name: "dev",
            cluster: "dev-fra",
            tone: Tone::Good,
            state: "Verified",
            running: None,
            eligible: Eligible::Warehouse { at: "09:19" },
            promotion: Promotion::Automatic { at: "09:20" },
            approvers: "kargo-approver: jon@acme, mira@acme",
            promoters: "kargo-promoter: mira@acme, ops-oncall",
            steps: steps(Tone::Good, ["09:20", "09:20", "09:20", "09:21"]),
            verification: vec![
                (Tone::Good, "checkout-smoke", "passed 09:27"),
                (Tone::Good, "error-rate", "0.1 %, under 1 %, 09:31"),
            ],
        },
        Stage {
            name: "stage",
            cluster: "stage-fra",
            tone: Tone::Good,
            state: "Verified",
            running: None,
            eligible: Eligible::Verified {
                upstream: "dev",
                at: "09:31",
                checks: "2 of 2 analyses",
            },
            promotion: Promotion::Automatic { at: "09:40" },
            approvers: "kargo-approver: jon@acme, mira@acme",
            promoters: "kargo-promoter: mira@acme, ops-oncall",
            steps: steps(Tone::Good, ["09:40", "09:40", "09:41", "09:41"]),
            verification: vec![
                (Tone::Good, "checkout-smoke", "passed 09:48"),
                (Tone::Good, "error-rate", "0.2 %, under 1 %, 09:58"),
                (Tone::Good, "latency-p99", "212 ms, under 400 ms, 10:02"),
            ],
        },
        Stage {
            name: "prod-ams",
            cluster: "prod-ams",
            tone: Tone::Unknown,
            state: "Waiting for promotion",
            running: Some("brave-lynx"),
            eligible: Eligible::Verified {
                upstream: "stage",
                at: "10:02",
                checks: "3 of 3 analyses",
            },
            promotion: Promotion::Waiting,
            approvers: "kargo-approver: jon@acme, mira@acme",
            promoters: "kargo-promoter: mira@acme, ops-oncall",
            steps: steps(Tone::Unknown, ["not started"; 4]),
            verification: vec![
                (Tone::Unknown, "checkout-smoke", "runs after promotion"),
                (Tone::Unknown, "error-rate", "runs after promotion"),
            ],
        },
        Stage {
            name: "prod-fra",
            cluster: "prod-fra",
            tone: Tone::Crit,
            state: "Verification failed",
            running: None,
            eligible: Eligible::Approved {
                by: "jon@acme",
                at: "09:50",
                past: "stage",
                upstream_then: "stage verified it later, at 10:02",
            },
            promotion: Promotion::Automatic { at: "09:51" },
            approvers: "kargo-approver: jon@acme, mira@acme",
            promoters: "kargo-promoter: mira@acme, ops-oncall",
            steps: steps(Tone::Good, ["09:51", "09:51", "09:52", "09:52"]),
            verification: vec![
                (Tone::Good, "checkout-smoke", "passed 09:58"),
                (
                    Tone::Crit,
                    "error-rate",
                    "2.4 % of requests failed, above 1 %, 10:01",
                ),
            ],
        },
    ]
}

/// Argo CD's Application and the pods for a Stage that runs this change,
/// both Confirmed.
fn deployed(
    group: usize,
    keys: [&'static str; 2],
    cluster: &'static str,
    app: &'static str,
    revision: &'static str,
    synced: &'static str,
    pods: &'static str,
) -> [Hop; 2] {
    [
        Hop {
            key: keys[0],
            group,
            tone: Tone::Good,
            name: app,
            detail: "Synced · Healthy",
            from: "core-fra",
            time: synced,
            link: Some(Confidence::Confirmed),
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "Argo CD Application",
                title: app,
                state: "Synced · Healthy",
                notice: None,
                fields: vec![
                    ("Project", "checkout", false),
                    ("Destination", cluster, true),
                    ("Namespace", "checkout", true),
                    ("Synced revision", revision, true),
                    ("Source", "acme/checkout-deploy, env branch", false),
                ],
                link: Some(Link {
                    confidence: Confidence::Confirmed,
                    by: "revision",
                    before: "Promotion",
                    before_says: "git-push wrote this revision to the env branch",
                    here_says: "Argo CD synced this revision",
                    why: "The revision matches on both sides.",
                }),
                unlinked: None,
                actions: vec![
                    open(Opens::Browser {
                        label: "Open diff in Argo CD",
                    }),
                    open(Opens::Resources {
                        what: "the Application",
                        cluster: "core-fra",
                    }),
                ],
            })),
        },
        Hop {
            key: keys[1],
            group,
            tone: Tone::Good,
            name: "Pods",
            detail: pods,
            from: cluster,
            time: synced,
            link: Some(Confidence::Confirmed),
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "Pods",
                title: "checkout-api",
                state: "Running",
                notice: None,
                fields: vec![
                    ("Cluster", cluster, true),
                    ("Namespace", "checkout", true),
                    ("Owner", "deploy/checkout-api", true),
                    ("Running", pods, false),
                    ("Image", "checkout-api:1.43.0 (tag, shown only)", true),
                    ("Image ID", DIGEST, true),
                ],
                link: Some(Link {
                    confidence: Confidence::Confirmed,
                    by: "digest",
                    before: "Freight wonky-otter",
                    before_says: DIGEST,
                    here_says: "every pod's imageID reports it",
                    why: "What runs is read from the pods, not from the plan, and the digest matches.",
                }),
                unlinked: None,
                actions: vec![open(Opens::Resources {
                    what: "the pods",
                    cluster,
                })],
            })),
        },
    ]
}

/// A Stage's three gate rows, which show its gates in the Inspector.
fn gates(
    group: usize,
    stage: usize,
    keys: [&'static str; 3],
    rows: [(Tone, &'static str, &'static str); 3],
) -> [Hop; 3] {
    let names = ["Eligible", "Promotion", "Verification"];
    std::array::from_fn(|ix| {
        let (tone, detail, time) = rows[ix];
        Hop {
            key: keys[ix],
            group,
            tone,
            name: names[ix],
            detail,
            from: "core-fra",
            time,
            link: None,
            shows: Shows::Stage(stage),
        }
    })
}

fn build() -> Vec<Hop> {
    vec![
        Hop {
            key: "pr",
            group: 0,
            tone: Tone::Good,
            name: "PR #418",
            detail: "Retry idempotency keys · merged by mira",
            from: "github.com",
            time: "09:12",
            link: None,
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "Pull request",
                title: "#418",
                state: "Merged",
                notice: None,
                fields: vec![
                    ("Title", "Retry idempotency keys", false),
                    ("Repository", "github.com/acme/checkout", true),
                    ("Merged", "09:12 by mira into main", false),
                    ("Checks", "6 of 6 passed", false),
                    ("Reviews", "2 approved", false),
                    ("Head commit", HEAD, true),
                    ("Merge commit", COMMIT, true),
                ],
                link: None,
                unlinked: Some(
                    "Where the change starts: every later hop joins back to its merge commit.",
                ),
                actions: vec![open(Opens::Browser {
                    label: "Open on GitHub",
                })],
            })),
        },
        Hop {
            key: "pr-run",
            group: 0,
            tone: Tone::Unknown,
            name: "Run checkout-pr-418-m2q8",
            detail: "Run pruned, logs gone",
            from: "cicd-fra",
            time: "09:05",
            link: Some(Confidence::Claimed),
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "PipelineRun",
                title: "checkout-pr-418-m2q8",
                state: "Pruned",
                notice: Some((
                    Tone::Warn,
                    "Run pruned, logs gone.",
                    "Tekton pruned this run and Tekton Results isn't installed, so its tasks and logs can't be read.",
                )),
                fields: vec![
                    ("Repository", "acme-checkout (Pipelines as Code)", false),
                    ("Event", "pull_request #418", false),
                    ("Commit", HEAD, true),
                    ("Tasks", "Not readable: the run is pruned", false),
                ],
                link: Some(Link {
                    confidence: Confidence::Claimed,
                    by: "provenance",
                    before: "PR #418",
                    before_says: "head commit 7d2e04b",
                    here_says: "Tekton Chains' SLSA provenance names this run and 7d2e04b",
                    why: "Only the provenance says so. The run itself is pruned, so its labels and results can't be compared with the pull request.",
                }),
                unlinked: None,
                actions: vec![Action {
                    opens: Opens::Logs {
                        cluster: "cicd-fra",
                    },
                    disabled: Some("Run pruned, logs gone"),
                }],
            })),
        },
        Hop {
            key: "push-run",
            group: 0,
            tone: Tone::Good,
            name: "Run checkout-push-x7k2",
            detail: "push to main · 7 of 7 tasks · 6 m 12 s",
            from: "cicd-fra",
            time: "09:18",
            link: Some(Confidence::Confirmed),
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "PipelineRun",
                title: "checkout-push-x7k2",
                state: "Succeeded",
                notice: None,
                fields: vec![
                    ("Repository", "acme-checkout (Pipelines as Code)", false),
                    ("Event", "push to main", false),
                    (
                        "Tasks",
                        "clone, lint, test, build, sbom, push, verify: 7 of 7",
                        false,
                    ),
                    ("Took", "6 m 12 s", false),
                    ("Result IMAGE_DIGEST", DIGEST, true),
                ],
                link: Some(Link {
                    confidence: Confidence::Confirmed,
                    by: "commit",
                    before: "PR #418",
                    before_says: "merge commit a1f3c9e",
                    here_says: "label pipelinesascode.tekton.dev/sha = a1f3c9e, and its image digest matches Harbor's",
                    why: "The commit matches, and the digest the run reports is the one downstream.",
                }),
                unlinked: None,
                actions: vec![
                    open(Opens::Logs {
                        cluster: "cicd-fra",
                    }),
                    open(Opens::Resources {
                        what: "the PipelineRun",
                        cluster: "cicd-fra",
                    }),
                ],
            })),
        },
        Hop {
            key: "signature",
            group: 0,
            tone: Tone::Good,
            name: "Signature",
            detail: "Tekton Chains, keyless · SLSA v1",
            from: "cicd-fra",
            time: "09:18",
            // Only Chains' annotation says it signed; no key is read.
            link: Some(Confidence::Claimed),
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "Tekton Chains",
                title: "checkout-push-x7k2-build",
                state: "Signed",
                notice: None,
                fields: vec![
                    ("Signature", "cosign, keyless", false),
                    ("Provenance", "SLSA v1", false),
                    ("Subject", DIGEST, true),
                ],
                link: Some(Link {
                    confidence: Confidence::Claimed,
                    by: "digest",
                    before: "Run checkout-push-x7k2",
                    before_says: DIGEST,
                    here_says: "Chains' annotation says it signed this digest",
                    why: "Only Chains' annotation says signed: no key or transparency log \
                          was read, so the signature isn't verified.",
                }),
                unlinked: None,
                actions: vec![open(Opens::Resources {
                    what: "the TaskRun",
                    cluster: "cicd-fra",
                })],
            })),
        },
        Hop {
            key: "policy",
            group: 0,
            tone: Tone::Good,
            name: "Policy acme-prod",
            detail: "Conforma: 41 of 41 rules passed",
            from: "cicd-fra",
            time: "09:18",
            link: Some(Confidence::Confirmed),
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "Conforma",
                title: "acme-prod",
                state: "Passed",
                notice: None,
                fields: vec![
                    ("Rules", "41 of 41 passed", false),
                    ("Task", "checkout-push-x7k2-verify", true),
                    ("Image", DIGEST, true),
                ],
                link: Some(Link {
                    confidence: Confidence::Confirmed,
                    by: "digest",
                    before: "Signature",
                    before_says: DIGEST,
                    here_says: "Conforma checked the same digest",
                    why: "The digest matches on both sides.",
                }),
                unlinked: None,
                actions: vec![open(Opens::Resources {
                    what: "the TaskRun",
                    cluster: "cicd-fra",
                })],
            })),
        },
        Hop {
            key: "image",
            group: 0,
            tone: Tone::Warn,
            name: "Image checkout-api",
            detail: "0 critical, 3 high (2 fixable) · SBOM · immutable",
            from: "harbor.acme.io",
            time: "09:18",
            link: Some(Confidence::Confirmed),
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "Harbor artifact",
                title: "shop/checkout-api",
                state: "3 high findings",
                notice: None,
                fields: vec![
                    ("Tag", "1.43.0 (shown, never joined on)", false),
                    ("Digest", DIGEST, true),
                    (
                        "Scan",
                        "Trivy: 0 critical, 3 high, 2 of them fixable",
                        false,
                    ),
                    ("Accessories", "signature, SBOM, provenance", false),
                    ("Pushed", "09:18", false),
                ],
                link: Some(Link {
                    confidence: Confidence::Confirmed,
                    by: "digest",
                    before: "Run checkout-push-x7k2",
                    before_says: DIGEST,
                    here_says: "Harbor holds the artifact by the same digest",
                    why: "The digest matches on both sides.",
                }),
                unlinked: None,
                actions: vec![open(Opens::Browser {
                    label: "Open in Harbor",
                })],
            })),
        },
        Hop {
            key: "freight",
            group: 1,
            tone: Tone::Good,
            name: "Freight wonky-otter",
            detail: "Found by Warehouse checkout · image + checkout-deploy",
            from: "core-fra",
            time: "09:19",
            link: Some(Confidence::Confirmed),
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "Kargo Freight",
                title: "wonky-otter",
                state: "Found",
                notice: None,
                fields: vec![
                    ("Project", "checkout", false),
                    ("Warehouse", "checkout", false),
                    ("Image", DIGEST, true),
                    ("Commit", COMMIT, true),
                    ("Found", "09:19", false),
                ],
                link: Some(Link {
                    confidence: Confidence::Confirmed,
                    by: "digest",
                    before: "Image checkout-api",
                    before_says: DIGEST,
                    here_says: "the Freight names the same digest, and commit a1f3c9e",
                    why: "The digest matches on both sides.",
                }),
                unlinked: None,
                actions: vec![
                    open(Opens::Resources {
                        what: "the Freight",
                        cluster: "core-fra",
                    }),
                    open(Opens::Browser {
                        label: "Open in Kargo",
                    }),
                ],
            })),
        },
    ]
}

/// The whole invented trail.
pub fn trail() -> Trail {
    let groups = vec![
        Group {
            label: "Build",
            detail: "github.com · cicd-fra · harbor.acme.io",
            stage: None,
        },
        Group {
            label: "Freight",
            detail: "Kargo on core-fra",
            stage: None,
        },
        Group {
            label: "Stage dev",
            detail: "deploys to dev-fra",
            stage: Some(0),
        },
        Group {
            label: "Stage stage",
            detail: "deploys to stage-fra",
            stage: Some(1),
        },
        Group {
            label: "Stage prod-ams",
            detail: "deploys to prod-ams",
            stage: Some(2),
        },
        Group {
            label: "Stage prod-fra",
            detail: "deploys to prod-fra",
            stage: Some(3),
        },
    ];
    let mut hops = build();
    hops.extend(gates(
        2,
        0,
        ["dev-eligible", "dev-promotion", "dev-verification"],
        [
            (Tone::Good, "From the Warehouse", "09:19"),
            (Tone::Good, "Automatic · 4 of 4 steps", "09:20"),
            (Tone::Good, "2 of 2 analyses passed", "09:31"),
        ],
    ));
    hops.extend(deployed(
        2,
        ["dev-argocd", "dev-pods"],
        "dev-fra",
        "checkout-dev",
        "3b07e9d",
        "09:22",
        "3 of 3 run the digest",
    ));
    hops.extend(gates(
        3,
        1,
        ["stage-eligible", "stage-promotion", "stage-verification"],
        [
            (Tone::Good, "Verified in dev", "09:31"),
            (Tone::Good, "Automatic · 4 of 4 steps", "09:41"),
            (Tone::Good, "3 of 3 analyses passed", "10:02"),
        ],
    ));
    hops.extend(deployed(
        3,
        ["stage-argocd", "stage-pods"],
        "stage-fra",
        "checkout-stage",
        "81c4a2f",
        "09:43",
        "3 of 3 run the digest",
    ));
    hops.extend(gates(
        4,
        2,
        [
            "prod-ams-eligible",
            "prod-ams-promotion",
            "prod-ams-verification",
        ],
        [
            (Tone::Good, "Verified in stage", "10:02"),
            (
                Tone::Unknown,
                "Waiting for promotion: auto-promotion off",
                "",
            ),
            (Tone::Unknown, "Runs after promotion", ""),
        ],
    ));
    hops.extend([
        Hop {
            key: "prod-ams-argocd",
            group: 4,
            tone: Tone::Good,
            name: "checkout-prod-ams",
            detail: "Synced · Healthy · runs brave-lynx",
            from: "core-fra",
            time: "08:12",
            link: None,
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "Argo CD Application",
                title: "checkout-prod-ams",
                state: "Synced · Healthy",
                notice: None,
                fields: vec![
                    ("Project", "checkout", false),
                    ("Destination", "prod-ams", true),
                    ("Namespace", "checkout", true),
                    ("Synced revision", "5d20b1a (brave-lynx)", true),
                ],
                link: None,
                unlinked: Some("Not reached yet: prod-ams runs brave-lynx until wonky-otter is promoted there."),
                actions: vec![
                    open(Opens::Browser {
                        label: "Open diff in Argo CD",
                    }),
                    open(Opens::Resources {
                        what: "the Application",
                        cluster: "core-fra",
                    }),
                ],
            })),
        },
        Hop {
            key: "prod-ams-pods",
            group: 4,
            tone: Tone::Unknown,
            name: "Pods",
            detail: "Not readable: pods is forbidden on prod-ams",
            from: "prod-ams",
            time: "",
            link: Some(Confidence::Unknown),
            shows: Shows::Hop(Box::new(HopDetail {
                kind: "Pods",
                title: "checkout-api",
                state: "Not readable",
                notice: Some((
                    Tone::Warn,
                    "Not readable.",
                    "pods is forbidden on prod-ams in namespace checkout for mira@acme. This says nothing about what runs there.",
                )),
                fields: vec![
                    ("Cluster", "prod-ams", true),
                    ("Namespace", "checkout", true),
                    ("Read as", "mira@acme", false),
                ],
                link: Some(Link {
                    confidence: Confidence::Unknown,
                    by: "digest",
                    before: "Freight wonky-otter",
                    before_says: DIGEST,
                    here_says: "can't be read",
                    why: "A side can't be read, so the link is Unknown. An unreadable list isn't an empty one: nothing is shown as missing.",
                }),
                unlinked: None,
                actions: vec![open(Opens::Resources {
                    what: "the pods",
                    cluster: "prod-ams",
                })],
            })),
        },
    ]);
    hops.extend(gates(
        5,
        3,
        [
            "prod-fra-eligible",
            "prod-fra-promotion",
            "prod-fra-verification",
        ],
        [
            (Tone::Info, "Approved by hand, past stage", "09:50"),
            (Tone::Good, "Automatic · 4 of 4 steps", "09:52"),
            (Tone::Crit, "Failed: error rate 2.4 %, above 1 %", "10:01"),
        ],
    ));
    hops.extend(deployed(
        5,
        ["prod-fra-argocd", "prod-fra-pods"],
        "prod-fra",
        "checkout-prod-fra",
        "e4a9c1b",
        "09:53",
        "6 of 6 run the digest",
    ));
    Trail {
        groups,
        stages: stages(),
        hops,
    }
}
