use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde_json::json;

use super::*;
use crate::change::Phase;
use crate::fixtures::FixtureReader;
use crate::kargo::{GROUP, parse_freight, parse_stage};
use crate::source::Truncation;
use freshkube_core::resources::{Failure, FailureKind};

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_760_000_000, 0).unwrap()
}

fn example() -> Changes {
    derive(&example::read(now()))
}

fn row<'a>(changes: &'a Changes, freight: &str) -> &'a Row {
    changes
        .rows
        .iter()
        .find(|row| row.freight == freight)
        .unwrap_or_else(|| panic!("{freight} is listed"))
}

const CONFIRMED: Reach = Reach::Linked(Confidence::Confirmed);
const CLAIMED: Reach = Reach::Linked(Confidence::Claimed);
const UNKNOWN: Reach = Reach::Linked(Confidence::Unknown);

/// A row's Stages' names, in its order.
fn names(row: &Row) -> Vec<&str> {
    row.stages.iter().map(|stage| stage.name.as_str()).collect()
}

/// A row's Stages' reaches, in its order.
fn reaches(row: &Row) -> Vec<Reach> {
    row.stages.iter().map(|stage| stage.reach.clone()).collect()
}

#[test]
fn a_row_names_its_projects_stages_in_promotion_order() {
    let changes = example();
    assert_eq!(
        names(row(&changes, "wonky-otter")),
        ["dev", "stage", "prod-ams", "prod-lon"]
    );
}

#[test]
fn every_state_shows_in_the_example() {
    let changes = example();
    let cells = |freight| reaches(row(&changes, freight));
    use Reach::NotReached as Not;
    assert_eq!(cells("wonky-otter"), [CONFIRMED, CONFIRMED, Not, CONFIRMED]);
    assert_eq!(
        cells("brave-lynx"),
        [Not, Not, Reach::Refused("Pods".into()), Not]
    );
    assert_eq!(cells("lucky-finch"), [CLAIMED, UNKNOWN, Not, Not]);
    assert_eq!(cells("sleepy-mole"), [Not, Not, CONFIRMED, CONFIRMED]);
    assert_eq!(cells("tidy-badger"), [Not, Not, Not, Not]);
}

#[test]
fn a_rows_state_is_its_weakest_reached_stage() {
    let changes = example();
    assert_eq!(row(&changes, "wonky-otter").state, CONFIRMED);
    assert_eq!(row(&changes, "lucky-finch").state, UNKNOWN);
    assert_eq!(
        row(&changes, "brave-lynx").state,
        Reach::Refused("Pods".into())
    );
    assert_eq!(row(&changes, "quiet-heron").state, Reach::NotReached);
}

#[test]
fn rows_come_by_group_then_newest_first() {
    let changes = example();
    let order: Vec<(Fold, &str)> = changes
        .rows
        .iter()
        .map(|row| (row.fold, row.freight.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            (Fold::NotConfirmed, "lucky-finch"),
            (Fold::NotConfirmed, "brave-lynx"),
            (Fold::Confirmed, "wonky-otter"),
            (Fold::Confirmed, "sleepy-mole"),
            (Fold::NotRunning, "tidy-badger"),
            (Fold::NotRunning, "quiet-heron"),
        ]
    );
    assert!(changes.needs_a_look());
    assert!(changes.notes.is_empty());
}

#[test]
fn a_change_is_its_pull_request_else_its_commit() {
    let changes = example();
    let wonky = row(&changes, "wonky-otter");
    assert_eq!(wonky.named, Named::PullRequest(418));
    assert_eq!(wonky.title.as_deref(), Some("Retry idempotency keys"));
    assert_eq!(wonky.project, "checkout");
    let mole = row(&changes, "sleepy-mole");
    assert_eq!(mole.named, Named::Commit("f41a9c2".into()));
    assert_eq!(mole.title.as_deref(), Some("Bump the session store client"));
    // Not followed: the commit and title from the Freight alone.
    let badger = row(&changes, "tidy-badger");
    assert_eq!(badger.named, Named::Commit("c7d1e20".into()));
    assert_eq!(
        badger.title.as_deref(),
        Some("Show the delivery estimate at checkout")
    );
    assert!(badger.trail.is_err());
    assert_eq!(
        badger.created,
        Some(now() - Duration::minutes(6)),
        "its age"
    );
}

