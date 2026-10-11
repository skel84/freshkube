//! The invented Changes list example mode shows: acme's `checkout` and
//! `cart` projects, with checkout's `wonky-otter` the change page's own
//! example ([`change::example`]) and a few more Freight, so every state
//! shows: confirmed, claimed, unknown, refused and not reached. Every name
//! is invented, and every address is under `example.test`.
//!
//! Its Freight and Stages go through Kargo's real parsers. Times count back
//! from one reference instant the caller captures once.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};

use super::{Project, Read};
use crate::applications::example::CORE;
use crate::delivery::change::example::{self as change_example, GIT, PREVIOUS, STAGES};
use crate::delivery::change::{
    Action, Change, Check, Destination, Eligible, Field, Group, Hop, HopDetail, LinkDetail, Object,
    Origin, Phase, Promotion, Reach, Shows, Stage, Target, Value as Shown,
};
use crate::delivery::join::Confidence;
use crate::delivery::kargo::{GROUP, parse_freight, parse_stage};
use crate::delivery::source::Source;
use crate::delivery::tekton::title_line;
use crate::indicators::HealthIndicator::{self, *};

pub const CHECKOUT: &str = change_example::PROJECT;
pub const CART: &str = "cart";

/// The environment cluster each Stage deploys to.
const CLUSTERS: [(&str, &str); 4] = [
    ("dev", "dev-fra"),
    ("stage", "stage-fra"),
    ("prod-ams", "prod-ams"),
    ("prod-lon", "prod-lon"),
];

/// One invented Freight: its project, name, age in minutes, commit, the
/// pull request a build names, and its commit's message as Pipelines as
/// Code's `sha-title` holds it, whose first line is the title.
struct Invented {
    project: &'static str,
    name: &'static str,
    minutes: i64,
    commit: &'static str,
    pull_request: Option<u64>,
    title: &'static str,
}

impl Invented {
    /// Its title, as the live read takes it ([`title_line`]).
    fn title(&self) -> String {
        title_line(self.title).unwrap_or_default()
    }
}

const FREIGHT: [Invented; 6] = [
    Invented {
        project: CHECKOUT,
        name: "tidy-badger",
        minutes: 6,
        commit: "c7d1e20",
        pull_request: Some(421),
        title: "Show the delivery estimate at checkout",
    },
    Invented {
        project: CHECKOUT,
        name: change_example::FREIGHT,
        minutes: 52,
        commit: "a1f3c9e",
        pull_request: Some(change_example::PULL_REQUEST),
        title: change_example::TITLE,
    },
    Invented {
        project: CHECKOUT,
        name: PREVIOUS,
        minutes: 26 * 60,
        commit: "5d20b1a",
        pull_request: Some(409),
        title: "Round totals in the shop-api currency",
    },
    Invented {
        project: CHECKOUT,
        name: "quiet-heron",
        minutes: 3 * 24 * 60,
        commit: "e93b6f4",
        pull_request: Some(402),
        title: "Log the payment provider's reply",
    },
    Invented {
        project: CART,
        name: "lucky-finch",
        minutes: 2 * 60,
        commit: "0b8c3d7",
        pull_request: Some(77),
        // A body, as a real sha-title carries one.
        title: "Keep the basket when signing in\n\nA guest's basket was dropped when they signed in;\nit is now merged into theirs.\n",
    },
    Invented {
        project: CART,
        name: "sleepy-mole",
        minutes: 2 * 24 * 60,
        commit: "f41a9c2",
        pull_request: None,
        title: "Bump the session store client",
    },
];

/// The Freight each Stage of a project runs.
fn current(project: &str, stage: &str) -> Option<&'static str> {
    match (project, stage) {
        (CHECKOUT, "prod-ams") => Some(PREVIOUS),
        (CHECKOUT, _) => Some(change_example::FREIGHT),
        (CART, "dev" | "stage") => Some("lucky-finch"),
        (CART, _) => Some("sleepy-mole"),
        _ => None,
    }
}

fn freight(now: DateTime<Utc>, invented: &Invented) -> Value {
    json!({
        "apiVersion": "kargo.akuity.io/v1alpha1",
        "kind": "Freight",
        "metadata": {
            "namespace": invented.project,
            "name": invented.name,
            "creationTimestamp": (now - Duration::minutes(invented.minutes)).to_rfc3339(),
        },
        "alias": invented.name,
        "origin": {"kind": "Warehouse", "name": format!("{}-images", invented.project)},
        "commits": [{
            "repoURL": format!("https://{GIT}/acme/{}", invented.project),
            "id": invented.commit,
            "message": format!("{}\n\nInvented for the example.", invented.title()),
        }],
    })
}

