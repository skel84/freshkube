use super::*;
use crate::write::everyday::{self, Revisions, Row, Subject};

// ---- #857: Roll back, a spec change on a Deployment's pod template ----

fn web_row() -> Row {
    Row {
        group: "apps".into(),
        version: "v1".into(),
        kind: "Deployment".into(),
        resource: "deployments".into(),
        namespace: Some("shop".into()),
        name: "web".into(),
        uid: "uid-web".into(),
    }
}

async fn read(cluster: &Cluster) -> Subject {
    let subject = Subject::read(&cluster.client, &web_row()).await.unwrap();
    subject.read_revisions(&cluster.client).await
}

fn template(cluster: &Cluster) -> serde_json::Value {
    cluster.object(fake::DEPLOYMENT)["spec"]["template"].clone()
}

#[tokio::test(start_paused = true)]
async fn the_revisions_are_the_replicasets_the_deployment_owns_newest_first() {
    let cluster = Cluster::with_history();
    let subject = read(&cluster).await;
    let Revisions::Read(revisions) = subject.revisions() else {
        panic!("{:?}", subject.revisions());
    };
    // The one another Deployment owns (9) and the one with no revision
    // annotation are not revisions of this one.
    let seen: Vec<_> = revisions
        .iter()
        .map(|r| (r.number, r.current, r.images.clone()))
        .collect();
    assert_eq!(
        seen,
        [
            (3, true, vec!["registry.example/web:3".to_owned()]),
            (2, false, vec!["registry.example/web:2".to_owned()]),
            (1, false, vec!["registry.example/web:1".to_owned()]),
        ]
    );
    assert!(revisions.iter().all(|r| r.created.is_some()));
}

#[tokio::test(start_paused = true)]
async fn replicasets_that_cannot_be_read_leave_roll_back_off() {
    // No ReplicaSet exists in the plain fake: the list is a 404.
    let cluster = Cluster::new();
    let subject = read(&cluster).await;
    assert!(matches!(subject.revisions(), Revisions::Failed(_)));
    let everyday::NotOffered(why) = everyday::rollback(&subject, 1).unwrap_err();
    assert!(why.contains("could not be read"), "{why}");
}

