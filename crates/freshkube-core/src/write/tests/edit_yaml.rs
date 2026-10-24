//! Edit YAML (#858) on the recording fake: what an edit reads, what it sends
//! and what it never touches.

use super::*;
use crate::write::edit_yaml::{EditError, EditSession, NOT_READ};
use crate::write::everyday::Row;

fn row(target: &Target) -> Row {
    Row {
        group: target.group.clone(),
        version: target.version.clone(),
        kind: target.kind.clone(),
        resource: target.resource.clone(),
        namespace: target.namespace.clone(),
        name: target.name.clone(),
        uid: target.uid.clone(),
    }
}

async fn open(cluster: &Cluster, target: &Target) -> EditSession {
    EditSession::read(&cluster.client, &row(target))
        .await
        .unwrap()
}

/// The text with a line added after the first line holding `after`.
fn with_line(text: &str, after: &str, line: &str) -> String {
    let mut out = vec![];
    let mut added = false;
    for each in text.lines() {
        out.push(each.to_owned());
        if !added && each.contains(after) {
            out.push(line.to_owned());
            added = true;
        }
    }
    out.join("\n") + "\n"
}

/// The session's text with one label added.
fn labelled(session: &EditSession, label: &str) -> String {
    let text = session.text();
    if text.contains("labels:") {
        with_line(&text, "labels:", &format!("    {label}"))
    } else {
        with_line(&text, "  name:", &format!("  labels:\n    {label}"))
    }
}

#[tokio::test(start_paused = true)]
async fn safety_g11_read_only_sends_an_edit_nothing_not_even_the_dry_run() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let object = cluster.object(fake::DEPLOYMENT);
    let session = EditSession::from_object(fake::web(), &object).unwrap();
    let action = session.describe(&labelled(&session, "a: \"1\"")).unwrap();
    let runner = fake::runner(&cluster, dir.path(), Protection::ReadOnly);
    assert!(matches!(
        runner.preflight(action).await,
        Verdict::Blocked(Block::ReadOnly(_))
    ));
    assert!(cluster.recorder.requests().is_empty());
}

#[tokio::test(start_paused = true)]
async fn an_edit_is_a_spec_change_asked_at_every_writable_level() {
    for (level, name) in [
        (Protection::Open, false),
        (Protection::Guarded, false),
        (Protection::Protected, true),
    ] {
        let (cluster, dir) = (Cluster::new(), tmp());
        let session = open(&cluster, &fake::web()).await;
        let action = session.describe(&labelled(&session, "a: \"1\"")).unwrap();
        assert_eq!(action.tier, Tier::SpecChange);
        let runner = fake::runner(&cluster, dir.path(), level);
        let prepared = ready(&runner, action).await;
        let requirement = prepared.requirement();
        assert!(requirement.confirm && requirement.diff, "{level:?}");
        assert_eq!(requirement.type_name, name, "{level:?}");
        cluster.assert_nothing_written();
    }
}

#[tokio::test(start_paused = true)]
async fn an_object_that_moved_since_it_was_opened_blocks_the_edit() {
    for recreated in [false, true] {
        let (cluster, dir) = (Cluster::new(), tmp());
        let session = open(&cluster, &fake::web()).await;
        let action = session.describe(&labelled(&session, "a: \"1\"")).unwrap();
        if recreated {
            cluster.recreate(fake::DEPLOYMENT);
        } else {
            cluster.foreign_write(
                fake::DEPLOYMENT,
                "kubectl",
                &json!({"spec": {"paused": true}}),
            );
        }
        let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
        let want = if recreated {
            ChangedWhat::Recreated
        } else {
            ChangedWhat::Modified
        };
        match runner.preflight(action).await {
            Verdict::Blocked(Block::Changed { what, .. }) => assert_eq!(what, want),
            other => panic!("{other:?}"),
        }
        assert!(
            cluster
                .recorder
                .lines()
                .iter()
                .all(|l| !l.contains("dryRun"))
        );
        cluster.assert_nothing_written();
    }
}

#[tokio::test(start_paused = true)]
async fn the_review_names_field_paths_and_kinds_and_no_value() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let session = open(&cluster, &fake::web()).await;
    let text = session.text().replace("replicas: 2", "replicas: 41");
    let text = text.replace("web:1", "web:77-unique");
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, session.describe(&text).unwrap()).await;
    let diffs = format!("{:?}", prepared.diffs());
    assert!(diffs.contains("spec.replicas"), "{diffs}");
    for value in ["41", "web:77-unique"] {
        assert!(!diffs.contains(value), "{value} is in the diff: {diffs}");
    }
}

