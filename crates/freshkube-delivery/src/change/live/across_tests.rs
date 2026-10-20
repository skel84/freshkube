//! The change read across the workspace: each part from its entry, and
//! what an entry that couldn't be read says, in its name.
use serde_json::json;

use super::super::{Change, Destination, Gap, Reach};
use super::tests::{on_deployment_at, stage_of};
use super::{BUDGET, Mapping, Place, read_in};
use crate::argocd::IN_CLUSTER_SERVER;
use crate::entry::pool::{Open, Pool};
use crate::fixtures::*;
use crate::join::Confidence;
use crate::tests::observed_at;
use freshkube_core::indicators::HealthIndicator;
use freshkube_core::resources::{Failure, FailureKind};
use freshkube_core::workspace::{Destination as Mapped, Entry, Key, Role, Sources, Workspace};

const ENV: &str = "https://env-c.example:6443";

/// Argo CD on `core-a`, Kargo and the builds on `cicd-b`, and the one
/// destination, `env-c`, mapped by its server.
fn workspace() -> Workspace {
    let mut workspace = Workspace::new(vec![
        Entry::new("core-a", Role::Core, "core-a-context"),
        Entry::new("cicd-b", Role::Cicd, "cicd-b-context"),
        Entry::new("env-c", Role::Environment, "env-c-context"),
    ]);
    workspace.destinations = vec![Mapped::new(Key::Server(ENV.into()), "env-c")];
    workspace
}

/// [`healthy`], its Application deploying to `env-c`, then `edit`.
fn world(edit: fn(&mut World)) -> World {
    let mut world = healthy();
    world.argocd =
        std::mem::take(&mut world.argocd).with("applications", vec![application(Some(ENV))]);
    world.kargo =
        std::mem::take(&mut world.kargo).with("stages", vec![stage_of("dev", &[], &["f-new"])]);
    edit(&mut world);
    world
}

/// What entry `id` holds of `world`.
fn entry_reader(id: &str, world: World) -> FixtureReader {
    let none = FixtureReader::default;
    match id {
        "cicd-b" => World {
            kargo: world.kargo,
            tekton: world.tekton,
            argocd: none(),
            environment: none(),
        },
        "core-a" => World {
            argocd: world.argocd,
            kargo: none(),
            tekton: none(),
            environment: none(),
        },
        _ => World {
            environment: world.environment,
            kargo: none(),
            argocd: none(),
            tekton: none(),
        },
    }
    .one_cluster()
}

fn place(workspace: &Workspace) -> Place {
    Place {
        cluster: "open-key".into(),
        label: "home".into(),
        project: "storefront".into(),
        freight: "f-new".into(),
        argocd_namespace: "argocd".into(),
        mapping: Mapping::of(workspace, None),
        sources: Sources::of(workspace),
        public_key_file: None,
        evidence_result: None,
        build_namespace: None,
    }
}

/// The change read across `workspace` with no cluster open, `edit` made to
/// the world and the entries `unreachable` refusing to connect.
async fn read_across(
    workspace: &Workspace,
    edit: fn(&mut World),
    unreachable: &[&str],
) -> Result<Change, Failure> {
    let place = place(workspace);
    let pool = Pool::new(
        &place.sources,
        None::<Open<'_, FixtureReader>>,
        |entry: Entry| {
            let result = match unreachable.contains(&entry.id.as_str()) {
                true => Err(Failure::new(FailureKind::Unreachable, "connection refused")),
                false => Ok(entry_reader(&entry.id, world(edit))),
            };
            async move { result }
        },
        BUDGET,
    );
    read_in(&pool, &place, observed_at()).await
}

/// What `change` may be missing, each line with what kept it out.
fn missing(change: &Change) -> Vec<(Gap, &str)> {
    change
        .missing
        .iter()
        .map(|line| (line.gap, line.text.as_str()))
        .collect()
}

fn hop<'a>(change: &'a Change, name: &str) -> &'a super::super::Hop {
    change
        .hops
        .iter()
        .find(|hop| hop.name == name)
        .unwrap_or_else(|| panic!("{name} in {:#?}", change.hops))
}

