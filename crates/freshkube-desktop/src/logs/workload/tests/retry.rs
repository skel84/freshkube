use super::*;

/// The metrics server refuses its logs, as a viewer without `pods/log`
/// would see; the other pods read on beside it.
#[gpui_kit::test]
fn a_failing_stream_says_why_while_the_others_read_on(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let coredns = deployment("coredns");
    let metrics = deployment("metrics-server");
    let mut both = pods(&coredns, "app=coredns");
    let refused = pods(&metrics, "app=metrics-server");
    both.extend(refused.clone());
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &coredns, cx);
        feed(&view, both, cx);
        window.render_frame(cx);
        let refused_tag = format!("{}/metrics-server", refused[0].name);
        let errors = view.read(cx).source().reads.errors.clone();
        let error = errors.get(&ServiceId::new(refused_tag.clone())).unwrap();
        assert!(error.contains("forbidden"), "{error}");
        // Its chip says so, in the row or behind "+N".
        let chips = view.read(cx).source().chips.clone();
        let chip = chips
            .iter()
            .find(|chip| chip.service.as_str() == refused_tag)
            .unwrap();
        assert!(
            chip.tooltip.starts_with(&format!("{refused_tag}: Failed")),
            "{}",
            chip.tooltip
        );
        // The coredns pods stream.
        assert!(tags(&view, cx).iter().all(|tag| tag.ends_with("/coredns")));
        assert!(lines(&view, cx) > 0);
        assert!(status(window).starts_with("Following: "));
        assert!(window.try_find("workload-logs-failed").is_none());
    })
    .unwrap();
}

/// A refused stream isn't read again on a timer: only when the pod list
/// changes (a pod added or removed), or on Retry.
#[gpui_kit::test]
fn a_refused_stream_waits_for_the_pods_to_change_or_retry(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let metrics = deployment("metrics-server");
    // One pod of the example's many, so one stream is refused.
    let refused = vec![pods(&metrics, "app=metrics-server").remove(0)];
    let refused_key = key(&refused[0]);
    let generation = |cx: &mut TestAppContext| {
        cx.update(|cx| view.read(cx).source().reads.streams[&refused_key].generation)
    };
    let still_refused = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let stream = &view.read(cx).source().reads.streams[&refused_key];
            stream.state.refused() && stream.retry.is_none()
        })
    };
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &metrics, cx);
        feed(&view, refused.clone(), cx);
        window.render_frame(cx);
        let tag = refused_key.service();
        let label = window
            .find(format!("workload-logs-stream-{}", tag.as_str()))
            .label()
            .unwrap()
            .to_owned();
        assert!(label.contains("Failed"), "{label}");
        assert!(label.contains("read again when the pods change"), "{label}");
        assert_eq!(
            window.find("workload-logs-refused").label(),
            Some("Logs refused for 1 container: read again when the pods change.")
        );
    })
    .unwrap();
    assert!(still_refused(cx));
    let first = generation(cx);
    cx.executor().advance_clock(RETRY_MAX * 4);
    cx.run_until_parked();
    assert_eq!(generation(cx), first);

    // The same pods listed again change nothing.
    cx.update(|cx| feed(&view, refused.clone(), cx));
    assert_eq!(generation(cx), first);

    // A pod joins: the refused stream is read again, and refused again.
    let mut joined = refused.clone();
    joined.push(pods(&deployment("coredns"), "app=coredns").remove(0));
    cx.update(|cx| feed(&view, joined, cx));
    let second = generation(cx);
    assert!(second > first);
    assert!(still_refused(cx));

    // It leaves: read again.
    cx.update(|cx| feed(&view, refused.clone(), cx));
    let third = generation(cx);
    assert!(third > second);
    cx.executor().advance_clock(RETRY_MAX * 4);
    cx.run_until_parked();
    assert_eq!(generation(cx), third);

    // Retry reads it again at once.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("workload-logs-retry-refused", cx);
    })
    .unwrap();
    assert!(generation(cx) > third);
    assert!(still_refused(cx));
}