/// The example's own reaches say what its hops say: a Stage's weakest link
/// is its cell, and a refused one leaves an Unknown link.
#[test]
fn the_examples_reaches_agree_with_their_trails() {
    for row in example().rows {
        let Ok(change) = &row.trail else { continue };
        for (ix, stage) in change.stages.iter().enumerate() {
            let group = change
                .groups
                .iter()
                .position(|group| group.phase == Phase::Stage(ix))
                .unwrap();
            let links: Vec<Confidence> = change.hops_of(group).filter_map(|hop| hop.link).collect();
            let weakest = [
                Confidence::Unknown,
                Confidence::Claimed,
                Confidence::Confirmed,
            ]
            .into_iter()
            .find(|confidence| links.contains(confidence));
            let expected = match &stage.reach {
                Reach::NotReached => continue,
                Reach::Linked(confidence) => Some(*confidence),
                Reach::Refused(_) => Some(Confidence::Unknown),
            };
            assert_eq!(weakest, expected, "{} on {}", row.freight, stage.name);
        }
    }
}

fn freight(project: &str, name: &str, minutes: i64) -> Freight {
    parse_freight(&json!({
        "metadata": {
            "namespace": project,
            "name": name,
            "creationTimestamp": (now() - Duration::minutes(minutes)).to_rfc3339(),
        },
        "commits": [{"repoURL": "https://git.example.test/acme/shop-api", "id": format!("c0{minutes}")}],
    }))
    .unwrap()
}

fn stage(project: &str, name: &str, upstream: &[&str], runs: Option<&str>) -> Stage {
    let sources = if upstream.is_empty() {
        json!({"direct": true})
    } else {
        json!({"stages": upstream})
    };
    let history = runs.map_or(
        json!([]),
        |freight| json!([{"items": {"Warehouse/images": {"name": freight}}}]),
    );
    parse_stage(&json!({
        "metadata": {"namespace": project, "name": name},
        "spec": {"requestedFreight": [{"sources": sources}]},
        "status": {"freightHistory": history},
    }))
    .unwrap()
}

fn project(name: &str, freight: Vec<Freight>, stages: Vec<Stage>) -> Project {
    Project {
        name: name.into(),
        freight: Source::Read(freight),
        stages: Source::Read(stages),
        followed: BTreeMap::new(),
    }
}

fn read(projects: Vec<Project>) -> Read {
    Read {
        cluster: "core-fra".into(),
        projects: Source::Read(projects),
        late: 0,
        unreached: None,
    }
}

#[test]
fn a_project_lists_its_newest_freight_and_counts_the_rest() {
    let mut all: Vec<Freight> = (0..MAX_FREIGHT as i64 + 5)
        .map(|n| freight("shop-api", &format!("f{n:03}"), n + 1))
        .collect();
    // The oldest, which a Stage runs, is listed past the cap.
    all.push(freight("shop-api", "old-but-running", 10_000));
    let stages = vec![stage("shop-api", "int", &[], Some("old-but-running"))];
    let changes = derive(&read(vec![project("shop-api", all, stages)]));
    assert_eq!(changes.rows.len(), MAX_FREIGHT + 1);
    assert!(
        changes
            .rows
            .iter()
            .any(|row| row.freight == "old-but-running")
    );
    assert!(!changes.rows.iter().any(|row| row.freight == "f054"));
    assert_eq!(
        changes.notes,
        [Note {
            gap: Gap::Capped,
            project: Some("shop-api".into()),
            text: "shop-api lists its 50 newest Freight; 5 older aren't listed".into(),
        }]
    );
}

#[test]
fn a_capped_listing_says_so() {
    let mut shop = project("shop-api", vec![freight("shop-api", "a", 1)], Vec::new());
    shop.freight = Source::Capped(vec![freight("shop-api", "a", 1)], Truncation { read: 1 });
    let changes = derive(&Read {
        cluster: "core-fra".into(),
        projects: Source::Capped(vec![shop], Truncation { read: 1 }),
        late: 0,
        unreached: None,
    });
    assert_eq!(changes.rows.len(), 1);
    let gaps: Vec<Gap> = changes.notes.iter().map(|note| note.gap).collect();
    assert_eq!(gaps, [Gap::Capped, Gap::Capped]);
}

