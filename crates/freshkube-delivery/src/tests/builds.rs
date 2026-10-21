use super::*;

#[tokio::test]
async fn a_busy_commit_has_only_its_first_builds_task_runs_read() {
    use super::tekton::MOST_TASK_READS;
    let runs: Vec<serde_json::Value> = (0..MOST_TASK_READS + 2)
        .map(|n| {
            let mut run = pipeline_run(SHA, true, Some(NEW));
            run["metadata"]["name"] = serde_json::json!(format!("storefront-push-{n}"));
            run
        })
        .collect();
    let tekton = FixtureReader::default()
        .serves("tekton.dev", "v1", &["pipelineruns", "taskruns"])
        .with("pipelineruns", runs)
        .with("taskruns", vec![task_run()]);
    let read = read_builds(&tekton, Some("acme-builds"), SHA).await;
    let builds = read.read().expect("read");
    assert_eq!(builds.len(), MOST_TASK_READS + 2, "every build is kept");
    assert!(
        builds[..MOST_TASK_READS]
            .iter()
            .all(|b| b.tasks_unread.is_none())
    );
    assert!(
        builds[MOST_TASK_READS..].iter().all(|b| b
            .tasks_unread
            .as_deref()
            .is_some_and(|why| why.contains("first"))),
        "{:?}",
        builds[MOST_TASK_READS].tasks_unread
    );
    let task_lists = tekton
        .requests
        .borrow()
        .iter()
        .filter(|r| r.contains("taskruns"))
        .count();
    assert!(task_lists <= MOST_TASK_READS, "{task_lists}");
}

#[tokio::test]
async fn an_older_tekton_version_is_used_when_it_is_all_that_is_served() {
    let world = healthy();
    let tekton = FixtureReader::default()
        .serves("tekton.dev", "v1beta1", &["pipelineruns", "taskruns"])
        .with("pipelineruns", vec![pipeline_run(SHA, true, Some(NEW))]);
    let read = read_builds(&tekton, Some("acme-builds"), SHA).await;
    assert_eq!(read.read().map(Vec::len), Some(1));
    assert!(
        tekton
            .requests
            .borrow()
            .iter()
            .any(|r| r.contains("tekton.dev/v1beta1/pipelineruns"))
    );
    drop(world);
}

#[tokio::test]
async fn task_runs_that_cannot_be_read_leave_the_build_standing() {
    let mut world = healthy();
    world.tekton = world.tekton.refusing("taskruns");
    let trail = run(&world, &ENV).await;
    let commit = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(commit.confidence, Confidence::Claimed);
    assert!(
        commit.reason.contains("its TaskRuns were not read"),
        "{}",
        commit.reason
    );
    let supply = one(&trail, Hop::PipelineRun, Hop::SupplyChain);
    assert!(
        supply
            .reason
            .contains("its TaskRuns were not read (forbidden by RBAC)"),
        "{}",
        supply.reason
    );
    assert_eq!(
        one(&trail, Hop::PipelineRun, Hop::Freight).confidence,
        Confidence::Confirmed,
        "the run's own image result still joins"
    );
}

/// Two builds of the same commit, `storefront-push-x` and
/// `storefront-push-y`, each with its own TaskRun.
fn two_builds() -> World {
    let mut second = pipeline_run(SHA, true, Some(NEW));
    second["metadata"]["name"] = serde_json::json!("storefront-push-y");
    second["status"]["childReferences"][0]["name"] = serde_json::json!("storefront-push-y-verify");
    let mut second_task = task_run();
    second_task["metadata"]["name"] = serde_json::json!("storefront-push-y-verify");
    second_task["metadata"]["labels"]["tekton.dev/pipelineRun"] =
        serde_json::json!("storefront-push-y");
    let mut world = healthy();
    world.tekton = world
        .tekton
        .with(
            "pipelineruns",
            vec![pipeline_run(SHA, true, Some(NEW)), second],
        )
        .with(
            "taskruns",
            vec![
                task_run(),
                clone_task_run("storefront-push-x", SHA),
                second_task,
            ],
        );
    world
}

fn of<'a>(links: &[&'a super::join::Link], run: &str) -> &'a super::join::Link {
    let subject = format!("acme-builds/{run}");
    let found: Vec<_> = links.iter().filter(|l| l.subject == subject).collect();
    assert_eq!(found.len(), 1, "{subject} in {links:#?}");
    found[0]
}