#[tokio::test]
async fn each_part_is_read_from_its_entry() {
    let change = read_across(&workspace(), |_| {}, &[]).await.expect("read");
    assert_eq!(change.kargo_cluster, "cicd-b");
    assert_eq!(change.groups[0].detail, "Tekton on cicd-b");
    assert_eq!(change.groups[1].detail, "Kargo on cicd-b");
    assert_eq!(hop(&change, "Freight wonky-otter").from, "cicd-b");
    assert_eq!(hop(&change, "Run storefront-push-x").from, "cicd-b");
    let dev = &change.stages[0];
    assert_eq!(
        dev.object.cluster, "cicd-b",
        "the Stage opens on Kargo's entry"
    );
    // The destination is read on its own entry, so its links are the join's.
    assert_eq!(dev.cluster, Destination::Cluster("env-c".into()));
    assert_eq!(change.groups[2].detail, "deploys to env-c");
    let application = change
        .hops
        .iter()
        .find(|hop| hop.key == "dev-argocd-0")
        .expect("the Application");
    assert_eq!(application.from, "core-a");
    let pods = hop(&change, "Pods");
    assert_eq!(pods.from, "env-c");
    assert_eq!(pods.link, Some(Confidence::Confirmed));
    assert_eq!(pods.state, HealthIndicator::Healthy);
    // The weakest link is the join's own: this world's Application names
    // its Stage by annotation alone.
    assert_eq!(application.link, Some(Confidence::Claimed));
    assert_eq!(dev.reach, Reach::Linked(Confidence::Claimed));
    assert!(change.missing.is_empty(), "{:?}", change.missing);
}

#[tokio::test]
async fn kargos_entry_unreachable_is_said_in_its_name() {
    let why = read_across(&workspace(), |_| {}, &["cicd-b"])
        .await
        .expect_err("no Freight");
    assert_eq!(
        why.message,
        "Freight f-new of storefront couldn't be read: Couldn't reach cicd-b: connection \
         refused"
    );
}

#[tokio::test]
async fn no_kargo_entry_says_the_workspace_has_none() {
    let mut workspace = workspace();
    workspace
        .clusters
        .retain(|entry| entry.role == Role::Environment);
    let why = read_across(&workspace, |_| {}, &[])
        .await
        .expect_err("no Freight");
    let Some(why) = why
        .message
        .strip_prefix("Freight f-new of storefront couldn't be read: ")
    else {
        panic!("{why}");
    };
    assert!(
        why.contains("workspace"),
        "says the workspace has none: {why}"
    );
}

#[tokio::test]
async fn builds_unreachable_keep_the_rest_and_say_so() {
    let mut workspace = workspace();
    workspace
        .clusters
        .push(Entry::new("ci-d", Role::Cicd, "ci-d-context"));
    let mut place = place(&workspace);
    // Builds on an entry of their own, which doesn't answer.
    place.sources = sources_with_builds(&workspace, "ci-d");
    let pool = Pool::new(
        &place.sources,
        None::<Open<'_, FixtureReader>>,
        |entry: Entry| {
            let result = match entry.id.as_str() {
                "ci-d" => Err(Failure::new(FailureKind::Unreachable, "connection refused")),
                id => Ok(entry_reader(id, world(|_| {}))),
            };
            async move { result }
        },
        BUDGET,
    );
    let change = read_in(&pool, &place, observed_at()).await.expect("read");
    assert_eq!(
        missing(&change),
        [(Gap::Unreadable, "Couldn't reach ci-d: connection refused")]
    );
    assert_eq!(change.groups[0].detail, "Tekton on ci-d");
    assert_eq!(hop(&change, "Pods").link, Some(Confidence::Confirmed));
}

/// `workspace`'s sources with the builds pinned to `entry`.
fn sources_with_builds(workspace: &Workspace, entry: &str) -> Sources {
    let mut workspace = workspace.clone();
    workspace.sources = serde_json::from_value(json!({"builds": entry})).expect("sources");
    Sources::of(&workspace)
}