fn stage(project: &str, name: &str, upstream: Option<&str>) -> Value {
    let sources = match upstream {
        Some(upstream) => json!({"stages": [upstream]}),
        None => json!({"direct": true}),
    };
    let current = current(project, name).map_or(
        json!([]),
        |freight| json!([{"items": {"Warehouse/images": {"name": freight}}}]),
    );
    json!({
        "apiVersion": "kargo.akuity.io/v1alpha1",
        "kind": "Stage",
        "metadata": {"namespace": project, "name": name},
        "spec": {"requestedFreight": [{
            "origin": {"kind": "Warehouse", "name": format!("{project}-images")},
            "sources": sources,
        }]},
        "status": {"freightHistory": current},
    })
}

fn stages(project: &str) -> Vec<Value> {
    vec![
        stage(project, "dev", None),
        stage(project, "stage", Some("dev")),
        stage(project, "prod-ams", Some("stage")),
        stage(project, "prod-lon", Some("stage")),
    ]
}

/// What the list reads in example mode.
pub fn read(now: DateTime<Utc>) -> Read {
    let projects = [CART, CHECKOUT]
        .into_iter()
        .map(|project| {
            let freight = FREIGHT
                .iter()
                .filter(|invented| invented.project == project)
                .filter_map(|invented| parse_freight(&self::freight(now, invented)))
                .collect();
            let stages: Vec<_> = stages(project).iter().filter_map(parse_stage).collect();
            let mut followed = BTreeMap::new();
            for name in STAGES.iter().filter_map(|stage| current(project, stage)) {
                if let Some(change) = change_of(project, name, now) {
                    followed.insert(name.to_owned(), Ok(Arc::new(change)));
                }
            }
            Project {
                name: project.to_owned(),
                freight: Source::Read(freight),
                stages: Source::Read(stages),
                followed,
            }
        })
        .collect();
    Read {
        cluster: CORE.to_owned(),
        projects: Source::Read(projects),
        late: 0,
    }
}

/// The change of an example Freight: the change page's own for
/// `wonky-otter`, and a shorter one for every other, which a Freight no
/// Stage runs shows with every Stage not reached.
pub fn change_of(project: &str, freight: &str, now: DateTime<Utc>) -> Option<Change> {
    if project == CHECKOUT && freight == change_example::FREIGHT {
        return Some(change_example::change(now));
    }
    let invented = FREIGHT
        .iter()
        .find(|invented| invented.project == project && invented.name == freight)?;
    let reaches: Vec<(&str, Reach)> = STAGES
        .iter()
        .map(|&stage| {
            let reach = match (current(project, stage) == Some(freight), project, stage) {
                (false, ..) => Reach::NotReached,
                (true, CART, "dev") => Reach::Linked(Confidence::Claimed),
                (true, CART, "stage") => Reach::Linked(Confidence::Unknown),
                (true, CHECKOUT, "prod-ams") => Reach::Refused("Pods".into()),
                (true, ..) => Reach::Linked(Confidence::Confirmed),
            };
            (stage, reach)
        })
        .collect();
    Some(short(invented, &reaches, now))
}

fn mono(label: &str, value: &str) -> Field {
    Field {
        label: label.into(),
        value: Shown::Mono(value.into()),
    }
}

fn object(cluster: &str, group: &str, kind: &str, plural: &str, ns: &str, name: &str) -> Object {
    Object {
        cluster: cluster.into(),
        group: group.into(),
        version: if group == "apps" { "v1" } else { "v1alpha1" }.into(),
        kind: kind.into(),
        plural: plural.into(),
        namespace: ns.into(),
        name: name.into(),
    }
}

/// One linked hop.
struct Linked<'a> {
    key: String,
    group: usize,
    state: HealthIndicator,
    kind: &'a str,
    name: String,
    detail: &'a str,
    from: &'a str,
    confidence: Confidence,
    by: &'a str,
    before: String,
    before_says: &'a str,
    here_says: &'a str,
    why: &'a str,
    action: Option<Action>,
}

