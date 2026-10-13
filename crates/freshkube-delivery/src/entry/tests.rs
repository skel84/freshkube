use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::fixtures::FixtureReader;
use freshkube_core::resources::FailureKind;
use freshkube_core::workspace::{Role, Workspace};

/// A cluster that serves every part's kind, and has one of each.
fn serving() -> FixtureReader {
    FixtureReader::default()
        .serves(kargo::GROUP, "v1alpha1", &["projects", "stages"])
        .serves(tekton::TEKTON_GROUP, "v1", &["pipelineruns", "taskruns"])
        .serves(argocd::GROUP, "v1alpha1", &["applications"])
}

#[tokio::test]
async fn an_answer_is_readable_even_when_empty() {
    let reader = serving();
    for part in Part::ALL {
        assert_eq!(check(&reader, part).await, Reach::Readable, "{part:?}");
    }
    let requests = reader.requests.borrow();
    assert!(
        requests.contains(&"LIST kargo.akuity.io/v1alpha1/projects ns=None selector=None".into()),
        "{requests:?}"
    );
    assert!(
        requests.contains(
            &"LIST tekton.dev/v1/pipelineruns ns=None selector=Some(\"pipelinesascode.tekton.dev/repository\")"
                .into()
        ),
        "{requests:?}"
    );
    // With nothing labelled part-of=argocd, Argo CD is read in `argocd`.
    assert!(
        requests.contains(
            &"LIST argoproj.io/v1alpha1/applications ns=Some(\"argocd\") selector=None".into()
        ),
        "{requests:?}"
    );
    assert!(
        requests.contains(
            &"LIST apps/v1/deployments ns=None selector=Some(\"app.kubernetes.io/part-of=argocd\")"
                .into()
        ),
        "{requests:?}"
    );
}

#[tokio::test]
async fn a_refusal_is_refused_never_none_found() {
    let reach = check(&serving().refusing("pipelineruns"), Part::Builds).await;
    assert_eq!(reach, Reach::refused("forbidden by RBAC"));
    assert_eq!(reach.label(), "Refused");
    assert_eq!(
        reach.words(Part::Builds.label(), "cluster-b"),
        "Builds refused on cluster-b: forbidden by RBAC"
    );
}

#[tokio::test]
async fn a_kind_the_cluster_does_not_serve_is_not_served() {
    let reach = check(&FixtureReader::default(), Part::Kargo).await;
    assert!(matches!(reach, Reach::NotServed(_)), "{reach:?}");
    assert_eq!(
        reach.words(Part::Kargo.label(), "cluster-a"),
        "Kargo isn't served on cluster-a"
    );
    // The group served without the kind is the same.
    let reach = check(
        &FixtureReader::default().serves(argocd::GROUP, "v1alpha1", &["appprojects"]),
        Part::ArgoCd,
    )
    .await;
    assert!(matches!(reach, Reach::NotServed(_)), "{reach:?}");
}

#[tokio::test]
async fn a_connection_that_fails_is_unreachable() {
    let reach = check(&serving().unreachable("applications"), Part::ArgoCd).await;
    assert_eq!(
        reach.words(Part::ArgoCd.label(), "cluster-a"),
        "Couldn't reach cluster-a: the server could not be reached"
    );
}

#[tokio::test(start_paused = true)]
async fn a_read_past_its_deadline_is_unreachable() {
    let reader = Deadline::new(serving().hanging("projects"), REQUEST_DEADLINE);
    let started = tokio::time::Instant::now();
    let reach = check(&reader, Part::Kargo).await;
    assert_eq!(started.elapsed(), REQUEST_DEADLINE);
    match reach {
        Reach::Unreachable(why) => assert_eq!(why.message, "the request timed out"),
        other => panic!("{other:?}"),
    }
    // The deadline is each request's, not the whole check's.
    let failure = Deadline::new(serving().hanging("projects"), Duration::from_secs(1))
        .get(&Resource::new("", "v1", "projects", false), None, "x")
        .await
        .unwrap_err();
    assert_eq!(failure.kind, FailureKind::Timeout);
}

/// A reader whose every listing takes `slow`.
struct Slow(Duration);

