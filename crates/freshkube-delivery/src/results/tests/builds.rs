use super::*;
use crate::change::live::{self, Mapping, Pages, Place};
use crate::change::{Gap, Shows, Target, Value as Shown};
use crate::collect::{Upstream, upstream_with_history};
use crate::fixtures::{self, FixtureGitHub, FixtureReader};
use crate::join::{self, Confidence, Evidence, Hop};
use crate::source::Source;
use crate::tekton;

pub(super) fn reader(runs: Vec<Value>, tasks: Vec<Value>) -> FixtureReader {
    fixtures::healthy()
        .with_meta()
        .one_cluster()
        .with("pipelineruns", runs)
        .with("taskruns", tasks)
}

pub(super) fn pipeline(uid: &str) -> Value {
    let mut run = manifest("PipelineRun", "v1");
    run["metadata"]["uid"] = json!(uid);
    run["metadata"]["annotations"] = json!({});
    run["status"]["results"] = json!([
        {"name": "commit", "value": SHA},
        {"name": "IMAGE_DIGEST", "value": fixtures::NEW},
        {"name": "IMAGE_URL", "value": "registry.example.com/acme/checkout"}
    ]);
    run
}

pub(super) async fn evidence(reader: &FixtureReader, client: Result<&Client, Error>) -> Evidence {
    let github = FixtureGitHub::with(vec![]);
    let upstream = Upstream {
        kargo: reader,
        argocd: reader,
        tekton: reader,
        github: &github,
    };
    let mut plan = crate::tests::plan(&[]);
    plan.sha = SHA.into();
    plan.build_namespace = Some("acme".into());
    within(upstream_with_history(
        &upstream,
        &plan,
        crate::tests::observed_at(),
        std::future::ready(Source::Read(vec![])),
        client,
        Instant::now() + Duration::from_secs(20),
    ))
    .await
}

fn place() -> Place {
    Place {
        cluster: "cluster-b".into(),
        label: "cluster-b".into(),
        project: "storefront".into(),
        freight: "f-new".into(),
        argocd_namespace: "argocd".into(),
        mapping: Mapping::default(),
        sources: Default::default(),
    }
}

pub(super) fn change(evidence: &Evidence) -> crate::change::Change {
    let freight = &evidence.kargo.freight.read().unwrap()[0];
    live::derive(
        &place(),
        evidence,
        freight,
        &Pages::unread("invented example"),
        &Default::default(),
    )
}

fn read_from(row: &crate::change::Hop) -> &str {
    let Shows::Hop(detail) = &row.shows else {
        panic!("hop detail");
    };
    let field = detail
        .fields
        .iter()
        .find(|field| field.label == "Read from")
        .unwrap();
    let Shown::Mono(value) = &field.value else {
        panic!("read source");
    };
    value
}

