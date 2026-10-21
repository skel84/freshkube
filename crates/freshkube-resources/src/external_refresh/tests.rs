use super::{ExternalRefresh, Phase};
use crate::{detail::DetailTarget, example, screen::ClusterAccess, ui::Tone};
use gpui_kit::{AppContext, Entity, TestAppContext};
use std::time::Duration;

fn target(name: &str) -> DetailTarget {
    let key = example::external_secrets::KEY;
    let (_, rows) = example::read("invented-context", key, None, 1_767_312_000).unwrap();
    DetailTarget {
        identity: rows
            .into_iter()
            .find(|r| r.identity.name == name)
            .unwrap()
            .identity,
        kind: example::kind(key).unwrap(),
    }
}
fn mount(cx: &mut TestAppContext) -> (tokio::runtime::Runtime, Entity<ExternalRefresh>) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let owner = cx.new(|cx| {
        let mut owner = ExternalRefresh::new(runtime.handle().clone(), cx);
        owner.utc = Some("2026-01-02T12:00:00.500Z".parse().unwrap());
        owner
    });
    (runtime, owner)
}

#[gpui_kit::test]
fn accepted_refresh_requires_new_time_and_preserves_reported_failure(cx: &mut TestAppContext) {
    let (_runtime, owner) = mount(cx);
    for name in ["lantern-api", "broken-store"] {
        let target = target(name);
        let mut secret = example::document(&target.identity, 1_767_312_000)
            .unwrap()
            .overview
            .external_secret
            .unwrap();
        cx.update(|cx| {
            owner.update(cx, |owner, cx| {
                owner
                    .submit(target.clone(), ClusterAccess::EXAMPLE, &secret, None, cx)
                    .unwrap();
                assert_eq!(owner.attempt.as_ref().unwrap().phase, Phase::Waiting);
                let confirmed = owner.attempt.as_ref().unwrap().confirmed;
                owner.observe(&target.identity, &secret, cx);
                assert_eq!(
                    owner.attempt.as_ref().unwrap().phase,
                    Phase::Waiting,
                    "old Ready=True is no evidence"
                );
                secret.refreshed = Some(confirmed);
                owner.observe(&target.identity, &secret, cx);
                assert_eq!(owner.attempt.as_ref().unwrap().phase, Phase::Waiting);
                secret.refreshed = Some(confirmed + chrono::Duration::seconds(1));
                let mut other = target.identity.clone();
                other.connection = "other-access".into();
                owner.observe(&other, &secret, cx);
                assert_eq!(owner.attempt.as_ref().unwrap().phase, Phase::Waiting);
                other = target.identity.clone();
                other.uid = "replacement".into();
                owner.observe(&other, &secret, cx);
                assert_eq!(owner.attempt.as_ref().unwrap().phase, Phase::Waiting);
                owner.observe(&target.identity, &secret, cx);
                let result = owner.attempt.as_ref().unwrap();
                assert_eq!(result.phase, Phase::Finished);
                assert_eq!(
                    result.tone,
                    if name == "lantern-api" {
                        Tone::Good
                    } else {
                        Tone::Crit
                    }
                );
                assert!(result.text.contains(if name == "lantern-api" {
                    "SecretSynced"
                } else {
                    "not ready"
                }));
            })
        });
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn accepted_unchanged_status_expires_once_on_the_executor_clock(cx: &mut TestAppContext) {
    let (_runtime, owner) = mount(cx);
    let target = target("old-refresh");
    let secret = example::document(&target.identity, 1_767_312_000)
        .unwrap()
        .overview
        .external_secret
        .unwrap();
    cx.update(|cx| {
        owner.update(cx, |owner, cx| {
            owner
                .submit(target.clone(), ClusterAccess::EXAMPLE, &secret, None, cx)
                .unwrap()
        })
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(29));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            owner.read(cx).attempt.as_ref().unwrap().phase,
            Phase::Waiting
        )
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    cx.update(|cx| {
        let attempt = owner.read(cx).attempt.as_ref().unwrap();
        assert_eq!(attempt.phase, Phase::Finished);
        assert!(
            attempt
                .text
                .contains("no resync observed within 30 seconds")
        );
    });
}

#[gpui_kit::test]
fn refused_unknown_and_same_second_results_never_replay(cx: &mut TestAppContext) {
    let (_runtime, owner) = mount(cx);
    let target = target("access-denied");
    let secret = example::document(&target.identity, 1_767_312_000)
        .unwrap()
        .overview
        .external_secret
        .unwrap();
    cx.update(|cx| {
        owner.update(cx, |owner, cx| {
            let token = owner.now(cx).timestamp().to_string();
            assert!(
                owner
                    .submit(
                        target.clone(),
                        ClusterAccess::EXAMPLE,
                        &secret,
                        Some(&token),
                        cx
                    )
                    .is_err()
            );
            assert!(owner.attempt.is_none());
            owner
                .submit(target.clone(), ClusterAccess::EXAMPLE, &secret, None, cx)
                .unwrap();
            assert!(
                owner
                    .attempt
                    .as_ref()
                    .unwrap()
                    .text
                    .contains("Not permitted to refresh: Invented viewer")
            );
            assert!(
                owner
                    .submit(target, ClusterAccess::EXAMPLE, &secret, None, cx)
                    .is_err()
            );
            owner.finish(
                Err(freshkube_core::resources::Failure::new(
                    freshkube_core::resources::FailureKind::Unreachable,
                    "connection lost after submission",
                )),
                cx,
            );
            assert!(
                owner
                    .attempt
                    .as_ref()
                    .unwrap()
                    .text
                    .contains("Refresh outcome unknown")
            );
            assert!(!owner.busy());
        })
    });
}
