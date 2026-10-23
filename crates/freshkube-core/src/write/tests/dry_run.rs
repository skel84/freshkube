use super::*;

#[tokio::test(start_paused = true)]
async fn a_dry_run_the_cluster_refuses_blocks_and_is_audited_but_nothing_is_written() {
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.refuse_dry_runs(
        422,
        "admission webhook \"policy.example\" denied the request",
    );
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    match runner.preflight(fake::set_image("bad")).await {
        Verdict::Blocked(Block::DryRunFailed { message, .. }) => {
            assert!(message.contains("denied"))
        }
        other => panic!("{other:?}"),
    }
    cluster.assert_nothing_written();
    assert_eq!(
        phases(&AuditStore::new(dir.path().join("audit"))),
        [Phase::DryRunFailed]
    );
}

#[tokio::test(start_paused = true)]
async fn the_diff_of_a_spec_change_is_the_fields_that_move() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::set_image("registry.example/web:2")).await;
    let diff = &prepared.diffs()[0].changes;
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].path, "spec.template.spec.containers[0].image");
    assert_eq!(diff[0].kind, ChangeKind::Changed);
    // Paths and kinds only, never what the fields hold.
    assert!(!format!("{diff:?}").contains("registry.example"));
}