#[tokio::test(flavor = "multi_thread")]
async fn archived_builds_join_with_the_recorded_facts_and_disable_live_actions() {
    let run = pipeline("archived-uid");
    let mut task = manifest("TaskRun", "v1");
    task["metadata"]["uid"] = json!("task-uid");
    let server = Server::new(
        false,
        vec![
            page(vec![record(run.clone())]),
            page(vec![record(task.clone())]),
            page(vec![]),
        ],
    );
    let client = client(&server).await;
    let archived = evidence(&reader(vec![], vec![]), Ok(&client)).await;
    let live = evidence(&reader(vec![run], vec![task]), Err(Error::Off)).await;
    let build = &archived.builds.read().unwrap()[0];
    assert_eq!(build.origin, tekton::Origin::Results);
    assert_eq!(
        build.archive.as_ref().unwrap().record,
        format!("{RESULT}/records/record-a")
    );
    let mut normalized = build.clone();
    normalized.origin = tekton::Origin::Cluster;
    normalized.archive = None;
    assert_eq!(&normalized, &live.builds.read().unwrap()[0]);
    assert_eq!(join::join(&archived), join::join(&live));
    let trail = join::join(&archived);
    assert!(
        trail
            .links
            .iter()
            .any(|link| link.to == Hop::PipelineRun && link.confidence == Confidence::Confirmed)
    );
    assert!(
        trail
            .links
            .iter()
            .any(|link| link.to == Hop::SupplyChain && link.confidence == Confidence::Claimed)
    );
    let change = change(&archived);
    let row = change
        .hops
        .iter()
        .find(|row| row.name == "Run checkout-7")
        .unwrap();
    assert_eq!(read_from(row), "Tekton Results · cluster-b");
    let Shows::Hop(detail) = &row.shows else {
        panic!();
    };
    assert!(
        detail
            .actions
            .iter()
            .any(|action| matches!(action.target, Target::Resource { .. })
                && action.disabled.as_deref() == Some("the cluster no longer holds this run"))
    );
    assert!(
        detail
            .actions
            .iter()
            .any(|action| matches!(action.target, Target::Logs { .. })
                && action.disabled.as_deref() == Some("Tekton Results kept no log for this task"))
    );
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(
        requests.iter().all(|request| !request.contains("/logs/")),
        "no log content is read"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn live_uid_wins_without_reading_archived_tasks_or_logs() {
    let mut live = pipeline("same-uid");
    live["status"]["conditions"][0]["status"] = json!("Unknown");
    let archived = pipeline("same-uid");
    let server = Server::new(false, vec![page(vec![record(archived)])]);
    let client = client(&server).await;
    let answer = evidence(&reader(vec![live.clone()], vec![]), Ok(&client)).await;
    let builds = answer.builds.read().unwrap();
    assert_eq!(builds.len(), 1);
    assert_eq!(builds[0].run, tekton::parse_pipeline_run(&live).unwrap());
    assert_eq!(builds[0].origin, tekton::Origin::Cluster);
    assert!(builds[0].archive.as_ref().unwrap().logs.is_none());
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    let change = change(&answer);
    let row = change
        .hops
        .iter()
        .find(|row| row.name == "Run checkout-7")
        .unwrap();
    assert_eq!(
        read_from(row),
        "Tekton · cluster-b · also in Tekton Results"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reused_names_and_missing_live_uids_never_collapse_archived_runs() {
    for uid in [Some("new-uid"), None] {
        let mut live = pipeline("new-uid");
        if uid.is_none() {
            live["metadata"].as_object_mut().unwrap().remove("uid");
        }
        // Avoid fixture stamping of a deliberately missing live UID.
        let reader = FixtureReader::default()
            .serves("tekton.dev", "v1", &["pipelineruns", "taskruns"])
            .with("pipelineruns", vec![live])
            .with("taskruns", vec![]);
        let server = Server::new(
            false,
            vec![
                page(vec![record(pipeline("old-uid"))]),
                page(vec![]),
                page(vec![]),
            ],
        );
        let client = client(&server).await;
        let answer = evidence(&reader, Ok(&client)).await;
        let builds = answer.builds.read().unwrap();
        assert_eq!(builds.len(), 2);
        assert_eq!(builds[0].origin, tekton::Origin::Cluster);
        assert_eq!(builds[1].origin, tekton::Origin::Results);
        let trail = join::join(&answer);
        assert_eq!(
            trail
                .links
                .iter()
                .filter(|link| link.to == Hop::PipelineRun)
                .count(),
            2
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn results_failures_preserve_live_builds_and_name_the_entry() {
    for status in [401, 403, 404, 503] {
        let server = Server::new(false, vec![Reply::raw(status, "invented-response")]);
        let client = client(&server).await;
        let answer = evidence(&reader(vec![pipeline("live-uid")], vec![]), Ok(&client)).await;
        assert_eq!(answer.builds.read().unwrap().len(), 1);
        assert_eq!(
            answer.builds.read().unwrap()[0].origin,
            tekton::Origin::Cluster
        );
        let change = change(&answer);
        assert!(
            change
                .missing
                .iter()
                .any(|line| line.text.contains("Tekton Results on cluster-b"))
        );
        let expected = match status {
            401 | 403 => Gap::Refused,
            404 => Gap::NotServed,
            _ => Gap::Unreadable,
        };
        assert!(change.missing.iter().any(|line| line.gap == expected));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unread_live_builds_never_request_results_or_publish_archive_builds() {
    let server = Server::new(false, vec![]);
    let client = client(&server).await;
    let answer = evidence(
        &reader(vec![], vec![]).refusing("pipelineruns"),
        Ok(&client),
    )
    .await;
    assert!(matches!(answer.builds, Source::Refused(_)));
    assert!(server.requests.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn capped_live_builds_cannot_establish_that_an_archived_run_was_pruned() {
    let server = Server::new(false, vec![]);
    let client = client(&server).await;
    let answer = evidence(
        &reader(vec![pipeline("live-uid")], vec![]).capped("pipelineruns"),
        Ok(&client),
    )
    .await;
    assert!(matches!(answer.builds, Source::Capped(..)));
    assert_eq!(answer.builds.read().unwrap().len(), 1);
    assert_eq!(
        answer.builds.read().unwrap()[0].origin,
        tekton::Origin::Cluster
    );
    assert!(server.requests.lock().unwrap().is_empty());
    assert!(
        answer
            .build_history
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("absence could not be established")
    );
    assert!(
        change(&answer)
            .missing
            .iter()
            .any(|line| line.text.contains("cap"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_record_without_chains_annotations_leaves_supply_chain_unknown() {
    let server = Server::new(
        false,
        vec![
            page(vec![record(pipeline("old-uid"))]),
            page(vec![]),
            page(vec![]),
        ],
    );
    let client = client(&server).await;
    let answer = evidence(&reader(vec![], vec![]), Ok(&client)).await;
    let trail = join::join(&answer);
    let link = trail
        .links
        .iter()
        .find(|link| link.to == Hop::SupplyChain)
        .unwrap();
    assert_eq!(link.confidence, Confidence::Unknown);
    assert!(!link.reason.contains("unsigned"));
}

#[tokio::test(flavor = "multi_thread")]
async fn reused_run_names_keep_their_own_source_status_and_actions() {
    let mut live = pipeline("new-uid");
    live["status"]["conditions"][0]["status"] = json!("Unknown");
    let server = Server::new(
        false,
        vec![
            page(vec![record(pipeline("old-uid"))]),
            page(vec![]),
            page(vec![]),
        ],
    );
    let client = client(&server).await;
    let answer = evidence(&reader(vec![live], vec![]), Ok(&client)).await;
    let change = change(&answer);
    let rows: Vec<_> = change
        .hops
        .iter()
        .filter(|row| row.name == "Run checkout-7")
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(read_from(rows[0]), "Tekton · cluster-b");
    assert_eq!(read_from(rows[1]), "Tekton Results · cluster-b");
    for (index, row) in rows.iter().enumerate() {
        let Shows::Hop(detail) = &row.shows else {
            panic!();
        };
        assert_eq!(
            detail.state,
            if index == 0 { "Running" } else { "Succeeded" }
        );
        let resource = detail
            .actions
            .iter()
            .find(|action| matches!(action.target, Target::Resource { .. }))
            .unwrap();
        assert_eq!(resource.disabled.is_some(), index == 1);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_runs_keep_the_no_run_statement_with_results_why() {
    let server = Server::new(false, vec![page(vec![])]);
    let client = client(&server).await;
    for history in [
        Err(Error::Off),
        Ok(client.as_ref()),
        Err(Error::Unreachable(Cause::Certificate)),
    ] {
        let answer = evidence(&reader(vec![], vec![]), history).await;
        let change = change(&answer);
        let row = change
            .hops
            .iter()
            .find(|row| row.name == "PipelineRun")
            .unwrap();
        assert!(row.detail.contains("Tekton Results"));
        assert!(!row.detail.contains("Run pruned"));
        assert_eq!(row.link, Some(Confidence::Unknown));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn capped_history_keeps_the_cap_in_read_from_and_missing() {
    // Five pages may contain the same record: only one UID is published.
    let replies = (0..5).map(|n| Reply::json(json!({"records": [record(pipeline("live-uid"))], "nextPageToken": format!("next-{n}")}))).collect();
    let server = Server::new(false, replies);
    let client = client(&server).await;
    let answer = evidence(&reader(vec![pipeline("live-uid")], vec![]), Ok(&client)).await;
    assert_eq!(answer.builds.read().unwrap().len(), 1);
    assert_eq!(answer.build_history.as_ref().unwrap().total, None);
    let change = change(&answer);
    assert!(
        change
            .missing
            .iter()
            .any(|line| line.text.contains("5-page cap"))
    );
    let row = change
        .hops
        .iter()
        .find(|row| row.name == "Run checkout-7")
        .unwrap();
    assert!(read_from(row).contains("5-page cap"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pool_opens_one_address_client_and_counts_a_two_change_refresh() {
    use crate::entry::pool::{Open, Pool};
    use freshkube_core::workspace::{Entry, Role, Sources, Workspace};
    let replies = (0..2)
        .flat_map(|_| {
            [
                page(vec![record(pipeline("archived-uid"))]).matching("PipelineRun"),
                page(vec![]).matching("TaskRun"),
                page(vec![]).matching("/logs?"),
            ]
        })
        .collect();
    let server = Server::new(false, replies);
    let store = Arc::new(MemoryStore::default());
    store
        .write(&token_account(&server.address).unwrap(), "invented-token")
        .unwrap();
    let mut entry = Entry::new("cluster-b", Role::Cicd, "cluster-b");
    let mut settings = settings(&server.address);
    settings.token = Some(KeysJson);
    entry.results = Some(Box::new(settings));
    let sources = Sources::of(&Workspace::new(vec![entry]));
    let reader = reader(vec![], vec![])
        .serves(
            "kargo.akuity.io",
            "v1alpha1",
            &["projects", "warehouses", "freights", "stages", "promotions"],
        )
        .with(
            "projects",
            vec![json!({"metadata": {"name": "storefront"}})],
        )
        .with(
            "freights",
            vec![
                fixtures::freight("f-new", fixtures::NEW, SHA),
                fixtures::freight("f-second", fixtures::NEW, SHA),
            ],
        )
        .with("stages", vec![fixtures::stage(&["f-new", "f-second"])]);
    let pool = Pool::new(
        &sources,
        Some(Open {
            entry: Some("cluster-b"),
            reader: Ok(&reader),
        }),
        |_| std::future::ready(Ok(FixtureReader::default())),
        crate::changes::live::BUDGET,
    )
    .with_secrets(Some(store.clone()));
    let until = pool.first().at().unwrap();
    let first = within(pool.results(until)).await.unwrap().id();
    store.unavailable.store(true, Ordering::SeqCst);
    assert_eq!(first, within(pool.results(until)).await.unwrap().id());
    let started = Instant::now();
    let at = crate::changes::live::Where {
        cluster: "cluster-b".into(),
        label: "cluster-b".into(),
        argocd_namespace: "argocd".into(),
        mapping: Mapping {
            open: Some("cluster-b".into()),
            ..Default::default()
        },
        sources: sources.clone(),
    };
    let read = within(crate::changes::live::read_in(
        &pool,
        &at,
        crate::tests::observed_at(),
        crate::changes::live::BUDGET,
    ))
    .await;
    let projects = read.projects.read().unwrap();
    assert_eq!(projects[0].followed.len(), 2);
    assert!(projects[0].followed.values().all(Result::is_ok));
    assert!(started.elapsed() < crate::changes::live::BUDGET);
    assert_eq!(server.requests.lock().unwrap().len(), 6);
}