#[tokio::test]
async fn argo_cd_refused_on_its_entry_says_so() {
    let change = read_across(
        &workspace(),
        |world| {
            world.argocd = std::mem::take(&mut world.argocd).refusing("applications");
            world.kargo = std::mem::take(&mut world.kargo).with(
                "stages",
                vec![
                    stage_of("dev", &[], &["f-new"]),
                    stage_of("east", &["dev"], &["f-new"]),
                    stage_of("west", &["dev"], &["f-new"]),
                    stage_of("later", &["dev"], &["f-old"]),
                ],
            );
        },
        &[],
    )
    .await
    .expect("read");
    let missing = missing(&change);
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert_eq!(missing[0].0, Gap::Refused);
    assert!(
        missing[0]
            .1
            .starts_with("Argo CD Applications were refused on core-a: "),
        "{missing:?}"
    );
    assert!(matches!(change.stages[0].reach, Reach::Refused(_)));
    for stage in ["dev", "east", "west"] {
        let app = change
            .hops
            .iter()
            .find(|hop| hop.key == format!("{stage}-argocd-0"))
            .unwrap();
        assert_unread(app, &format!("{stage}-eligible"), missing[0].1);
        assert_eq!(app.name, "Application not read");
        let super::super::Shows::Hop(detail) = &app.shows else {
            unreachable!()
        };
        assert_eq!(detail.title, "Application not read");
    }
    let later = change
        .stages
        .iter()
        .position(|stage| stage.name == "later")
        .unwrap();
    let group = change
        .groups
        .iter()
        .position(|group| group.phase == super::super::Phase::Stage(later))
        .unwrap();
    assert_eq!(
        change.hops_of(group).count(),
        3,
        "the unlooked-up Stage has only gates"
    );
}

fn assert_unread(hop: &super::super::Hop, after: &str, reason: &str) {
    assert_eq!(hop.state, HealthIndicator::Unknown);
    assert_eq!(hop.link, Some(Confidence::Unknown));
    assert_eq!(hop.after.as_deref(), Some(after));
    assert_eq!(hop.at, None);
    assert!(hop.detail.contains(reason), "{}", hop.detail);
    let super::super::Shows::Hop(detail) = &hop.shows else {
        panic!("placeholder");
    };
    let link = detail.link.as_ref().unwrap();
    assert_eq!(link.confidence, Confidence::Unknown);
    assert_eq!(link.key, None);
    assert!(link.why.contains(reason));
    assert!(detail.actions.is_empty());
}

#[tokio::test]
async fn a_destination_unreachable_reports_what_argo_cd_says() {
    let change = read_across(&workspace(), |_| {}, &["env-c"])
        .await
        .expect("read");
    assert_eq!(
        missing(&change),
        [(Gap::Unreadable, "Couldn't reach env-c: connection refused")]
    );
    let dev = &change.stages[0];
    assert!(
        matches!(&dev.cluster, Destination::Entry { entry, .. } if entry == "env-c"),
        "{:?}",
        dev.cluster
    );
    // Its entry was asked and couldn't be read: a side is unread.
    assert_eq!(dev.reach, Reach::Linked(Confidence::Unknown));
    let reported = change
        .hops
        .iter()
        .find(|hop| hop.key.starts_with("dev-reported-"))
        .expect("what Argo CD reports");
    let super::super::Shows::Hop(detail) = &reported.shows else {
        panic!("{reported:?}");
    };
    let pods = change
        .hops
        .iter()
        .find(|hop| hop.after.as_deref() == Some(reported.key.as_str()))
        .unwrap();
    assert_unread(
        pods,
        &reported.key,
        "Couldn't reach env-c: connection refused",
    );
    assert_eq!(pods.name, "Pods");
    assert_eq!(pods.from, "env-c");
    assert_eq!(reported.after.as_deref(), Some("dev-argocd-0"));
    // What Argo CD reports stays its claim, saying why it isn't more.
    assert_eq!(reported.link, Some(Confidence::Claimed));
    assert_eq!(
        detail.link.as_ref().expect("its link").why,
        "Only Argo CD's status says so: Couldn't reach env-c: connection refused, so nothing \
         there confirms it."
    );
    let notice = detail.notice.as_ref().expect("a notice");
    assert_eq!(
        notice.body,
        "Couldn't reach env-c: connection refused, so this is what Argo CD reports, not what \
         was read there."
    );
}