#[test]
fn refused_kargo_is_a_note_never_an_empty_list() {
    let changes = derive(&Read {
        cluster: "core-fra".into(),
        projects: Source::Refused("projects is forbidden".into()),
        late: 0,
        unreached: None,
    });
    assert!(changes.rows.is_empty());
    assert_eq!(changes.notes.len(), 1);
    assert_eq!(changes.notes[0].gap, Gap::Refused);
    assert!(
        changes.notes[0]
            .text
            .starts_with("Kargo projects were refused on core-fra"),
        "{}",
        changes.notes[0].text
    );
    let missing = derive(&Read {
        cluster: "core-fra".into(),
        projects: Source::NotInstalled("404".into()),
        late: 0,
        unreached: None,
    });
    assert_eq!(missing.notes[0].gap, Gap::NotServed);
    assert_eq!(
        missing.notes[0].text,
        "Kargo projects aren't served on core-fra"
    );
}

#[test]
fn a_refused_freight_list_keeps_the_other_projects() {
    let mut cart = project("cart", Vec::new(), Vec::new());
    cart.freight = Source::Refused("freights is forbidden".into());
    let shop = project(
        "shop-api",
        vec![freight("shop-api", "a", 1)],
        vec![stage("shop-api", "int", &[], None)],
    );
    let changes = derive(&read(vec![cart, shop]));
    assert_eq!(changes.rows.len(), 1);
    assert_eq!(changes.notes.len(), 1);
    assert_eq!(changes.notes[0].gap, Gap::Refused);
    assert_eq!(changes.notes[0].project.as_deref(), Some("cart"));
}

#[test]
fn freight_whose_stages_werent_read_is_unknown_not_unreached() {
    let mut shop = project("shop-api", vec![freight("shop-api", "a", 1)], Vec::new());
    shop.stages = Source::Unreadable("timed out".into());
    let changes = derive(&read(vec![shop]));
    let row = &changes.rows[0];
    assert_eq!(row.state, UNKNOWN);
    assert_eq!(row.fold, Fold::NotConfirmed);
    assert_eq!(changes.notes[0].gap, Gap::Unreadable);
    assert_eq!(
        changes.notes[0].text,
        "shop-api's Stages couldn't be read on core-fra: timed out"
    );
}

#[test]
fn a_change_that_couldnt_be_read_is_unknown_where_it_runs() {
    let mut shop = project(
        "shop-api",
        vec![freight("shop-api", "a", 1)],
        vec![
            stage("shop-api", "int", &[], Some("a")),
            stage("shop-api", "prod", &["int"], None),
        ],
    );
    shop.followed.insert(
        "a".into(),
        Err(Failure::new(
            FailureKind::Other,
            "Freight a couldn't be read",
        )),
    );
    let changes = derive(&read(vec![shop]));
    let row = &changes.rows[0];
    assert_eq!(reaches(row), [UNKNOWN, Reach::NotReached]);
    assert_eq!(row.fold, Fold::NotConfirmed);
    assert!(row.trail.is_err());
}

#[test]
fn each_project_keeps_its_own_stages() {
    let shop = project(
        "shop-api",
        vec![freight("shop-api", "a", 1)],
        vec![
            stage("shop-api", "prod", &["stg"], None),
            stage("shop-api", "int", &[], None),
            stage("shop-api", "stg", &["int"], None),
        ],
    );
    let cart = project(
        "cart",
        vec![freight("cart", "b", 2)],
        vec![
            stage("cart", "int", &[], None),
            stage("cart", "qa", &["int"], None),
        ],
    );
    let changes = derive(&read(vec![shop, cart]));
    assert_eq!(names(row(&changes, "a")), ["int", "stg", "prod"]);
    assert_eq!(names(row(&changes, "b")), ["int", "qa"]);
    assert!(changes.notes.is_empty());
}

#[test]
fn a_capped_stage_list_says_a_stage_may_be_missing() {
    let mut shop = project(
        "shop-api",
        vec![freight("shop-api", "a", 1)],
        vec![stage("shop-api", "int", &[], Some("a"))],
    );
    shop.stages = match shop.stages {
        Source::Read(stages) => Source::Capped(stages, Truncation { read: 1000 }),
        other => other,
    };
    let changes = derive(&read(vec![shop]));
    assert_eq!(changes.notes.len(), 1, "{:?}", changes.notes);
    assert_eq!(changes.notes[0].gap, Gap::Capped);
    assert!(changes.notes[0].text.contains("Stage list stopped"));
    assert_eq!(names(&changes.rows[0]), ["int"]);
}

