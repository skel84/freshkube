use super::*;
use crate::fake_api::Recorded;
use crate::write::delete::{self, Found, Impact};
use crate::write::everyday::Row;

// ---- #857: Delete, the destructive tier, with an impact preview ----

/// The row the list would have for the object served at `path`.
fn row_of(cluster: &Cluster, path: &str) -> Row {
    let object = cluster.object(path);
    let api_version = object["apiVersion"].as_str().unwrap();
    let (group, version) = api_version.split_once('/').unwrap_or(("", api_version));
    Row {
        group: group.into(),
        version: version.into(),
        kind: object["kind"].as_str().unwrap().into(),
        resource: path.rsplit('/').nth(1).unwrap().into(),
        namespace: object["metadata"]["namespace"].as_str().map(str::to_owned),
        name: object["metadata"]["name"].as_str().unwrap().into(),
        uid: object["metadata"]["uid"].as_str().unwrap().into(),
    }
}

async fn impact_of(cluster: &Cluster, path: &str) -> Impact {
    Impact::read(&cluster.client, &row_of(cluster, path))
        .await
        .unwrap()
}

fn deletes(cluster: &Cluster) -> Vec<Recorded> {
    cluster
        .recorder
        .requests()
        .into_iter()
        .filter(|r| r.method == http::Method::DELETE)
        .collect()
}

fn real_deletes(cluster: &Cluster) -> Vec<Recorded> {
    cluster.writes()
}

fn lines_of(action: &ActionDescription, title: &str) -> Vec<String> {
    action
        .preview
        .iter()
        .find(|section| section.title == title)
        .unwrap_or_else(|| panic!("no section {title}: {:?}", action.preview))
        .lines
        .clone()
}