/// A destination that hangs ends with the read's budget: the rest shows,
/// and it says it didn't finish.
#[tokio::test(start_paused = true)]
async fn a_hanging_destination_ends_with_the_budget() {
    let change = read_across(
        &workspace(),
        |world| world.environment = std::mem::take(&mut world.environment).hanging("rollouts"),
        &[],
    )
    .await
    .expect("read");
    // It started at once, so its own deadline cut it short.
    assert_eq!(
        missing(&change),
        [(Gap::Unreadable, "env-c didn't finish reading within 30 s")]
    );
    assert_eq!(hop(&change, "Freight wonky-otter").from, "cicd-b");
}

/// A destination started late has only what the budget leaves: the read's
/// end, not its own deadline, cuts it short, and it says so.
#[tokio::test(start_paused = true)]
async fn a_destination_started_late_ends_with_the_reads_budget() {
    let change = read_across(
        &workspace(),
        |world| {
            world.argocd = std::mem::take(&mut world.argocd).slow(LATE);
            world.environment = std::mem::take(&mut world.environment).hanging("rollouts");
        },
        &[],
    )
    .await
    .expect("read");
    let late = format!(
        "env-c didn't finish reading within the read's {} s",
        BUDGET.as_secs()
    );
    assert_eq!(missing(&change), [(Gap::Unreadable, late.as_str())]);
}

/// How long Argo CD's slow entry takes each listing: the three discovery
/// lists and the Applications list end past 15 s and within 45 s, so a
/// destination's own 30 s would pass the read's end.
const LATE: std::time::Duration = std::time::Duration::from_secs(6);

/// The first round's reads end with the budget too: an entry that answers
/// but never finishes says so in its name, and the rest shows.
#[tokio::test(start_paused = true)]
async fn argo_cds_entry_hanging_ends_with_the_budget() {
    let change = read_across(
        &workspace(),
        |world| world.argocd = std::mem::take(&mut world.argocd).hanging("applications"),
        &[],
    )
    .await
    .expect("read");
    let unfinished = format!(
        "Argo CD Applications couldn't be read on core-a: not finished within the read's {} s",
        BUDGET.as_secs()
    );
    assert_eq!(missing(&change), [(Gap::Unreadable, unfinished.as_str())]);
    assert_eq!(hop(&change, "Freight wonky-otter").from, "cicd-b");
}

#[tokio::test(start_paused = true)]
async fn a_freight_that_never_answers_ends_with_the_budget() {
    let why = read_across(
        &workspace(),
        |world| world.kargo = std::mem::take(&mut world.kargo).hanging("freights"),
        &[],
    )
    .await
    .expect_err("no Freight");
    assert_eq!(why.kind, FailureKind::Timeout);
    assert_eq!(
        why.message,
        format!(
            "Freight f-new of storefront couldn't be read: cicd-b didn't finish reading within \
             the read's {} s",
            BUDGET.as_secs()
        )
    );
}

#[tokio::test]
async fn an_unreadable_freight_names_kargos_entry() {
    let why = read_across(
        &workspace(),
        |world| {
            world.kargo = std::mem::take(&mut world.kargo)
                .failing("freights", Failure::new(FailureKind::Other, "bad gateway"))
        },
        &[],
    )
    .await
    .expect_err("no Freight");
    assert!(
        why.message
            .starts_with("Freight f-new of storefront couldn't be read on cicd-b: "),
        "{why}"
    );
}