impl Linked<'_> {
    fn hop(self) -> Hop {
        Hop {
            key: self.key,
            group: self.group,
            state: self.state,
            name: self.name.clone(),
            detail: self.detail.into(),
            from: self.from.into(),
            at: None,
            link: Some(self.confidence),
            shows: Shows::Hop(Box::new(HopDetail {
                kind: self.kind.into(),
                title: self.name,
                state: self.detail.into(),
                notice: None,
                fields: vec![mono("Read from", self.from)],
                link: Some(LinkDetail {
                    confidence: self.confidence,
                    by: self.by.into(),
                    before: self.before,
                    before_says: self.before_says.into(),
                    here_says: self.here_says.into(),
                    why: self.why.into(),
                }),
                unlinked: None,
                actions: self.action.into_iter().collect(),
            })),
        }
    }
}

/// A Stage's hops past its record, as far as its reach says.
fn deployed(project: &str, freight: &str, stage: &str, group: usize, reach: &Reach) -> Vec<Hop> {
    let cluster = CLUSTERS
        .iter()
        .find(|(name, _)| *name == stage)
        .map_or(stage, |(_, cluster)| cluster);
    let app = format!("{project}-{stage}");
    let api = format!("{project}-api");
    let before = format!("Freight {freight}");
    let deployment = Action::new(Target::Resource {
        what: "the Deployment".into(),
        object: object(cluster, "apps", "Deployment", "deployments", project, &api),
    });
    let application = |confidence, detail, why| {
        Linked {
            key: format!("{stage}-argocd"),
            group,
            state: if confidence == Confidence::Unknown {
                Unknown
            } else {
                Healthy
            },
            kind: "Argo CD Application",
            name: app.clone(),
            detail,
            from: CORE,
            confidence,
            by: "revision",
            before: "Promotion".into(),
            before_says: "git-push wrote the env branch",
            here_says: if confidence == Confidence::Unknown {
                "nothing was read"
            } else {
                "Argo CD synced the revision"
            },
            why,
            action: None,
        }
        .hop()
    };
    let workload = |confidence, detail, why| {
        Linked {
            key: format!("{stage}-deployment"),
            group,
            state: if confidence == Confidence::Confirmed {
                Healthy
            } else {
                Info
            },
            kind: "Deployment",
            name: format!("Deployment {api}"),
            detail,
            from: cluster,
            confidence,
            by: if confidence == Confidence::Confirmed {
                "digest"
            } else {
                "name"
            },
            before: before.clone(),
            before_says: "the Freight's image digest",
            here_says: if confidence == Confidence::Confirmed {
                "its spec pins it"
            } else {
                "not read"
            },
            why,
            action: Some(deployment.clone()),
        }
        .hop()
    };
    let pods = |confidence, detail, here_says, why| {
        Linked {
            key: format!("{stage}-pods"),
            group,
            state: if confidence == Confidence::Confirmed {
                Healthy
            } else {
                Unknown
            },
            kind: "Pods",
            name: "Pods".into(),
            detail,
            from: cluster,
            confidence,
            by: "digest",
            before: before.clone(),
            before_says: "the Freight's image digest",
            here_says,
            why,
            action: Some(deployment.clone()),
        }
        .hop()
    };
    let synced = "The revision matches on both sides.";
    let pinned = "Managed by the Application, and its pod template pins the Freight's digest.";
    match reach {
        Reach::NotReached => Vec::new(),
        Reach::Linked(Confidence::Confirmed) => vec![
            application(Confidence::Confirmed, "Synced · Healthy", synced),
            workload(Confidence::Confirmed, "Pins the digest", pinned),
            pods(
                Confidence::Confirmed,
                "3 of 3 run the digest",
                "every pod's imageID reports it",
                "What runs is read from the pods, and the digest matches.",
            ),
        ],
        Reach::Linked(Confidence::Claimed) => vec![
            application(Confidence::Confirmed, "Synced · Healthy", synced),
            workload(
                Confidence::Claimed,
                "Argo CD reports it on dev-fra; not read there",
                "Only Argo CD's status says so: dev-fra isn't read, so nothing there confirms it.",
            ),
        ],
        Reach::Linked(Confidence::Unknown) => vec![application(
            Confidence::Unknown,
            "No Application found",
            "No Argo CD Application in argocd names this Stage, so where it deploys isn't known.",
        )],
        Reach::Refused(kinds) => vec![
            application(Confidence::Confirmed, "Synced · Healthy", synced),
            workload(Confidence::Confirmed, "Pins the digest", pinned),
            pods(
                Confidence::Unknown,
                "Refused: can't read pods",
                "can't be read",
                if kinds == "Pods" {
                    "Reading pods there was refused, so what runs can't be confirmed. A refused \
                     list isn't an empty one."
                } else {
                    "A source on the way was refused, so what runs can't be confirmed."
                },
            ),
        ],
    }
}

