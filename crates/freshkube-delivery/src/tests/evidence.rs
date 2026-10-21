use super::*;

fn supply_chain_reason(trail: &Trail) -> String {
    link(trail, Hop::PipelineRun, Hop::SupplyChain)[0]
        .reason
        .clone()
}

#[tokio::test]
async fn an_evidence_record_is_checked_only_when_its_result_is_configured() {
    let mut world = healthy();
    world.tekton = world
        .tekton
        .with("taskruns", vec![evidence_task_run(SHA, NEW)]);
    let configured = |plan: &mut Plan| plan.evidence_result = Some(evidence_result());

    let none = run(&world, &ENV).await;
    assert!(supply_chain_reason(&none).contains("no evidence result configured"));

    let agrees = run_configured(&world, &ENV, &FixtureGitHub::default(), None, configured).await;
    assert!(supply_chain_reason(&agrees).contains("evidence record agrees with"));

    world.tekton = world
        .tekton
        .with("taskruns", vec![evidence_task_run(SHA, OLD)]);
    let differs = run_configured(&world, &ENV, &FixtureGitHub::default(), None, configured).await;
    assert!(supply_chain_reason(&differs).contains("evidence record disagrees with"));

    // A name the build does not carry is said, not guessed.
    let absent = run_configured(
        &healthy(),
        &ENV,
        &FixtureGitHub::default(),
        None,
        configured,
    )
    .await;
    assert!(
        supply_chain_reason(&absent).contains("configured evidence result is not on the build")
    );
}

#[tokio::test]
async fn without_an_evidence_result_the_hint_names_the_runs_results() {
    let trail = run(&healthy(), &ENV).await;
    assert!(
        supply_chain_reason(&trail)
            .contains("no evidence result configured (the run's results: IMAGE_DIGEST, IMAGE_URL)"),
        "{}",
        supply_chain_reason(&trail)
    );
}
