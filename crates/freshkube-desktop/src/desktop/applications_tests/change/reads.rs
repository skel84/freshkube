use super::*;

/// Follows prod-ams with the example's change answering after `delay`, so
/// its read stays in flight until the clock moves.
fn open_slow(
    cx: &mut TestAppContext,
    delay: std::time::Duration,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = on_stage(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, _| page.set_example_delay(delay));
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

fn reading(view: &Entity<Pilot>, cx: &gpui_kit::App) -> bool {
    change(view, cx)
        .expect("the change shows")
        .read(cx)
        .is_reading()
}

#[gpui_kit::test]
fn the_change_shows_loading_rows_until_it_answers_then_its_stage(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open_slow(cx, std::time::Duration::from_secs(2));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(reading(&view, cx));
        assert!(
            window
                .within("change-list")
                .find("change-loading")
                .visible()
        );
        assert!(list(&view, cx).read(cx).loading_motion(cx).is_some());
        assert_eq!(change(&view, cx).unwrap().read(cx).selected(), None);
        // The title names the Freight before anything is read.
        assert!(window.find("change-title").visible());
        assert_eq!(change(&view, cx).unwrap().read(cx).freight(), "wonky-otter");
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!reading(&view, cx));
        assert!(window.try_find("change-loading").is_none());
        assert!(list(&view, cx).read(cx).loading_motion(cx).is_none());
        assert_eq!(
            change(&view, cx)
                .unwrap()
                .read(cx)
                .selected()
                .map(|k| k.as_ref()),
            Some("prod-ams-eligible"),
            "the first answer selects the Stage it was opened on"
        );
        assert!(window.find(hop("prod-ams-eligible")).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn hiding_drops_the_read_and_showing_reads_again_once_it_is_old(cx: &mut TestAppContext) {
    let delay = std::time::Duration::from_secs(1);
    let (_runtime, handle, view) = open_slow(cx, delay);
    let away_and_back = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.click("nav-overview", cx);
            window.render_frame(cx);
            assert!(!reading(&view, cx), "hidden, it reads nothing");
            window.click("nav-applications", cx);
            window.render_frame(cx);
        })
        .unwrap();
    };
    away_and_back(cx);
    cx.update_window(handle, |_, _, cx| {
        assert!(reading(&view, cx), "shown with nothing read, it reads");
    })
    .unwrap();
    cx.executor().advance_clock(delay);
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| assert!(!reading(&view, cx)))
        .unwrap();

    // What it has is fresh: showing it again reads nothing.
    away_and_back(cx);
    cx.update_window(handle, |_, _, cx| assert!(!reading(&view, cx)))
        .unwrap();

    // Older than half a minute, it reads again.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(31));
    away_and_back(cx);
    cx.update_window(handle, |_, _, cx| assert!(reading(&view, cx)))
        .unwrap();
}

#[gpui_kit::test]
fn refresh_reads_the_change_again(cx: &mut TestAppContext) {
    let delay = std::time::Duration::from_secs(1);
    let (_runtime, handle, view) = open_slow(cx, delay);
    cx.executor().advance_clock(delay);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!reading(&view, cx));
        window.click("change-refresh", cx);
        window.render_frame(cx);
        assert!(reading(&view, cx));
        // The trail stays while it reads again.
        assert!(window.find(hop("prod-ams-eligible")).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_failed_read_says_why_and_retry_reads_again(cx: &mut TestAppContext) {
    let delay = std::time::Duration::from_secs(1);
    let (_runtime, handle, view) = open_slow(cx, delay);
    cx.update_window(handle, |_, window, cx| {
        change(&view, cx).unwrap().update(cx, |page, cx| {
            page.fail_read(
                "connection refused by core-fra.example.test:6443",
                window,
                cx,
            )
        });
        window.render_frame(cx);
        let failed = window.find("change-failed");
        assert!(failed.visible());
        assert!(window.try_find("change-loading").is_none());
        window.click("change-retry", cx);
        window.render_frame(cx);
        assert!(reading(&view, cx));
    })
    .unwrap();
    cx.executor().advance_clock(delay);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("change-failed").is_none());
        assert_eq!(
            change(&view, cx)
                .unwrap()
                .read(cx)
                .selected()
                .map(|k| k.as_ref()),
            Some("prod-ams-eligible")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_failed_refresh_keeps_the_trail_marked_stale(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.press("down", cx);
        let selected = change(&view, cx).unwrap().read(cx).selected().cloned();
        change(&view, cx).unwrap().update(cx, |page, cx| {
            page.fail_read(
                "connection refused by core-fra.example.test:6443",
                window,
                cx,
            )
        });
        window.render_frame(cx);
        assert!(window.find("change-stale").visible());
        assert!(window.try_find("change-failed").is_none());
        assert_eq!(
            change(&view, cx).unwrap().read(cx).selected().cloned(),
            selected,
            "the selection stays"
        );
        assert!(window.find(hop("prod-ams-eligible")).visible());
    })
    .unwrap();
}

/// The shell's Refresh while a read is in flight supersedes it: the first read's
/// answer never lands, and the page answers once, from the second.
#[gpui_kit::test]
fn a_read_superseded_by_refresh_never_lands(cx: &mut TestAppContext) {
    let delay = std::time::Duration::from_secs(2);
    let (_runtime, handle, view) = open_slow(cx, delay);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The button waits for the read; the shell's Refresh doesn't.
        window.press("secondary-r", cx);
        window.render_frame(cx);
    })
    .unwrap();
    // The first read would have answered now.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = change(&view, cx).unwrap();
        assert!(page.read(cx).is_reading());
        assert!(
            !page.read(cx).answered(),
            "the superseded answer never lands"
        );
        assert!(window.find("change-loading").visible());
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        let page = change(&view, cx).unwrap();
        assert!(page.read(cx).answered());
        assert!(!page.read(cx).is_reading());
    })
    .unwrap();
}

/// A read dropped on hiding never lands, even once its time has passed;
/// showing the page again reads afresh.
#[gpui_kit::test]
fn hiding_drops_a_late_answer(cx: &mut TestAppContext) {
    let delay = std::time::Duration::from_secs(1);
    let (_runtime, handle, view) = open_slow(cx, delay);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-overview", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.executor().advance_clock(delay * 3);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let page = change(&view, cx).expect("kept while hidden");
        assert!(
            !page.read(cx).answered(),
            "the dropped read's answer never lands"
        );
        window.click("nav-applications", cx);
        window.render_frame(cx);
        assert!(change(&view, cx).unwrap().read(cx).is_reading());
    })
    .unwrap();
}

/// Another connection drops the change shown with the application page.
#[gpui_kit::test]
fn another_source_drops_the_change(cx: &mut TestAppContext) {
    use crate::resources::{ClusterAccess, ClusterSource};
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        assert!(change(&view, cx).is_some());
        list(&view, cx).update(cx, |page, cx| {
            page.set_source(
                Some(ClusterSource {
                    id: "connection-other".into(),
                    context: "other".into(),
                    access: ClusterAccess::EXAMPLE,
                }),
                None,
                cx,
            )
        });
        window.render_frame(cx);
        assert!(change(&view, cx).is_none());
        assert!(window.try_find("change-page").is_none());
    })
    .unwrap();
}