/// A shorter change than the change page's own: its commit, its Freight,
/// and each Stage's gates and hops as far as its reach says.
fn short(invented: &Invented, reaches: &[(&str, Reach)], now: DateTime<Utc>) -> Change {
    let project = invented.project;
    let freight = invented.name;
    let title = invented.title();
    let mut groups = vec![
        Group {
            phase: Phase::Build,
            detail: format!("{GIT} · cicd-fra"),
        },
        Group {
            phase: Phase::Freight,
            detail: format!("Kargo on {CORE}"),
        },
    ];
    let mut hops = vec![
        Linked {
            key: "commit".into(),
            group: 0,
            state: Healthy,
            kind: "Commit",
            name: format!("Commit {}", invented.commit),
            detail: &title,
            from: GIT,
            confidence: Confidence::Confirmed,
            by: "commit",
            before: "Pull request".into(),
            before_says: invented.commit,
            here_says: invented.commit,
            why: "The build ran on this commit.",
            action: None,
        }
        .hop(),
        Linked {
            key: "freight".into(),
            group: 1,
            state: Healthy,
            kind: "Kargo Freight",
            name: format!("Freight {freight}"),
            detail: "From the Warehouse",
            from: CORE,
            confidence: Confidence::Confirmed,
            by: "commit",
            before: "Commit".into(),
            before_says: invented.commit,
            here_says: invented.commit,
            why: "The Freight names the build's commit.",
            action: None,
        }
        .hop(),
    ];
    let mut stages = Vec::new();
    for (ix, (name, reach)) in reaches.iter().enumerate() {
        let group = groups.len();
        let cluster = CLUSTERS
            .iter()
            .find(|(stage, _)| stage == name)
            .map_or(*name, |(_, cluster)| cluster);
        groups.push(Group {
            phase: Phase::Stage(ix),
            detail: format!("deploys to {cluster}"),
        });
        let runs = *reach != Reach::NotReached;
        let (state, words, promotion) = match runs {
            true => (
                Healthy,
                "Verified",
                Promotion::Automatic {
                    at: Some(now - Duration::minutes(invented.minutes / 2)),
                },
            ),
            false => (Pending, "Runs other Freight", Promotion::NotYet),
        };
        for (gate, detail) in [
            (
                "Eligible",
                if runs {
                    "From the Warehouse"
                } else {
                    "Not yet"
                },
            ),
            ("Promotion", if runs { "Automatic" } else { "Not promoted" }),
            ("Verification", if runs { "Passed" } else { "Not run" }),
        ] {
            hops.push(Hop {
                key: format!("{name}-{}", gate.to_lowercase()),
                group,
                state,
                name: gate.into(),
                detail: detail.into(),
                from: CORE.into(),
                at: None,
                link: None,
                shows: Shows::Stage(ix),
            });
        }
        hops.extend(deployed(project, freight, name, group, reach));
        stages.push(Stage {
            name: (*name).into(),
            cluster: Destination::Cluster(cluster.into()),
            state,
            words: words.into(),
            running: (!runs)
                .then(|| current(project, name).map(str::to_owned))
                .flatten(),
            eligible: if runs {
                Eligible::Warehouse { at: None }
            } else {
                Eligible::NotYet {
                    upstream: Vec::new(),
                }
            },
            promotion,
            approvers: None,
            promoters: None,
            steps: Vec::<Check>::new(),
            verification: Vec::new(),
            link: None,
            reach: reach.clone(),
            object: object(CORE, GROUP, "Stage", "stages", project, name),
        });
    }
    Change {
        project: project.into(),
        freight: freight.into(),
        kargo_cluster: CORE.into(),
        observed_at: now,
        page: Err("Example data has no Kargo page for this Freight".into()),
        origin: Origin {
            pull_request: invented.pull_request,
            commit: Some(invented.commit.into()),
            title: Some(invented.title()),
        },
        groups,
        stages,
        hops,
    }
}