#[tokio::test]
async fn kargo_not_served_on_its_entry_says_so_in_its_name() {
    let why = read_across(
        &workspace(),
        |world| world.kargo = FixtureReader::default(),
        &[],
    )
    .await
    .expect_err("no Freight");
    assert_eq!(why.kind, FailureKind::NotFound);
    assert_eq!(
        why.message,
        "Freight f-new of storefront couldn't be read: it wasn't found on cicd-b"
    );
}

/// With a part's entry open, it is read through the open connection, never
/// connected to again, and Argo CD's own cluster is read there.
#[tokio::test]
async fn the_open_entry_is_read_through_the_open_connection() {
    let workspace = workspace();
    let mut place = place(&workspace);
    place.mapping = Mapping::of(&workspace, Some("core-a"));
    let edit = |world: &mut World| {
        world.argocd = std::mem::take(&mut world.argocd)
            .with("applications", vec![application(Some(IN_CLUSTER_SERVER))]);
    };
    let open = {
        let world = world(edit);
        World {
            argocd: world.argocd,
            environment: world.environment,
            kargo: FixtureReader::default(),
            tekton: FixtureReader::default(),
        }
        .one_cluster()
    };
    let connected = std::cell::RefCell::new(Vec::new());
    let pool = Pool::new(
        &place.sources,
        Some(Open {
            entry: Some("core-a"),
            reader: Ok(&open),
        }),
        |entry: Entry| {
            connected.borrow_mut().push(entry.id.clone());
            let reader = entry_reader(&entry.id, world(edit));
            async move { Ok::<_, Failure>(reader) }
        },
        BUDGET,
    );
    let change = read_in(&pool, &place, observed_at()).await.expect("read");
    assert_eq!(*connected.borrow(), ["cicd-b"]);
    let dev = &change.stages[0];
    assert_eq!(dev.cluster, Destination::Cluster("home".into()));
    let pods = hop(&change, "Pods");
    assert_eq!(pods.from, "home");
    assert_eq!(pods.link, Some(Confidence::Confirmed));
    assert!(change.missing.is_empty(), "{:?}", change.missing);
}

/// With a part's entry open and its connection unread, the page names it
/// by its context, as it names the open cluster everywhere else.
#[tokio::test]
async fn the_open_entry_unreached_is_named_by_its_context() {
    let workspace = workspace();
    let mut place = place(&workspace);
    place.mapping = Mapping::of(&workspace, Some("core-a"));
    let pool = Pool::new(
        &place.sources,
        Some(Open::<'_, FixtureReader> {
            entry: Some("core-a"),
            reader: Err(Failure::new(
                FailureKind::Unreachable,
                "no answer within 5 s",
            )),
        }),
        |entry: Entry| {
            let reader = entry_reader(&entry.id, world(|_| {}));
            async move { Ok::<_, Failure>(reader) }
        },
        BUDGET,
    );
    let change = read_in(&pool, &place, observed_at()).await.expect("read");
    assert_eq!(
        missing(&change),
        [(Gap::Unreadable, "Couldn't reach home: no answer within 5 s")]
    );
}

