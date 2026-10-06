use super::super::{Report, example, tests::mount};
use super::LEAST_LIST_HEIGHT;
use freshkube_core::types::LogLevel;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext};
use std::time::Duration;

type Page = Entity<super::ObservabilityPage>;

fn open(cx: &mut TestAppContext, page: &Page, app: &str) {
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.open_app(example::id(app), Report::Logs, cx)
        })
    });
}

/// The retained lines' text, markers included, oldest first.
fn lines(cx: &mut TestAppContext, page: &Page) -> Vec<String> {
    cx.read(|cx| {
        let view = page.read(cx).live_logs.view.read(cx);
        view.retained().iter().map(|e| e.raw.clone()).collect()
    })
}

/// The id of the shown row whose line contains `text`.
fn row(cx: &mut TestAppContext, page: &Page, text: &str) -> SharedString {
    cx.read(|cx| {
        let view = page.read(cx).live_logs.view.read(cx);
        let ix = view
            .visible_rows()
            .iter()
            .position(|&ix| view.retained()[ix].raw.contains(text))
            .unwrap_or_else(|| panic!("no row shows {text}"));
        format!("log-line-{}-{}", view.generation(), view.row_id(ix)).into()
    })
}

fn draw(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_logs_report_shows_coroot_s_histogram_and_messages_at_coroot_s_levels(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, page) = mount(cx, true);
    open(cx, &page, example::WORKER);
    draw(cx, handle);
    cx.update_window(handle, |_, window, _| {
        for id in [
            "obs-logs",
            "obs-logs-origin-containers",
            "obs-logs-origin-otel",
            "obs-logs-mode-messages",
            "obs-logs-mode-patterns",
            "obs-logs-limit",
            "obs-logs-histogram",
            "obs-logs-list",
            "logs-panel",
        ] {
            assert!(window.find(id).visible(), "{id}");
        }
        assert_eq!(window.find("logs-panel").label(), Some("Coroot logs panel"));
        // The checks still read Coroot's judgement of the logs.
        assert!(window.find("obs-app-checks").visible());
    })
    .unwrap();
    let lines = lines(cx, &page);
    assert_eq!(lines.len(), 12, "{lines:#?}");
    cx.read(|cx| {
        let view = page.read(cx).live_logs.view.read(cx);
        let level = |text: &str| {
            view.retained()
                .iter()
                .find(|e| e.raw.contains(text))
                .map(|e| e.level.clone())
        };
        // Coroot's severity, not one the words suggest.
        assert_eq!(level("GC forced"), Some(LogLevel::Unknown));
        assert_eq!(level("retrying batch"), Some(LogLevel::Error));
        assert_eq!(level("goroutine 1"), Some(LogLevel::Error));
        assert_eq!(view.level_counts(), [3, 2, 5, 1, 1]);
    });
}

#[gpui_kit::test]
fn a_long_trace_is_one_row_and_a_search_finds_its_last_line(cx: &mut TestAppContext) {
    // Tall enough that the whole list is on screen below the map and checks.
    let (_runtime, handle, page) = super::super::tests::mount_size(cx, true, 1260., 1800.);
    let measured = || freshkube_probe::probe::count("logs.measure");
    let before = measured();
    open(cx, &page, example::WORKER);
    // The first draw lays out every row on screen, the trace's included.
    // Wall-clock times are printed for the record, never asserted.
    let started = std::time::Instant::now();
    draw(cx, handle);
    let first = started.elapsed();
    let laid_out = measured() - before;
    let lines = lines(cx, &page);
    let traces: Vec<&String> = lines.iter().filter(|l| l.contains("goroutine 1")).collect();
    assert_eq!(traces.len(), 1, "the trace is one row");
    assert_eq!(traces[0].lines().count(), 200);
    assert!(traces[0].len() <= 64 * 1024 && traces[0].len() > 60 * 1024);
    let trace = row(cx, &page, "goroutine 1");
    let last = row(cx, &page, "worker restarted");
    // From the top, the trace's last line is far below the list.
    cx.update_window(handle, |_, window, cx| {
        window.click(last, cx);
        window.press("home", cx);
        window.render_frame(cx);
        let viewport = window.find("logs-viewport").bounds();
        let bounds = window.find(trace.clone()).bounds();
        assert!(bounds.top() > viewport.top(), "{bounds:?} in {viewport:?}");
    })
    .unwrap();
    let started = std::time::Instant::now();
    cx.update_window(handle, |_, window, cx| {
        window.click("logs-search", cx);
        window.input("drainSettlementQueue", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.click("logs-search-next", cx);
        window.render_frame(cx);
        window.render_frame(cx);
        // The search finds the trace by its last line and scrolls into
        // it. The shared view reveals a matched row by its middle, not by
        // the line that matched.
        let viewport = window.find("logs-viewport").bounds();
        let bounds = window.find(trace.clone()).bounds();
        assert!(bounds.top() < viewport.top(), "{bounds:?} in {viewport:?}");
        assert!(
            bounds.bottom() > viewport.bottom(),
            "{bounds:?} in {viewport:?}"
        );
    })
    .unwrap();
    cx.run_until_parked();
    let search = started.elapsed();
    // Once measured, the next frame lays out no row again.
    let settled = measured();
    let started = std::time::Instant::now();
    cx.update_window(handle, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    let again = started.elapsed();
    assert_eq!(measured(), settled, "a settled frame measures nothing");
    eprintln!(
        "200-line trace: first draw {first:?} ({laid_out} rows laid out), search {search:?}, next frame {again:?}"
    );
}

#[gpui_kit::test]
fn a_refresh_adds_only_newer_messages(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    open(cx, &page, example::WORKER);
    draw(cx, handle);
    let before = lines(cx, &page);
    cx.executor().advance_clock(Duration::from_secs(65));
    cx.update(|cx| page.update(cx, |page, cx| page.refresh_current(cx)));
    draw(cx, handle);
    let after = lines(cx, &page);
    assert_eq!(after.len(), before.len() + 2, "{after:#?}");
    assert_eq!(after[..before.len()], before[..]);
    assert!(after.iter().all(|l| !l.contains("weren't read")));
    // Again at once: nothing new, nothing twice.
    cx.update(|cx| page.update(cx, |page, cx| page.refresh_current(cx)));
    draw(cx, handle);
    assert_eq!(lines(cx, &page), after);
}

#[gpui_kit::test]
fn a_page_hidden_past_its_window_marks_what_it_missed(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    open(cx, &page, example::WORKER);
    draw(cx, handle);
    let before = lines(cx, &page).len();
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(false, cx)));
    cx.executor().advance_clock(Duration::from_secs(4 * 3600));
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(true, cx)));
    draw(cx, handle);
    let after = lines(cx, &page);
    let marker = after
        .iter()
        .position(|l| l.contains("weren't read"))
        .expect("a note on the messages missed");
    // The note sits after the earlier lines and before the window's.
    assert_eq!(marker, before);
    assert!(after.len() > before + 1);
    // The earlier lines are kept; markers aren't counted as messages.
    cx.read(|cx| {
        let view = page.read(cx).live_logs.view.read(cx);
        assert!(view.retained()[marker].timestamp.is_some());
    });
}

