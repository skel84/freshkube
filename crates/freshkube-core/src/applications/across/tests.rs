use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::applications::read::read_session;
use crate::applications::{CoverageState, Inputs, Override, SourceKind, derive, example};
use crate::delivery::fixtures::FixtureReader;
use crate::delivery::read::{ListRequest, Listing, Resource, ServedGroup};
use crate::delivery::source::Source;
use crate::resources::Failure;
use crate::workspace::{Destination, Key, Role, Workspace};

fn entry(id: &str, role: Role) -> Entry {
    Entry::new(id, role, format!("{id}-context"))
}

/// Argo CD in cluster-a, Kargo in cluster-b, and each environment an Argo
/// CD destination maps to.
fn workspace(environments: &[&str]) -> Workspace {
    let mut clusters = vec![
        entry("cluster-a", Role::Core),
        entry("cluster-b", Role::Cicd),
    ];
    clusters.extend(environments.iter().map(|id| entry(id, Role::Environment)));
    let mut workspace = Workspace::new(clusters);
    workspace.destinations = environments
        .iter()
        .map(|id| Destination::new(Key::Name((*id).into()), *id))
        .collect();
    workspace
}

fn namespaced(namespace: &str, name: &str, labels: Value) -> Value {
    json!({"metadata": {"namespace": namespace, "name": name, "labels": labels}})
}

/// A cluster that serves and holds every source, so only what is asked of
/// it shows what it was read for.
fn everything() -> FixtureReader {
    FixtureReader::default()
        .serves(
            "kargo.akuity.io",
            "v1alpha1",
            &["projects", "stages", "warehouses"],
        )
        .serves(
            "argoproj.io",
            "v1alpha1",
            &["applications", "applicationsets"],
        )
        .with("projects", vec![json!({"metadata": {"name": "checkout"}})])
        .with("stages", vec![namespaced("checkout", "dev", json!({}))])
        .with(
            "applications",
            vec![json!({"metadata": {"namespace": "argocd", "name": "storefront"}})],
        )
        .with(
            "deployments",
            vec![namespaced(
                "shop",
                "web",
                json!({"app.kubernetes.io/part-of": "storefront"}),
            )],
        )
}

/// A reader the test keeps a handle on, to see what it was asked.
#[derive(Clone)]
struct Shared(Rc<FixtureReader>);

