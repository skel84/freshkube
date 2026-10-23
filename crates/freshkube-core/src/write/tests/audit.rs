use super::*;

#[tokio::test(start_paused = true)]
async fn the_sending_line_comes_first_and_if_it_cannot_be_written_nothing_is_sent() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let blocker = dir.path().join("audit");
    std::fs::write(&blocker, "a file where the directory should be").unwrap();
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::scale(3)).await;
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert!(matches!(
        finished.status,
        Status::NotSent(NotSent::AuditUnavailable(_))
    ));
    cluster.assert_nothing_written();
}

#[tokio::test(start_paused = true)]
async fn a_secret_is_diffed_and_audited_by_key_names_never_values() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::rotate_password()).await;
    let diff = &prepared.diffs()[0].changes;
    assert_eq!(diff[0].path, "data.password");
    run(&runner, prepared, Confirmation::Confirmed).await;
    let text = std::fs::read_to_string(dir.path().join("audit/writes.jsonl")).unwrap();
    assert!(text.contains("data.password"));
    for value in ["aHVudGVyMg", "bmV3cGFzc3dvcmQ", "hunter2"] {
        assert!(!text.contains(value), "{value} leaked into the audit log");
    }
}

#[tokio::test(start_paused = true)]
async fn the_audit_log_names_fields_and_context_and_holds_no_values() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::scale(3)).await;
    run(&runner, prepared, Confirmation::Confirmed).await;
    let read = AuditStore::new(dir.path().join("audit")).read().unwrap();
    let first = &read.records[0];
    assert_eq!(
        (first.entry.as_str(), first.context.as_str()),
        ("prod-fra", "ctx-a")
    );
    assert_eq!(
        (
            first.actor.as_str(),
            first.protection.as_str(),
            first.tier.as_str()
        ),
        ("tester", "guarded", "reversible")
    );
    assert_eq!(first.objects[0].fields, ["spec.replicas"]);
    assert_eq!(first.objects[0].uid, "uid-web");
    let runs: std::collections::BTreeSet<_> = read.records.iter().map(|r| &r.run).collect();
    assert_eq!(runs.len(), 1, "one write is one run");
}

#[tokio::test(start_paused = true)]
async fn the_audit_line_records_the_level_at_send_time() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let level = LevelCell::new(Protection::Guarded);
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded).following(level.clone());
    let prepared = ready(&runner, fake::scale(3)).await;
    // Looser now: the stricter requirement it was prepared under still
    // applies, and the line says what the level was when it was sent.
    level.set(Protection::Open);
    run(&runner, prepared, Confirmation::Confirmed).await;
    let read = AuditStore::new(dir.path().join("audit")).read().unwrap();
    assert!(
        read.records.iter().all(|r| r.protection == "open"),
        "{:?}",
        read.records
    );
}

#[tokio::test(start_paused = true)]
async fn an_error_that_echoes_a_value_reaches_neither_the_dialog_nor_the_audit_log() {
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.refuse_dry_runs(
        422,
        "Deployment.apps \"web\" is invalid: spec.template.spec.containers[0].env[0].value: Invalid value: \"hunter2-pass\": must not contain a secret",
    );
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let verdict = runner.preflight(fake::set_image("x")).await;
    let Dialog::Refused { text } = Dialog::for_verdict(&verdict, "e") else {
        panic!("{verdict:?}")
    };
    assert!(
        text.contains("env[0].value") && !text.contains("hunter2"),
        "{text}"
    );
    let log = std::fs::read_to_string(dir.path().join("audit/writes.jsonl")).unwrap();
    assert!(!log.contains("hunter2"), "{log}");
}