#[tokio::test]
async fn a_live_read_says_why_kargo_wasnt_read() {
    let at = live::Where {
        cluster: "example:core".into(),
        label: "core-fra".into(),
        argocd_namespace: "argocd".into(),
        mapping: Default::default(),
        sources: Default::default(),
        public_key_file: None,
        transparency_log: false,
        transparency_url: None,
        evidence_result: None,
        build_namespace: None,
        stage_naming: None,
    };
    let unserved = live::read(&FixtureReader::default(), &at, now(), live::BUDGET).await;
    assert!(matches!(unserved.projects, Source::NotInstalled(_)));
    let refused = FixtureReader::default()
        .serves(GROUP, "v1alpha1", &["projects"])
        .refusing("projects");
    let read = live::read(&refused, &at, now(), live::BUDGET).await;
    assert!(matches!(read.projects, Source::Refused(_)));
    assert_eq!(derive(&read).notes[0].gap, Gap::Refused);
}

#[tokio::test]
async fn a_live_read_lists_freight_and_follows_only_what_runs() {
    let at = live::Where {
        cluster: "example:core".into(),
        label: "core-fra".into(),
        argocd_namespace: "argocd".into(),
        mapping: Default::default(),
        sources: Default::default(),
        public_key_file: None,
        transparency_log: false,
        transparency_url: None,
        evidence_result: None,
        build_namespace: None,
        stage_naming: None,
    };
    let freight = |name: &str| {
        json!({
            "metadata": {"namespace": "shop-api", "name": name},
            "commits": [{"repoURL": "https://git.example.test/acme/shop-api", "id": "abc1234"}],
        })
    };
    let reader = FixtureReader::default()
        .serves(GROUP, "v1alpha1", &["projects", "freights", "stages"])
        .with("projects", vec![json!({"metadata": {"name": "shop-api"}})])
        .with("freights", vec![freight("running"), freight("idle")])
        .with(
            "stages",
            vec![json!({
                "metadata": {"namespace": "shop-api", "name": "int"},
                "status": {"currentFreight": {"name": "running"}},
            })],
        );
    let read = live::read(&reader, &at, now(), live::BUDGET).await;
    let Source::Read(projects) = &read.projects else {
        panic!("read: {:?}", read.projects)
    };
    let followed: Vec<&String> = projects[0].followed.keys().collect();
    assert_eq!(followed, ["running"]);
    let changes = derive(&read);
    assert_eq!(changes.rows.len(), 2);
    let idle = changes
        .rows
        .iter()
        .find(|row| row.freight == "idle")
        .unwrap();
    assert_eq!(idle.fold, Fold::NotRunning);
}

/// Two projects whose Stages each run a Freight, in one cluster with Argo
/// CD. `hang` names a project whose Freight never answers.
fn two_projects(hang: Option<&str>) -> FixtureReader {
    let freight = |project: &str, name: &str| {
        json!({
            "metadata": {"namespace": project, "name": name},
            "commits": [{"repoURL": "https://git.example.test/acme/shop", "id": "abc1234"}],
        })
    };
    let stage = |project: &str, name: &str, runs: &str| {
        json!({
            "metadata": {"namespace": project, "name": name},
            "status": {"currentFreight": {"name": runs}},
        })
    };
    let reader = FixtureReader::default()
        .serves(
            GROUP,
            "v1alpha1",
            &["projects", "freights", "stages", "warehouses", "promotions"],
        )
        .serves("argoproj.io", "v1alpha1", &["applications"])
        .with(
            "projects",
            vec![
                json!({"metadata": {"name": "cart"}}),
                json!({"metadata": {"name": "shop"}}),
            ],
        )
        .with(
            "freights",
            vec![
                freight("cart", "c1"),
                freight("cart", "c2"),
                freight("shop", "s1"),
            ],
        )
        .with(
            "stages",
            vec![
                stage("cart", "int", "c1"),
                stage("cart", "prod", "c2"),
                stage("shop", "int", "s1"),
            ],
        );
    match hang {
        Some(project) => reader.hanging_list("freights", project),
        None => reader,
    }
}

