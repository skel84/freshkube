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
