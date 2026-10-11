use super::*;
use crate::delivery::fixtures::FixtureReader;
use crate::resources::FailureKind;
use crate::workspace::{Role, Workspace};

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
    assert!(
        requests.contains(&"LIST argoproj.io/v1alpha1/applications ns=Some(\"argocd\") selector=None".into()),
        "{requests:?}"
    );
}

#[tokio::test]
async fn a_refusal_is_refused_never_none_found() {
    let reach = check(&serving().refusing("pipelineruns"), Part::Builds).await;
    assert_eq!(reach, Reach::Refused("forbidden by RBAC".into()));
    assert_eq!(reach.label(), "Refused");
    assert_eq!(
        reach.words(Part::Builds, "cluster-b"),
        "Builds refused on cluster-b: forbidden by RBAC"
    );
}

#[tokio::test]
async fn a_kind_the_cluster_does_not_serve_is_not_served() {
    let reach = check(&FixtureReader::default(), Part::Kargo).await;
    assert!(matches!(reach, Reach::NotServed(_)), "{reach:?}");
    assert_eq!(
        reach.words(Part::Kargo, "cluster-a"),
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
        reach.words(Part::ArgoCd, "cluster-a"),
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
        Reach::Unreachable(why) => assert_eq!(why, "the request timed out"),
        other => panic!("{other:?}"),
    }
    // The deadline is each request's, not the whole check's.
    let failure = Deadline::new(serving().hanging("projects"), Duration::from_secs(1))
        .get(&Resource::new("", "v1", "projects", false), None, "x")
        .await
        .unwrap_err();
    assert_eq!(failure.kind, FailureKind::Timeout);
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
    let checked = check_all(&Sources::of(&workspace), &[], None::<&FixtureReader>).await;
    let [(kargo, kargo_reach), (builds, builds_reach), (argocd, argocd_reach)] =
        checked.parts.as_slice()
    else {
        panic!("{checked:?}");
    };
    assert_eq!(
        (*kargo, *builds, *argocd),
        (Part::Kargo, Part::Builds, Part::ArgoCd)
    );
    let unreachable = Reach::Unreachable("the kubeconfig has no context cluster-b-context".into());
    assert_eq!(kargo_reach, &unreachable);
    assert_eq!(builds_reach, &unreachable);
    assert_eq!(
        argocd_reach,
        &Reach::Missing("No Core entry in the workspace".into())
    );
}

#[tokio::test]
async fn a_workspace_of_one_checks_the_open_connection() {
    let sources = Sources::of(&Workspace::default());
    let open = serving().refusing("applications");
    let checked = check_all(&sources, &[], Some(&open)).await;
    assert_eq!(
        checked.parts,
        vec![
            (Part::Kargo, Reach::Readable),
            (Part::Builds, Reach::Readable),
            (Part::ArgoCd, Reach::Refused("forbidden by RBAC".into())),
        ]
    );
    let none = check_all(&sources, &[], None::<&FixtureReader>).await;
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
    assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
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
