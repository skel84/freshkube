use gpui_kit::{
    AppContext, Entity, ScrollDelta, SharedString, TestAppContext, WindowHandle,
    component::{Root, Theme},
    point, px, size,
    test::{TestAppContextExt, TestWindowExt},
};
use tokio::runtime::{Builder, Runtime};

use freshkube_core::logs::{LogEvent, ServiceId};

use super::LogPanel;
use crate::backend::StreamEvent;

fn fixture_events() -> Vec<LogEvent> {
    (0..120)
        .map(|ix| {
            let detail = if ix == 10 {
                "error needle first 東京".to_owned()
            } else if ix == 90 {
                format!(
                    "error needle last {}",
                    "wrapped Unicode Δ 東京 🚀 ".repeat(24)
                )
            } else {
                format!("info ordinary complete line {ix}")
            };
            LogEvent::new(if ix % 2 == 0 { "apid" } else { "kubelet" }, detail)
        })
        .collect()
}

fn mount(cx: &mut TestAppContext) -> (Runtime, Entity<LogPanel>, WindowHandle<Root>) {
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(gpui_kit::init);
    let mut panel = None;
    let handle = cx.open_window(size(px(620.), px(760.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = LogPanel::new(runtime.handle().clone(), 100, window, cx);
            view.set_fixture(fixture_events(), window, cx);
            view
        });
        panel = Some(view.clone());
        Root::new(view, window, cx)
    });
    (runtime, panel.unwrap(), handle)
}

/// Lets a resize settle, then delivers frames until no row is estimated.
fn settle(cx: &mut TestAppContext, panel: &Entity<LogPanel>, handle: WindowHandle<Root>) {
    cx.executor().advance_clock(super::RESIZE_SETTLE);
    cx.run_until_parked();
    for _ in 0..64 {
        let settled = cx
            .update_window(handle.into(), |_, window, cx| {
                window.simulate_next_frame(cx);
                window.render_frame(cx);
                !panel.read(cx).row_exact.contains(&false)
            })
            .unwrap();
        if settled {
            return;
        }
    }
    panic!("estimated rows never settled");
}

