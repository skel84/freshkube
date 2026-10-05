use gpui_kit::{
    AppContext, Entity, ScrollDelta, TestAppContext, WindowHandle,
    component::Root,
    point, px, size,
    test::{TestAppContextExt, TestWindowExt},
};
use tokio::runtime::{Builder, Runtime};

use freshkube_core::logs::{LogEvent, ServiceId};

use super::talos::Collection;
use super::{LogPanel, TalosPanel};
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
        let offset = panel.read(cx).scroll_offset();
        let (target, revision) = {
            let view = panel.read(cx);
            (
                view.source().fixture_target.clone().unwrap(),
                view.source().stream_revision,
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
        assert_eq!(panel.read(cx).scroll_offset(), offset);
        assert!(!panel.read(cx).anchor_evicted());
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
        assert!(panel.read(cx).anchor_evicted());
        assert!(panel.read(cx).status_line().contains("evicted"));
        panel.update(cx, |view, cx| view.set_target(None, Vec::new(), window, cx));
        window.render_frame(cx);
        assert!(panel.read(cx).visible_rows().is_empty());
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
        assert!(panel.read(cx).visible_rows().is_empty());
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
                view.retained()
                    .iter()
                    .all(|entry| { !entry.raw.contains("seq=") })
            );
            let collected = view.source().collecting.clone();
            assert_eq!(collected.len(), 2);
            let initial_next_id = view.next_line_id();
            window.click("logs-collection", cx);
            assert!(panel.read(cx).source().collection_active);
            assert!(panel.read(cx).source().job.is_some());
            assert!(panel.read(cx).source().delivery.is_some());
            (collected, initial_next_id)
        })
        .unwrap();
    cx.wait_for(handle.into(), std::time::Duration::from_secs(2), |_, cx| {
        let view = panel.read(cx);
        collected.iter().all(|service| {
            view.retained()
                .iter()
                .any(|entry| &entry.service == service && entry.raw.contains("seq="))
        })
    })
    .await;

    let paused_counts = cx
        .update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(panel.read(cx).next_line_id() > initial_next_id);
            assert!(panel.read(cx).following());
            window.click("logs-follow", cx);
            assert!(!panel.read(cx).following());
            assert!(panel.read(cx).source().collection_active);
            window.click("show-apid", cx);
            let view = panel.read(cx);
            assert!(!view.shown().contains(&ServiceId::from("apid")));
            assert_eq!(view.source().collecting, collected);
            assert!(view.shown().contains(&ServiceId::from("kubelet")));
            assert!(
                view.visible_rows()
                    .iter()
                    .any(|&ix| { view.retained()[ix].raw.contains("seq=") })
            );
            assert!(
                view.visible_rows()
                    .iter()
                    .all(|&ix| { view.retained()[ix].service.as_str() == "kubelet" })
            );
            collected
                .iter()
                .map(|service| {
                    let count = view
                        .retained()
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
        assert!(!view.following());
        assert!(view.source().collection_active);
        paused_counts.iter().all(|(service, previous_count)| {
            view.retained()
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
            assert!(!view.source().collection_active);
            assert!(view.source().job.is_none());
            assert!(view.source().delivery.is_none());
            assert!(!view.following());
            view.next_line_id()
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
            assert_eq!(view.next_line_id(), stopped_next_id);
            assert!(!view.source().collection_active);
            assert!(view.source().job.is_none());
            assert!(view.source().delivery.is_none());
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
        assert!(panel.read(cx).following());
        assert!(!panel.read(cx).source().collection_active);
        assert_eq!(panel.read(cx).next_line_id(), stopped_next_id);
    })
    .unwrap();
}
