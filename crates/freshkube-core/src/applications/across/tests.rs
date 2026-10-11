use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::applications::read::read_session;
use crate::applications::{CoverageState, Inputs, Override, SourceKind, derive};
use crate::delivery::fixtures::FixtureReader;
use crate::delivery::read::{ListRequest, Listing, Resource, ServedGroup};
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
    // Argo CD's entry lists only the workloads that say where it runs.
    let argo = asked("cluster-a");
    assert!(!argo.contains("kargo"), "{argo}");
    assert!(
        argo.lines()
            .filter(|line| line.contains("deployments"))
            .all(|line| line.contains("part-of=argocd")),
        "{argo}"
    );

    let derived = derive(
        &Inputs {
            sessions: sessions.clone(),
            stage_naming: None,
        },
        &Override::default(),
    );
    let names: Vec<&str> = derived
        .applications
        .iter()
        .map(|a| a.name.as_str())
        .collect();
    assert_eq!(names, ["checkout", "storefront"]);
    // The workload in cluster-c joins the Application read in cluster-a.
    let storefront = &derived.applications[1];
    assert!(
        storefront
            .members
            .iter()
            .any(|m| m.at.name == "web" && m.at.session.0 == "cluster-c")
    );
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
    assert_eq!(state(&covered, "cluster-a", SourceKind::Workloads), None);
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
        state(&covered, WORKSPACE, SourceKind::ArgoApplications),
        Some(CoverageState::NotInWorkspace(
            "No Core entry in the workspace".into()
        ))
    );
    assert_eq!(
        state(&covered, "cluster-b", SourceKind::KargoProjects),
        Some(CoverageState::Read)
    );
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
async fn an_entry_holding_two_sources_is_read_once_for_both() {
    // cluster-a holds Argo CD and is a destination too.
    let mut workspace = workspace(&[]);
    workspace.destinations = vec![Destination::new(
        Key::Name("in-cluster".into()),
        "cluster-a",
    )];
    let sources = Sources::of(&workspace);
    let connected = RefCell::new(Vec::new());
    let connect = |entry: Entry| {
        connected.borrow_mut().push(entry.id);
        async { Ok::<_, String>(everything()) }
    };
    let sessions = read_workspace(&sources, None::<Open<'_, FixtureReader>>, connect).await;
    assert_eq!(connected.into_inner(), ["cluster-b", "cluster-a"]);
    let a = sessions.iter().find(|s| s.key.0 == "cluster-a").unwrap();
    assert_eq!(
        a.asked,
        Asked {
            argo_cd: true,
            workloads: true,
            ..Asked::NONE
        }
    );
}