/// Six projects, each with one Freight its one Stage runs.
fn six_projects() -> FixtureReader {
    let names = ["alder", "birch", "cedar", "elm", "maple", "oak"];
    FixtureReader::default()
        .serves(
            GROUP,
            "v1alpha1",
            &["projects", "freights", "stages", "warehouses", "promotions"],
        )
        .serves("argoproj.io", "v1alpha1", &["applications"])
        .with(
            "projects",
            names.map(|name| json!({"metadata": {"name": name}})).to_vec(),
        )
        .with(
            "freights",
            names
                .map(|name| {
                    json!({
                        "metadata": {"namespace": name, "name": format!("{name}-1")},
                        "commits": [{"repoURL": "https://git.example.test/acme/shop", "id": "abc1234"}],
                    })
                })
                .to_vec(),
        )
        .with(
            "stages",
            names
                .map(|name| {
                    json!({
                        "metadata": {"namespace": name, "name": "int"},
                        "status": {"currentFreight": {"name": format!("{name}-1")}},
                    })
                })
                .to_vec(),
        )
}

fn at() -> live::Where {
    live::Where {
        cluster: "example:core".into(),
        label: "core-fra".into(),
        argocd_namespace: "argocd".into(),
        mapping: Default::default(),
        sources: Default::default(),
        public_key_file: None,
        transparency_log: false,
        transparency_url: None,
        evidence_result: None,
        build_namespace: None,
        stage_naming: None,
    }
}

#[tokio::test]
async fn a_live_read_asks_what_its_changes_share_once() {
    let reader = two_projects(None);
    let read = live::read(&reader, &at(), now(), live::BUDGET).await;
    let Source::Read(projects) = &read.projects else {
        panic!("read: {:?}", read.projects)
    };
    let followed: Vec<usize> = projects.iter().map(|p| p.followed.len()).collect();
    assert_eq!(followed, [2, 1], "every running Freight is followed");
    let requests = reader.requests.borrow();
    let count = |needle: &str| requests.iter().filter(|r| r.contains(needle)).count();
    let discovery: Vec<&String> = requests
        .iter()
        .filter(|r| r.starts_with("GET /apis"))
        .collect();
    let once: std::collections::BTreeSet<&&String> = discovery.iter().collect();
    assert_eq!(
        discovery.len(),
        once.len(),
        "discovery is read once: {discovery:#?}"
    );
    assert_eq!(
        count("argoproj.io/v1alpha1/applications"),
        1,
        "{requests:#?}"
    );
    assert_eq!(count("configmaps"), 2, "argocd-cm and kargo-api, once each");
    for project in ["cart", "shop"] {
        for kind in ["freights", "stages", "warehouses", "promotions"] {
            let listed = requests
                .iter()
                .filter(|r| r.contains(&format!("/{kind} ns=Some(\"{project}\")")))
                .count();
            assert_eq!(listed, 1, "{project}'s {kind}: {requests:#?}");
        }
    }
    assert!(
        !requests
            .iter()
            .any(|r| r.starts_with("GET kargo.akuity.io/freights")),
        "a listed Freight isn't read again by name"
    );
}

#[tokio::test(start_paused = true)]
async fn a_read_past_its_budget_keeps_what_answered() {
    let reader = two_projects(Some("shop"));
    let read = live::read(&reader, &at(), now(), live::BUDGET).await;
    let changes = derive(&read);
    let listed: Vec<&str> = changes.rows.iter().map(|row| row.key.as_str()).collect();
    assert_eq!(listed.len(), 2, "cart's rows are kept: {listed:?}");
    assert!(listed.iter().all(|key| key.starts_with("cart/")));
    let note = changes
        .notes
        .iter()
        .find(|note| note.project.as_deref() == Some("shop"))
        .expect("shop says why it isn't listed");
    assert_eq!(note.gap, Gap::Unreadable);
    assert!(
        note.text.contains("no answer within the read's 90 s"),
        "{}",
        note.text
    );
}

/// One project that never answers holds one turn, never the others: the
/// five after it keep their rows.
#[tokio::test(start_paused = true)]
async fn a_hung_project_doesnt_keep_the_others_waiting() {
    // Slow, so a project left to start after the hung one gives up runs
    // out of time rather than answering at once.
    let reader = six_projects()
        .hanging_list("freights", "alder")
        .slow(std::time::Duration::from_secs(2));
    let read = live::read(&reader, &at(), now(), live::BUDGET).await;
    let changes = derive(&read);
    let listed: BTreeSet<&str> = changes
        .rows
        .iter()
        .map(|row| row.key.split('/').next().unwrap())
        .collect();
    assert_eq!(
        listed,
        BTreeSet::from(["birch", "cedar", "elm", "maple", "oak"]),
        "{:?}",
        changes.notes
    );
    assert!(
        changes
            .notes
            .iter()
            .any(|note| note.project.as_deref() == Some("alder")),
        "alder says why it isn't listed"
    );
}