#[gpui_kit::test]
fn origin_view_and_limit_start_over(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    open(cx, &page, example::WORKER);
    draw(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("obs-logs-mode-patterns", cx);
        window.render_frame(cx);
        assert!(window.try_find("obs-logs-list").is_none());
        assert!(window.find("obs-log-patterns-row-0").visible());
        assert!(window.try_find("obs-logs-pattern-chart").is_none());
        window.click("obs-log-patterns-row-2", cx);
    })
    .unwrap();
    draw(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("obs-logs-pattern-chart").visible());
        window.click("obs-logs-mode-messages", cx);
        window.render_frame(cx);
        assert!(window.find("obs-logs-list").visible());
    })
    .unwrap();
    assert_eq!(lines(cx, &page).len(), 12);
    // Ten of twelve: Coroot says it sent no more than asked.
    cx.update(|cx| page.update(cx, |page, cx| page.change_logs(|q| q.limit = 10, cx)));
    draw(cx, handle);
    assert_eq!(lines(cx, &page).len(), 10);
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("obs-logs-note").visible());
    })
    .unwrap();
    assert!(cx.read(|cx| page.read(cx).live_logs.capped));
    cx.update_window(handle, |_, window, cx| {
        window.click("obs-logs-origin-containers", cx)
    })
    .unwrap();
    draw(cx, handle);
    assert_eq!(lines(cx, &page).len(), 10);
    assert_eq!(
        cx.read(|cx| page.read(cx).live_logs.origin),
        Some(freshkube_core::coroot::LogOrigin::Containers)
    );
}

#[gpui_kit::test]
fn without_a_logs_store_coroot_s_reason_and_patterns_show(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    open(cx, &page, "payments/ledger-db");
    draw(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("obs-logs-note").visible());
        assert!(window.find("obs-log-patterns-row-1").visible());
        // Nothing to pick between: no origin, view or limit.
        for id in ["obs-logs-mode-messages", "obs-logs-limit", "obs-logs-list"] {
            assert!(window.try_find(id).is_none(), "{id}");
        }
        window.click("obs-log-patterns-row-0", cx);
    })
    .unwrap();
    draw(cx, handle);
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("obs-logs-pattern-chart").visible());
    })
    .unwrap();
    // Another application's patterns never stay.
    open(cx, &page, "payments/api");
    draw(cx, handle);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("obs-log-patterns-row-0").is_none());
        assert_eq!(
            window.find("logs-empty").label(),
            Some("No logs found in ClickHouse")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_short_window_still_shows_the_toolbar_and_three_rows(cx: &mut TestAppContext) {
    for text_size in [13., 20.] {
        let (_runtime, handle, page) = super::super::tests::mount_geometry(cx, 760., 560.);
        cx.update_window(handle, |_, _, cx| crate::text_size::set(text_size, cx))
            .unwrap();
        cx.run_until_parked();
        open(cx, &page, example::WORKER);
        draw(cx, handle);
        let last = row(cx, &page, "worker restarted");
        cx.update_window(handle, |_, window, _| {
            let list = window.find("obs-logs-list").bounds();
            assert!(
                list.size.height >= crate::ui::dp_px(LEAST_LIST_HEIGHT, window) - gpui_kit::px(1.),
                "{text_size}: {list:?}"
            );
            let toolbar = window.find("logs-toolbar").bounds();
            let viewport = window.find("logs-viewport").bounds();
            assert!(toolbar.top() >= list.top() && viewport.bottom() <= list.bottom());
            let one = window.find(last.clone()).bounds().size.height;
            assert!(
                viewport.size.height >= one * 3.,
                "{text_size}: {viewport:?} holds fewer than three {one:?} rows"
            );
        })
        .unwrap();
    }
}