impl Reader for Shared {
    async fn get(
        &self,
        resource: &Resource,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<Value, Failure> {
        self.0.get(resource, namespace, name).await
    }
    async fn list(&self, request: &ListRequest) -> Result<Listing, Failure> {
        self.0.list(request).await
    }
    async fn groups(&self) -> Result<Vec<ServedGroup>, Failure> {
        self.0.groups().await
    }
    async fn plurals(&self, group: &str, version: &str) -> Result<Vec<String>, Failure> {
        self.0.plurals(group, version).await
    }
}

fn coverage(sessions: Vec<SessionInputs>) -> Vec<(String, SourceKind, CoverageState)> {
    let derived = derive(
        &Inputs {
            sessions,
            stage_naming: None,
            destinations: Vec::new(),
        },
        &Override::default(),
    );
    derived
        .coverage
        .into_iter()
        .map(|c| (c.session.0, c.source, c.state))
        .collect()
}

fn state(
    coverage: &[(String, SourceKind, CoverageState)],
    session: &str,
    source: SourceKind,
) -> Option<CoverageState> {
    coverage
        .iter()
        .find(|(s, k, _)| s == session && *k == source)
        .map(|(_, _, state)| state.clone())
}

#[tokio::test]
async fn each_part_is_read_from_its_own_entry_and_nothing_else_there() {
    let sources = Sources::of(&workspace(&["cluster-c"]));
    let readers: RefCell<HashMap<String, Rc<FixtureReader>>> = RefCell::default();
    let connect = |entry: Entry| {
        let reader = Rc::new(everything());
        readers.borrow_mut().insert(entry.id, reader.clone());
        async move { Ok::<_, String>(Shared(reader)) }
    };
    let sessions = read_workspace(&sources, None::<Open<'_, FixtureReader>>, connect).await;
    let keys: Vec<&str> = sessions.iter().map(|s| s.key.0.as_str()).collect();
    assert_eq!(keys, ["cluster-b", "cluster-a", "cluster-c"]);

    let readers = readers.borrow();
    let asked = |id: &str| readers[id].requests.borrow().join("\n");
    let kargo = asked("cluster-b");
    assert!(
        !kargo.contains("argoproj") && !kargo.contains("deployments"),
        "{kargo}"
    );
    let environment = asked("cluster-c");
    assert!(
        !environment.contains("kargo") && !environment.contains("argoproj"),
        "{environment}"
    );
    // Argo CD's entry is its own cluster's destination: its labelled
    // workloads are read with it.
    let argo = asked("cluster-a");
    assert!(!argo.contains("kargo"), "{argo}");
    assert!(argo.contains("argoproj"), "{argo}");
    assert!(
        argo.lines().any(|line| line.contains("deployments")
            && line.ends_with("selector=Some(\"app.kubernetes.io/part-of\")")),
        "{argo}"
    );

    let derived = derive(
        &Inputs {
            sessions: sessions.clone(),
            stage_naming: None,
            destinations: Vec::new(),
        },
        &Override::default(),
    );
    let names: Vec<&str> = derived
        .applications
        .iter()
        .map(|a| a.name.as_str())
        .collect();
    assert_eq!(names, ["checkout", "storefront"]);
    // The workloads in cluster-c and in cluster-a join the Application
    // read in cluster-a.
    let storefront = &derived.applications[1];
    for cluster in ["cluster-a", "cluster-c"] {
        assert!(
            storefront
                .members
                .iter()
                .any(|m| m.at.name == "web" && m.at.session.0 == cluster),
            "{cluster}"
        );
    }
    // No cluster says anything of a source it wasn't asked for.
    let covered = coverage(sessions);
    assert_eq!(
        state(&covered, "cluster-b", SourceKind::ArgoApplications),
        None
    );
    assert_eq!(
        state(&covered, "cluster-c", SourceKind::KargoProjects),
        None
    );
    assert_eq!(
        state(&covered, "cluster-a", SourceKind::Workloads),
        Some(CoverageState::Read)
    );
}

#[tokio::test]
async fn each_source_says_why_it_wasnt_read_in_its_entrys_name() {
    // Kargo's entry can't be reached, Argo CD refuses, one destination is
    // gone from the workspace and another can't be reached.
    let mut workspace = workspace(&["cluster-c", "cluster-d"]);
    workspace
        .destinations
        .push(Destination::new(Key::Name("x".into()), "cluster-x"));
    let sources = Sources::of(&workspace);
    let connect = |entry: Entry| async move {
        match entry.id.as_str() {
            "cluster-b" | "cluster-d" => Err("no answer within 5 s".to_owned()),
            "cluster-a" => Ok(everything().refusing("applications")),
            _ => Ok(everything()),
        }
    };
    let covered =
        coverage(read_workspace(&sources, None::<Open<'_, FixtureReader>>, connect).await);
    let unreachable = Some(CoverageState::Unreachable("no answer within 5 s".into()));
    assert_eq!(
        state(&covered, "cluster-b", SourceKind::KargoProjects),
        unreachable
    );
    assert!(matches!(
        state(&covered, "cluster-a", SourceKind::ArgoApplications),
        Some(CoverageState::Refused(_))
    ));
    assert_eq!(
        state(&covered, "cluster-c", SourceKind::Workloads),
        Some(CoverageState::Read)
    );
    assert_eq!(
        state(&covered, "cluster-d", SourceKind::Workloads),
        unreachable
    );
    assert_eq!(
        state(&covered, "cluster-x", SourceKind::Workloads),
        Some(CoverageState::NotInWorkspace(
            "cluster-x is no longer in the workspace".into()
        ))
    );

    // Without a Core entry, Argo CD isn't in the workspace.
    let sources = Sources::of(&Workspace::new(vec![entry("cluster-b", Role::Cicd)]));
    let covered = coverage(
        read_workspace(&sources, None::<Open<'_, FixtureReader>>, |_| async {
            Ok::<_, String>(everything())
        })
        .await,
    );
    assert_eq!(
        state(
            &covered,
            &missing_key(Part::ArgoCd).0,
            SourceKind::ArgoApplications
        ),
        Some(CoverageState::NotInWorkspace(
            "No Core entry in the workspace".into()
        ))
    );
    assert_eq!(
        state(&covered, "cluster-b", SourceKind::KargoProjects),
        Some(CoverageState::Read)
    );

    // With neither, each part is missing under its own name.
    let sources = Sources::of(&Workspace::new(vec![entry("cluster-c", Role::Environment)]));
    let sessions = read_workspace(&sources, None::<Open<'_, FixtureReader>>, |_| async {
        Ok::<_, String>(everything())
    })
    .await;
    let keys: Vec<&str> = sessions.iter().map(|s| s.key.0.as_str()).collect();
    assert_eq!(keys, ["the workspace (Kargo)", "the workspace (Argo CD)"]);
}

/// A reader that counts itself among the live clients until dropped.
struct Counted {
    reader: FixtureReader,
    live: Rc<Cell<usize>>,
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.live.set(self.live.get() - 1);
    }
}

impl Reader for Counted {
    async fn get(
        &self,
        resource: &Resource,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<Value, Failure> {
        self.reader.get(resource, namespace, name).await
    }
    async fn list(&self, request: &ListRequest) -> Result<Listing, Failure> {
        // Long enough that the reads overlap.
        tokio::time::sleep(Duration::from_millis(10)).await;
        self.reader.list(request).await
    }
    async fn groups(&self) -> Result<Vec<ServedGroup>, Failure> {
        self.reader.groups().await
    }
    async fn plurals(&self, group: &str, version: &str) -> Result<Vec<String>, Failure> {
        self.reader.plurals(group, version).await
    }
}

#[tokio::test(start_paused = true)]
async fn kargo_and_argo_cd_come_first_then_at_most_four_destinations_at_once() {
    let environments = [
        "env-1", "env-2", "env-3", "env-4", "env-5", "env-6", "env-7",
    ];
    let sources = Sources::of(&workspace(&environments));
    let live = Rc::new(Cell::new(0));
    let most = Cell::new(0);
    let order = RefCell::new(Vec::new());
    let connect = |entry: Entry| {
        live.set(live.get() + 1);
        most.set(most.get().max(live.get()));
        order.borrow_mut().push(entry.id);
        let live = live.clone();
        async move {
            Ok::<_, String>(Counted {
                reader: everything(),
                live,
            })
        }
    };
    let sessions = read_workspace(&sources, None::<Open<'_, FixtureReader>>, connect).await;
    assert_eq!(sessions.len(), 9);
    assert_eq!(most.get(), MAX_AT_ONCE, "{:?}", order.borrow());
    // Every client is dropped when its read ends.
    assert_eq!(live.get(), 0);
    let order = order.into_inner();
    assert_eq!(order[..2], ["cluster-b", "cluster-a"]);
    assert_eq!(order[2..], environments);
}

#[tokio::test]
async fn the_workspace_of_one_reads_the_open_connection_as_before() {
    let sources = Sources::of(&Workspace::default());
    let reader = everything();
    let open = Open {
        entry: None,
        key: SessionKey::new("connection-1"),
        reader: Ok(&reader),
    };
    let never = |_: Entry| async { Err::<FixtureReader, _>("not asked".to_owned()) };
    let sessions = read_workspace(&sources, Some(open), never).await;
    let alone = read_session(&everything(), SessionKey::new("connection-1")).await;
    assert_eq!(sessions, vec![alone]);

    // With nothing open, it says so, never none found.
    let covered = coverage(read_workspace(&sources, None::<Open<'_, FixtureReader>>, never).await);
    assert!(
        covered
            .iter()
            .all(|(_, _, state)| *state == CoverageState::Unreachable("no cluster is open".into())),
        "{covered:?}"
    );
}

#[tokio::test]
async fn the_open_entry_is_read_through_the_open_connection() {
    let sources = Sources::of(&workspace(&["cluster-c"]));
    let reader = everything();
    let open = Open {
        entry: Some("cluster-a"),
        key: SessionKey::new("connection-1"),
        reader: Ok(&reader),
    };
    let connected = RefCell::new(Vec::new());
    let connect = |entry: Entry| {
        connected.borrow_mut().push(entry.id);
        async { Ok::<_, String>(everything()) }
    };
    let sessions = read_workspace(&sources, Some(open), connect).await;
    assert_eq!(connected.into_inner(), ["cluster-b", "cluster-c"]);
    // Keyed by its entry, as every other entry is.
    assert_eq!(sessions[1].key, SessionKey::new("cluster-a"));
    assert!(
        reader
            .requests
            .borrow()
            .iter()
            .any(|r| r.contains("argoproj"))
    );
}

#[tokio::test]
async fn an_entry_holding_two_sources_is_read_once_for_all_it_holds() {
    // cluster-a is the workspace's only Core entry: Kargo falls back to it,
    // Argo CD runs there, and its labelled workloads are its own cluster's.
    let mut workspace = Workspace::new(vec![
        entry("cluster-a", Role::Core),
        entry("cluster-c", Role::Environment),
    ]);
    workspace.destinations = vec![Destination::new(
        Key::Server("https://cluster-c.example.test:6443".into()),
        "cluster-c",
    )];
    assert_eq!(workspace.destinations[0].fault(), None, "a valid row");
    let sources = Sources::of(&workspace);
    let connected = RefCell::new(Vec::new());
    let connect = |entry: Entry| {
        connected.borrow_mut().push(entry.id);
        async { Ok::<_, String>(everything()) }
    };
    let sessions = read_workspace(&sources, None::<Open<'_, FixtureReader>>, connect).await;
    assert_eq!(connected.into_inner(), ["cluster-a", "cluster-c"]);
    let a = sessions.iter().find(|s| s.key.0 == "cluster-a").unwrap();
    assert_eq!(a.asked, Asked::ALL);
}

/// A reader serving what `objects` hold, as the example's clusters hold
/// them.
fn reader_of(objects: Vec<example::CoreObject>) -> FixtureReader {
    let mut lists: std::collections::BTreeMap<&str, Vec<Value>> = Default::default();
    for object in objects {
        let plural = object.key.split('.').next().unwrap_or_default();
        lists.entry(plural).or_default().push(object.value);
    }
    let mut reader = FixtureReader::default()
        .serves(
            "kargo.akuity.io",
            "v1alpha1",
            &["projects", "stages", "warehouses"],
        )
        .serves(
            "argoproj.io",
            "v1alpha1",
            &["applications", "applicationsets"],
        );
    for (plural, items) in lists {
        reader = reader.with(plural, items);
    }
    reader
}

/// The example workspace's entries, each holding what its acme cluster
/// holds; cicd-fra holds core-fra's objects, of which only Kargo is asked.
fn example_reader(id: &str) -> FixtureReader {
    match id {
        "core-fra" | "cicd-fra" => reader_of(example::core_objects()),
        cluster => reader_of(example::environment_objects(cluster)),
    }
}

async fn read_example(open: Option<&str>) -> crate::applications::Derived {
    let workspace = crate::workspace::example();
    let sources = Sources::of(&workspace);
    let reader = open.map(example_reader);
    let open = open.zip(reader.as_ref()).map(|(entry, reader)| Open {
        entry: Some(entry),
        key: SessionKey::new("connection-1"),
        reader: Ok(reader),
    });
    let sessions = read_workspace(&sources, open, |entry: Entry| async move {
        Ok::<_, String>(example_reader(&entry.id))
    })
    .await;
    derive(
        &Inputs {
            sessions,
            stage_naming: None,
            destinations: workspace.destinations,
        },
        &Override::default(),
    )
}

#[tokio::test]
async fn the_same_applications_are_read_whichever_entry_is_open() {
    let alone = read_example(None).await;
    for open in ["core-fra", "cicd-fra", "dev-fra", "prod-fra"] {
        assert_eq!(read_example(Some(open)).await, alone, "with {open} open");
    }
    let names: Vec<&str> = alone.applications.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(
        names,
        ["cart", "catalog", "checkout", "loyalty", "status-page"]
    );
    // Argo CD's own cluster's labelled workloads are read with it: the
    // Deployment its local Application lists and names back is Tracked.
    let status = alone
        .applications
        .iter()
        .find(|a| a.name == "status-page")
        .unwrap();
    let deployment = status
        .members
        .iter()
        .find(|m| matches!(m.at.kind, crate::applications::MemberKind::Workload(_)))
        .expect("status-page's Deployment");
    assert_eq!(deployment.at.session, SessionKey::new("core-fra"));
    assert_eq!(deployment.basis, crate::applications::Basis::Tracked);
}

#[tokio::test(start_paused = true)]
async fn one_entry_that_hangs_keeps_no_other_waiting() {
    let environments = ["env-1", "env-2", "env-3", "env-4", "env-5", "env-6"];
    let sources = Sources::of(&workspace(&environments));
    let started = tokio::time::Instant::now();
    let connect = |entry: Entry| async move {
        // The second destination never connects.
        if entry.id == "env-2" {
            std::future::pending::<()>().await;
        }
        Ok::<_, String>(match entry.id.as_str() {
            // The first connects, then never answers its listing.
            "env-1" => everything().hanging("deployments"),
            _ => everything(),
        })
    };
    let sessions = read_workspace(&sources, None::<Open<'_, FixtureReader>>, connect).await;
    assert_eq!(
        started.elapsed(),
        ENTRY_DEADLINE,
        "ends with the ones that hung"
    );
    let covered = coverage(sessions);
    let seconds = ENTRY_DEADLINE.as_secs();
    assert_eq!(
        state(&covered, "env-1", SourceKind::Workloads),
        Some(CoverageState::Unreadable(format!(
            "didn't finish reading within {seconds} s"
        ))),
        "it answered, so it was reached"
    );
    assert_eq!(
        state(&covered, "env-2", SourceKind::Workloads),
        Some(CoverageState::Unreachable(format!(
            "no answer within {seconds} s"
        )))
    );
    for id in &environments[2..] {
        assert_eq!(
            state(&covered, id, SourceKind::Workloads),
            Some(CoverageState::Read),
            "{id}"
        );
    }
    assert_eq!(
        state(&covered, "cluster-b", SourceKind::KargoProjects),
        Some(CoverageState::Read)
    );
}

#[tokio::test(start_paused = true)]
async fn the_read_keeps_what_answered_within_its_budget() {
    // Twelve destinations that all hang: three rounds of four would pass
    // the budget, so the third round is cut where it stands.
    let environments: Vec<String> = (1..=12).map(|n| format!("env-{n}")).collect();
    let names: Vec<&str> = environments.iter().map(String::as_str).collect();
    let sources = Sources::of(&workspace(&names));
    let started = tokio::time::Instant::now();
    let connect = |entry: Entry| async move {
        Ok::<_, String>(match entry.id.starts_with("env-") {
            true => everything().hanging("deployments"),
            false => everything(),
        })
    };
    let sessions = read_workspace(&sources, None::<Open<'_, FixtureReader>>, connect).await;
    assert_eq!(started.elapsed(), BUDGET);
    assert_eq!(sessions.len(), 14, "every entry is said");
    let covered = coverage(sessions);
    assert_eq!(
        state(&covered, "cluster-b", SourceKind::KargoProjects),
        Some(CoverageState::Read),
        "kept, though others never answered"
    );
    // The second round started 30 s in, so the budget, not its own
    // deadline, cut it short: it names the read's.
    assert_eq!(
        state(&covered, "env-5", SourceKind::Workloads),
        Some(CoverageState::Unreadable(format!(
            "didn't finish reading within the read's {} s",
            BUDGET.as_secs()
        )))
    );
    assert_eq!(
        state(&covered, "env-12", SourceKind::Workloads),
        Some(CoverageState::Unreachable(format!(
            "not read within the read's {} s",
            BUDGET.as_secs()
        )))
    );
}

#[tokio::test]
async fn a_source_its_entry_doesnt_serve_says_so_in_its_name() {
    let sources = Sources::of(&workspace(&["cluster-c"]));
    let connect = |entry: Entry| async move {
        Ok::<_, String>(match entry.id.as_str() {
            // Kargo isn't installed on its entry.
            "cluster-b" => FixtureReader::default(),
            _ => everything(),
        })
    };
    let covered =
        coverage(read_workspace(&sources, None::<Open<'_, FixtureReader>>, connect).await);
    assert!(
        matches!(
            state(&covered, "cluster-b", SourceKind::KargoProjects),
            Some(CoverageState::NotInstalled(_))
        ),
        "{covered:?}"
    );
}

/// A reader whose every request, discovery too, takes a round trip, and
/// which counts them.
struct Distant {
    reader: FixtureReader,
    requests: Rc<Cell<usize>>,
}

impl Distant {
    const ROUND_TRIP: Duration = Duration::from_millis(100);

    async fn trip(&self) {
        self.requests.set(self.requests.get() + 1);
        tokio::time::sleep(Self::ROUND_TRIP).await;
    }
}

impl Reader for Distant {
    async fn get(
        &self,
        resource: &Resource,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<Value, Failure> {
        self.trip().await;
        self.reader.get(resource, namespace, name).await
    }
    async fn list(&self, request: &ListRequest) -> Result<Listing, Failure> {
        self.trip().await;
        self.reader.list(request).await
    }
    async fn groups(&self) -> Result<Vec<ServedGroup>, Failure> {
        self.trip().await;
        self.reader.groups().await
    }
    async fn plurals(&self, group: &str, version: &str) -> Result<Vec<String>, Failure> {
        self.trip().await;
        self.reader.plurals(group, version).await
    }
}

/// A Kargo of the most projects read, behind a slow link, is read in full:
/// discovered once, a few projects at a time.
#[tokio::test(start_paused = true)]
async fn a_large_kargo_far_away_is_read_in_full() {
    use crate::applications::read::{MAX_PROJECTS, PROJECTS_AT_ONCE};

    let sources = Sources::of(&workspace(&["cluster-c"]));
    let names: Vec<String> = (1..=MAX_PROJECTS).map(|n| format!("shop-{n:02}")).collect();
    let kargo = everything()
        .with(
            "projects",
            names
                .iter()
                .map(|name| json!({"metadata": {"name": name}}))
                .collect(),
        )
        .with(
            "stages",
            names
                .iter()
                .map(|name| namespaced(name, "dev", json!({})))
                .collect(),
        );
    let asked_of_kargo = Rc::new(Cell::new(0));
    let kargo = RefCell::new(Some(kargo));
    let connect = |entry: Entry| {
        let (reader, requests) = match entry.id.as_str() {
            "cluster-b" => (
                kargo.borrow_mut().take().expect("one connection"),
                asked_of_kargo.clone(),
            ),
            _ => (everything(), Rc::new(Cell::new(0))),
        };
        async move { Ok::<_, String>(Distant { reader, requests }) }
    };
    let started = tokio::time::Instant::now();
    let sessions = read_workspace(&sources, None::<Open<'_, FixtureReader>>, connect).await;
    let session = sessions
        .iter()
        .find(|session| session.key == SessionKey::new("cluster-b"))
        .expect("Kargo's entry");
    let Source::Read(projects) = &session.kargo else {
        panic!("Kargo read in full: {:?}", session.kargo);
    };
    assert_eq!(projects.len(), MAX_PROJECTS);
    assert!(projects.iter().all(|project| matches!(
        &project.stages,
        Source::Read(stages) if stages.len() == 1
    )));
    let rounds = MAX_PROJECTS.div_ceil(PROJECTS_AT_ONCE) as u32;
    assert!(
        started.elapsed() <= Distant::ROUND_TRIP * (rounds + 10),
        "{:?}",
        started.elapsed()
    );
    assert!(
        started.elapsed() < ENTRY_DEADLINE,
        "within even a destination's deadline"
    );
    // The projects' discovery and listing, then Stages' and Warehouses'
    // discovery once, then each project's two lists.
    assert_eq!(asked_of_kargo.get(), 3 + 2 + 2 * MAX_PROJECTS);
}

/// Kargo's entry has the rest of the budget; a destination its own
/// deadline, and one that answered but ran out says it didn't finish.
#[tokio::test(start_paused = true)]
async fn kargo_has_the_budget_and_a_destination_its_deadline() {
    let sources = Sources::of(&workspace(&["env-1"]));
    // Two listings one after another: past a destination's deadline.
    let slow = Duration::from_secs(ENTRY_DEADLINE.as_secs() / 2 + 1);
    let connect = |entry: Entry| async move {
        Ok::<_, String>(match entry.id.as_str() {
            "cluster-b" | "env-1" => everything().slow(slow),
            _ => everything(),
        })
    };
    let covered =
        coverage(read_workspace(&sources, None::<Open<'_, FixtureReader>>, connect).await);
    assert_eq!(
        state(&covered, "cluster-b", SourceKind::KargoProjects),
        Some(CoverageState::Read)
    );
    assert_eq!(
        state(&covered, "env-1", SourceKind::Workloads),
        Some(CoverageState::Unreadable(format!(
            "didn't finish reading within {} s",
            ENTRY_DEADLINE.as_secs()
        )))
    );
}

#[test]
fn an_entry_that_never_answered_is_unreachable_not_empty() {
    let asked = Asked {
        workloads: true,
        ..Asked::NONE
    };
    let session = answered(None, SessionKey::new("env-1"), asked);
    assert_eq!(
        session.absent,
        Some(Absent::Unreachable("no answer".into()))
    );
    assert!(matches!(session.workloads, Source::Unreadable(_)));
}

/// A Kargo with no projects asks nothing of Stages' or Warehouses'
/// discovery.
#[tokio::test(start_paused = true)]
async fn a_kargo_with_no_projects_discovers_nothing_more() {
    let sources = Sources::of(&workspace(&["cluster-c"]));
    let asked_of_kargo = Rc::new(Cell::new(0));
    let connect = |entry: Entry| {
        let (reader, requests) = match entry.id.as_str() {
            "cluster-b" => (
                everything().with("projects", Vec::new()),
                asked_of_kargo.clone(),
            ),
            _ => (everything(), Rc::new(Cell::new(0))),
        };
        async move { Ok::<_, String>(Distant { reader, requests }) }
    };
    let sessions = read_workspace(&sources, None::<Open<'_, FixtureReader>>, connect).await;
    let session = sessions
        .iter()
        .find(|session| session.key == SessionKey::new("cluster-b"))
        .expect("Kargo's entry");
    assert_eq!(session.kargo, Source::Read(Vec::new()));
    // The projects' discovery and listing alone.
    assert_eq!(asked_of_kargo.get(), 3);
}
