//! A build found by image digest in the archive's latest records.
use super::builds::{change, pipeline, reader};
use super::*;
use crate::collect::{Upstream, upstream_with_history};
use crate::digest::Digest;
use crate::fixtures::{self, FixtureGitHub, FixtureReader};
use crate::join::{self, Confidence, Evidence, Hop};
use crate::source::Source;
use crate::tekton::{self, FoundBy};

async fn by_digest(reader: &FixtureReader, client: Result<&Client, Error>) -> Evidence {
    let github = FixtureGitHub::with(vec![]);
    let upstream = Upstream {
        kargo: reader,
        argocd: reader,
        tekton: reader,
        github: &github,
    };
    let mut plan = crate::tests::plan(&[]);
    plan.sha = String::new();
    plan.digests = vec![Digest::parse(fixtures::NEW).unwrap()];
    plan.build_namespace = Some("acme".into());
    within(upstream_with_history(
        &upstream,
        &plan,
        crate::tests::observed_at(),
        std::future::ready(Source::Read(vec![])),
        client,
        Instant::now() + Duration::from_secs(20),
    ))
    .await
}

/// The archived run with no commit label, as a pipeline without Pipelines as
/// Code's makes it.
fn bare(uid: &str, digest: Option<&str>) -> Value {
    let mut run = pipeline(uid);
    run["metadata"]["labels"]
        .as_object_mut()
        .unwrap()
        .remove(SHA_LABEL);
    run["status"]["results"] = match digest {
        Some(digest) => json!([
            {"name": "IMAGE_DIGEST", "value": digest},
            {"name": "IMAGE_URL", "value": "registry.example.com/acme/checkout"}
        ]),
        None => json!([]),
    };
    run
}

fn building_task(digest: &str) -> Value {
    let mut task = manifest("TaskRun", "v1");
    task["metadata"]["uid"] = json!("task-uid");
    task["status"]["results"] = json!([
        {"name": "IMAGE_DIGEST", "value": digest},
        {"name": "IMAGE_URL", "value": "registry.example.com/acme/checkout"}
    ]);
    task
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_result_in_the_archive_joins_an_image_only_freight() {
    let server = Server::new(
        false,
        vec![
            page(vec![record(bare("archived-uid", Some(fixtures::NEW)))]),
            page(vec![]),
            page(vec![]),
        ],
    );
    let client = client(&server).await;
    let evidence = by_digest(&reader(vec![], vec![]), Ok(&client)).await;
    let builds = evidence.builds.read().unwrap();
    assert_eq!(builds.len(), 1);
    assert_eq!(builds[0].origin, tekton::Origin::Results);
    assert_eq!(builds[0].found_by, FoundBy::Digest);
    let requests = server.requests.lock().unwrap().clone();
    assert!(
        requests[0].contains("order_by=create_time+desc") && requests[0].contains("page_size=10"),
        "{}",
        requests[0]
    );
    assert!(!requests[0].contains(SHA), "{}", requests[0]);
    let trail = join::join(&evidence);
    let link = trail
        .links
        .iter()
        .find(|link| link.from == Hop::Image && link.to == Hop::PipelineRun)
        .unwrap();
    assert_eq!(link.confidence, Confidence::Confirmed);
    assert!(
        link.reason.contains("read from the archive"),
        "{}",
        link.reason
    );
    let freight = trail
        .links
        .iter()
        .find(|link| link.to == Hop::Freight && link.from == Hop::PipelineRun)
        .unwrap();
    assert_eq!(freight.confidence, Confidence::Confirmed);
    // The change page shows the build as one read from the archive.
    let change = change(&evidence);
    assert!(
        change.hops.iter().any(|hop| hop.name == "Run checkout-7"),
        "{:?}",
        change.hops.iter().map(|hop| &hop.name).collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_taskrun_result_in_the_archive_joins_an_image_only_freight() {
    let server = Server::new(
        false,
        vec![
            page(vec![record(bare("archived-uid", None))]),
            page(vec![record(building_task(fixtures::NEW))]),
            page(vec![]),
        ],
    );
    let client = client(&server).await;
    let evidence = by_digest(&reader(vec![], vec![]), Ok(&client)).await;
    let builds = evidence.builds.read().unwrap();
    assert_eq!(builds.len(), 1);
    assert_eq!(builds[0].tasks.len(), 1);
    let trail = join::join(&evidence);
    let link = trail
        .links
        .iter()
        .find(|link| link.from == Hop::Image && link.to == Hop::PipelineRun)
        .unwrap();
    assert_eq!(link.confidence, Confidence::Confirmed);
    assert!(
        link.reason.contains("one of its TaskRuns"),
        "{}",
        link.reason
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn no_archived_record_reports_the_digest() {
    let server = Server::new(
        false,
        vec![
            page(vec![record(bare("archived-uid", Some(fixtures::OLD)))]),
            page(vec![]),
        ],
    );
    let client = client(&server).await;
    let evidence = by_digest(&reader(vec![], vec![]), Ok(&client)).await;
    assert!(evidence.builds.read().unwrap().is_empty());
    let history = evidence.build_history.as_ref().unwrap();
    assert_eq!(history.runs, 1);
    let trail = join::join(&evidence);
    assert!(
        trail
            .links
            .iter()
            .any(|link| link.from == Hop::Image && link.confidence == Confidence::Unknown)
    );
    let change = change(&evidence);
    let text = format!("{change:?}");
    assert!(
        text.contains("was searched for the image digest in its latest 1 run only"),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_latest_runs_are_asked_for_and_read() {
    let records = (0..11)
        .map(|n| record(bare(&format!("uid-{n}"), Some(fixtures::OLD))))
        .collect();
    let server = Server::new(false, vec![page(records)]);
    let client = client(&server).await;
    let listing =
        within(client.latest_pipeline_runs("acme", Instant::now() + Duration::from_secs(20)))
            .await
            .unwrap();
    assert_eq!(listing.records.len(), MOST_RESULT_RUNS);
    assert_eq!(listing.cap, Some(Cap::Runs));
    let requests = server.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("page_size=10"), "{}", requests[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_capped_live_read_asks_the_archive_nothing() {
    let server = Server::new(false, vec![page(vec![])]);
    let client = client(&server).await;
    let live = Source::Capped(vec![], crate::source::Truncation { read: 1000 });
    let (live, history) = within(supplement_digests(
        live,
        Ok(&client),
        "acme",
        &[Digest::parse(fixtures::NEW).unwrap()],
        Instant::now() + Duration::from_secs(20),
    ))
    .await;
    assert!(matches!(live, Source::Capped(..)));
    assert_eq!(history.unwrap_err(), Error::Skipped(Skip::LiveCapped));
    assert!(server.requests.lock().unwrap().is_empty());
}
