//! The invented Waiting to ship example mode shows: acme's `checkout`, which
//! fans out to two production regions and is furthest behind in one of
//! them, `catalog` with a single production Stage, two applications that
//! are caught up, and two that weren't read. Every state of the page shows:
//! a verification that failed, one that passed and waits for a promotion, a
//! Freight no Stage has run, a pull request GitHub doesn't name, and a wait
//! Kargo doesn't date. Every name is invented, and every address is under
//! `example.test`.
//!
//! Its Freight and Stages go through Kargo's real parsers. Times count back
//! from one reference instant the caller captures once.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};

use super::{Project, Pull, PullRead, Read};
use crate::applications::example::CICD;
use crate::change::example::{GIT, Shape};
use crate::github::PullState;
use crate::kargo::{parse_freight, parse_stage};
use crate::source::Source;

pub const CHECKOUT: &str = "checkout";
pub const CATALOG: &str = "catalog";
pub const CART: &str = "cart";
pub const SEARCH: &str = "search";
pub const PAYMENTS: &str = "payments";
pub const BILLING: &str = "billing";

/// One invented Freight: its name, age in minutes, commit, pull request and
/// title, and where it is verified.
struct Invented {
    name: &'static str,
    minutes: i64,
    commit: &'static str,
    pull: Option<(u64, &'static str, PullState)>,
    title: &'static str,
    verified: &'static [&'static str],
}

const fn freight(
    name: &'static str,
    minutes: i64,
    commit: &'static str,
    pull: Option<(u64, &'static str, PullState)>,
    title: &'static str,
    verified: &'static [&'static str],
) -> Invented {
    Invented {
        name,
        minutes,
        commit,
        pull,
        title,
        verified,
    }
}

const DAY: i64 = 24 * 60;

const CHECKOUT_FREIGHT: [Invented; 5] = [
    freight(
        "tidy-badger",
        6,
        "c7d1e20a9b",
        Some((421, "mira", PullState::Open)),
        "Show the delivery estimate at checkout",
        &[],
    ),
    freight(
        "wonky-otter",
        52,
        "a1f3c9e5d2",
        Some((418, "ken", PullState::Merged)),
        "Retry idempotency keys",
        &["dev"],
    ),
    freight(
        "merry-owl",
        26 * 60,
        "3be09d1c47",
        Some((415, "mira", PullState::Merged)),
        "Cache the shipping quote for a minute",
        &["dev"],
    ),
    freight(
        "quiet-heron",
        3 * DAY,
        "e93b6f4a10",
        Some((402, "ken", PullState::Merged)),
        "Log the payment provider's reply",
        &["dev", "stage"],
    ),
    freight(
        "calm-wren",
        9 * DAY,
        "5d20b1a7ee",
        Some((409, "ken", PullState::Merged)),
        "Round totals in the shop-api currency",
        &["dev", "stage", "prod-ams", "prod-lon"],
    ),
];

const CATALOG_FREIGHT: [Invented; 2] = [
    freight(
        "plucky-newt",
        2 * DAY,
        "0c4e8b1f36",
        None,
        "Index the new product fields",
        &["dev"],
    ),
    freight(
        "gentle-vole",
        12 * DAY,
        "7a92d0c5b8",
        Some((88, "ken", PullState::Merged)),
        "Trim the product feed",
        &["dev", "prod"],
    ),
];

const CART_FREIGHT: [Invented; 1] = [freight(
    "lucky-finch",
    2 * 60,
    "0b8c3d7f12",
    Some((77, "mira", PullState::Merged)),
    "Keep the basket when signing in",
    &["dev", "stage", "prod"],
)];

const SEARCH_FREIGHT: [Invented; 1] = [freight(
    "sleepy-mole",
    2 * DAY,
    "f41a9c2e60",
    None,
    "Bump the session store client",
    &["dev", "prod"],
)];