/// A project's changes are followed as soon as it answers, not after a
/// project that hangs: on a slow server they're followed well within the
/// budget the hung one runs out.
#[tokio::test(start_paused = true)]
async fn changes_are_followed_while_another_project_hangs() {
    let reader = six_projects()
        .hanging_list("freights", "alder")
        .slow(std::time::Duration::from_secs(2));
    let read = live::read(&reader, &at(), now(), live::BUDGET).await;
    let Source::Read(projects) = &read.projects else {
        panic!("read: {:?}", read.projects)
    };
    for project in projects.iter().filter(|p| p.name != "alder") {
        let change = &project.followed[&format!("{}-1", project.name)];
        assert!(change.is_ok(), "{}: {change:?}", project.name);
    }
    assert_eq!(read.late, 0);
    assert!(
        derive(&read)
            .notes
            .iter()
            .all(|note| !note.text.contains("weren't followed"))
    );
}

/// A change the budget cut off says so in the banner, not only in its row.
#[test]
fn changes_not_followed_in_time_are_a_note() {
    let mut read = example::read(now());
    read.late = 2;
    let changes = derive(&read);
    let note = changes
        .notes
        .iter()
        .find(|note| note.project.is_none())
        .expect("a note for the read");
    assert_eq!(note.gap, Gap::Unreadable);
    assert_eq!(
        note.text,
        "2 changes on cicd-fra weren't followed in the time a read may take, so their Stages show \
         Unknown"
    );
}

/// An example build's message has a body, as a live sha-title does; the
/// row and its trail take its first line.
#[test]
fn a_title_with_a_body_shows_its_first_line() {
    let changes = example();
    let finch = row(&changes, "lucky-finch");
    assert_eq!(
        finch.title.as_deref(),
        Some("Keep the basket when signing in")
    );
    let trail = finch.trail.as_ref().expect("followed");
    assert_eq!(
        trail.origin.title.as_deref(),
        Some("Keep the basket when signing in")
    );
    assert!(changes.rows.iter().all(|row| {
        !row.title
            .as_deref()
            .unwrap_or_default()
            .contains(['\n', '\r'])
    }));
}

/// Argo CD on `core-a`, Kargo and the builds on `cicd-b`.
fn across() -> live::Where {
    use freshkube_core::workspace::{Entry, Role, Sources, Workspace};
    let workspace = Workspace::new(vec![
        Entry::new("core-a", Role::Core, "core-a-context"),
        Entry::new("cicd-b", Role::Cicd, "cicd-b-context"),
    ]);
    live::Where {
        sources: Sources::of(&workspace),
        public_key_file: None,
        transparency_log: false,
        transparency_url: None,
        evidence_result: None,
        build_namespace: None,
        ..at()
    }
}

/// The list read across `at`, with `unreachable` refusing to connect.
async fn read_across(at: &live::Where, unreachable: &'static str) -> Read {
    use crate::entry::pool::{Open, Pool};
    use freshkube_core::workspace::Entry;
    let pool = Pool::new(
        &at.sources,
        None::<Open<'_, FixtureReader>>,
        |entry: Entry| async move {
            match entry.id == unreachable {
                true => Err(Failure::new(FailureKind::Unreachable, "connection refused")),
                false => Ok(two_projects(None)),
            }
        },
        live::BUDGET,
    );
    live::read_in(&pool, at, now(), live::BUDGET).await
}

#[tokio::test]
async fn kargo_is_listed_on_its_own_entry() {
    let read = read_across(&across(), "").await;
    assert_eq!(read.cluster, "cicd-b");
    assert_eq!(read.unreached, None);
    let Source::Read(projects) = &read.projects else {
        panic!("read: {:?}", read.projects)
    };
    assert_eq!(projects.len(), 2);
}

#[tokio::test]
async fn kargos_entry_unreached_is_a_note_in_its_name() {
    let read = read_across(&across(), "cicd-b").await;
    assert_eq!(read.cluster, "cicd-b");
    let changes = derive(&read);
    assert!(changes.rows.is_empty());
    assert_eq!(
        changes.notes,
        [Note {
            gap: Gap::Unreadable,
            project: None,
            text: "Couldn't reach cicd-b: connection refused".into(),
        }]
    );
}