#[tokio::test(start_paused = true)]
async fn an_edit_sends_server_side_apply_as_freshkube_with_the_version_and_never_force() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let session = open(&cluster, &fake::web()).await;
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let action = session.describe(&labelled(&session, "a: \"1\"")).unwrap();
    let prepared = ready(&runner, action).await;
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert!(finished.succeeded(), "{:?}", finished.headline());
    let writes = cluster.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(writes[0].method, "PATCH");
    assert!(writes[0].content_type.contains("apply-patch"));
    assert!(writes[0].query.contains("fieldManager=freshkube"));
    assert!(!writes[0].query.contains("force"));
    assert!(writes[0].body.contains("\"resourceVersion\":\"100\""));
    assert_eq!(
        cluster.object(fake::DEPLOYMENT)["metadata"]["labels"]["a"],
        "1"
    );
    assert!(finished.undo.is_none(), "a spec change has no Undo");
}

#[tokio::test(start_paused = true)]
async fn safety_g11_an_edit_never_sends_status_managed_fields_or_the_servers_own_fields() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let session = open(&cluster, &fake::web()).await;
    assert!(!session.text().contains("managedFields"));
    assert!(!session.text().contains("readyReplicas"));
    let text = with_line(&session.text(), "metadata:", "  generation: 9");
    let text = format!("{text}status:\n  readyReplicas: 99\n");
    let text = with_line(&text, "metadata:", "  managedFields:\n  - manager: sneaky");
    let text = text.replace("replicas: 2", "replicas: 3");
    let action = session.describe(&text).unwrap();
    let notes: Vec<_> = action
        .preview
        .iter()
        .flat_map(|p| p.lines.clone())
        .collect();
    for ignored in [
        "status.readyReplicas",
        "metadata.generation",
        "metadata.managedFields",
    ] {
        assert!(
            notes.iter().any(|line| line.starts_with(ignored)),
            "{notes:?}"
        );
    }
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, action).await;
    run(&runner, prepared, Confirmation::Confirmed).await;
    let writes = cluster.writes();
    for hidden in [
        "status",
        "managedFields",
        "generation",
        "sneaky",
        "readyReplicas",
    ] {
        assert!(
            !writes[0].body.contains(hidden),
            "{hidden} was sent: {}",
            writes[0].body
        );
    }
    assert_eq!(cluster.object(fake::DEPLOYMENT)["spec"]["replicas"], 3);
    // Only a locked field changed: nothing to send.
    let only = format!("{}status:\n  readyReplicas: 5\n", session.text());
    assert_eq!(
        session.describe(&only).unwrap_err(),
        EditError::NothingChanged
    );
}

#[tokio::test(start_paused = true)]
async fn an_edit_cannot_change_the_objects_identity() {
    let (cluster, _dir) = (Cluster::new(), tmp());
    let session = open(&cluster, &fake::web()).await;
    for (from, to, field) in [
        ("name: web\n", "name: other\n", "metadata.name"),
        (
            "namespace: shop",
            "namespace: kube-system",
            "metadata.namespace",
        ),
        ("kind: Deployment", "kind: StatefulSet", "kind"),
    ] {
        let text = session.text().replacen(from, to, 1);
        assert_eq!(
            session.describe(&text).unwrap_err(),
            EditError::Identity(field)
        );
    }
    assert!(matches!(
        session.describe("- a\n- b\n"),
        Err(EditError::NotAnObject)
    ));
    assert!(matches!(session.describe("a: [b"), Err(EditError::Yaml(_))));
}

