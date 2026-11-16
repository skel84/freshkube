use super::*;
use crate::change::NextStep;

/// What the dev Stage's Next step says, with the storefront Rollout edited
/// by `edit` in a cluster where everything else is read.
async fn dev_next(edit: impl FnOnce(&mut Value)) -> NextStep {
    let reader = one_cluster(|world| {
        let mut changed = rollout(&format!("{REPO}:v1.4.0"));
        edit(&mut changed);
        world.environment = std::mem::take(&mut world.environment).with("rollouts", vec![changed]);
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    change.stages[0].next_step.clone()
}

fn canary(rollout: &mut Value, steps: usize, index: Option<u64>) {
    rollout["spec"]["strategy"] = json!({"canary": {"steps": vec![json!({"pause": {}}); steps]}});
    rollout["status"]["phase"] = json!("Progressing");
    if let Some(index) = index {
        rollout["status"]["currentStepIndex"] = json!(index);
    }
}

#[tokio::test]
async fn a_healthy_rollout_with_no_steps_is_done() {
    assert_eq!(dev_next(|_| {}).await, NextStep::Done);
}

#[tokio::test]
async fn a_canary_counts_its_steps_from_one() {
    let next = dev_next(|r| canary(r, 5, Some(1))).await;
    assert_eq!(next, NextStep::Step { at: 2, of: 5 });
    assert_eq!(next.word(), "Step 2 of 5");
    // No index reported yet: it is on its first step.
    let next = dev_next(|r| canary(r, 3, None)).await;
    assert_eq!(next, NextStep::Step { at: 1, of: 3 });
}

#[tokio::test]
async fn an_index_past_the_last_step_is_done() {
    assert_eq!(dev_next(|r| canary(r, 3, Some(3))).await, NextStep::Done);
}

#[tokio::test]
async fn a_paused_rollout_says_which_step_it_waits_on() {
    let next = dev_next(|r| {
        canary(r, 4, Some(2));
        r["status"]["pauseConditions"] = json!([{"reason": "CanaryPauseStep"}]);
    })
    .await;
    assert_eq!(next, NextStep::Paused { step: Some((3, 4)) });
    assert_eq!(next.word(), "Paused");
}

#[tokio::test]
async fn an_aborted_rollout_is_aborted_whatever_its_step() {
    let next = dev_next(|r| {
        canary(r, 4, Some(1));
        r["status"]["abort"] = json!(true);
    })
    .await;
    assert_eq!(next, NextStep::Aborted);
}

#[tokio::test]
async fn refused_rollouts_are_unknown_with_their_cause_never_blank() {
    let reader = one_cluster(|world| {
        world.environment = std::mem::take(&mut world.environment).refusing("rollouts");
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let NextStep::Unknown { why, cause } = &change.stages[0].next_step else {
        panic!("{:?}", change.stages[0].next_step);
    };
    assert!(!why.is_empty());
    assert_eq!(cause.as_ref().map(|cause| cause.word()), Some("Can't read"));
}

#[tokio::test]
async fn a_capped_listing_without_the_rollout_is_unknown_and_says_so() {
    let reader = one_cluster(|world| {
        world.environment = std::mem::take(&mut world.environment)
            .with("rollouts", Vec::new())
            .capped("rollouts");
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let next = &change.stages[0].next_step;
    let NextStep::Unknown { why, .. } = next else {
        panic!("{next:?}");
    };
    assert!(why.contains("page cap"), "{why}");
    assert_eq!(next.word(), "Unknown");
}

#[tokio::test]
async fn a_stage_whose_application_runs_a_deployment_has_no_rollout() {
    let image = format!("{REPO}@{NEW}");
    let reader = super::workloads::deployed(vec![deployment_set("6fdf", "2", &image, 1, 1)]);
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    assert_eq!(change.stages[0].next_step, NextStep::NoRollout);
}