/// One Stage: its upstream (none takes Freight straight from the Warehouse),
/// and the Freight it ran, newest first, each with the verification it was
/// given: its phase and how many minutes ago it started.
struct StageSpec {
    name: &'static str,
    upstream: Option<&'static str>,
    history: &'static [(&'static str, Option<(&'static str, i64)>)],
}

const fn stage(
    name: &'static str,
    upstream: Option<&'static str>,
    history: &'static [(&'static str, Option<(&'static str, i64)>)],
) -> StageSpec {
    StageSpec {
        name,
        upstream,
        history,
    }
}

const CHECKOUT_STAGES: [StageSpec; 4] = [
    stage(
        "dev",
        None,
        &[
            ("wonky-otter", Some(("Successful", 40))),
            ("merry-owl", Some(("Successful", 25 * 60 + 30))),
        ],
    ),
    stage(
        "stage",
        Some("dev"),
        &[
            ("merry-owl", Some(("Failed", 25 * 60))),
            ("quiet-heron", Some(("Successful", 3 * DAY - 70))),
        ],
    ),
    stage(
        "prod-ams",
        Some("stage"),
        &[("calm-wren", Some(("Successful", 8 * DAY)))],
    ),
    stage(
        "prod-lon",
        Some("stage"),
        &[
            ("quiet-heron", Some(("Successful", 2 * DAY))),
            ("calm-wren", Some(("Successful", 8 * DAY))),
        ],
    ),
];

const CATALOG_STAGES: [StageSpec; 2] = [
    stage(
        "dev",
        None,
        &[
            ("plucky-newt", Some(("Successful", 2 * DAY - 20))),
            ("gentle-vole", Some(("Successful", 12 * DAY))),
        ],
    ),
    stage(
        "prod",
        Some("dev"),
        &[("gentle-vole", Some(("Successful", 11 * DAY)))],
    ),
];

const CART_STAGES: [StageSpec; 3] = [
    stage("dev", None, &[("lucky-finch", Some(("Successful", 100)))]),
    stage(
        "stage",
        Some("dev"),
        &[("lucky-finch", Some(("Successful", 80)))],
    ),
    stage(
        "prod",
        Some("stage"),
        &[("lucky-finch", Some(("Successful", 60)))],
    ),
];

const SEARCH_STAGES: [StageSpec; 2] = [
    stage(
        "dev",
        None,
        &[("sleepy-mole", Some(("Successful", 2 * DAY - 10)))],
    ),
    stage("prod", Some("dev"), &[("sleepy-mole", None)]),
];

fn freight_json(project: &str, now: DateTime<Utc>, invented: &Invented) -> Value {
    let verified: serde_json::Map<String, Value> = invented
        .verified
        .iter()
        .map(|stage| ((*stage).to_owned(), json!({})))
        .collect();
    json!({
        "apiVersion": "kargo.akuity.io/v1alpha1",
        "kind": "Freight",
        "metadata": {
            "namespace": project,
            "name": invented.name,
            "creationTimestamp": (now - Duration::minutes(invented.minutes)).to_rfc3339(),
        },
        "alias": invented.name,
        "origin": {"kind": "Warehouse", "name": format!("{project}-images")},
        "commits": [{
            "repoURL": format!("https://{GIT}/acme/{project}"),
            "id": invented.commit,
            "message": format!("{}\n\nInvented for the example.", invented.title),
        }],
        "status": {"verifiedIn": verified},
    })
}

fn stage_json(project: &str, now: DateTime<Utc>, spec: &StageSpec) -> Value {
    let sources = match spec.upstream {
        Some(upstream) => json!({"stages": [upstream]}),
        None => json!({"direct": true}),
    };
    let history: Vec<Value> = spec
        .history
        .iter()
        .map(|(freight, verification)| {
            let verifications: Vec<Value> = verification
                .iter()
                .map(|(phase, minutes)| {
                    let started = now - Duration::minutes(*minutes);
                    json!({
                        "phase": phase,
                        "startTime": started.to_rfc3339(),
                        "finishTime": (started + Duration::minutes(2)).to_rfc3339(),
                    })
                })
                .collect();
            json!({
                "items": {"Warehouse/images": {"name": freight}},
                "verificationHistory": verifications,
            })
        })
        .collect();
    json!({
        "apiVersion": "kargo.akuity.io/v1alpha1",
        "kind": "Stage",
        "metadata": {"namespace": project, "name": spec.name},
        "spec": {"requestedFreight": [{
            "origin": {"kind": "Warehouse", "name": format!("{project}-images")},
            "sources": sources,
        }]},
        "status": {"freightHistory": history},
    })
}

/// The pull request GitHub names for an example Freight, under `shape`.
fn pull_read(invented: &Invented, now: DateTime<Utc>, shape: Shape, pulls_exist: bool) -> PullRead {
    use crate::github::example::Trouble;
    let Some((number, author, state)) = invented.pull.filter(|_| pulls_exist) else {
        return PullRead::NoPull;
    };
    let trouble = shape.github();
    if trouble == Some(Trouble::Refused) {
        return PullRead::Unread(format!(
            "Pull requests couldn't be read: {}",
            Trouble::Refused.words()
        ));
    }
    PullRead::Found(Pull {
        number,
        title: Some(invented.title.to_owned()),
        author: Some(author.to_owned()),
        state,
        merged_at: (state == PullState::Merged)
            .then(|| now - Duration::minutes(invented.minutes.saturating_sub(5))),
        head: None,
        merge: Some(invented.commit.to_owned()),
        stale: trouble.map(|trouble| format!("{}; showing what was read earlier", trouble.words())),
    })
}

fn project(
    now: DateTime<Utc>,
    shape: Shape,
    name: &str,
    freight: &[Invented],
    stages: &[StageSpec],
) -> Project {
    let pulls: BTreeMap<String, PullRead> = freight
        .iter()
        .map(|invented| {
            (
                invented.name.to_owned(),
                pull_read(invented, now, shape, true),
            )
        })
        .collect();
    Project {
        name: name.to_owned(),
        freight: Source::Read(
            freight
                .iter()
                .filter_map(|invented| parse_freight(&freight_json(name, now, invented)))
                .collect(),
        ),
        stages: Source::Read(
            stages
                .iter()
                .filter_map(|spec| parse_stage(&stage_json(name, now, spec)))
                .collect(),
        ),
        pulls,
    }
}

/// What the page reads in example mode.
#[cfg(any(test, feature = "testing"))]
pub fn read(now: DateTime<Utc>) -> Read {
    read_shaped(now, Shape::Whole)
}

/// The example as `shape` reads it: nothing when Kargo's entry couldn't be
/// reached.
pub fn read_shaped(now: DateTime<Utc>, shape: Shape) -> Read {
    if let Some(why) = shape.unreached() {
        return Read {
            cluster: CICD.to_owned(),
            projects: Source::Unreadable(why.clone()),
            unreached: Some(why),
            pulls_read: true,
            observed_at: now,
        };
    }
    let mut projects = vec![
        project(now, shape, CHECKOUT, &CHECKOUT_FREIGHT, &CHECKOUT_STAGES),
        project(now, shape, CATALOG, &CATALOG_FREIGHT, &CATALOG_STAGES),
        project(now, shape, CART, &CART_FREIGHT, &CART_STAGES),
        project(now, shape, SEARCH, &SEARCH_FREIGHT, &SEARCH_STAGES),
    ];
    // `catalog`'s newest Freight has no pull request on GitHub.
    // `payments` was refused its Freight, `billing` its Stages.
    projects.push(Project {
        name: PAYMENTS.to_owned(),
        freight: Source::Refused(
            "freights.kargo.akuity.io is forbidden for this identity".to_owned(),
        ),
        stages: Source::Read(Vec::new()),
        pulls: BTreeMap::new(),
    });
    projects.push(Project {
        name: BILLING.to_owned(),
        freight: Source::Read(Vec::new()),
        stages: Source::Unreadable("no answer within the read's 90 s".to_owned()),
        pulls: BTreeMap::new(),
    });
    Read {
        cluster: CICD.to_owned(),
        projects: Source::Read(projects),
        unreached: None,
        pulls_read: true,
        observed_at: now,
    }
}