#[tokio::test(start_paused = true)]
async fn delete_checks_access_then_dry_runs_then_sends_one_delete_with_its_preconditions() {
    let (cluster, dir) = (Cluster::with_owned(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let impact = impact_of(&cluster, fake::POD).await;
    let action = delete::delete(&impact, Propagation::Background).unwrap();
    assert_eq!(action.tier, Tier::Destructive);
    let prepared = ready(&runner, action).await;
    cluster.assert_nothing_written();
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Done);
    assert!(finished.undo.is_none(), "a delete can't be undone");
    assert!(!cluster.exists(fake::POD));

    let all = deletes(&cluster);
    assert_eq!(all.len(), 2, "{:?}", cluster.recorder.lines());
    // The dry run comes first, then the real one; both carry the guard and
    // the policy.
    assert!(all[0].body.contains("\"dryRun\""), "{}", all[0].body);
    assert!(!all[1].body.contains("\"dryRun\""), "{}", all[1].body);
    for sent in &all {
        assert!(
            sent.body.contains("\"uid\":\"uid-pod-x1\""),
            "{}",
            sent.body
        );
        assert!(
            sent.body.contains("\"resourceVersion\":\"700\""),
            "{}",
            sent.body
        );
        assert!(
            sent.body.contains("\"propagationPolicy\":\"Background\""),
            "{}",
            sent.body
        );
    }
    let lines = cluster.recorder.lines();
    let review = lines
        .iter()
        .position(|l| l.contains("selfsubjectaccessreviews"))
        .unwrap();
    let first_delete = lines.iter().position(|l| l.starts_with("DELETE")).unwrap();
    assert!(review < first_delete, "{lines:?}");
}

#[tokio::test(start_paused = true)]
async fn the_chosen_propagation_is_sent_in_the_dry_run_and_the_delete() {
    let (cluster, dir) = (Cluster::with_owned(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let impact = impact_of(&cluster, &format!("{}/web-a1", fake::REPLICASETS)).await;
    assert_eq!(impact.choices(), Propagation::ALL);
    let action = delete::delete(&impact, Propagation::Orphan).unwrap();
    let prepared = ready(&runner, action).await;
    run(&runner, prepared, Confirmation::Confirmed).await;
    let all = deletes(&cluster);
    assert_eq!(all.len(), 2);
    assert!(
        all.iter()
            .all(|sent| sent.body.contains("\"propagationPolicy\":\"Orphan\"")),
        "{all:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn safety_g17_delete_sends_nothing_on_read_only_and_not_for_an_object_that_moved() {
    // Read-only: refused before any request about the object is made.
    let (cluster, dir) = (Cluster::with_owned(), tmp());
    let impact = impact_of(&cluster, fake::POD).await;
    let before = cluster.recorder.lines().len();
    let runner = fake::runner(&cluster, dir.path(), Protection::ReadOnly);
    let action = delete::delete(&impact, Propagation::Background).unwrap();
    assert!(matches!(
        runner.preflight(action).await,
        Verdict::Blocked(Block::ReadOnly(_))
    ));
    assert_eq!(
        cluster.recorder.lines().len(),
        before,
        "read-only asks nothing"
    );
    assert!(deletes(&cluster).is_empty());

    // The object changed after the preview was read: another resourceVersion.
    let (cluster, dir) = (Cluster::with_owned(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let impact = impact_of(&cluster, fake::POD).await;
    cluster.foreign_write(
        fake::POD,
        "someone",
        &json!({"metadata": {"labels": {"a": "b"}}}),
    );
    let action = delete::delete(&impact, Propagation::Background).unwrap();
    assert!(matches!(
        runner.preflight(action).await,
        Verdict::Blocked(Block::Changed { .. })
    ));
    assert!(
        deletes(&cluster).is_empty(),
        "{:?}",
        cluster.recorder.lines()
    );
    assert!(cluster.exists(fake::POD));

    // The name belongs to another object now: another UID.
    let (cluster, dir) = (Cluster::with_owned(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let impact = impact_of(&cluster, fake::POD).await;
    cluster.recreate(fake::POD);
    let action = delete::delete(&impact, Propagation::Background).unwrap();
    assert!(matches!(
        runner.preflight(action).await,
        Verdict::Blocked(Block::Changed {
            what: ChangedWhat::Recreated,
            ..
        })
    ));
    assert!(deletes(&cluster).is_empty());
    assert!(cluster.exists(fake::POD));

    // A move between the dry run and the send is refused by the server's
    // preconditions: the real request is sent once and nothing is deleted.
    let (cluster, dir) = (Cluster::with_owned(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let impact = impact_of(&cluster, fake::POD).await;
    let action = delete::delete(&impact, Propagation::Background).unwrap();
    let prepared = ready(&runner, action).await;
    cluster.foreign_write(
        fake::POD,
        "someone",
        &json!({"metadata": {"labels": {"a": "b"}}}),
    );
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_ne!(finished.headline(), Headline::Done);
    assert!(cluster.exists(fake::POD));
}

#[tokio::test(start_paused = true)]
async fn safety_g18_nothing_is_sent_during_the_countdown_or_after_cancel() {
    // Cancel at any moment of the window: nothing is deleted.
    for cancel_at in [0u64, 2, 4] {
        let (cluster, dir) = (Cluster::with_owned(), tmp());
        let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
        let impact = impact_of(&cluster, fake::POD).await;
        let prepared = ready(
            &runner,
            delete::delete(&impact, Propagation::Background).unwrap(),
        )
        .await;
        let started = tokio::time::Instant::now();
        let finished = runner
            .execute(
                *prepared,
                Confirmation::Confirmed,
                || started.elapsed() >= Duration::from_secs(cancel_at),
                |_| {},
            )
            .await;
        assert_eq!(finished.status, Status::Cancelled, "{cancel_at}");
        assert!(real_deletes(&cluster).is_empty(), "{cancel_at}");
        assert!(cluster.exists(fake::POD));
    }

    // Not cancelled: the delete goes out only once the window has ended.
    let (cluster, dir) = (Cluster::with_owned(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let impact = impact_of(&cluster, fake::POD).await;
    let prepared = ready(
        &runner,
        delete::delete(&impact, Propagation::Background).unwrap(),
    )
    .await;
    let started = tokio::time::Instant::now();
    let early = std::sync::atomic::AtomicBool::new(false);
    let watched = cluster.clone();
    let finished = runner
        .execute(
            *prepared,
            Confirmation::Confirmed,
            || {
                if started.elapsed() < Duration::from_secs(5) && !watched.writes().is_empty() {
                    early.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                false
            },
            |_| {},
        )
        .await;
    assert_eq!(finished.headline(), Headline::Done);
    assert!(
        !early.load(std::sync::atomic::Ordering::SeqCst),
        "sent early"
    );
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert_eq!(real_deletes(&cluster).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn what_the_object_owns_is_read_one_level_deep_and_named() {
    let cluster = Cluster::with_owned();
    // A ReplicaSet owns its Pod: two reads, the object and the Pods.
    let impact = impact_of(&cluster, &format!("{}/web-a1", fake::REPLICASETS)).await;
    let lines = cluster.recorder.lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[1].contains("/pods") && lines[1].contains("limit=500"),
        "{lines:?}"
    );
    let action = delete::delete(&impact, Propagation::Background).unwrap();
    assert_eq!(
        lines_of(&action, "What it owns"),
        ["1 Pod owned by it:", "Pod/web-a1-x1"]
    );
    // A Deployment owns ReplicaSets; their Pods are one level further.
    let impact = impact_of(&cluster, fake::DEPLOYMENT).await;
    let action = delete::delete(&impact, Propagation::Background).unwrap();
    let owns = lines_of(&action, "What it owns");
    assert_eq!(
        owns[..2],
        ["1 ReplicaSet owned by it:", "ReplicaSet/web-a1"]
    );
    assert!(owns.last().unwrap().starts_with("Their Pods"), "{owns:?}");
}

#[tokio::test(start_paused = true)]
async fn the_preview_reads_are_bounded_and_the_list_is_capped() {
    let cluster = Cluster::with_owned();
    // Any other namespaced kind scans the three common kinds, one page each.
    let impact = impact_of(&cluster, fake::SECRET).await;
    let lists: Vec<String> = cluster
        .recorder
        .lines()
        .into_iter()
        .filter(|l| !l.contains(fake::SECRET))
        .collect();
    assert_eq!(lists.len(), 3, "{lists:?}");
    assert!(lists.len() <= delete::MOST_LISTS);
    assert!(
        lists
            .iter()
            .all(|l| l.starts_with("GET") && l.contains("limit=500"))
    );
    assert!(
        impact
            .dependents()
            .iter()
            .all(|d| matches!(d.found, Found::Read { total: 0, .. }))
    );

    // Twelve Pods owned: ten are named, the rest counted.
    let (rs_path, mut rs) = fake::replicaset("a1", 1, "registry.example/web:1", "uid-web");
    rs["metadata"]["uid"] = json!("uid-rs-a1");
    let mut objects = vec![(rs_path.clone(), rs)];
    for n in 0..12 {
        objects.push((
            format!("{}/web-a1-p{n:02}", fake::PODS),
            json!({"apiVersion": "v1", "kind": "Pod",
                "metadata": {"name": format!("web-a1-p{n:02}"), "namespace": "shop",
                    "uid": format!("uid-p{n}"), "resourceVersion": "1",
                    "ownerReferences": [{"kind": "ReplicaSet", "name": "web-a1",
                        "uid": "uid-rs-a1", "controller": true}]}}),
        ));
    }
    let cluster = Cluster::with(objects);
    let impact = impact_of(&cluster, &rs_path).await;
    let action = delete::delete(&impact, Propagation::Background).unwrap();
    let owns = lines_of(&action, "What it owns");
    assert_eq!(owns.len(), 1 + delete::SHOWN + 1, "{owns:?}");
    assert_eq!(owns[0], "12 Pod owned by it:");
    assert_eq!(owns.last().unwrap(), "and 2 more");
}

#[tokio::test(start_paused = true)]
async fn a_refused_list_is_unknown_and_never_none() {
    let cluster = Cluster::with_owned();
    let row = row_of(&cluster, fake::DEPLOYMENT);
    cluster.refuse_lists(403);
    let impact = Impact::read(&cluster.client, &row).await.unwrap();
    assert!(matches!(impact.dependents()[0].found, Found::Unknown(_)));
    // Unknown offers every policy, since what they touch is not known.
    assert_eq!(impact.choices(), Propagation::ALL);
    let action = delete::delete(&impact, Propagation::Background).unwrap();
    let owns = lines_of(&action, "What it owns");
    assert_eq!(owns.len(), 2, "{owns:?}");
    assert!(owns[0].starts_with("ReplicaSet: unknown"), "{owns:?}");
    assert!(!owns.join(" ").contains("No ReplicaSet"));
}

#[tokio::test(start_paused = true)]
async fn whether_it_comes_back_names_its_controller_and_the_gitops_tool() {
    let cluster = Cluster::with_owned();
    let impact = impact_of(&cluster, fake::POD).await;
    let action = delete::delete(&impact, Propagation::Background).unwrap();
    assert_eq!(
        lines_of(&action, "Will it come back"),
        ["ReplicaSet/web-a1 controls it and will usually make it again."]
    );
    // Nothing owns the claim: it stays deleted.
    let impact = impact_of(&cluster, fake::CLAIM).await;
    let action = delete::delete(&impact, Propagation::Background).unwrap();
    assert!(lines_of(&action, "Will it come back")[0].starts_with("Nothing in the cluster owns"));
    // Argo CD stamps the object.
    let row = row_of(&cluster, fake::CLAIM);
    let mut object = cluster.object(fake::CLAIM);
    object["metadata"]["annotations"] =
        json!({"argocd.argoproj.io/tracking-id": "shop:/PersistentVolumeClaim:shop/data"});
    let impact = Impact::new(&row, &object, vec![]).unwrap();
    let action = delete::delete(&impact, Propagation::Background).unwrap();
    assert!(lines_of(&action, "Will it come back")[0].starts_with("Argo CD manages it"));
    assert!(action.sentence.contains("Argo CD manages it"));
}

#[tokio::test(start_paused = true)]
async fn finalizers_are_named_and_only_the_default_policy_is_offered_when_nothing_is_owned() {
    let cluster = Cluster::with_owned();
    let impact = impact_of(&cluster, fake::CLAIM).await;
    assert_eq!(impact.choices(), [Propagation::Background]);
    assert!(delete::delete(&impact, Propagation::Orphan).is_err());
    let action = delete::delete(&impact, Propagation::Background).unwrap();
    let lines = lines_of(&action, "Finalizers");
    assert!(
        lines.contains(&"kubernetes.io/pvc-protection".to_owned()),
        "{lines:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn some_kinds_and_a_deleting_object_are_refused_with_a_reason() {
    for (group, kind) in [
        ("", "Namespace"),
        ("", "Node"),
        ("apiextensions.k8s.io", "CustomResourceDefinition"),
    ] {
        assert!(delete::refusal(group, kind).is_some(), "{kind}");
    }
    assert!(delete::refusal("apps", "Deployment").is_none());
    let cluster = Cluster::with_owned();
    let row = row_of(&cluster, fake::CLAIM);
    let mut object = cluster.object(fake::CLAIM);
    object["metadata"]["deletionTimestamp"] = json!("2026-10-24T09:00:00Z");
    let impact = Impact::new(&row, &object, vec![]).unwrap();
    let why = delete::delete(&impact, Propagation::Background).unwrap_err();
    assert_eq!(why.0, "It is already being deleted.");
    let namespace = Row {
        group: String::new(),
        version: "v1".into(),
        kind: "Namespace".into(),
        resource: "namespaces".into(),
        namespace: None,
        name: "shop".into(),
        uid: "uid-ns".into(),
    };
    let object = json!({"apiVersion": "v1", "kind": "Namespace",
        "metadata": {"name": "shop", "uid": "uid-ns", "resourceVersion": "1"}});
    let impact = Impact::new(&namespace, &object, vec![]).unwrap();
    assert!(delete::delete(&impact, Propagation::Background).is_err());
}

#[tokio::test(start_paused = true)]
async fn the_dialog_asks_the_name_on_protected_and_has_a_five_second_cancel_window() {
    let cluster = Cluster::with_owned();
    let impact = impact_of(&cluster, fake::POD).await;
    for (level, typed) in [
        (Protection::Protected, true),
        (Protection::Guarded, false),
        (Protection::Open, false),
    ] {
        let dir = tmp();
        let runner = fake::runner(&cluster, dir.path(), level);
        let action = delete::delete(&impact, Propagation::Background).unwrap();
        let Dialog::Ask(ask) = Dialog::for_verdict(&runner.preflight(action).await, "prod-fra")
        else {
            panic!("{level:?} asks");
        };
        assert_eq!(ask.type_name.is_some(), typed, "{level:?}");
        if typed {
            assert_eq!(ask.type_name.as_deref(), Some("web-a1-x1"));
        }
        assert_eq!(ask.cancel_window, Some(Duration::from_secs(5)));
        assert!(ask.danger);
        assert_eq!(ask.impact, ["Pod/web-a1-x1"]);
        assert_eq!(ask.preview.len(), 4);
    }
}

#[tokio::test(start_paused = true)]
async fn gone_recreated_and_terminating_are_told_apart() {
    // Gone.
    let (cluster, dir) = (Cluster::with_owned(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let impact = impact_of(&cluster, fake::POD).await;
    let prepared = ready(
        &runner,
        delete::delete(&impact, Propagation::Background).unwrap(),
    )
    .await;
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Done);

    // Deleted, and a controller made another under the same name.
    let (cluster, dir) = (Cluster::with_owned(), tmp());
    cluster.recreate_when_deleted(fake::POD);
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let impact = impact_of(&cluster, fake::POD).await;
    let prepared = ready(
        &runner,
        delete::delete(&impact, Propagation::Background).unwrap(),
    )
    .await;
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Recreated);
    assert!(
        finished.message().contains("new UID"),
        "{}",
        finished.message()
    );
    assert!(!finished.succeeded());

    // A finalizer keeps it: Terminating, with the finalizer named.
    let (cluster, dir) = (Cluster::with_owned(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let impact = impact_of(&cluster, fake::CLAIM).await;
    let prepared = ready(
        &runner,
        delete::delete(&impact, Propagation::Background).unwrap(),
    )
    .await;
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Terminating);
    assert!(
        finished.message().contains("kubernetes.io/pvc-protection"),
        "{}",
        finished.message()
    );
    assert!(cluster.exists(fake::CLAIM));
    // The audit line says so too, and never "failed".
    let last = AuditStore::new(dir.path().join("audit"))
        .read()
        .unwrap()
        .records
        .pop()
        .unwrap();
    assert!(last.result.contains("Terminating"), "{}", last.result);
}
