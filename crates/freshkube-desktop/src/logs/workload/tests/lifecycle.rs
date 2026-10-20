use super::*;

#[gpui_kit::test]
fn hiding_stops_everything_and_showing_reads_on_without_repeats(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let read = cx
        .update_window(handle, |_, window, cx| {
            show(&view, &api, cx);
            window.render_frame(cx);
            assert!(view.read(cx).reading());
            view.update(cx, |view, cx| view.set_active(false, cx));
            window.render_frame(cx);
            assert!(!view.read(cx).reading());
            assert!(view.read(cx).source().reads.streams.is_empty());
            assert_eq!(status(window), "Idle");
            lines(&view, cx)
        })
        .unwrap();
    // Nothing is written while hidden.
    cx.executor().advance_clock(EXAMPLE_INTERVAL * 3);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| lines(&view, cx)), read);

    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.set_active(true, cx));
        window.render_frame(cx);
        assert!(view.read(cx).reading());
        // Each container reads on from its last line: nothing repeats, and
        // nobody counts as joining.
        assert_eq!(lines(&view, cx), read);
        assert!(markers(&view, cx).is_empty());
    })
    .unwrap();
    cx.executor().advance_clock(EXAMPLE_INTERVAL * 3);
    cx.run_until_parked();
    assert!(cx.update(|cx| lines(&view, cx)) > read);
}

/// Showing again asks each container for its live log from where it
/// stopped, not its tail again.
#[gpui_kit::test]
fn showing_again_reads_on_from_each_streams_saved_position(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let saved = cx
        .update_window(handle, |_, window, cx| {
            show(&view, &api, cx);
            window.render_frame(cx);
            let source = view.read(cx).source();
            // The first read asks for the tail.
            assert!(source.reads.streams.values().all(|stream| {
                stream.request.resume.is_none() && stream.request.tail.is_some()
            }));
            let saved = source.reads.positions.clone();
            view.update(cx, |view, cx| view.set_active(false, cx));
            saved
        })
        .unwrap();
    cx.executor().advance_clock(EXAMPLE_INTERVAL * 3);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.set_active(true, cx));
        window.render_frame(cx);
        let source = view.read(cx).source();
        assert!(!source.reads.streams.is_empty());
        let mut resumed = 0;
        for (key, stream) in &source.reads.streams {
            assert!(!stream.request.previous);
            assert_eq!(stream.request.pod, key.pod);
            assert_eq!(stream.request.container, key.container);
            match saved.get(key).filter(|position| position.time().is_some()) {
                Some(position) => {
                    assert_eq!(stream.request.resume, Some(*position), "{key:?}");
                    resumed += 1;
                }
                None => assert_eq!(stream.request.resume, None, "{key:?}"),
            }
        }
        // Every container that wrote reads on.
        assert_eq!(resumed, tags(&view, cx).len());
    })
    .unwrap();
}

#[gpui_kit::test]
fn another_workload_or_none_drops_every_stream(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        show(&view, &deployment("api"), cx);
        window.render_frame(cx);
        assert!(lines(&view, cx) > 0);

        view.update(cx, |view, cx| {
            view.show_workload(Some(deployment("ledger")), None, cx)
        });
        window.render_frame(cx);
        assert!(!view.read(cx).reading());
        assert!(view.read(cx).source().reads.streams.is_empty());
        assert_eq!(lines(&view, cx), 0);
        assert!(window.try_find("workload-logs-streams").is_none());

        show(&view, &deployment("ledger"), cx);
        window.render_frame(cx);
        assert!(tags(&view, cx).iter().all(|tag| tag.starts_with("ledger-")));
        view.update(cx, |view, cx| view.show_workload(None, None, cx));
        assert!(!view.read(cx).reading());
        assert!(view.read(cx).source().reads.streams.is_empty());
    })
    .unwrap();
    // Nothing writes on once dropped.
    cx.executor().advance_clock(EXAMPLE_INTERVAL * 3);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| lines(&view, cx)), 0);
}