#[tokio::test(start_paused = true)]
async fn roll_back_sends_one_merge_patch_that_makes_the_template_the_revisions() {
    let (cluster, dir) = (Cluster::with_history(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let action = everyday::rollback(&read(&cluster).await, 1).unwrap();
    assert_eq!(action.tier, Tier::SpecChange);
    let prepared = ready(&runner, action).await;
    cluster.assert_nothing_written();
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Done);
    assert!(finished.undo.is_none(), "a spec change has no Undo");
    let writes = cluster.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert!(writes[0].content_type.contains("merge-patch"));
    assert!(
        writes[0].query.contains("fieldManager=freshkube") && !writes[0].query.contains("force")
    );
    // Revision 1's template, without the controller's hash label.
    let now = template(&cluster);
    assert_eq!(
        now["spec"]["containers"][0]["image"],
        "registry.example/web:1"
    );
    assert_eq!(now["spec"]["containers"][0]["env"][0]["name"], "TOKEN");
    assert!(now["metadata"]["labels"].get("pod-template-hash").is_none());
}

#[tokio::test(start_paused = true)]
async fn a_key_the_revision_lacks_is_removed_not_left_behind() {
    let (cluster, dir) = (Cluster::with_history(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Open);
    for revision in [1, 2] {
        let action = everyday::rollback(&read(&cluster).await, revision).unwrap();
        let prepared = ready(&runner, action).await;
        let finished = run(&runner, prepared, Confirmation::Confirmed).await;
        assert_eq!(finished.headline(), Headline::Done, "revision {revision}");
    }
    // Revision 1 had an env var and revision 2 has none: a merge patch that
    // only listed what 2 has would have kept it.
    let container = template(&cluster)["spec"]["containers"][0].clone();
    assert_eq!(container["image"], "registry.example/web:2");
    assert!(container.get("env").is_none(), "{container}");
}

#[tokio::test(start_paused = true)]
async fn the_dry_run_shows_changed_paths_and_never_a_value() {
    let (cluster, dir) = (Cluster::with_history(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    // From revision 3 (no env) to revision 1 (an env var with a value).
    let action = everyday::rollback(&read(&cluster).await, 1).unwrap();
    let verdict = runner.preflight(action).await;
    let Verdict::Ready(prepared) = &verdict else {
        panic!("{verdict:?}");
    };
    let paths: Vec<_> = prepared.diffs()[0]
        .changes
        .iter()
        .map(|change| (change.path.as_str(), change.kind))
        .collect();
    assert!(paths.contains(&(
        "spec.template.spec.containers[0].image",
        ChangeKind::Changed
    )));
    assert!(paths.contains(&(
        "spec.template.spec.containers[0].env[0].name",
        ChangeKind::Added
    )));
    // The patch names the whole containers list, but the diff compares the
    // object before and after: a container field that stays the same is
    // not listed.
    assert!(
        !paths
            .iter()
            .any(|(path, _)| path.ends_with("containers[0].name")),
        "{paths:?}"
    );
    let Dialog::Ask(ask) = Dialog::for_verdict(&verdict, "prod-fra") else {
        panic!("a spec change asks");
    };
    assert!(!ask.diff.is_empty() && ask.type_name.is_none());
    let shown = format!("{:?} {}", ask.diff, ask.sentence);
    for value in ["web:1", "web:3", "s3cr3t-old", "registry.example"] {
        assert!(!shown.contains(value), "{value} in {shown}");
    }
    cluster.assert_nothing_written();
}

#[tokio::test(start_paused = true)]
async fn safety_g02_roll_back_asks_the_typed_name_when_protected_and_shows_the_diff_even_when_open()
{
    let (cluster, dir) = (Cluster::with_history(), tmp());
    let action = everyday::rollback(&read(&cluster).await, 2).unwrap();
    for (level, typed) in [(Protection::Protected, true), (Protection::Open, false)] {
        let given = requirement_for(&action, level);
        assert!(given.confirm && given.diff && !given.undo, "{level:?}");
        assert_eq!(given.type_name, typed, "{level:?}");
    }
    let runner = fake::runner(&cluster, dir.path(), Protection::Protected);
    let prepared = ready(&runner, action).await;
    assert_eq!(prepared.expected_name("prod-fra"), "web");
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert!(matches!(finished.status, Status::NotSent(_)));
    cluster.assert_nothing_written();
}

#[tokio::test(start_paused = true)]
async fn a_deployment_that_changed_since_it_was_read_blocks_and_sends_nothing() {
    let (cluster, dir) = (Cluster::with_history(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let action = everyday::rollback(&read(&cluster).await, 1).unwrap();
    cluster.foreign_write(
        fake::DEPLOYMENT,
        "kubectl",
        &json!({"spec": {"replicas": 5}}),
    );
    assert!(matches!(
        runner.preflight(action).await,
        Verdict::Blocked(Block::Changed {
            what: ChangedWhat::Modified,
            ..
        })
    ));
    cluster.assert_nothing_written();
}

#[tokio::test(start_paused = true)]
async fn safety_g01_roll_back_on_a_read_only_cluster_sends_no_request_and_a_denied_review_sends_nothing()
 {
    let (cluster, dir) = (Cluster::with_history(), tmp());
    let action = everyday::rollback(&read(&cluster).await, 1).unwrap();
    let before = cluster.recorder.requests().len();
    let runner = fake::runner(&cluster, dir.path(), Protection::ReadOnly);
    assert!(matches!(
        runner.preflight(action.clone()).await,
        Verdict::Blocked(Block::ReadOnly(_))
    ));
    assert_eq!(
        cluster.recorder.requests().len(),
        before,
        "no request at all"
    );
    cluster.deny_access();
    let runner = fake::runner(&cluster, dir.path(), Protection::Open);
    assert!(matches!(
        runner.preflight(action).await,
        Verdict::Blocked(Block::AccessDenied { .. })
    ));
    cluster.assert_nothing_written();
}

#[tokio::test(start_paused = true)]
async fn it_refuses_what_cannot_be_rolled_back_to() {
    let cluster = Cluster::with_history();
    let subject = read(&cluster).await;
    let why = |result: Result<_, everyday::NotOffered>| match result {
        Err(everyday::NotOffered(why)) => why,
        Ok(_) => panic!("offered"),
    };
    // The revision it runs now, one it does not have, and one another
    // Deployment owns.
    assert_eq!(
        why(everyday::rollback(&subject, 3)),
        "It is already at this revision."
    );
    assert_eq!(
        why(everyday::rollback(&subject, 7)),
        "It has no such revision any more."
    );
    assert_eq!(
        why(everyday::rollback(&subject, 9)),
        "It has no such revision any more."
    );
    // An object made from a document has read no revisions.
    let unread = Subject::from_object(&cluster.object(fake::DEPLOYMENT)).unwrap();
    assert_eq!(
        why(everyday::rollback(&unread, 1)),
        "Its revisions were not read."
    );
    // A paused one, as `kubectl rollout undo` refuses it.
    cluster.foreign_write(
        fake::DEPLOYMENT,
        "kubectl",
        &json!({"spec": {"paused": true}}),
    );
    let paused = read(&cluster).await;
    assert!(why(everyday::rollback(&paused, 1)).contains("paused"));
    // Not a Deployment.
    let cronjob = Subject::from_object(&cluster.object(fake::CRONJOB)).unwrap();
    assert!(everyday::rollback(&cronjob, 1).is_err());
}

#[tokio::test(start_paused = true)]
async fn a_template_equal_to_the_revisions_is_already_there_even_when_the_number_differs() {
    let (cluster, dir) = (Cluster::with_history(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Open);
    let action = everyday::rollback(&read(&cluster).await, 2).unwrap();
    let prepared = ready(&runner, action).await;
    run(&runner, prepared, Confirmation::Confirmed).await;
    // The Deployment's revision annotation still says 3, but its template
    // is revision 2's now: rolling back to 2 again changes nothing.
    let everyday::NotOffered(why) = everyday::rollback(&read(&cluster).await, 2).unwrap_err();
    assert_eq!(why, "It is already at this revision.");
}

#[tokio::test(start_paused = true)]
async fn a_gitops_managed_deployment_adds_the_warning() {
    let cluster = Cluster::with_history();
    cluster.foreign_write(
        fake::DEPLOYMENT,
        "argocd",
        &json!({"metadata": {"annotations": {"argocd.argoproj.io/tracking-id": "shop:web"}}}),
    );
    let action = everyday::rollback(&read(&cluster).await, 1).unwrap();
    assert!(
        action.sentence.contains("Argo CD manages this object"),
        "{}",
        action.sentence
    );
    assert!(action.sentence.contains("OutOfSync"));
}

#[tokio::test(start_paused = true)]
async fn safety_g12_roll_back_to_a_template_that_changed_since_it_was_read_sends_nothing() {
    let (cluster, dir) = (Cluster::with_history(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let action = everyday::rollback(&read(&cluster).await, 2).unwrap();
    // A Restart adds an annotation to the template after the page read it:
    // the patch worked out from the old copy would not remove it.
    cluster.foreign_write(
        fake::DEPLOYMENT,
        "kubectl",
        &json!({"spec": {"template": {"metadata": {"annotations": {"restartedAt": "2026-10-24"}}}}}),
    );
    let blocked = runner.preflight(action.clone()).await;
    assert!(
        matches!(
            blocked,
            Verdict::Blocked(Block::Changed {
                what: ChangedWhat::Moved,
                ..
            })
        ),
        "{blocked:?}"
    );
    // Review the new version takes the new resourceVersion and no more: the
    // template still differs, so the patch is still refused.
    let rebased = runner.review_new_version(action).await.unwrap();
    assert!(matches!(
        runner.preflight(rebased).await,
        Verdict::Blocked(Block::Changed {
            what: ChangedWhat::Moved,
            ..
        })
    ));
    cluster.assert_nothing_written();
}

#[tokio::test(start_paused = true)]
async fn a_roll_back_makes_these_requests_in_this_order() {
    let (cluster, dir) = (Cluster::with_history(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let action = everyday::rollback(&read(&cluster).await, 1).unwrap();
    let before = cluster.recorder.lines().len();
    let prepared = ready(&runner, action).await;
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Done);
    let deployment = "/apis/apps/v1/namespaces/shop/deployments/web";
    let lines = cluster.recorder.lines();
    assert_eq!(
        lines[before..],
        [
            "POST /apis/authorization.k8s.io/v1/selfsubjectaccessreviews".to_owned(),
            format!("GET {deployment}"),
            format!("GET {deployment}"),
            format!("PATCH {deployment}?&dryRun=All&fieldManager=freshkube"),
            format!("GET {deployment}"),
            format!("PATCH {deployment}?&fieldManager=freshkube"),
            format!("GET {deployment}"),
        ]
    );
}
