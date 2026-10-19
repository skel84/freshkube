use super::*;

/// A frame that scrolls around a log, as a short node pane does.
struct Frame {
    panel: Entity<LogPanel>,
    scroll: gpui_kit::ScrollHandle,
    /// The log's height inside the frame.
    height: f32,
}

impl gpui_kit::Render for Frame {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui_kit::IntoElement {
        use gpui_kit::{TestSupportExt, prelude::*};
        gpui_kit::div()
            .id("frame")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                gpui_kit::div()
                    .id("frame-header")
                    .test_support()
                    .h(px(120.)),
            )
            .child(gpui_kit::div().h(px(self.height)).child(self.panel.clone()))
            .child(gpui_kit::div().h(px(600.)))
    }
}

/// A log `height` tall in a [`Frame`] that scrolls around it.
fn mount_framed(
    cx: &mut TestAppContext,
    height: f32,
) -> (
    Runtime,
    Entity<LogPanel>,
    gpui_kit::ScrollHandle,
    WindowHandle<Root>,
) {
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(gpui_kit::init);
    let scroll = gpui_kit::ScrollHandle::new();
    let mut panel = None;
    let handle = cx.open_window(size(px(620.), px(760.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = LogPanel::new(runtime.handle().clone(), 100, window, cx);
            view.set_fixture(fixture_events(), window, cx);
            view
        });
        panel = Some(view.clone());
        let frame = cx.new(|_| Frame {
            panel: view,
            scroll: scroll.clone(),
            height,
        });
        Root::new(frame, window, cx)
    });
    (runtime, panel.unwrap(), scroll, handle)
}

#[gpui_kit::test]
fn a_wheel_over_a_short_logs_controls_scrolls_the_log_and_not_the_frame(cx: &mut TestAppContext) {
    let (_runtime, panel, scroll, handle) = mount_framed(cx, 180.);
    panel.update(cx, |view, cx| {
        view.source_mut().banner = true;
        cx.notify();
    });
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        let panel_scroll = panel.read(cx).panel_scroll.clone();
        assert!(
            panel_scroll.max_offset().y > px(0.),
            "the log doesn't scroll"
        );
        window.scroll(
            "frame-header",
            ScrollDelta::Pixels(point(px(0.), px(-60.))),
            cx,
        );
        window.render_frame(cx);
        let framed = scroll.offset().y;
        assert!(
            framed < px(-1.),
            "the header's wheel didn't scroll the frame"
        );
        // Down to the lines, then back up to the banner.
        for by in [-400., 400.] {
            window.scroll(
                "logs-toolbar",
                ScrollDelta::Pixels(point(px(0.), px(by))),
                cx,
            );
            window.render_frame(cx);
            let expected = if by < 0. {
                -panel_scroll.max_offset().y
            } else {
                px(0.)
            };
            assert_eq!(
                panel_scroll.offset().y,
                expected,
                "the log didn't scroll by {by}"
            );
            assert_eq!(
                scroll.offset().y,
                framed,
                "the log's wheel by {by} scrolled the frame"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_wheel_over_the_log_scrolls_the_log_and_not_the_frame_around_it(cx: &mut TestAppContext) {
    let (_runtime, panel, scroll, handle) = mount_framed(cx, 560.);
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(scroll.max_offset().y > px(0.), "the frame doesn't scroll");
        let down = ScrollDelta::Pixels(point(px(0.), px(-60.)));
        window.scroll("frame-header", down, cx);
        window.render_frame(cx);
        let framed = scroll.offset().y;
        assert!(
            framed < px(0.),
            "the header's wheel didn't scroll the frame"
        );
        assert!(panel.read(cx).following);
        let before = panel.read(cx).scroll.offset();
        window.scroll(
            "logs-viewport",
            ScrollDelta::Pixels(point(px(0.), px(300.))),
            cx,
        );
        window.render_frame(cx);
        assert!(!panel.read(cx).following, "the wheel didn't reach the log");
        assert_ne!(
            panel.read(cx).scroll.offset(),
            before,
            "the log didn't scroll"
        );
        assert_eq!(
            scroll.offset().y,
            framed,
            "the log's wheel scrolled the frame"
        );
    })
    .unwrap();
}

fn wheel(cx: &mut TestAppContext, handle: WindowHandle<Root>, x: f32, y: f32) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.scroll(
            "logs-viewport",
            ScrollDelta::Pixels(point(px(x), px(y))),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn at_the_end_a_wheel_down_or_none_at_all_keeps_following(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    let tail = cx.read(|cx| panel.read(cx).scroll.offset());
    assert!(cx.read(|cx| panel.read(cx).following));

    wheel(cx, handle, 0., -300.);
    cx.read(|cx| {
        assert!(panel.read(cx).following, "a wheel down at the end paused");
        assert_eq!(panel.read(cx).scroll.offset(), tail);
    });
    wheel(cx, handle, 0., 0.);
    assert!(
        cx.read(|cx| panel.read(cx).following),
        "an empty wheel paused"
    );

    deliver_more(cx, &panel, 0, 30);
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(panel.read(cx).following);
        assert!(
            panel.read(cx).scroll.offset().y < tail.y,
            "the tail moved on"
        );
        assert!(window.find(SharedString::from("log-line-1-149")).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_wheel_up_pauses_follow_and_new_lines_leave_the_list_where_it_is(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    let tail = cx.read(|cx| panel.read(cx).scroll.offset());

    wheel(cx, handle, 0., 300.);
    let paused = cx.read(|cx| {
        assert!(!panel.read(cx).following, "a wheel up kept following");
        panel.read(cx).scroll.offset()
    });
    assert!(paused.y > tail.y, "the list didn't scroll up");
    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.find(SharedString::from("log-line-1-99")).visible());
    })
    .unwrap();

    deliver_more(cx, &panel, 0, 30);
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(!panel.read(cx).following);
        assert_eq!(panel.read(cx).scroll.offset(), paused);
        assert!(window.find(SharedString::from("log-line-1-99")).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_sideways_wheel_keeps_following(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("logs-wrap", cx);
        assert!(!panel.read(cx).wrapped);
    })
    .unwrap();
    // A wide newest line, so the list has somewhere to go sideways.
    cx.update(|cx| {
        deliver(
            &panel,
            vec![format!("info wide {}", "東京 ".repeat(200))],
            cx,
        )
    });
    settle(cx, &panel, handle);
    let tail = cx.read(|cx| panel.read(cx).scroll.offset());
    assert!(cx.read(|cx| panel.read(cx).following));

    wheel(cx, handle, -200., 0.);
    cx.read(|cx| {
        let offset = panel.read(cx).scroll.offset();
        assert!(panel.read(cx).following, "a sideways wheel paused");
        assert_eq!(offset.y, tail.y);
        assert!(offset.x < tail.x, "the wheel didn't reach the list");
    });
}
