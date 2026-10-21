use super::*;
use crate::forwards::State;

#[gpui_kit::test]
async fn service_matching_uses_the_whole_target_and_keeps_the_ended_owner(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let mounted = mount(cx);
    let spec = mounted.spec("services", "gateway", 443, None);
    let identity = spec.identity.clone();
    let first = mounted.start(cx, spec.clone()).await;
    let second = mounted.start(cx, spec).await;
    let owner = cx.read(|cx| {
        let list = mounted.list.read(cx);
        for (connection, namespace, service, port) in [
            ("other", identity.namespace.as_str(), "gateway", 443),
            (identity.connection.as_str(), "other", "gateway", 443),
            (
                identity.connection.as_str(),
                identity.namespace.as_str(),
                "other",
                443,
            ),
            (
                identity.connection.as_str(),
                identity.namespace.as_str(),
                "gateway",
                80,
            ),
        ] {
            assert!(
                list.find(connection, namespace, service, port, cx)
                    .is_none()
            );
        }
        let owner = list
            .find(
                &identity.connection,
                &identity.namespace,
                "gateway",
                443,
                cx,
            )
            .unwrap();
        assert_eq!(owner.read(cx).id, second);
        assert!(matches!(
            owner.read(cx).snapshot().state,
            State::Listening(_)
        ));
        owner
    });
    let before = cx.read(|cx| owner.read(cx).snapshot());
    cx.update(|cx| mounted.list.update(cx, |list, cx| list.stop(second, cx)));
    cx.read(|cx| {
        assert!(matches!(
            owner.read(cx).snapshot().state,
            State::Ended(ForwardEnd::Stopped)
        ));
        assert_eq!(
            mounted
                .list
                .read(cx)
                .find(
                    &identity.connection,
                    &identity.namespace,
                    "gateway",
                    443,
                    cx
                )
                .unwrap()
                .read(cx)
                .id,
            second
        );
        assert_ne!(second, first);
    });
    cx.update(|cx| {
        mounted
            .list
            .update(cx, |list, cx| list.start_again(second, cx))
    });
    mounted.settle(cx, second).await;
    let restarted = cx.read(|cx| owner.read(cx).snapshot());
    assert_eq!(restarted.epoch.id, before.epoch.id);
    assert!(restarted.epoch.start > before.epoch.start);
    cx.update(|cx| mounted.list.update(cx, |list, cx| list.remove(second, cx)));
    cx.read(|cx| {
        assert!(matches!(
            owner.read(cx).snapshot().state,
            State::Ended(ForwardEnd::Stopped)
        ));
        assert!(!mounted.list.read(cx).contains(second, cx));
    });
}

#[gpui_kit::test]
async fn owner_availability_preserves_no_ready_pod_and_terminal_reasons(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let mounted = mount(cx);
    let (sender, receiver) = watch::channel(status(ForwardState::NoReadyPod));
    let (id, _) = mounted.with_status(cx, receiver).await;
    cx.read(|cx| {
        let list = mounted.list.read(cx);
        let owner = list.get(id, cx).unwrap();
        assert_eq!(owner.read(cx).snapshot().state, State::NoReadyPod);
        let pod = object("pods", "grafana");
        assert!(
            list.find(&pod.connection, &pod.namespace, &pod.name, 8080, cx)
                .is_none()
        );
    });
    sender
        .send(status(ForwardState::Ended(ForwardEnd::TargetDeleted(
            "Service removed".into(),
        ))))
        .unwrap();
    mounted
        .wait_until(cx, |text| text.contains("Service removed"), id)
        .await;
    cx.read(|cx| {
        assert_eq!(
            mounted
                .list
                .read(cx)
                .get(id, cx)
                .unwrap()
                .read(cx)
                .snapshot()
                .state,
            State::Ended(ForwardEnd::TargetDeleted("Service removed".into()))
        )
    });
}