#[tokio::test(start_paused = true)]
async fn a_conflict_names_its_manager_and_fields_and_is_not_retried() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let session = open(&cluster, &fake::web()).await;
    let text = session.text().replace("replicas: 2", "replicas: 3");
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, session.describe(&text).unwrap()).await;
    cluster.refuse_writes(
        409,
        "Apply failed with 1 conflict: conflict with \"hpa\" using apps/v1: .spec.replicas",
    );
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Conflict);
    match &finished.objects[0].step {
        Step::Conflict(conflict) => {
            assert_eq!(conflict.managers, ["hpa"]);
            assert_eq!(conflict.fields, [".spec.replicas"]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(cluster.writes().len(), 1);
    assert!(cluster.writes().iter().all(|w| !w.query.contains("force")));
}

#[tokio::test(start_paused = true)]
async fn safety_g11_an_edit_after_an_edit_keeps_what_the_first_one_set() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    for label in ["a: \"1\"", "b: \"2\""] {
        // Each edit is read anew, as the pane does when it is opened.
        let session = open(&cluster, &fake::web()).await;
        let action = session.describe(&labelled(&session, label)).unwrap();
        let prepared = ready(&runner, action).await;
        assert!(
            run(&runner, prepared, Confirmation::Confirmed)
                .await
                .succeeded()
        );
    }
    // Server-side apply drops what this manager owned and then left out, so
    // the second body had to carry the first edit's field.
    let labels = &cluster.object(fake::DEPLOYMENT)["metadata"]["labels"];
    assert_eq!(
        (labels["a"].as_str(), labels["b"].as_str()),
        (Some("1"), Some("2"))
    );
    // And it did not take over the fields it never set.
    let second = &cluster.writes()[1].body;
    assert!(
        !second.contains("replicas") && !second.contains("containers"),
        "{second}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_deleted_line_freshkube_owns_goes_and_one_another_manager_owns_stays_and_is_said() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let session = open(&cluster, &fake::web()).await;
    let action = session.describe(&labelled(&session, "a: \"1\"")).unwrap();
    let prepared = ready(&runner, action).await;
    run(&runner, prepared, Confirmation::Confirmed).await;
    cluster.foreign_write(
        fake::DEPLOYMENT,
        "kubectl",
        &json!({"spec": {"paused": true}}),
    );
    // Delete the label freshkube owns and the paused field another manager owns.
    let session = open(&cluster, &fake::web()).await;
    let text: String = session
        .text()
        .lines()
        .filter(|line| !line.contains("a: '1'") && !line.contains("paused"))
        .map(|line| format!("{line}\n"))
        .collect();
    let action = session.describe(&text).unwrap();
    let kept: Vec<_> = action
        .preview
        .iter()
        .flat_map(|p| p.lines.clone())
        .collect();
    assert_eq!(
        kept,
        ["spec.paused"],
        "only the other manager's field is said to stay"
    );
    let prepared = ready(&runner, action).await;
    run(&runner, prepared, Confirmation::Confirmed).await;
    let object = cluster.object(fake::DEPLOYMENT);
    assert!(object["metadata"]["labels"].get("a").is_none());
    assert_eq!(object["spec"]["paused"], true);
}

#[tokio::test(start_paused = true)]
async fn safety_g11_a_secret_is_read_as_metadata_and_no_value_is_asked_sent_or_kept() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let session = open(&cluster, &fake::db()).await;
    let text = session.text();
    assert!(text.contains(&format!("data: {NOT_READ}")), "{text}");
    assert!(!text.contains("aHVudGVyMg") && !text.contains("hunter2"));
    assert!(session.locked().contains(&"data"));
    // Its values can't be edited: a value written there is not applied.
    let typed = text.replace(&format!("data: {NOT_READ}"), "data:\n  password: bmV3");
    assert_eq!(
        session.describe(&typed).unwrap_err(),
        EditError::NothingChanged
    );
    let action = session
        .describe(&labelled(&session, "owner: \"ops\""))
        .unwrap();
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, action).await;
    let diffs = format!("{:?}", prepared.diffs());
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert!(finished.succeeded());
    // Every read of the Secret asked for its metadata alone.
    for request in cluster
        .recorder
        .requests()
        .iter()
        .filter(|r| r.path == fake::SECRET && r.method == "GET")
    {
        assert!(
            request.accept.contains("as=PartialObjectMetadata"),
            "a full read of a Secret: {request:?}"
        );
    }
    // The server's replies carry the values; none reached anything we hold.
    let everything = format!("{diffs}{finished:?}{:?}", cluster.writes());
    for value in ["aHVudGVyMg", "hunter2", "bmV3"] {
        assert!(!everything.contains(value), "{value} leaked: {everything}");
    }
    let audit = std::fs::read_to_string(dir.path().join("audit/writes.jsonl")).unwrap();
    assert!(!audit.contains("aHVudGVyMg"));
    // The Secret's data is untouched.
    assert_eq!(
        cluster.object(fake::SECRET)["data"]["password"],
        "aHVudGVyMg=="
    );
}

#[tokio::test(start_paused = true)]
async fn the_gitops_warning_names_the_source_to_change() {
    let cluster = Cluster::new();
    let mut object = cluster.object(fake::DEPLOYMENT);
    object["metadata"]["annotations"] = json!({
        "argocd.argoproj.io/tracking-id": "ledger:apps/Deployment:shop/web"});
    let session = EditSession::from_object(fake::web(), &object).unwrap();
    let action = session
        .describe(&session.text().replace("replicas: 2", "replicas: 3"))
        .unwrap();
    assert!(
        action.sentence.contains("application ledger"),
        "{}",
        action.sentence
    );
    assert!(
        action.sentence.contains("change the source"),
        "{}",
        action.sentence
    );
}

#[tokio::test(start_paused = true)]
async fn safety_g11_the_reply_to_a_secrets_write_is_dropped_of_its_values_whoever_asked() {
    for dry_run in [true, false] {
        for change in [
            Change::Apply(json!({"apiVersion": "v1", "kind": "Secret",
                "metadata": {"name": "db", "namespace": "shop", "labels": {"x": "1"}}})),
            Change::MergePatch(json!({"metadata": {"labels": {"y": "1"}}})),
        ] {
            let cluster = Cluster::new();
            let reply = crate::write::send::send(&cluster.client, &fake::db(), &change, dry_run)
                .await
                .unwrap();
            assert!(
                reply.get("data").is_none() && reply.get("stringData").is_none(),
                "{reply}"
            );
            assert!(!reply.to_string().contains("aHVudGVyMg"));
        }
    }
}
