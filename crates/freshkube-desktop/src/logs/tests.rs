use gpui_kit::{
    AppContext, Entity, ScrollDelta, SharedString, TestAppContext, WindowHandle,
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
            // The test window is short, so the eye is in the services list.
            window.click("logs-services-more", cx);
            window.render_frame(cx);
            window.click("list-show-apid", cx);
            window.render_frame(cx);
            window.click("logs-services-more", cx);
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

#[gpui_kit::test]
fn talos_logs_keep_the_shared_stream_wording(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    let label = cx
        .update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.find("logs-panel").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("Live logs panel"));
    panel.read_with(cx, |view, _| {
        assert_eq!(
            <super::TalosLogs as super::LogSource>::follow_tooltip(view),
            "Pause to review. Collection keeps running."
        )
    });
}

/// Twelve services, one line each, in a window of the given size.
fn mount_catalog(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (Runtime, Entity<LogPanel>, WindowHandle<Root>) {
    mount_services(cx, width, height, CATALOG.map(str::to_owned).to_vec())
}

/// These services, one line each, in a window of the given size.
fn mount_services(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    services: Vec<String>,
) -> (Runtime, Entity<LogPanel>, WindowHandle<Root>) {
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let events: Vec<LogEvent> = services
        .iter()
        .map(|service| LogEvent::new(service.as_str(), format!("info {service} started")))
        .collect();
    let mut panel = None;
    let handle = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = LogPanel::new(runtime.handle().clone(), 100, window, cx);
            view.set_fixture(events, window, cx);
            view
        });
        panel = Some(view.clone());
        Root::new(view, window, cx)
    });
    (runtime, panel.unwrap(), handle)
}

const CATALOG: [&str; 12] = [
    "apid",
    "auditd",
    "containerd",
    "cri",
    "dashboard",
    "etcd",
    "kubelet",
    "machined",
    "syslogd",
    "trustd",
    "udevd",
    "ext-example",
];

/// The services whose pill draws in the panel's rows.
fn in_rows(window: &gpui_kit::Window) -> Vec<&'static str> {
    CATALOG
        .into_iter()
        .filter(|service| {
            window
                .try_find(SharedString::from(format!("collect-{service}")))
                .is_some()
        })
        .collect()
}

#[gpui_kit::test]
fn a_wide_panel_shows_every_service_in_its_rows(cx: &mut TestAppContext) {
    let (_runtime, _panel, handle) = mount_catalog(cx, 1100., 820.);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(in_rows(window), CATALOG.to_vec());
        assert!(window.try_find("logs-services-more").is_none());
        assert!(window.find("logs-services").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_narrow_panel_fits_two_rows_and_lists_every_service(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount_catalog(cx, 460., 820.);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let shown = in_rows(window);
        assert!(
            !shown.is_empty() && shown.len() < CATALOG.len(),
            "{shown:?}"
        );
        // The pills that fit come first, in the catalog's order, in two
        // rows at most, and "+N" counts the rest.
        assert_eq!(shown, CATALOG[..shown.len()].to_vec());
        let mut tops: Vec<_> = shown
            .iter()
            .map(|service| {
                window
                    .find(SharedString::from(format!("collect-{service}")))
                    .bounds()
                    .top()
            })
            .collect();
        tops.dedup();
        assert!(tops.len() <= 2, "{tops:?}");
        let hidden = CATALOG.len() - shown.len();
        assert_eq!(
            window.find("logs-services-more").label(),
            Some(format!("+{hidden}").as_str())
        );

        // The list holds every service; a hidden one collects from it.
        window.click("logs-services-more", cx);
        window.render_frame(cx);
        assert!(window.find("logs-services-list").visible());
        for service in CATALOG {
            assert!(
                window
                    .try_find(SharedString::from(format!("list-collect-{service}")))
                    .is_some()
            );
        }
        let last = CATALOG[CATALOG.len() - 1];
        window.click(SharedString::from(format!("list-collect-{last}")), cx);
        window.render_frame(cx);
        assert!(
            panel
                .read(cx)
                .source()
                .collecting
                .contains(&ServiceId::from(last))
        );
        assert_eq!(
            window
                .find(SharedString::from(format!("list-collect-{last}")))
                .checked(),
            Some(true)
        );
        // Its eye joins it, and hides its lines without stopping it.
        window.click(SharedString::from(format!("list-show-{last}")), cx);
        window.render_frame(cx);
        assert!(!panel.read(cx).shown().contains(&ServiceId::from(last)));
        assert!(
            panel
                .read(cx)
                .source()
                .collecting
                .contains(&ServiceId::from(last))
        );
    })
    .unwrap();
}