#[test]
fn every_example_shape_says_what_it_couldnt_read() {
    use crate::change::example::Shape;
    let notes = |shape| derive(&example::read_shaped(now(), shape)).notes;
    assert!(
        notes(Shape::Whole)
            .iter()
            .all(|note| note.gap != Gap::Refused)
    );
    let unreached = derive(&example::read_shaped(now(), Shape::KargoUnreachable));
    assert!(unreached.rows.is_empty());
    assert_eq!(
        unreached.notes[0].text,
        "Couldn't reach cicd-fra: no answer within 5 s; a credential plugin may be waiting for a sign-in"
    );
    let refused = notes(Shape::ArgoRefused);
    assert!(
        refused
            .iter()
            .any(|note| note.gap == Gap::Refused && note.text.contains("refused on core-fra")),
        "{refused:#?}"
    );
    // A Stage whose Application wasn't read deploys to an unknown cluster.
    let change = example::change_of("checkout", "wonky-otter", now())
        .map(|change| Shape::ArgoRefused.apply(change))
        .expect("checkout's change")
        .expect("read");
    let stages: Vec<&str> = change
        .groups
        .iter()
        .filter(|group| matches!(group.phase, Phase::Stage(_)))
        .map(|group| group.detail.as_str())
        .collect();
    assert!(
        stages
            .iter()
            .all(|detail| detail.starts_with("destination unknown: ")),
        "{stages:?}"
    );
    let destination = notes(Shape::DestinationUnreachable);
    assert!(
        destination.iter().any(|note| note.text
            == "Couldn't reach dev-fra: connection refused by dev-fra.example.test:6443"),
        "{destination:#?}"
    );
}

#[test]
fn the_spread_example_follows_applications_in_later_namespaces() {
    use crate::change::{Target, example::Shape};
    let changes = derive(&example::read_shaped(now(), Shape::Spread));
    let checkout = row(&changes, "wonky-otter");
    assert_eq!(
        reaches(checkout),
        [CONFIRMED, CONFIRMED, Reach::NotReached, CONFIRMED]
    );
    let trail = checkout.trail.as_ref().unwrap();
    for (stage, namespace) in [("dev", "gitops"), ("stage", "platform")] {
        let hop = trail
            .hops
            .iter()
            .find(|hop| hop.key == format!("{stage}-argocd"))
            .unwrap();
        assert!(hop.detail.contains(namespace), "{}", hop.detail);
        let crate::change::Shows::Hop(detail) = &hop.shows else {
            panic!("Application details")
        };
        assert!(detail.actions.iter().any(|action| matches!(&action.target,
            Target::Resource { object, .. } if object.kind == "Application" && object.namespace == namespace
        )));
        assert!(detail.actions.iter().any(|action| matches!(&action.target,
            Target::Browser { address: Some(address), .. }
                if address.as_str().ends_with(&format!("/applications/{namespace}/checkout-{stage}"))
        )));
    }
    assert_eq!(changes.notes.len(), 1, "the shared omission is shown once");
    assert!(changes.notes[0].text.contains("sandbox, tenants"));
    assert!(changes.notes[0].text.contains("namespace cap"));
    assert_eq!(trail.missing[0].text, changes.notes[0].text);
}

#[test]
fn the_list_hands_its_log_settings_to_every_followed_change() {
    let mut at = live::Where {
        cluster: "cluster-b".into(),
        label: "cluster-b".into(),
        argocd_namespace: "argocd".into(),
        mapping: Default::default(),
        sources: Default::default(),
        public_key_file: None,
        transparency_log: false,
        transparency_url: None,
        evidence_result: None,
        build_namespace: None,
        stage_naming: None,
    };
    let off = at.place("checkout", "freight-a");
    assert!(!off.transparency_log);
    assert_eq!(off.transparency_url, None);
    at.transparency_log = true;
    at.transparency_url = Some("https://rekor.example.test/log".into());
    let on = at.place("checkout", "freight-b");
    assert!(on.transparency_log);
    assert_eq!(on.transparency_url, at.transparency_url);
    assert_eq!(on.project, "checkout");
    assert_eq!(on.freight, "freight-b");
}