impl Reader for Slow {
    async fn get(
        &self,
        _: &Resource,
        _: Option<&str>,
        _: &str,
    ) -> Result<serde_json::Value, Failure> {
        unreachable!()
    }
    async fn list(&self, _: &ListRequest) -> Result<Listing, Failure> {
        tokio::time::sleep(self.0).await;
        Ok(Listing::default())
    }
    async fn groups(&self) -> Result<Vec<ServedGroup>, Failure> {
        Ok(Vec::new())
    }
    async fn plurals(&self, _: &str, _: &str) -> Result<Vec<String>, Failure> {
        Ok(Vec::new())
    }
}

#[tokio::test(start_paused = true)]
async fn an_entry_readers_listing_has_the_deadline_once_for_each_page_it_may_read() {
    let request = ListRequest {
        resource: Resource::new("apps", "v1", "deployments", true),
        scope: Scope::Cluster,
    };
    // Longer than one request, as several pages that each answer take.
    let entry = |slow| bounded(Slow(slow));
    assert!(entry(REQUEST_DEADLINE * 3).list(&request).await.is_ok());
    let failure = entry(REQUEST_DEADLINE * (MAX_PAGES + 1))
        .list(&request)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, FailureKind::Timeout);

    // Any other reader's listing keeps the one deadline it was given.
    let started = tokio::time::Instant::now();
    let other = Deadline::new(Slow(REQUEST_DEADLINE * 3), REQUEST_DEADLINE);
    let failure = other.list(&request).await.unwrap_err();
    assert_eq!(failure.kind, FailureKind::Timeout);
    assert_eq!(started.elapsed(), REQUEST_DEADLINE);
}

/// Credentials the cluster rejects are said to whoever asked, so a served
/// kubeconfig is asked for again; any other failure isn't.
#[tokio::test]
async fn rejected_credentials_are_said() {
    let rejected = Arc::new(AtomicUsize::new(0));
    let told = rejected.clone();
    let reader = Deadline::new(
        serving()
            .failing(
                "projects",
                Failure::new(FailureKind::Unauthorized, "Unauthorized"),
            )
            .failing("stages", Failure::new(FailureKind::Forbidden, "forbidden")),
        REQUEST_DEADLINE,
    )
    .on_rejected(move || {
        told.fetch_add(1, Ordering::SeqCst);
    });
    let list = |plural: &str| ListRequest {
        resource: Resource::new("kargo.akuity.io", "v1alpha1", plural, false),
        scope: Scope::Cluster,
    };
    assert!(reader.list(&list("stages")).await.is_err());
    assert_eq!(rejected.load(Ordering::SeqCst), 0, "refused, not rejected");
    assert!(reader.list(&list("projects")).await.is_err());
    assert_eq!(rejected.load(Ordering::SeqCst), 1);
}

/// Entries connect a few at a time in whatever order they answer, and come
/// back in the order given; one that never connects keeps none waiting.
#[tokio::test(start_paused = true)]
async fn entries_connect_without_waiting_on_each_other() {
    let entries: Vec<Entry> = (1..=6)
        .map(|n| entry(&format!("env-{n}"), Role::Environment))
        .collect();
    let started = tokio::time::Instant::now();
    let connected = connect_each(entries, |entry| async move {
        match entry.id.as_str() {
            // The first never answers within its deadline.
            "env-1" => {
                tokio::time::sleep(CONNECT_DEADLINE).await;
                Err("no answer".to_owned())
            }
            _ => Ok(entry.id),
        }
    })
    .await;
    assert_eq!(
        started.elapsed(),
        CONNECT_DEADLINE,
        "the others didn't wait"
    );
    let ids: Vec<&str> = connected.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["env-1", "env-2", "env-3", "env-4", "env-5", "env-6"]);
    assert_eq!(connected[0].1, Err("no answer".to_owned()));
    assert_eq!(connected[5].1, Ok("env-6".to_owned()));
}

fn entry(id: &str, role: Role) -> Entry {
    Entry::new(id, role, format!("{id}-context"))
}

#[tokio::test]
async fn every_part_is_checked_and_a_missing_entry_reads_nothing() {
    // No core entry: Argo CD has nowhere to be read. The cicd entry's
    // context isn't in any kubeconfig, so it can't be reached.
    let workspace = Workspace::new(vec![
        entry("cluster-b", Role::Cicd),
        entry("cluster-c", Role::Environment),
    ]);
    let checked = check_all(
        &Sources::of(&workspace),
        &[],
        None::<Result<&FixtureReader, _>>,
    )
    .await;
    let [
        (kargo, kargo_reach),
        (builds, builds_reach),
        (argocd, argocd_reach),
    ] = checked.parts.as_slice()
    else {
        panic!("{checked:?}");
    };
    assert_eq!(
        (*kargo, *builds, *argocd),
        (Part::Kargo, Part::Builds, Part::ArgoCd)
    );
    let unreachable = Reach::unreachable("the kubeconfig has no context cluster-b-context");
    assert_eq!(kargo_reach, &unreachable);
    assert_eq!(builds_reach, &unreachable);
    assert_eq!(
        argocd_reach,
        &Reach::missing("No Core entry in the workspace")
    );
}