/// A pod replaced under the same name, with a new uid, is a change too: a
/// refused stream is read again, its backoff from the start.
#[gpui_kit::test]
fn a_pod_replaced_under_its_name_wakes_a_refused_stream(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let metrics = deployment("metrics-server");
    let mut both = pods(&metrics, "app=metrics-server");
    both.truncate(2);
    let refused_key = key(&both[0]);
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &metrics, cx);
        feed(&view, both.clone(), cx);
        window.render_frame(cx);
    })
    .unwrap();
    let first = cx.update(|cx| {
        let stream = &view.read(cx).source().reads.streams[&refused_key];
        assert!(stream.state.refused());
        stream.generation
    });
    // As if it had failed for a while before it was refused.
    cx.update(|cx| {
        view.update(cx, |view, _| {
            view.source_mut()
                .reads
                .failures
                .insert(refused_key.clone(), 3);
        })
    });

    let mut replaced = both.clone();
    replaced[1].uid = format!("{}-replaced", replaced[1].uid);
    assert_eq!(replaced[1].name, both[1].name);
    cx.update(|cx| feed(&view, replaced.clone(), cx));
    cx.update(|cx| {
        let source = view.read(cx).source();
        assert!(source.reads.streams[&refused_key].generation > first);
        assert!(!source.reads.failures.contains_key(&refused_key));
        // The old incarnation's stream is gone; the new one's is read.
        assert!(!source.reads.streams.contains_key(&key(&both[1])));
        assert!(source.reads.streams.contains_key(&key(&replaced[1])));
    });
}

/// Retry reads every refused stream at once and starts its backoff over;
/// once none is refused, the notice goes.
#[gpui_kit::test]
fn retry_reads_refused_streams_and_starts_their_backoff_over(cx: &mut TestAppContext) {
    use freshkube_core::resources::{Failure, FailureKind};
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        window.render_frame(cx);
    })
    .unwrap();
    let find = |cx: &mut TestAppContext, waiting: bool| {
        cx.update(|cx| {
            view.read(cx)
                .source()
                .reads
                .streams
                .iter()
                .find(|(_, stream)| {
                    if waiting {
                        matches!(stream.state, StreamState::Waiting(_))
                    } else {
                        stream.state == StreamState::Streaming
                    }
                })
                .map(|(key, _)| key.clone())
                .unwrap()
        })
    };
    // The container still being created waits again when read again, so
    // its failures add up; a running one streams.
    let waiting = find(cx, true);
    let streaming = find(cx, false);
    let generation = |key: &StreamKey, cx: &mut TestAppContext| {
        cx.update(|cx| view.read(cx).source().reads.streams[key].generation)
    };
    let failures = |key: &StreamKey, cx: &mut TestAppContext| {
        cx.update(|cx| view.read(cx).source().reads.failures.get(key).copied())
    };
    let fail = |key: &StreamKey, kind: FailureKind, cx: &mut TestAppContext| {
        let generation = generation(key, cx);
        let failure = Failure::new(kind, "an invented failure".to_owned());
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                assert!(view.apply_updates(
                    key,
                    generation,
                    vec![PodLogUpdate::Failed(failure)],
                    cx,
                ));
            })
        });
    };
    let mut jitter = cx.update(|cx| view.read(cx).source().reads.jitter.clone());
    let tick = Duration::from_millis(1);

    // A transient failure first: the creating container backs off once.
    let delay = jitter.spread(RETRY_FIRST);
    fail(&waiting, FailureKind::Unreachable, cx);
    cx.executor().advance_clock(delay);
    cx.run_until_parked();
    assert_eq!(failures(&waiting, cx), Some(1));

    // Then both are refused, and Retry reads them again.
    fail(&waiting, FailureKind::Forbidden, cx);
    fail(&streaming, FailureKind::Forbidden, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("workload-logs-refused").label(),
            Some("Logs refused for 2 containers: read again when the pods change.")
        );
        window.click("workload-logs-retry-refused", cx);
        window.render_frame(cx);
        let source = view.read(cx).source();
        assert_eq!(
            source.reads.streams[&streaming].state,
            StreamState::Streaming
        );
        assert!(matches!(
            source.reads.streams[&waiting].state,
            StreamState::Waiting(_)
        ));
        assert!(source.refused_note.is_none());
        assert!(window.try_find("workload-logs-refused").is_none());
    })
    .unwrap();
    assert_eq!(failures(&waiting, cx), None);

    // Its next failure waits the first step again, not the second.
    let delay = jitter.spread(RETRY_FIRST);
    let before = generation(&waiting, cx);
    fail(&waiting, FailureKind::Unreachable, cx);
    cx.executor().advance_clock(delay - tick);
    cx.run_until_parked();
    assert_eq!(generation(&waiting, cx), before);
    cx.executor().advance_clock(tick);
    cx.run_until_parked();
    assert!(generation(&waiting, cx) > before);
}

