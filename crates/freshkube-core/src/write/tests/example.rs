use super::*;

#[tokio::test(start_paused = true)]
async fn an_example_runner_shows_each_level_s_dialog_and_makes_no_request() {
    use Protection::*;
    for (level, asks, typed) in [(Guarded, true, false), (Protected, true, true)] {
        let (cluster, dir) = (Cluster::new(), tmp());
        let runner = Runner::example(
            cluster.client.clone(),
            AuditStore::new(dir.path().join("audit")),
            Who {
                entry: "prod-fra".into(),
                context: "ctx".into(),
                protection: level,
                actor: "tester".into(),
            },
        );
        let verdict = runner.preflight(fake::create_notes(&["stage-a"])).await;
        let Dialog::Ask(ask) = Dialog::for_verdict(&verdict, "prod-fra") else {
            panic!("{level:?} asks: {asks}");
        };
        assert_eq!(ask.type_name.is_some(), typed);
        let Verdict::Ready(prepared) = verdict else {
            panic!()
        };
        let finished = runner
            .execute(
                *prepared,
                Confirmation::Typed("stage-a".into()),
                || false,
                |_| {},
            )
            .await;
        assert_eq!(finished.status, Status::NotSent(NotSent::Example));
        assert!(finished.message().contains("Example data"));
        // Not a request of any kind, and nothing audited.
        assert!(cluster.recorder.requests().is_empty(), "{level:?}");
        assert!(phases(&AuditStore::new(dir.path().join("audit"))).is_empty());
    }
    // Read-only still blocks, as for any runner.
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = Runner::example(
        cluster.client.clone(),
        AuditStore::new(dir.path().join("audit")),
        Who {
            entry: "e".into(),
            context: "c".into(),
            protection: ReadOnly,
            actor: "t".into(),
        },
    );
    assert!(matches!(
        runner.preflight(fake::annotate_web()).await,
        Verdict::Blocked(Block::ReadOnly(_))
    ));
}

#[tokio::test(start_paused = true)]
async fn a_cancel_on_example_data_writes_nothing_to_any_audit_log() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = Runner::example(
        cluster.client.clone(),
        AuditStore::new(dir.path().join("audit")),
        Who {
            entry: "e".into(),
            context: "c".into(),
            protection: Protection::Guarded,
            actor: "t".into(),
        },
    );
    let prepared = ready(&runner, fake::delete_web()).await;
    // Cancelled during the countdown.
    let finished = runner
        .execute(*prepared, Confirmation::Confirmed, || true, |_| {})
        .await;
    assert_eq!(finished.status, Status::Cancelled);
    assert!(cluster.recorder.requests().is_empty());
    assert!(phases(&AuditStore::new(dir.path().join("audit"))).is_empty());
}

/// #983: example data's diff compares before and after, as a live diff does,
/// so a changed field reads Changed (`~`) and a new one Added (`+`); an object
/// the runner doesn't hold keeps naming every field as Added.
#[tokio::test(start_paused = true)]
async fn an_example_diff_compares_before_and_after_like_a_live_one() {
    use crate::write::diff::ChangeKind;
    use crate::write::testing::web;
    let (cluster, dir) = (Cluster::new(), tmp());
    let who = || Who {
        entry: "e".into(),
        context: "c".into(),
        protection: Protection::Guarded,
        actor: "t".into(),
    };
    let object = cluster.object(fake::DEPLOYMENT);
    let patch = |body: serde_json::Value| {
        let mut action = fake::set_image("registry.example/web:9");
        action.changes[0].target = web();
        action.changes[0].change = Change::MergePatch(body);
        action
    };
    let kinds = |verdict: Verdict| match verdict {
        Verdict::Ready(prepared) => prepared.diffs()[0]
            .changes
            .iter()
            .map(|change| (change.path.clone(), change.kind))
            .collect::<Vec<_>>(),
        Verdict::Blocked(block) => panic!("{block:?}"),
    };
    let runner = Runner::example(
        cluster.client.clone(),
        AuditStore::new(dir.path().join("audit")),
        who(),
    )
    .with_example_objects(vec![object.clone()]);
    let replicas = object["spec"]["replicas"].as_i64().unwrap();
    let body = json!({"spec": {"replicas": replicas + 3, "minReadySeconds": 7}});
    let seen = kinds(runner.preflight(patch(body.clone())).await);
    assert!(
        seen.contains(&("spec.replicas".into(), ChangeKind::Changed)),
        "{seen:?}"
    );
    assert!(
        seen.contains(&("spec.minReadySeconds".into(), ChangeKind::Added)),
        "{seen:?}"
    );
    // The same value is no change at all.
    let same = kinds(
        runner
            .preflight(patch(
                json!({"spec": {"replicas": replicas, "minReadySeconds": 7}}),
            ))
            .await,
    );
    assert_eq!(
        same,
        [("spec.minReadySeconds".to_owned(), ChangeKind::Added)]
    );
    // No object held: every field the action names, as Added.
    let bare = Runner::example(
        cluster.client.clone(),
        AuditStore::new(dir.path().join("audit")),
        who(),
    );
    let seen = kinds(bare.preflight(patch(body)).await);
    assert!(
        seen.iter().all(|(_, kind)| *kind == ChangeKind::Added),
        "{seen:?}"
    );
}
