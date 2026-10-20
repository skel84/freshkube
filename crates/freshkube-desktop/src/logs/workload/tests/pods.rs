use super::*;

#[gpui_kit::test]
fn pods_joining_and_leaving_start_and_stop_their_streams(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    let (joining, leaving) = (all[0].clone(), all[1].clone());
    let without_joining: Vec<_> = all[1..].to_vec();
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, without_joining.clone(), cx);
        window.render_frame(cx);
        // The first listing marks nobody as joining.
        assert!(markers(&view, cx).is_empty());
        let tag = format!("{}/api", joining.name);
        assert!(
            window
                .try_find(format!("workload-logs-stream-{tag}"))
                .is_none()
        );

        feed(&view, all.clone(), cx);
        window.render_frame(cx);
        assert_eq!(markers(&view, cx), [format!("Pod {} joined", joining.name)]);
        assert!(window.find(format!("workload-logs-stream-{tag}")).visible());
        assert!(tags(&view, cx).contains(&tag));

        let left_tag = format!("{}/api", leaving.name);
        let left_lines = view
            .read(cx)
            .retained()
            .iter()
            .filter(|entry| entry.service.as_str() == left_tag)
            .count();
        assert!(left_lines > 0);
        let rest: Vec<_> = all
            .iter()
            .filter(|pod| pod.name != leaving.name)
            .cloned()
            .collect();
        feed(&view, rest, cx);
        window.render_frame(cx);
        assert!(
            markers(&view, cx).contains(&format!("Pod {} left", leaving.name)),
            "{:?}",
            markers(&view, cx)
        );
        // Its stream stops; what it wrote stays.
        assert!(
            !view
                .read(cx)
                .source()
                .reads
                .streams
                .contains_key(&key(&leaving))
        );
        assert!(
            window
                .try_find(format!("workload-logs-stream-{left_tag}"))
                .is_none()
        );
        let kept = view
            .read(cx)
            .retained()
            .iter()
            .filter(|entry| entry.service.as_str() == left_tag)
            .count();
        assert_eq!(kept, left_lines);
        // The markers' pod tags name no container, so the labels stay the
        // pods' short names.
        let labels = view.read(cx).source().labels.clone();
        assert!(labels.contains_key(&ServiceId::new(leaving.name.clone())));
        for (tag, label) in &labels {
            let tag = tag.as_str();
            assert!(!label.contains('/'), "{tag}: {label}");
            assert!(tag.starts_with("api-"), "{tag}");
            assert!(label.len() < tag.len(), "{tag}: {label}");
        }
        let chips = view.read(cx).source().chips.clone();
        assert!(chips.iter().all(|chip| !chip.label.contains('/')));
    })
    .unwrap();
}

#[gpui_kit::test]
fn past_the_cap_the_newest_pods_are_read_and_the_rest_counted(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    // Every pod gains a sidecar, which makes more containers than the cap.
    let mut all = pods(&api, "app=api");
    for pod in &mut all {
        let mut sidecar: Container = pod.containers.containers[0].clone();
        sidecar.name = "proxy".into();
        sidecar.role = ContainerRole::App;
        sidecar.state = ContainerState::Running(None);
        pod.containers.containers.push(sidecar);
    }
    let containers: usize = all
        .iter()
        .map(|pod| {
            pod.containers
                .containers
                .iter()
                .filter(|container| container.role == ContainerRole::App)
                .count()
        })
        .sum();
    assert!(containers > MAX_STREAMS, "{containers}");
    let newest = all
        .iter()
        .max_by_key(|pod| pod.created)
        .unwrap()
        .name
        .clone();
    let oldest = all
        .iter()
        .min_by_key(|pod| pod.created)
        .unwrap()
        .name
        .clone();
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all, cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).source().reads.streams.len(), MAX_STREAMS);
        assert_eq!(
            window.find("workload-logs-capped").label(),
            Some(
                format!("Reading {MAX_STREAMS} of {containers} containers, the newest pods first.")
                    .as_str()
            )
        );
        let streams = &view.read(cx).source().reads.streams;
        assert!(streams.keys().any(|key| key.pod == newest));
        assert!(streams.keys().all(|key| key.pod != oldest));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_failed_pod_watch_says_why_and_retry_watches_again(cx: &mut TestAppContext) {
    use freshkube_core::resources::{Failure, FailureKind};
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    cx.update_window(handle, |_, window, cx| {
        show(&view, &api, cx);
        view.update(cx, |view, cx| {
            view.apply_pods(
                WorkloadPods {
                    listed: false,
                    pods: Vec::new(),
                    failure: Some(Failure::new(
                        FailureKind::Forbidden,
                        "pods is forbidden".to_owned(),
                    )),
                },
                cx,
            );
        });
        window.render_frame(cx);
        assert!(matches!(
            view.read(cx).source().pods_state,
            PodsState::Failed(_)
        ));
        let failed = window
            .find("workload-logs-failed")
            .label()
            .unwrap()
            .to_owned();
        assert!(failed.contains("pods is forbidden"), "{failed}");
    })
    .unwrap();
    // A listing from the watch before Retry lands nowhere.
    let older = cx.update(|cx| view.read(cx).source().pod_watch.sender().unwrap());
    cx.update_window(handle, |_, window, cx| {
        window.click("workload-logs-retry", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).source().pods_state, PodsState::Watching);
        assert!(window.try_find("workload-logs-failed").is_none());
    })
    .unwrap();
    let _ = older.try_send(WorkloadPods {
        listed: false,
        pods: Vec::new(),
        failure: Some(Failure::new(FailureKind::Forbidden, "older".to_owned())),
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(view.read(cx).source().pods_state, PodsState::Watching);
        assert!(
            view.read(cx)
                .source()
                .reads
                .streams
                .values()
                .any(|stream| stream.state == StreamState::Streaming)
        );
    });
}

#[test]
fn a_busy_pod_watch_hands_over_the_newest_listing_after_the_one_in_flight() {
    use freshkube_core::resources::{Failure, FailureKind};
    use tokio::sync::{mpsc, watch};

    let listing = |name: &str| WorkloadPods {
        listed: true,
        failure: Some(Failure::new(FailureKind::Other, name)),
        ..WorkloadPods::default()
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (latest, listings) = watch::channel(WorkloadPods::default());
        let (sender, mut received) = mpsc::channel(1);
        let forward = tokio::spawn(super::forward_listings(listings, sender));
        let settle = || async {
            for _ in 0..8 {
                tokio::task::yield_now().await;
            }
        };

        latest.send_replace(listing("a"));
        settle().await;
        // "a" fills the slot; three more arrive before the owner takes it.
        for name in ["b", "c", "d"] {
            latest.send_replace(listing(name));
            settle().await;
        }

        assert_eq!(received.recv().await, Some(listing("a")));
        settle().await;
        assert_eq!(received.recv().await, Some(listing("d")));
        settle().await;
        assert!(received.try_recv().is_err(), "b and c never land");

        drop(latest);
        forward.await.unwrap();
    });
}