/// A stream that fails for a while reads again with the watcher's backoff,
/// each wait spread by the jitter, and stops once its pod goes.
#[gpui_kit::test]
fn a_failed_stream_reads_again_with_a_jittered_backoff(cx: &mut TestAppContext) {
    use freshkube_core::resources::{Failure, FailureKind};
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        window.render_frame(cx);
    })
    .unwrap();
    // The container still being created: read again, it waits again, so
    // its failures add up.
    let waiting = cx.update(|cx| {
        view.read(cx)
            .source()
            .reads
            .streams
            .iter()
            .find(|(_, stream)| matches!(stream.state, StreamState::Waiting(_)))
            .map(|(key, _)| key.clone())
            .unwrap()
    });
    let generation = |cx: &mut TestAppContext| {
        cx.update(|cx| view.read(cx).source().reads.streams[&waiting].generation)
    };
    let fail = |cx: &mut TestAppContext| {
        let generation = generation(cx);
        let unreachable = Failure::new(FailureKind::Unreachable, "connection refused".to_owned());
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                assert!(view.apply_updates(
                    &waiting,
                    generation,
                    vec![PodLogUpdate::Failed(unreachable)],
                    cx,
                ));
            })
        });
    };
    // The waits the view will draw, from the same sequence.
    let mut jitter = cx.update(|cx| view.read(cx).source().reads.jitter.clone());
    let tick = Duration::from_millis(1);
    for wait in [RETRY_FIRST, RETRY_FIRST * 2] {
        let delay = jitter.spread(wait);
        assert_ne!(delay, wait);
        let before = generation(cx);
        fail(cx);
        cx.executor().advance_clock(delay - tick);
        cx.run_until_parked();
        assert_eq!(generation(cx), before, "{wait:?} spread to {delay:?}");
        cx.executor().advance_clock(tick);
        cx.run_until_parked();
        assert!(generation(cx) > before, "{wait:?} spread to {delay:?}");
    }

    // Once the pod goes, nothing reads it again.
    fail(cx);
    let rest: Vec<_> = all
        .iter()
        .filter(|pod| pod.name != waiting.pod)
        .cloned()
        .collect();
    cx.update(|cx| feed(&view, rest, cx));
    cx.executor().advance_clock(RETRY_MAX * 4);
    cx.run_until_parked();
    assert!(cx.update(|cx| !view.read(cx).source().reads.streams.contains_key(&waiting)));
}

#[test]
fn retry_waits_spread_across_the_jitter() {
    let mut jitter = Jitter::new();
    let wait = Duration::from_secs(10);
    let (least, most) = (
        wait.mul_f64(1. - RETRY_JITTER),
        wait.mul_f64(1. + RETRY_JITTER),
    );
    let draws: Vec<Duration> = (0..1_000).map(|_| jitter.spread(wait)).collect();
    assert!(draws.iter().all(|&delay| delay >= least && delay <= most));
    // Spread over the whole range, not bunched at the plain wait.
    assert!(draws.iter().any(|&delay| delay < wait.mul_f64(0.85)));
    assert!(draws.iter().any(|&delay| delay > wait.mul_f64(1.15)));
}
