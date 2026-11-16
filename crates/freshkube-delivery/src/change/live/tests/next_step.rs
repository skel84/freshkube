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
    let first = dev_next(|r| canary(r, 3, Some(0))).await;
    assert_eq!(first, NextStep::Step { at: 1, of: 3 });
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

#[tokio::test]
async fn a_blue_green_or_step_less_rollout_that_progresses_says_progressing() {
    let next = dev_next(|r| {
        r["spec"]["strategy"] = json!({"blueGreen": {}});
        r["status"]["phase"] = json!("Progressing");
    })
    .await;
    assert_eq!(next, NextStep::Progressing);
    assert_eq!(next.word(), "Progressing");
}

#[tokio::test]
async fn a_degraded_canary_that_is_not_aborted_says_degraded_before_any_step() {
    let next = dev_next(|r| {
        canary(r, 4, Some(1));
        r["status"]["phase"] = json!("Degraded");
    })
    .await;
    assert_eq!(next, NextStep::Degraded { step: Some((2, 4)) });
    assert_eq!(next.word(), "Degraded");
}

#[tokio::test]
async fn steps_with_no_current_step_or_no_status_are_unknown_never_step_one() {
    let next = dev_next(|r| canary(r, 3, None)).await;
    let NextStep::Unknown { why, .. } = &next else {
        panic!("{next:?}");
    };
    assert!(why.contains("no current step"), "{why}");
    let next = dev_next(|r| {
        r["status"]["phase"] = Value::Null;
    })
    .await;
    let NextStep::Unknown { why, .. } = &next else {
        panic!("{next:?}");
    };
    assert!(why.contains("no status"), "{why}");
}

#[tokio::test]
async fn refused_applications_leave_the_next_step_unknown_not_no_rollout() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).refusing("applications");
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let next = &change.stages[0].next_step;
    let NextStep::Unknown { why, cause } = next else {
        panic!("{next:?}");
    };
    assert!(!why.is_empty());
    assert_eq!(cause.as_ref().map(|cause| cause.word()), Some("Can't read"));
}

#[tokio::test]
async fn an_application_nobody_found_or_a_capped_list_is_unknown_not_no_rollout() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).with("applications", Vec::new());
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    assert!(
        matches!(change.stages[0].next_step, NextStep::Unknown { .. }),
        "{:?}",
        change.stages[0].next_step
    );
}

#[tokio::test]
async fn an_application_that_deploys_to_a_cluster_nothing_was_read_in_is_unknown() {
    let change = read_mapped(Some(ENV_A), mapped(&[(by_server(), "prod")])).await;
    let next = &change.stages[0].next_step;
    let NextStep::Unknown { why, .. } = next else {
        panic!("{next:?}");
    };
    assert!(why.contains("prod"), "{why}");
}

#[tokio::test]
async fn a_stage_the_freight_hasnt_reached_is_not_reached_not_no_rollout() {
    let reader = one_cluster(|world| {
        world.kargo = std::mem::take(&mut world.kargo).with(
            "stages",
            vec![
                stage_of("dev", &[], &["f-new"]),
                stage_of("prod", &["dev"], &[]),
            ],
        );
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let prod = change.stages.iter().find(|s| s.name == "prod").unwrap();
    assert_eq!(prod.next_step, NextStep::NotReached);
    assert_eq!(prod.next_step.word(), "Not reached");
}