#[tokio::test]
async fn one_builds_refused_task_runs_leave_the_other_build_whole() {
    let mut world = two_builds();
    world.tekton = world
        .tekton
        .refusing_selector("taskruns", "tekton.dev/pipelineRun=storefront-push-y");

    let read = read_builds(&world.tekton, Some("acme-builds"), SHA).await;
    let Source::Read(builds) = &read else {
        panic!("the builds were read: {read:?}");
    };
    assert_eq!(builds.len(), 2);
    let read_one = builds
        .iter()
        .find(|b| b.run.name == "storefront-push-x")
        .unwrap();
    assert_eq!(read_one.tasks.len(), 2);
    assert_eq!(read_one.tasks_unread, None);
    let refused = builds
        .iter()
        .find(|b| b.run.name == "storefront-push-y")
        .unwrap();
    assert!(refused.tasks.is_empty());
    assert_eq!(refused.tasks_unread.as_deref(), Some("forbidden by RBAC"));

    let trail = run(&world, &ENV).await;
    let commits = link(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(commits.len(), 2, "{trail:#?}");
    let x = of(&commits, "storefront-push-x");
    // x's TaskRuns were read whole, and its clone result reports the commit.
    assert_eq!(x.confidence, Confidence::Confirmed);
    assert!(!x.reason.contains("not read"), "{}", x.reason);
    let y = of(&commits, "storefront-push-y");
    assert_eq!(
        y.confidence,
        Confidence::Claimed,
        "the run's own revision is declared, so it only claims"
    );
    assert!(
        y.reason
            .contains("its TaskRuns were not read (forbidden by RBAC)"),
        "{}",
        y.reason
    );

    let supply = link(&trail, Hop::PipelineRun, Hop::SupplyChain);
    let x = of(&supply, "storefront-push-x");
    assert!(
        x.reason
            .contains("Conforma SUCCESS (0 failures, 1 warnings)"),
        "the other build's TaskRun result still joins: {}",
        x.reason
    );
    assert!(!x.reason.contains("not read"), "{}", x.reason);
    let y = of(&supply, "storefront-push-y");
    assert!(
        y.reason
            .contains("its TaskRuns were not read (forbidden by RBAC), so their results are unknown; no Conforma result"),
        "{}",
        y.reason
    );
    assert!(!y.reason.contains("Conforma FAILURE"), "{}", y.reason);
}

#[tokio::test]
async fn tekton_is_discovered_once_for_every_build_it_reads() {
    let world = two_builds();
    let read = read_builds(&world.tekton, Some("acme-builds"), SHA).await;
    assert_eq!(read.read().map(Vec::len), Some(2));
    let requests = world.tekton.requests.borrow();
    let discovery: Vec<&str> = requests
        .iter()
        .map(String::as_str)
        .filter(|r| r.starts_with("GET /apis"))
        .collect();
    assert_eq!(discovery, ["GET /apis", "GET /apis/tekton.dev/v1"]);
    let count = |prefix: &str| requests.iter().filter(|r| r.starts_with(prefix)).count();
    assert_eq!(count("LIST tekton.dev/v1/pipelineruns"), 1, "{requests:#?}");
    assert_eq!(count("LIST tekton.dev/v1/taskruns"), 2, "{requests:#?}");
}

#[tokio::test]
async fn task_runs_that_are_not_served_leave_the_build_standing() {
    let tekton = FixtureReader::default()
        .serves("tekton.dev", "v1", &["pipelineruns"])
        .with("pipelineruns", vec![pipeline_run(SHA, true, Some(NEW))])
        .with("taskruns", vec![task_run()]);
    let read = read_builds(&tekton, Some("acme-builds"), SHA).await;
    let builds = read.read().expect("the builds were read");
    assert_eq!(builds.len(), 1);
    assert_eq!(
        builds[0].tasks_unread.as_deref(),
        Some("taskruns.tekton.dev is not served at any version")
    );
    assert!(
        tekton
            .requests
            .borrow()
            .iter()
            .all(|r| !r.contains("taskruns")),
        "nothing is listed at a version that doesn't serve it"
    );
}

#[tokio::test]
async fn a_short_sha_finds_no_build() {
    let world = healthy();
    let trail = run_configured(&world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.sha = SHA[..12].into()
    })
    .await;
    let builds = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(builds.confidence, Confidence::Unknown);
    assert!(
        builds.reason.contains("40 or 64 hexadecimal digits"),
        "{}",
        builds.reason
    );
    assert!(world.tekton.requests.borrow().is_empty());
}

#[tokio::test]
async fn a_sha_256_commit_says_why_no_build_is_found() {
    let world = healthy();
    let trail = run_configured(&world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.sha = "b".repeat(64)
    })
    .await;
    let builds = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(builds.confidence, Confidence::Unknown);
    assert!(builds.reason.contains("SHA-256"), "{}", builds.reason);
    assert!(world.tekton.requests.borrow().is_empty());
}

#[tokio::test]
async fn only_upstream_commit_names_are_read_unless_more_are_configured() {
    // The run names its revision under a parameter upstream doesn't define.
    let mut world = healthy();
    let mut run_value = pipeline_run(SHA, false, Some(NEW));
    run_value["spec"]["params"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"name": "made-up-param", "value": SHA}));
    world.tekton = world.tekton.with("pipelineruns", vec![run_value]);

    let plain = run(&world, &ENV).await;
    assert_eq!(
        one(&plain, Hop::Commit, Hop::PipelineRun).confidence,
        Confidence::Claimed
    );
    let configured = run_configured(&world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.commit_names.params = vec!["made-up-param".into()]
    })
    .await;
    // The configured parameter is read, but it is declared: a claim.
    let configured = one(&configured, Hop::Commit, Hop::PipelineRun);
    assert_eq!(configured.confidence, Confidence::Claimed);
    assert!(
        configured.reason.contains("both declared"),
        "{}",
        configured.reason
    );

    // Results: upstream's `commit` counts, an unknown name doesn't.
    let names = super::tekton::CommitNames::default();
    let results = |key: &str| {
        [(key.to_owned(), SHA.to_owned())]
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    assert_eq!(
        super::tekton::result_commit(&results("commit"), &names).as_deref(),
        Some(SHA)
    );
    assert_eq!(
        super::tekton::result_commit(&results("CHAINS-GIT_COMMIT"), &names).as_deref(),
        Some(SHA)
    );
    assert_eq!(
        super::tekton::result_commit(&results("made-up-result"), &names),
        None
    );
}