#[tokio::test]
async fn a_workspace_of_one_checks_the_open_connection() {
    let sources = Sources::of(&Workspace::default());
    let open = serving().refusing("applications");
    let checked = check_all(&sources, &[], Some(Ok(&open))).await;
    assert_eq!(
        checked.parts,
        vec![
            (Part::Kargo, Reach::Readable),
            (Part::Builds, Reach::Readable),
            (Part::ArgoCd, Reach::refused("forbidden by RBAC")),
        ]
    );
    assert_eq!(checked.argocd_in, vec!["argocd"]);
    assert_eq!(checked.workloads, vec![(OPEN.to_owned(), Reach::Readable)]);
    let none = check_all(&sources, &[], None::<Result<&FixtureReader, _>>).await;
    assert!(
        none.parts
            .iter()
            .all(|(_, reach)| matches!(reach, Reach::Unreachable(_)))
    );
}

/// A kubeconfig whose one context authenticates with `command`.
#[cfg(unix)]
fn exec_kubeconfig(directory: &std::path::Path, command: &str, args: &[&str]) -> PathBuf {
    let file = directory.join("kubeconfig");
    let config = serde_json::json!({
        "apiVersion": "v1",
        "kind": "Config",
        "clusters": [{"name": "cluster-a", "cluster": {"server": "https://127.0.0.1:1"}}],
        "users": [{"name": "plugin", "user": {"exec": {
            "apiVersion": "client.authentication.k8s.io/v1beta1",
            "command": command,
            "args": args,
            "interactiveMode": "Never",
        }}}],
        "contexts": [{"name": "cluster-a", "context": {"cluster": "cluster-a", "user": "plugin"}}],
    });
    std::fs::write(&file, serde_json::to_vec(&config).unwrap()).unwrap();
    file
}