/// The Changes list reads Kargo on its entry and each Stage's destination
/// on its own, so a digest that reaches the pods there is Confirmed.
#[tokio::test]
async fn the_changes_list_confirms_a_destination_on_its_own_entry() {
    use crate::changes::{self, live as list};
    let workspace = workspace();
    let at = list::Where {
        cluster: "open-key".into(),
        label: "home".into(),
        argocd_namespace: "argocd".into(),
        mapping: Mapping::of(&workspace, None),
        sources: Sources::of(&workspace),
        public_key_file: None,
        evidence_result: None,
        build_namespace: None,
    };
    let world = || {
        let mut world = on_deployment_at(ENV, |_| {});
        world.kargo = std::mem::take(&mut world.kargo)
            .serves(
                "kargo.akuity.io",
                "v1alpha1",
                &["projects", "freights", "stages", "warehouses", "promotions"],
            )
            .with(
                "projects",
                vec![json!({"metadata": {"name": "storefront"}})],
            );
        world
    };
    let pool = Pool::new(
        &at.sources,
        None::<Open<'_, FixtureReader>>,
        |entry: Entry| {
            let reader = entry_reader(&entry.id, world());
            async move { Ok::<_, Failure>(reader) }
        },
        list::BUDGET,
    );
    let read = list::read_in(&pool, &at, observed_at(), list::BUDGET).await;
    assert_eq!(read.cluster, "cicd-b");
    let listed = changes::derive(&read);
    assert!(listed.notes.is_empty(), "{:?}", listed.notes);
    let row = listed
        .rows
        .iter()
        .find(|row| row.freight == "f-new")
        .expect("f-new's row");
    let stages: Vec<(&str, &Reach)> = row
        .stages
        .iter()
        .map(|stage| (stage.name.as_str(), &stage.reach))
        .collect();
    assert_eq!(stages, [("dev", &Reach::Linked(Confidence::Confirmed))]);
}

/// A destination with nothing to read for the change is read, empty, so
/// it says what wasn't there, never that it isn't the open cluster.
#[tokio::test]
async fn a_destination_with_nothing_to_read_says_what_wasnt_there() {
    let change = read_across(
        &workspace(),
        |world| {
            let mut app = application(Some(ENV));
            app["spec"]["destination"]
                .as_object_mut()
                .expect("a destination")
                .remove("namespace");
            app["status"]["resources"] = json!([]);
            world.argocd = std::mem::take(&mut world.argocd).with("applications", vec![app]);
        },
        &[],
    )
    .await
    .expect("read");
    assert_eq!(
        change.stages[0].cluster,
        Destination::Cluster("env-c".into())
    );
    let said = format!("{:?}", change.hops);
    assert!(!said.contains("isn't the open cluster"), "{said}");
    assert!(!said.contains("nothing was read there"), "{said}");
}

/// A destination where Argo CD reports only a workload the read doesn't
/// look for, a StatefulSet, is never asked: it stays unread, and its Stage
/// is Claimed with Argo CD's report as its row.
#[tokio::test]
async fn a_destination_reporting_only_a_statefulset_is_claimed_unread() {
    let change = read_across(
        &workspace(),
        |world| {
            let mut app = application(Some(ENV));
            app["spec"]["destination"]
                .as_object_mut()
                .expect("a destination")
                .remove("namespace");
            app["status"]["resources"] = json!([
                {"group": "apps", "kind": "StatefulSet", "namespace": "shop", "name": "db"}
            ]);
            world.argocd = std::mem::take(&mut world.argocd).with("applications", vec![app]);
        },
        &[],
    )
    .await
    .expect("read");
    let stage = &change.stages[0];
    assert!(
        matches!(&stage.cluster, Destination::Entry { entry, .. } if entry == "env-c"),
        "{:?}",
        stage.cluster
    );
    assert_eq!(stage.reach, Reach::Linked(Confidence::Claimed));
    let reported = change
        .hops
        .iter()
        .find(|hop| hop.name == "StatefulSet db")
        .expect("Argo CD's StatefulSet");
    assert_eq!(reported.link, Some(Confidence::Claimed));
    assert!(change.missing.is_empty(), "{:?}", change.missing);
}