#[gpui_kit::test]
fn live_resize_lays_out_rows_on_screen_and_settles_the_rest_afterwards(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    let measured = || crate::desktop::probe::count("logs.measure");
    let tall = SharedString::from("log-line-1-90");
    let mut original = Vec::new();
    let mut before = 0;
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        // Review from the top, so the tall row 90 is off screen.
        panel.update(cx, |view, cx| view.navigate(isize::MIN, false, cx));
        window.render_frame(cx);
        window.render_frame(cx);
        let view = panel.read(cx);
        assert!(!view.following);
        assert_eq!(view.scroll.offset().y, px(0.));
        assert!(window.try_find(tall.clone()).is_none());
        original = view.sizes.iter().map(|row| row.height).collect::<Vec<_>>();
        before = measured();
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(900.), px(760.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let view = panel.read(cx);
        let laid_out = measured() - before;
        assert!(
            laid_out > 0 && laid_out < original.len() / 2,
            "a resize frame laid out {laid_out} of {} rows",
            original.len()
        );
        assert!(!view.settled);
        // Off screen, the tall row keeps its last height until the
        // width holds.
        assert!(!view.row_exact[90]);
        assert_eq!(view.sizes[90].height, original[90]);
        assert_eq!(view.scroll.offset().y, px(0.));
        let mut shown = 0;
        for ix in 0..view.sizes.len() {
            let id = SharedString::from(format!("log-line-1-{}", view.review.id(ix)));
            if window.try_find(id).is_some_and(|row| row.visible()) {
                assert!(view.row_exact[ix], "row {ix} is on screen with an estimate");
                shown += 1;
            }
        }
        assert!(shown > 10);
    })
    .unwrap();
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, _, cx| {
        let view = panel.read(cx);
        assert!(view.settled);
        assert!(view.sizes[90].height < original[90]);
        // Settling rows below the review position never moves it.
        assert_eq!(view.scroll.offset().y, px(0.));
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(620.), px(760.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, _, cx| {
        let heights: Vec<_> = panel.read(cx).sizes.iter().map(|row| row.height).collect();
        assert_eq!(
            heights, original,
            "settled heights differ from a fresh layout"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn searches_reveal_inner_rows_and_keyboard_copies_complete_selected_lines(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    let row = SharedString::from("log-line-1-10");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let toolbar = window.find("logs-toolbar").bounds();
        assert!(toolbar.size.height > window.rem_size());
        let viewport = window.find("logs-viewport").bounds();
        let search = window.find("logs-search").bounds();
        assert!(search.bottom() <= viewport.top());
        assert!(viewport.top() - search.bottom() <= window.rem_size() * 2.);
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("unchanged".into()));
        let feedback = panel.read(cx).feedback.clone();
        window.click("logs-copy", cx);
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("unchanged")
        );
        assert_eq!(panel.read(cx).feedback, feedback);
        assert!(window.try_find(row.clone()).is_none());
        window.click("logs-search", cx);
        window.input("needle", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        let toolbar = window.find("logs-search").bounds();
        window.click("logs-search-next", cx);
        let viewport = window.find("logs-viewport").bounds();
        let line = window.find(row.clone());
        assert!(line.visible());
        assert!(line.bounds().top() >= viewport.top());
        assert!(line.bounds().bottom() <= viewport.bottom());
        assert_eq!(window.find("logs-search").bounds(), toolbar);
        assert_eq!(panel.read(cx).review.visible.len(), 120);
        assert!(!panel.read(cx).following);
        assert_eq!(panel.read(cx).review.current_match, Some(10));
        window.click(row, cx);
        assert_eq!(panel.read(cx).review.selected.len(), 1);
        assert!(panel.read(cx).focus.is_focused(window));
        window.press("shift-down", cx);
        assert_eq!(panel.read(cx).review.selected.len(), 2);
        window.press("secondary-c", cx);
        let text = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap();
        assert_eq!(
            text,
            "error needle first 東京\ninfo ordinary complete line 11"
        );
        assert!(!text.contains("[apid]"));
        window.press("escape", cx);
        assert!(panel.read(cx).review.selected.is_empty());
        window.click("logs-search-next", cx);
        assert_eq!(panel.read(cx).review.current_match, Some(90));
        assert!(window.find(SharedString::from("log-line-1-90")).visible());
        window.click("logs-search-prev", cx);
        assert_eq!(panel.read(cx).review.current_match, Some(10));
    })
    .unwrap();
}

#[gpui_kit::test]
fn wrap_and_rem_remeasure_real_rows_and_wheel_pauses_follow(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let original = panel.read(cx).sizes[90].height;
        assert!(original > panel.read(cx).sizes[89].height);
        let toolbar = window.find("logs-follow").bounds();
        let before = panel.read(cx).scroll.offset();
        let viewport = window.find("logs-viewport").bounds();
        // Use the toolkit's visible native thumb, not the model seam.
        window.drag(
            point(viewport.right() - px(8.), viewport.bottom() - px(24.)),
            point(viewport.right() - px(8.), viewport.top() + px(40.)),
            cx,
        );
        assert!(!panel.read(cx).following);
        assert_ne!(panel.read(cx).scroll.offset(), before);
        window.click("logs-follow", cx);
        assert!(panel.read(cx).following);
        window.scroll(
            "logs-viewport",
            ScrollDelta::Pixels(point(px(0.), px(900.))),
            cx,
        );
        assert!(!panel.read(cx).following);
        assert_ne!(panel.read(cx).scroll.offset(), before);
        assert_eq!(window.find("logs-follow").bounds(), toolbar);
        window.click("logs-wrap", cx);
        assert!(!panel.read(cx).wrapped);
        let unwrapped = panel.read(cx).sizes[90].height;
        assert!(unwrapped < original);
        assert_eq!(unwrapped, panel.read(cx).sizes[89].height);
        window.click("logs-wrap", cx);
        assert_eq!(panel.read(cx).sizes[90].height, original);
        Theme::update(cx, |theme| theme.font_size = px(20.));
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(panel.read(cx).sizes[90].height > original);
        window.click("logs-follow", cx);
        assert!(panel.read(cx).following);
        assert!(window.find(SharedString::from("log-line-1-119")).visible());
        assert_eq!(window.find("logs-wrap").checked(), Some(true));
        panel.update(cx, |view, cx| {
            let target = view.source.fixture_target.clone().unwrap();
            assert!(view.apply_batch(
                &target,
                view.source.stream_revision,
                vec![StreamEvent {
                    target: target.clone(),
                    service: ServiceId::from("apid"),
                    result: Ok(format!(
                        "info tall final row {}",
                        "Unicode 東京 latest text ".repeat(500)
                    )),
                }],
                cx
            ));
        });
        window.render_frame(cx);
        let viewport = window.find("logs-viewport").bounds();
        let last = window.find(SharedString::from("log-line-1-120")).bounds();
        assert!(last.size.height > viewport.size.height);
        assert!(last.bottom() <= viewport.bottom());
        assert!(last.bottom() >= viewport.bottom() - px(2.));
        let total: gpui_kit::Pixels = panel.read(cx).sizes.iter().map(|row| row.height).sum();
        assert_eq!(
            panel.read(cx).scroll.offset().y,
            viewport.size.height - px(2.) - total
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn paused_anchor_survives_arrival_then_labels_eviction_and_rejects_stale_delivery(
    cx: &mut TestAppContext,
) {
    let (_runtime, panel, handle) = mount(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        window.scroll(
            "logs-viewport",
            ScrollDelta::Pixels(point(px(0.), px(600.))),
            cx,
        );
        let offset = panel.read(cx).scroll.offset();
        let (target, revision) = {
            let view = panel.read(cx);
            (
                view.source.fixture_target.clone().unwrap(),
                view.source.stream_revision,
            )
        };
        panel.update(cx, |view, cx| {
            assert!(view.apply_batch(
                &target,
                revision,
                vec![StreamEvent {
                    target: target.clone(),
                    service: ServiceId::from("apid"),
                    result: Ok("info newly received line".into()),
                }],
                cx
            ));
        });
        window.render_frame(cx);
        assert_eq!(panel.read(cx).scroll.offset(), offset);
        assert!(!panel.read(cx).anchor_evicted);
        panel.update(cx, |view, cx| {
            let batch = (0..freshkube_core::constants::MAX_LOG_ENTRIES)
                .map(|ix| StreamEvent {
                    target: target.clone(),
                    service: ServiceId::from("apid"),
                    result: Ok(format!("info eviction {ix}")),
                })
                .collect();
            assert!(view.apply_batch(&target, revision, batch, cx));
        });
        window.render_frame(cx);
        assert!(panel.read(cx).anchor_evicted);
        assert!(panel.read(cx).status_line().contains("evicted"));
        panel.update(cx, |view, cx| view.set_target(None, Vec::new(), window, cx));
        window.render_frame(cx);
        assert!(panel.read(cx).review.visible.is_empty());
        panel.update(cx, |view, cx| {
            assert!(!view.apply_batch(
                &target,
                revision,
                vec![StreamEvent {
                    target: target.clone(),
                    service: ServiceId::from("apid"),
                    result: Ok("error stale".into()),
                }],
                cx
            ));
        });
        assert!(panel.read(cx).review.visible.is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn synthetic_collection_start_stop_and_filters_use_real_controls(cx: &mut TestAppContext) {
    // Tokio's producer uses real time. Let GPUI park for external wakes
    // rather than racing through its deterministic test-clock timers.
    cx.executor().allow_parking();
    let (_runtime, panel, handle) = mount(cx);
    let (collected, initial_next_id) = cx
        .update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            let view = panel.read(cx);
            assert!(
                view.review
                    .logs
                    .buffer()
                    .entries()
                    .iter()
                    .all(|entry| { !entry.raw.contains("seq=") })
            );
            let collected = view.source.collecting.clone();
            assert_eq!(collected.len(), 2);
            let initial_next_id = view.review.next_id;
            window.click("logs-collection", cx);
            assert!(panel.read(cx).source.collection_active);
            assert!(panel.read(cx).source.job.is_some());
            assert!(panel.read(cx).source.delivery.is_some());
            (collected, initial_next_id)
        })
        .unwrap();
    cx.wait_for(handle.into(), std::time::Duration::from_secs(2), |_, cx| {
        let view = panel.read(cx);
        collected.iter().all(|service| {
            view.review
                .logs
                .buffer()
                .entries()
                .iter()
                .any(|entry| &entry.service == service && entry.raw.contains("seq="))
        })
    })
    .await;

    let paused_counts = cx
        .update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(panel.read(cx).review.next_id > initial_next_id);
            assert!(panel.read(cx).following);
            window.click("logs-follow", cx);
            assert!(!panel.read(cx).following);
            assert!(panel.read(cx).source.collection_active);
            window.click("show-apid", cx);
            let view = panel.read(cx);
            assert!(!view.showing.contains(&ServiceId::from("apid")));
            assert_eq!(view.source.collecting, collected);
            assert!(view.showing.contains(&ServiceId::from("kubelet")));
            assert!(
                view.review
                    .visible
                    .iter()
                    .any(|&ix| { view.review.logs.buffer().entries()[ix].raw.contains("seq=") })
            );
            assert!(view.review.visible.iter().all(|&ix| {
                view.review.logs.buffer().entries()[ix].service.as_str() == "kubelet"
            }));
            collected
                .iter()
                .map(|service| {
                    let count = view
                        .review
                        .logs
                        .buffer()
                        .entries()
                        .iter()
                        .filter(|entry| &entry.service == service && entry.raw.contains("seq="))
                        .count();
                    (service.clone(), count)
                })
                .collect::<Vec<_>>()
        })
        .unwrap();
    cx.wait_for(handle.into(), std::time::Duration::from_secs(2), |_, cx| {
        let view = panel.read(cx);
        assert!(!view.following);
        assert!(view.source.collection_active);
        paused_counts.iter().all(|(service, previous_count)| {
            view.review
                .logs
                .buffer()
                .entries()
                .iter()
                .filter(|entry| &entry.service == service && entry.raw.contains("seq="))
                .count()
                > *previous_count
        })
    })
    .await;

    let stopped_next_id = cx
        .update_window(handle.into(), |_, window, cx| {
            window.click("logs-collection", cx);
            let view = panel.read(cx);
            assert!(!view.source.collection_active);
            assert!(view.source.job.is_none());
            assert!(view.source.delivery.is_none());
            assert!(!view.following);
            view.review.next_id
        })
        .unwrap();
    let stopped_at = std::time::Instant::now();
    cx.wait_for(
        handle.into(),
        std::time::Duration::from_secs(2),
        |window, cx| {
            let view = panel.read(cx);
            // Use a monotonic arrival identity, not bounded buffer size,
            // and observe six real producer ticks with GPUI still driven.
            assert_eq!(view.review.next_id, stopped_next_id);
            assert!(!view.source.collection_active);
            assert!(view.source.job.is_none());
            assert!(view.source.delivery.is_none());
            assert_eq!(
                window.find("logs-collection").label(),
                Some("Start collecting")
            );
            stopped_at.elapsed() >= std::time::Duration::from_millis(120)
        },
    )
    .await;
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("logs-follow", cx);
        assert!(panel.read(cx).following);
        assert!(!panel.read(cx).source.collection_active);
        assert_eq!(panel.read(cx).review.next_id, stopped_next_id);
    })
    .unwrap();
}