/// Presses Tab until `id`, whose focus is observed, has the keyboard.
fn tab_to(window: &mut gpui_kit::Window, id: &str, cx: &mut gpui_kit::App) {
    for _ in 0..64 {
        if window.find(SharedString::from(id.to_owned())).focused() == Some(true) {
            return;
        }
        window.press("tab", cx);
        window.render_frame(cx);
    }
    panic!("{id} is not reachable with Tab");
}

#[gpui_kit::test]
fn the_keyboard_toggles_pills_in_the_list_and_escape_hands_it_back(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount_catalog(cx, 460., 820.);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        panel.update(cx, |view, cx| view.focus_lines(window, cx));
        window.render_frame(cx);
        tab_to(window, "logs-services-more", cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(window.find("logs-services-list").visible());

        // The list's first pill takes the next Tab. Space and Enter toggle
        // it, and the list stays open.
        let first = ServiceId::from(CATALOG[0]);
        let collecting = |cx: &gpui_kit::App| panel.read(cx).source().collecting.contains(&first);
        let before = collecting(cx);
        window.press("tab", cx);
        window.render_frame(cx);
        window.press("space", cx);
        window.render_frame(cx);
        assert_eq!(collecting(cx), !before);
        assert!(window.find("logs-services-list").visible());
        window.press("enter", cx);
        window.render_frame(cx);
        assert_eq!(collecting(cx), before);
        assert!(window.find("logs-services-list").visible());
        // Its eye hides its lines.
        window.press("tab", cx);
        window.render_frame(cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(!panel.read(cx).shown().contains(&first));
        assert_eq!(collecting(cx), before);
        assert!(window.find("logs-services-list").visible());

        // Escape closes the list and gives the keyboard back to its button.
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("logs-services-list").is_none());
        assert_eq!(window.find("logs-services-more").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn closing_a_list_the_mouse_opened_gives_the_keyboard_to_the_lines(cx: &mut TestAppContext) {
    let (_runtime, _panel, handle) = mount_catalog(cx, 460., 820.);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        window.click("logs-services-more", cx);
        window.render_frame(cx);
        assert!(window.find("logs-services-list").visible());
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("logs-services-list").is_none());
        assert_eq!(window.find("logs-viewport").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_long_list_scrolls_inside_a_short_window(cx: &mut TestAppContext) {
    // Thirty services at 20 px text in the shortest window.
    let services: Vec<String> = (0..30).map(|ix| format!("ext-example-{ix:02}")).collect();
    let (_runtime, _panel, handle) = mount_services(cx, 760., 560., services.clone());
    cx.update_window(handle.into(), |_, window, cx| {
        window.set_rem_size(px(20.));
        window.render_frame(cx);
        window.render_frame(cx);
        window.click("logs-services-more", cx);
        window.render_frame(cx);
        window.render_frame(cx);
        let list = window.find("logs-services-list").bounds();
        assert!(list.top() >= px(0.), "{list:?}");
        assert!(list.bottom() <= px(560.), "{list:?}");
        // The pills scroll inside it: the last starts below its room.
        let scroll = window.find("logs-services-list-scroll").bounds();
        assert!(scroll.bottom() <= list.bottom());
        let last = window
            .find(SharedString::from(format!(
                "list-collect-{}",
                services[services.len() - 1]
            )))
            .bounds();
        assert!(last.top() > scroll.bottom(), "{last:?} {scroll:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_short_window_puts_the_service_picker_on_the_collection_row(cx: &mut TestAppContext) {
    // Short at any text size; the test window's rem is 16 px.
    let (_runtime, panel, handle) = mount_catalog(cx, 760., 560.);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        // No row of pills: the picker sits beside Start collecting.
        assert!(in_rows(window).is_empty());
        assert!(window.try_find("logs-services").is_none());
        let picker = window.find("logs-services-more");
        assert_eq!(picker.label(), Some("3 of 12 services"));
        let start = window.find("logs-collection").bounds();
        assert!(picker.bounds().bottom() > start.top() && picker.bounds().top() < start.bottom());

        window.click("logs-services-more", cx);
        window.render_frame(cx);
        assert!(window.find("logs-services-list").visible());
        window.click("list-collect-cri", cx);
        window.render_frame(cx);
        assert!(
            panel
                .read(cx)
                .source()
                .collecting
                .contains(&ServiceId::from("cri"))
        );
        assert_eq!(
            window.find("logs-services-more").label(),
            Some("4 of 12 services")
        );

        // Another node's catalog starts with the list closed.
        panel.update(cx, |view, cx| {
            view.set_fixture(fixture_events(), window, cx);
        });
        window.render_frame(cx);
        assert!(window.try_find("logs-services-list").is_none());
    })
    .unwrap();
}