#[tokio::test]
async fn each_reported_workload_keeps_its_own_unread_pods() {
    let change = read_across(
        &workspace(),
        |world| {
            let mut app = application(Some(ENV));
            app["status"]["resources"] = json!([
                {"group":"argoproj.io", "kind":"Rollout", "namespace":"shop", "name":"api"},
                {"group":"apps", "kind":"Deployment", "namespace":"shop", "name":"api"},
                {"group":"apps", "kind":"StatefulSet", "namespace":"shop", "name":"store"},
                {"group":"apps", "kind":"DaemonSet", "namespace":"shop", "name":"agent"},
                {"kind":"Service", "namespace":"shop", "name":"api"}
            ]);
            world.argocd = std::mem::take(&mut world.argocd).with("applications", vec![app]);
        },
        &["env-c"],
    )
    .await
    .unwrap();
    let workloads: Vec<_> = change
        .hops
        .iter()
        .filter(|hop| hop.after.as_deref() == Some("dev-argocd-0"))
        .collect();
    assert_eq!(workloads.len(), 4, "only the reported workload controllers");
    for workload in workloads {
        assert_eq!(workload.link, Some(Confidence::Claimed));
        let pods: Vec<_> = change
            .hops
            .iter()
            .filter(|hop| hop.after.as_deref() == Some(workload.key.as_str()))
            .collect();
        assert_eq!(pods.len(), 1);
        assert_unread(
            pods[0],
            &workload.key,
            "Couldn't reach env-c: connection refused",
        );
        assert_eq!(pods[0].detail.matches("env-c").count(), 1);
        let super::super::Shows::Hop(detail) = &pods[0].shows else {
            panic!("Pods detail");
        };
        assert!(detail.notice.as_ref().unwrap().body.ends_with('.'));
        assert!(detail.link.as_ref().unwrap().why.ends_with('.'));
    }
    let mut keys: Vec<_> = change.hops.iter().map(|hop| &hop.key).collect();
    keys.sort();
    let count = keys.len();
    keys.dedup();
    assert_eq!(keys.len(), count);
}

#[tokio::test]
async fn an_unreachable_destination_with_no_reported_workload_adds_no_pods_placeholder() {
    let change = read_across(
        &workspace(),
        |world| {
            let mut app = application(Some(ENV));
            app["status"]["resources"] = json!([]);
            world.argocd = std::mem::take(&mut world.argocd).with("applications", vec![app]);
        },
        &["env-c"],
    )
    .await
    .unwrap();
    assert!(!change.hops.iter().any(|hop| hop.key.contains("-reported-")));
    assert!(
        !change.hops.iter().any(|hop| matches!(&hop.shows,
            super::super::Shows::Hop(detail) if detail.kind == "Pods"
        )),
        "no Pods hop belongs anywhere in this trail"
    );
}

#[tokio::test]
async fn reported_missing_workloads_have_no_unread_pods() {
    let change = read_across(&workspace(), |world| {
        let mut app = application(Some(ENV));
        app["status"]["resources"] = json!([
            {"group":"apps", "kind":"Deployment", "namespace":"shop", "name":"absent", "health":{"status":"Missing"}},
            {"group":"argoproj.io", "kind":"Rollout", "namespace":"shop", "name":"absent", "health":{"status":"Missing"}},
            {"group":"apps", "kind":"StatefulSet", "namespace":"shop", "name":"absent", "health":{"status":"Missing"}},
            {"group":"apps", "kind":"DaemonSet", "namespace":"shop", "name":"absent", "health":{"status":"Missing"}},
            {"group":"apps", "kind":"Deployment", "namespace":"shop", "name":"present", "health":{"status":"Healthy"}}
        ]);
        world.argocd = std::mem::take(&mut world.argocd).with("applications", vec![app]);
    }, &["env-c"]).await.unwrap();
    let reported: Vec<_> = change
        .hops
        .iter()
        .filter(|hop| hop.after.as_deref() == Some("dev-argocd-0"))
        .collect();
    assert_eq!(
        reported.len(),
        5,
        "all four Missing workload reports remain beside the existing one"
    );
    for workload in reported {
        let present = workload.name == "Deployment present";
        assert_eq!(workload.link, Some(Confidence::Claimed));
        let pods: Vec<_> = change
            .hops
            .iter()
            .filter(|hop| hop.after.as_deref() == Some(workload.key.as_str()))
            .collect();
        assert_eq!(pods.len(), usize::from(present), "{}", workload.name);
    }
    assert_eq!(
        change
            .hops
            .iter()
            .filter(|hop| matches!(&hop.shows,
                super::super::Shows::Hop(detail) if detail.kind == "Pods"
            ))
            .count(),
        1,
        "only the existing Deployment has unread Pods"
    );
}
