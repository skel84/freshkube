use super::*;

/// A stream behind the others delivers lines older than ones already
/// shown: they take their place by time, a paused review stays on the line
/// it showed, and the selection keeps its line.
#[gpui_kit::test]
fn a_lagging_streams_lines_take_their_place_without_moving_the_review(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    let lagging = all[0].clone();
    let now = cx.update(|cx| view.read(cx).source().reads.clock.now(cx));
    let line = |minutes_ago: i64, text: &str| {
        let at = now - chrono::TimeDelta::minutes(minutes_ago);
        PodLogUpdate::Line(format!(
            "{} level=info msg=\"{text}\"",
            at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
        ))
    };
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        window.render_frame(cx);

        // Following: a late line lands among the old ones, and the newest
        // row stays in view.
        let generation = view.read(cx).source().reads.streams[&key(&lagging)].generation;
        let newest = view.read(cx).retained().last().unwrap().raw.clone();
        view.update(cx, |view, cx| {
            assert!(view.apply_updates(
                &key(&lagging),
                generation,
                vec![line(30, "late while following")],
                cx,
            ));
        });
        window.render_frame(cx);
        let retained = view.read(cx).retained();
        let late = retained
            .iter()
            .position(|entry| entry.raw.contains("late while following"))
            .unwrap();
        assert!(late + 1 < retained.len(), "placed by time, not appended");
        assert_eq!(retained.last().unwrap().raw, newest);
        assert!(view.read(cx).following());

        // Paused on a selected line in the middle of the review.
        window.click("logs-follow", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).following());
        let view_ref = view.read(cx);
        let rows = view_ref.visible_rows().len();
        let middle = view_ref.row_id(rows - 5);
        let generation_id = view_ref.generation();
        let row = format!("log-line-{generation_id}-{middle}");
        window.click(row.clone(), cx);
        window.render_frame(cx);
        assert_eq!(window.find(row.clone()).selected(), Some(true));
        let top_before = window.find(row.clone()).bounds().origin.y;

        let generation = view.read(cx).source().reads.streams[&key(&lagging)].generation;
        view.update(cx, |view, cx| {
            assert!(
                view.apply_updates(
                    &key(&lagging),
                    generation,
                    (0..12)
                        .map(|ix| line(50 - ix, "late while paused"))
                        .collect(),
                    cx,
                )
            );
        });
        window.render_frame(cx);
        window.render_frame(cx);
        // The new lines sit above the selection, which stays put.
        let first_late = view
            .read(cx)
            .retained()
            .iter()
            .position(|entry| entry.raw.contains("late while paused"))
            .unwrap();
        let view_ref = view.read(cx);
        let selected_row = (0..view_ref.visible_rows().len())
            .find(|&ix| view_ref.row_id(ix) == middle)
            .unwrap();
        assert!(first_late < view_ref.visible_rows()[selected_row]);
        assert!(!view_ref.following());
        assert!(
            view_ref
                .review_status(true)
                .contains(&"1 selected".to_owned())
        );
        assert_eq!(window.find(row.clone()).selected(), Some(true));
        assert_eq!(window.find(row).bounds().origin.y, top_before);
    })
    .unwrap();
}

/// However many containers write, one delivery hands their lines to the
/// view: one apply, one ingest, one notify a frame.
#[gpui_kit::test]
fn busy_streams_are_applied_once_a_delivery(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let mut all = pods(&api, "app=api");
    for pod in &mut all {
        let mut sidecar: Container = pod.containers.containers[0].clone();
        sidecar.name = "proxy".into();
        sidecar.role = ContainerRole::App;
        sidecar.state = ContainerState::Running(None);
        pod.containers.containers.push(sidecar);
    }
    let now = cx.update(|cx| view.read(cx).source().reads.clock.now(cx));
    for busy in [13, MAX_STREAMS] {
        cx.update_window(handle, |_, window, cx| {
            show_fed(&view, &api, cx);
            feed(&view, all.clone(), cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        let streams: Vec<(StreamKey, u64)> = cx.update(|cx| {
            view.read(cx)
                .source()
                .reads
                .streams
                .iter()
                .map(|(key, stream)| (key.clone(), stream.generation))
                .take(busy)
                .collect()
        });
        assert_eq!(streams.len(), busy);
        let sender = cx.update(|cx| view.update(cx, |view, cx| view.feed(cx)));
        let before = (
            probe::count("workload-logs.apply"),
            cx.update(|cx| lines(&view, cx)),
        );
        // Each container writes ten lines before the next delivery.
        for round in 0..10 {
            for (ix, (key, generation)) in streams.iter().enumerate() {
                let at = now + chrono::TimeDelta::milliseconds((round * 100 + ix) as i64);
                let line = format!(
                    "{} level=info msg=\"busy\"",
                    at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
                );
                sender
                    .try_send(Fed {
                        key: key.clone(),
                        generation: *generation,
                        update: PodLogUpdate::Line(line),
                    })
                    .unwrap();
            }
        }
        cx.run_until_parked();
        assert_eq!(probe::count("workload-logs.apply"), before.0 + 1, "{busy}");
        assert_eq!(cx.update(|cx| lines(&view, cx)), before.1 + busy * 10);
        // Another workload starts afresh.
        cx.update(|cx| view.update(cx, |view, cx| view.show_workload(None, None, cx)));
    }
}