#[cfg(unix)]
#[tokio::test]
async fn a_credential_plugin_waiting_for_a_sign_in_ends_at_the_connect_deadline() {
    let directory = tempfile::tempdir().unwrap();
    let kubeconfig = exec_kubeconfig(directory.path(), "sleep", &["1"]);
    let started = std::time::Instant::now();
    let error = connect_within(
        vec![kubeconfig],
        "cluster-a".into(),
        Duration::from_millis(300),
    )
    .await
    .err()
    .expect("the plugin never answers in time");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        error,
        "no answer within 300 ms; a credential plugin may be waiting for a sign-in"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_credential_plugin_that_fails_is_said_without_its_output() {
    let directory = tempfile::tempdir().unwrap();
    let kubeconfig = exec_kubeconfig(directory.path(), "false", &[]);
    let error = connect_within(vec![kubeconfig], "cluster-a".into(), CONNECT_DEADLINE)
        .await
        .err()
        .expect("the plugin fails");
    assert_eq!(error, "its credential plugin failed");
}

#[tokio::test]
async fn a_context_no_kubeconfig_has_is_said() {
    let directory = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    let kubeconfig = exec_kubeconfig(directory.path(), "false", &[]);
    #[cfg(not(unix))]
    let kubeconfig = directory.path().join("missing");
    let error = connect(vec![kubeconfig], "cluster-z".into())
        .await
        .err()
        .expect("no such context");
    assert_eq!(error, "the kubeconfig has no context cluster-z");
}

#[tokio::test]
async fn an_open_connection_that_failed_says_why() {
    let sources = Sources::of(&Workspace::default());
    let checked = check_all(
        &sources,
        &[],
        Some(Err::<&FixtureReader, _>(
            "its credential plugin failed".into(),
        )),
    )
    .await;
    let failed = Reach::unreachable("its credential plugin failed");
    assert!(
        checked.parts.iter().all(|(_, reach)| reach == &failed),
        "{checked:?}"
    );
    assert_eq!(checked.workloads, vec![(OPEN.to_owned(), failed.clone())]);
    assert_eq!(
        failed.words("Kargo", OPEN),
        "Couldn't reach the open cluster: its credential plugin failed"
    );
}

fn argo_server(namespace: &str) -> serde_json::Value {
    serde_json::json!({"metadata": {
        "name": "argocd-server",
        "namespace": namespace,
        "labels": {"app.kubernetes.io/part-of": "argocd"},
    }})
}

#[tokio::test]
async fn argo_cd_is_checked_where_it_runs() {
    // Argo CD runs in `gitops`, and its Applications are read there only.
    let reader = serving().with("deployments", vec![argo_server("gitops")]);
    let (reach, namespaces) = check_argo(&reader).await;
    assert_eq!(
        (reach, namespaces),
        (Reach::Readable, vec!["gitops".to_owned()])
    );
    let requests = reader.requests.borrow();
    assert!(
        requests.contains(
            &"LIST argoproj.io/v1alpha1/applications ns=Some(\"gitops\") selector=None".into()
        ),
        "{requests:?}"
    );
    assert!(
        !requests
            .iter()
            .any(|request| request.contains("ns=Some(\"argocd\")")),
        "{requests:?}"
    );
}

#[tokio::test]
async fn a_refused_workload_read_falls_back_to_argocd_as_the_read_does() {
    let reader = serving().refusing("deployments");
    assert_eq!(
        check_argo(&reader).await,
        (Reach::Readable, vec!["argocd".to_owned()])
    );
}

#[tokio::test]
async fn workloads_are_checked_on_each_mapped_entry() {
    use freshkube_core::workspace::{Destination, Key};
    let mut workspace = Workspace::new(vec![
        entry("cluster-a", Role::Core),
        entry("cluster-c", Role::Environment),
    ]);
    workspace.destinations = vec![
        Destination::new(Key::Name("c".into()), "cluster-c"),
        Destination::new(Key::Name("x".into()), "cluster-x"),
    ];
    let checked = check_all(
        &Sources::of(&workspace),
        &[],
        None::<Result<&FixtureReader, _>>,
    )
    .await;
    assert_eq!(
        checked.workloads,
        vec![
            (
                "cluster-c".to_owned(),
                Reach::unreachable("the kubeconfig has no context cluster-c-context")
            ),
            (
                "cluster-x".to_owned(),
                Reach::missing("cluster-x is no longer in the workspace")
            ),
        ]
    );
    assert_eq!(check_workloads(&serving()).await, Reach::Readable);
    assert!(matches!(
        check_workloads(&serving().refusing("deployments")).await,
        Reach::Refused(_)
    ));
}

/// A Talos entry whose context `cluster-t` names `talos_context`.
fn talos_entry(talosconfig: PathBuf, talos_context: Option<&str>) -> Entry {
    let mut entry = entry("cluster-t", Role::Core);
    entry.context = "cluster-t".into();
    entry.talosconfig = Some(talosconfig);
    entry.talos_context = talos_context.map(str::to_owned);
    entry
}

#[tokio::test]
async fn a_talos_entry_outside_the_kubeconfigs_is_opened_through_its_talosconfig() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("talosconfig");
    let error = connect_entry(vec![], talos_entry(missing.clone(), None))
        .await
        .err()
        .expect("no talosconfig");
    assert_eq!(
        error,
        format!("its talosconfig {} couldn't be read", missing.display())
    );
    // Its Talos context, else its context; never the file's current one.
    let talosconfig = directory.path().join("talosconfig");
    std::fs::write(
        &talosconfig,
        "context: other\ncontexts:\n  other:\n    endpoints: [127.0.0.1]\n    ca: ''\n    crt: ''\n    key: ''\n",
    )
    .unwrap();
    for (talos_context, named) in [(None, "cluster-t"), (Some("lab"), "lab")] {
        let error = connect_entry(vec![], talos_entry(talosconfig.clone(), talos_context))
            .await
            .err()
            .expect("no such Talos context");
        assert_eq!(error, format!("its talosconfig has no context {named}"));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_talos_entry_in_the_kubeconfigs_is_opened_with_its_context() {
    let directory = tempfile::tempdir().unwrap();
    let kubeconfig = exec_kubeconfig(directory.path(), "false", &[]);
    let mut entry = talos_entry(directory.path().join("talosconfig"), None);
    entry.context = "cluster-a".into();
    let error = connect_entry(vec![kubeconfig], entry)
        .await
        .err()
        .expect("the plugin fails");
    assert_eq!(error, "its credential plugin failed");
}
