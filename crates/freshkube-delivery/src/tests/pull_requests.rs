use super::*;

#[tokio::test]
async fn a_squash_merged_pull_request_joins_through_its_merge_commit() {
    let github = FixtureGitHub::with(vec![pull_request(7, OTHER_SHA, Some(SHA))]);
    let trail = run_with(&squash_world(), &ENV, &github, Some(GH_REPO)).await;
    let commit = one(&trail, Hop::PullRequest, Hop::Commit);
    assert_eq!(commit.confidence, Confidence::Confirmed);
    assert_eq!(commit.subject, "acme/storefront#7");
    assert!(
        commit.reason.contains("its merge commit"),
        "{}",
        commit.reason
    );
    // Two PipelineRuns hang off the pull request: main's and the head's.
    let runs = link(&trail, Hop::PullRequest, Hop::PipelineRun);
    assert_eq!(runs.len(), 2, "{trail:#?}");
    let merged_run = runs.iter().find(|l| l.key == Key::Sha(SHA.into())).unwrap();
    assert_eq!(merged_run.confidence, Confidence::Claimed);
    let head_run = runs
        .iter()
        .find(|l| l.key == Key::Sha(OTHER_SHA.into()))
        .unwrap();
    assert!(
        head_run.reason.contains("is not the digest that shipped"),
        "{}",
        head_run.reason
    );
    // The head's image is old freight in Kargo: joined on the digest.
    // The head build is tied to the head commit by declared fields only, so
    // the link is a claim, though GitHub and the build each report their part.
    let freight = one(&trail, Hop::PullRequest, Hop::Freight);
    assert_eq!(freight.confidence, Confidence::Claimed);
    assert!(
        freight
            .reason
            .contains("no result of the build reports the head commit"),
        "{}",
        freight.reason
    );
    assert_eq!(freight.subject, "storefront/f-old");
    assert!(matches!(&freight.key, Key::Digest(d) if d.as_str() == OLD));
    // The rest of the chain is untouched.
    assert_eq!(trail.running(), Some(Confidence::Confirmed));
}

#[tokio::test]
async fn a_pull_request_whose_head_is_the_commit_reads_no_second_build() {
    let github = FixtureGitHub::with(vec![pull_request(7, SHA, Some(SHA))]);
    let world = healthy();
    let trail = run_with(&world, &ENV, &github, Some(GH_REPO)).await;
    assert_eq!(
        one(&trail, Hop::PullRequest, Hop::Commit).confidence,
        Confidence::Confirmed
    );
    assert_eq!(link(&trail, Hop::PullRequest, Hop::PipelineRun).len(), 1);
    let sha_selectors = world
        .tekton
        .requests
        .borrow()
        .iter()
        .filter(|r| r.contains("pipelinesascode.tekton.dev/sha=") && r.contains("pipelineruns"))
        .count();
    assert_eq!(sha_selectors, 1);
}

#[tokio::test]
async fn a_pull_request_listed_by_ancestry_only_is_a_claim() {
    let github = FixtureGitHub::with(vec![pull_request(
        7,
        OTHER_SHA,
        Some("cccccccccccccccccccccccccccccccccccccccc"),
    )]);
    let trail = run_with(&healthy(), &ENV, &github, Some(GH_REPO)).await;
    let commit = one(&trail, Hop::PullRequest, Hop::Commit);
    assert_eq!(commit.confidence, Confidence::Claimed);
    assert!(commit.reason.contains("only by ancestry"));
    assert!(
        link(&trail, Hop::PullRequest, Hop::PipelineRun)
            .iter()
            .all(|l| l.key != Key::Sha(SHA.into()))
    );
}

#[tokio::test]
async fn a_run_that_names_another_pull_request_is_flagged() {
    let mut world = healthy();
    let mut run = pipeline_run(SHA, true, Some(NEW));
    run["metadata"]["annotations"]["pipelinesascode.tekton.dev/pull-request"] =
        serde_json::json!("9");
    world.tekton = world.tekton.with("pipelineruns", vec![run]);
    let github = FixtureGitHub::with(vec![pull_request(7, SHA, Some(SHA))]);
    let trail = run_with(&world, &ENV, &github, Some(GH_REPO)).await;
    let runs = link(&trail, Hop::PullRequest, Hop::PipelineRun);
    assert!(
        runs[0]
            .reason
            .contains("the run says pull request #9, not #7"),
        "{}",
        runs[0].reason
    );
}

#[tokio::test]
async fn github_that_cannot_be_read_is_unknown_and_the_rest_still_joins() {
    let trail = run_with(&healthy(), &ENV, &FixtureGitHub::refusing(), Some(GH_REPO)).await;
    let commit = one(&trail, Hop::PullRequest, Hop::Commit);
    assert_eq!(commit.confidence, Confidence::Unknown);
    assert!(
        commit.reason.contains("not readable (refused)"),
        "{}",
        commit.reason
    );
    assert_eq!(trail.running(), Some(Confidence::Confirmed));

    // Read, but no pull request: empty is said as empty.
    let none = run_with(
        &healthy(),
        &ENV,
        &FixtureGitHub::with(vec![]),
        Some(GH_REPO),
    )
    .await;
    assert!(
        one(&none, Hop::PullRequest, Hop::Commit)
            .reason
            .contains("no pull request is associated")
    );
}

#[tokio::test]
async fn github_is_not_asked_without_a_repository() {
    let github = FixtureGitHub::with(vec![pull_request(7, SHA, Some(SHA))]);
    let trail = run_with(&healthy(), &ENV, &github, None).await;
    assert!(github.requests.borrow().is_empty());
    assert!(link(&trail, Hop::PullRequest, Hop::Commit).is_empty());
}

#[tokio::test]
async fn a_pull_request_number_leads_to_its_merge_commit_or_its_head() {
    let github = FixtureGitHub::with(vec![
        pull_request(7, OTHER_SHA, Some(SHA)),
        pull_request(8, OTHER_SHA, None),
    ]);
    assert_eq!(
        commit_of_pull_request(&github, GH_REPO, 7).await,
        Source::Read((SHA.to_owned(), true))
    );
    assert_eq!(
        commit_of_pull_request(&github, GH_REPO, 8).await,
        Source::Read((OTHER_SHA.to_owned(), false))
    );
    assert!(matches!(
        commit_of_pull_request(&github, GH_REPO, 99).await,
        Source::NotInstalled(_)
    ));
}
