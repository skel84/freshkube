//! The change read across the workspace: each part from its entry, and
//! what an entry that couldn't be read says, in its name.
use serde_json::json;

use super::super::{Change, Destination, Reach};
use super::tests::stage_of;
use super::{BUDGET, Mapping, Place, read_in};
use crate::delivery::entry::pool::{Open, Pool};
use crate::delivery::fixtures::*;
use crate::delivery::join::Confidence;
use crate::delivery::tests::observed_at;
use crate::indicators::HealthIndicator;
use crate::workspace::{Destination as Mapped, Entry, Key, Role, Sources, Workspace};

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
    }
}

/// The change read across `workspace` with no cluster open, `edit` made to
/// the world and the entries `unreachable` refusing to connect.
async fn read_across(
    workspace: &Workspace,
    edit: fn(&mut World),
    unreachable: &[&str],
) -> Result<Change, String> {
    let place = place(workspace);
    let pool = Pool::new(
        &place.sources,
        None::<Open<'_, FixtureReader>>,
        |entry: Entry| {
            let result = match unreachable.contains(&entry.id.as_str()) {
                true => Err("connection refused".to_owned()),
                false => Ok(entry_reader(&entry.id, world(edit))),
            };
            async move { result }
        },
        BUDGET,
    );
    read_in(&pool, &place, observed_at()).await
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
        why,
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
    let Some(why) = why.strip_prefix("Freight f-new of storefront couldn't be read: ") else {
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
                "ci-d" => Err("connection refused".to_owned()),
                id => Ok(entry_reader(id, world(|_| {}))),
            };
            async move { result }
        },
        BUDGET,
    );
    let change = read_in(&pool, &place, observed_at()).await.expect("read");
    assert_eq!(change.missing, ["Couldn't reach ci-d: connection refused"]);
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
        |world| world.argocd = std::mem::take(&mut world.argocd).refusing("applications"),
        &[],
    )
    .await
    .expect("read");
    assert_eq!(change.missing.len(), 1, "{:?}", change.missing);
    assert!(
        change.missing[0].starts_with("Argo CD Applications were refused on core-a: "),
        "{:?}",
        change.missing
    );
    assert!(matches!(change.stages[0].reach, Reach::Refused(_)));
}

#[tokio::test]
async fn a_destination_unreachable_reports_what_argo_cd_says() {
    let change = read_across(&workspace(), |_| {}, &["env-c"])
        .await
        .expect("read");
    assert_eq!(change.missing, ["Couldn't reach env-c: connection refused"]);
    let dev = &change.stages[0];
    assert!(
        matches!(&dev.cluster, Destination::Entry { entry, .. } if entry == "env-c"),
        "{:?}",
        dev.cluster
    );
    assert_eq!(dev.reach, Reach::Linked(Confidence::Claimed));
    let reported = change
        .hops
        .iter()
        .find(|hop| hop.key.starts_with("dev-reported-"))
        .expect("what Argo CD reports");
    let super::super::Shows::Hop(detail) = &reported.shows else {
        panic!("{reported:?}");
    };
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
    assert_eq!(change.missing.len(), 1, "{:?}", change.missing);
    assert!(
        change.missing[0].starts_with("env-c didn't finish reading within"),
        "{:?}",
        change.missing
    );
    assert_eq!(hop(&change, "Freight wonky-otter").from, "cicd-b");
}
