use super::*;

#[tokio::test(start_paused = true)]
async fn a_refused_access_review_blocks_before_anything_else_is_asked() {
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.deny_access();
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    match runner.preflight(fake::scale(3)).await {
        Verdict::Blocked(Block::AccessDenied { verb, reason, .. }) => {
            assert_eq!(verb, "patch");
            assert!(reason.contains("RBAC"));
        }
        other => panic!("{other:?}"),
    }
    cluster
        .recorder
        .assert_lines(&["POST /apis/authorization.k8s.io/v1/selfsubjectaccessreviews"]);
}

#[tokio::test(start_paused = true)]
async fn an_access_review_that_fails_is_unknown_not_allowed() {
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.break_access_review();
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    assert!(matches!(
        runner.preflight(fake::scale(3)).await,
        Verdict::Blocked(Block::AccessUnknown { .. })
    ));
    cluster.assert_nothing_written();
}
