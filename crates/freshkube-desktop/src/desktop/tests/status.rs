use super::*;

/// The status bar's page segment, as its id's accessibility label, if the
/// bar shows one.
fn segment(window: &mut gpui_kit::Window, id: &'static str) -> Option<String> {
    let element = window.try_find(id)?;
    assert!(
        element
            .path()
            .contains(&gpui_kit::ElementId::from("status-bar")),
        "{id} is in the status bar"
    );
    element.label().map(str::to_owned)
}

#[gpui_kit::test]
fn the_visible_page_fills_the_status_bar_and_its_reads_reach_it(cx: &mut TestAppContext) {
    use crate::resources::model::ReadState;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Overview has no segment.
        for id in [
            "resource-scope",
            "system-services-scope",
            "nodes-scope",
            "obs-scope",
        ] {
            assert!(window.try_find(id).is_none(), "{id} shows on Overview");
        }
        view.update(cx, |view, cx| view.navigate(Page::Resources, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let resources = cx.update(|cx| view.read(cx).pages.resources.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let line = segment(window, "resource-scope").expect("Pods fill the status bar");
        assert!(line.starts_with("Example data"), "{line}");
        assert!(!line.contains("reconnecting"), "{line}");
    })
    .unwrap();

    // A read that goes stale, then recovers, reaches the bar on the next
    // frame with no other input: the page notifies, which asks for a frame,
    // and the shell reads the page's segment while the page's own view
    // stays cached.
    let notices = Rc::new(Cell::new(0usize));
    cx.update(|cx| {
        let seen = notices.clone();
        cx.observe(&resources, move |_, _| seen.set(seen.get() + 1))
            .detach();
    });
    for (state, stale) in [
        (ReadState::Stale("connection reset".into()), true),
        (ReadState::Loaded, false),
    ] {
        let before = notices.get();
        cx.update(|cx| resources.update(cx, |screen, cx| screen.deliver_read(state, cx)));
        assert!(notices.get() > before, "the read asks for a frame");
        cx.update_window(handle, |_, window, cx| {
            draw(window, cx);
            let line = segment(window, "resource-scope").unwrap();
            assert_eq!(line.contains("reconnecting"), stale, "{line}");
        })
        .unwrap();
    }

    // Another page replaces the segment.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.navigate(Page::SystemServices, window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("resource-scope").is_none());
        let line = segment(window, "system-services-scope").expect("System services' segment");
        assert!(line.contains("services on"), "{line}");
    })
    .unwrap();
}

/// A screen whose status bar segment a test sets.
struct SegmentScreen(Option<freshkube_ui::status::Segment>);

impl gpui_kit::EventEmitter<crate::screens::ScreenEvent> for SegmentScreen {}

impl gpui_kit::Render for SegmentScreen {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
    }
}

impl crate::screens::ScreenPanel for SegmentScreen {
    fn set_source(
        &mut self,
        _: Option<crate::screens::ScreenSource>,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) {
    }

    fn activate(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) {}

    fn refresh(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) {}

    fn status(&mut self, _: &gpui_kit::App) -> Option<freshkube_ui::status::Segment> {
        self.0.clone()
    }
}

#[gpui_kit::test]
fn a_screens_segment_shows_only_while_its_page_is_visible(cx: &mut TestAppContext) {
    use super::super::pages::PageId;
    use freshkube_ui::status::Segment;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    // etcd's page draws a screen that sets its segment as the test says.
    let screen = cx.new(|_| SegmentScreen(Some(Segment::new(None::<SharedString>, ["3 members"]))));
    cx.update(|cx| {
        view.update(cx, |view, _| {
            let slot = view
                .pages
                .map
                .get_mut(&PageId::Etcd)
                .expect("etcd has a screen");
            *slot = crate::screens::ScreenHandle::new(screen.clone());
        })
    });
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.navigate(Page::Etcd, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let line = segment(window, "etcd-scope").expect("etcd's segment");
        assert!(line.contains("3 members"), "{line}");
    })
    .unwrap();

    // A new segment reaches the bar on the next frame: the screen's notify
    // asks for one and marks the shell above it dirty, with no event of its
    // own.
    let notices = Rc::new(Cell::new(0usize));
    cx.update(|cx| {
        let seen = notices.clone();
        cx.observe(&screen, move |_, _| seen.set(seen.get() + 1))
            .detach();
        screen.update(cx, |screen, cx| {
            screen.0 = Some(Segment::new(None::<SharedString>, ["2 members"]));
            cx.notify();
        })
    });
    assert_eq!(notices.get(), 1, "the change asks for a frame");
    cx.update_window(handle, |_, window, cx| {
        draw(window, cx);
        let line = segment(window, "etcd-scope").unwrap();
        assert!(line.contains("2 members"), "{line}");
    })
    .unwrap();

    // Hidden, the screen's segment never shows, even when it changes: the
    // bar shows the visible page's own.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.navigate(Page::Security, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        screen.update(cx, |screen, cx| {
            screen.0 = Some(Segment::new(None::<SharedString>, ["1 member"]));
            cx.notify();
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("etcd-scope").is_none());
        let line = segment(window, "security-scope");
        assert!(
            line.as_ref().is_none_or(|line| !line.contains("member")),
            "{line:?}"
        );
    })
    .unwrap();
}

/// etcd and Security hand their line to the status bar, and their headers
/// keep none: the quorum and leader, then where the audit read from.
#[gpui_kit::test]
fn etcd_and_security_fill_the_status_bar(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    for (page, id, expect) in [
        (Page::Etcd, "etcd-scope", ["3 members", "Quorum", "leader "]),
        (
            Page::Security,
            "security-scope",
            ["endpoints ", "volume target ", "example data"],
        ),
    ] {
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.navigate(page, window, cx));
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let line = segment(window, id).unwrap_or_else(|| panic!("{id} in the bar"));
            for part in expect {
                assert!(line.contains(part), "{part:?} in {line}");
            }
        })
        .unwrap();
    }
}

/// In the narrowest window, a page's segment drops its minor parts and
/// then its other parts before its warnings. Example mode's bar is about
/// 200 px narrower than a live one, for Simulate failure and Example data:
/// at the largest text the first part goes there, and even the warnings
/// are cut, but the first one still leads.
#[gpui_kit::test]
fn a_narrow_status_bar_drops_minor_parts_and_keeps_the_toned_ones(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    for text_size in [13., 20.] {
        for (page, id, lead, warnings) in [
            (
                Page::Health,
                "health-scope",
                "deployments",
                &["degraded", "failing"][..],
            ),
            // No alerts in the example: nothing outranks the first part.
            (Page::Lifecycle, "lifecycle-scope", "Talos ", &[]),
        ] {
            cx.update_window(handle, |_, window, cx| {
                crate::text_size::set(text_size, cx);
                view.update(cx, |view, cx| view.navigate(page, window, cx));
                window.render_frame(cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                let line = segment(window, id).unwrap_or_else(|| panic!("{id} in the bar"));
                let shown = freshkube_ui::status::shown(id).expect("a drawn line");
                let room = format!(
                    "{shown:?} of {line:?} in {:?} of a {:?} bar at {text_size}",
                    window.find(id).bounds().size.width,
                    window.find("status-bar").bounds().size.width,
                );
                // The label and tooltip keep the whole line.
                assert!(line.ends_with("example data"), "{line}");
                assert!(!shown.contains("example data"), "{room}");
                assert!(!shown.contains("updated"), "{room}");
                assert!(!shown.contains("no alerts"), "a count of none goes: {room}");
                let Some(first) = warnings.first() else {
                    assert!(shown.starts_with(lead), "{room}");
                    return;
                };
                assert!(shown.contains(first), "{room}");
                if text_size == 13. {
                    // Room for the first part beside the warnings.
                    assert!(shown.contains(lead), "{room}");
                    assert!(warnings.iter().all(|part| shown.contains(part)), "{room}");
                    assert!(!shown.ends_with('…'), "the warnings fit whole: {room}");
                } else {
                    assert!(
                        !shown.contains(lead),
                        "warnings outrank the first part: {room}"
                    );
                }
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn a_compact_status_bar_keeps_the_shells_glyph_and_gives_the_page_the_room(
    cx: &mut TestAppContext,
) {
    for (width, compact) in [(1280., false), (1000., true)] {
        let (_runtime, handle, view) = fixture(cx, width, 820.);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.navigate(Page::SystemServices, window, cx)
            });
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let status = window.find("overview-status");
            let label = status.label().unwrap_or_default().to_owned();
            assert!(!label.is_empty(), "the shell's status keeps its label");
            let glyph_only = status.bounds().size.width < crate::ui::dp_px(24., window);
            assert_eq!(glyph_only, compact, "at {width}: {:?}", status.bounds());
            let page = window.find("system-services-scope");
            assert!(page.visible());
            assert!(page.bounds().left() >= status.bounds().right());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_long_segment_truncates_and_leaves_the_right_side_whole(cx: &mut TestAppContext) {
    // At 760 and 20 px text Nodes' line is wider than the bar can give it.
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    let mut widths = Vec::new();
    for page in [Page::Overview, Page::Nodes] {
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(20., cx);
            view.update(cx, |view, cx| view.navigate(page, window, cx));
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // The right side's last item is the first a squeezed bar loses.
            let rate = window.find("frame-rate").bounds();
            assert!(rate.right() <= window.viewport_size().width, "{rate:?}");
            widths.push(rate.size.width);
            let fail = window.find("fixture-fail").bounds();
            if page == Page::Nodes {
                let scope = window.find("nodes-scope").bounds();
                assert!(scope.right() <= fail.left(), "{scope:?} {fail:?}");
            }
        })
        .unwrap();
    }
    assert_eq!(widths[0], widths[1], "the right side keeps its width");
}

/// Rests the pointer on the header's Refresh button until its tooltip shows.
fn hover_refresh(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    use gpui_kit::{InputEvent, MouseMoveEvent};
    cx.update_window(handle, |_, window, cx| {
        draw(window, cx);
        let position = window.find("refresh-tip").bounds().center();
        window.dispatch_event(
            MouseMoveEvent {
                position,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    settle_tip(cx, handle);
}

/// Lets an open Refresh tooltip read the shell again, and draws.
fn settle_tip(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| draw(window, cx))
        .unwrap();
}

fn refresh_tip(view: &Entity<Pilot>, cx: &mut TestAppContext) -> String {
    cx.update(|cx| view.read(cx).refresh_tip().to_string())
}

/// Steps the clock by `steps` quarter seconds, drawing after each, and
/// returns how many times the header drew and how many seconds the shell
/// counted.
fn quarter_seconds(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<Pilot>,
    steps: usize,
) -> (usize, u64) {
    let (header, elapsed) = (
        count("chrome.header"),
        cx.update(|cx| view.read(cx).chrome.elapsed),
    );
    for _ in 0..steps {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(250));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| draw(window, cx))
            .unwrap();
    }
    let elapsed = cx.update(|cx| view.read(cx).chrome.elapsed) - elapsed;
    (count("chrome.header") - header, elapsed.as_secs())
}

/// An open Refresh tooltip counts down with the ring (#409). Its reads
/// redraw the tooltip alone: the cached header still draws once a second,
/// for the ring inside it, exactly as with the tooltip closed.
#[gpui_kit::test]
fn the_open_refresh_tooltip_counts_down_without_redrawing_the_header(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| draw(window, cx))
        .unwrap();
    let (closed, seconds) = quarter_seconds(cx, handle, &view, 12);
    assert_eq!(closed, seconds as usize, "closed: the ring's tick alone");
    hover_refresh(cx, handle);
    let first = refresh_tip(&view, cx);
    assert!(
        first.starts_with("Refresh now · next Talos refresh in "),
        "{first}"
    );
    assert!(freshkube_ui::tooltip::drawn(&first) > 0, "{first} shows");
    let (open, seconds) = quarter_seconds(cx, handle, &view, 12);
    assert_eq!(open, seconds as usize, "open: still the ring's tick alone");
    let later = refresh_tip(&view, cx);
    assert_ne!(later, first, "the countdown moved");
    assert!(freshkube_ui::tooltip::drawn(&later) > 0, "{later} shows");
}

/// While a read is in flight, the open tooltip says so.
#[gpui_kit::test]
fn the_open_refresh_tooltip_reads_refreshing_during_a_read(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    hover_refresh(cx, handle);
    let before = freshkube_ui::tooltip::drawn("Refreshing…");
    cx.update(|cx| view.update(cx, |pilot, _| pilot.config_loading = true));
    settle_tip(cx, handle);
    assert!(freshkube_ui::tooltip::drawn("Refreshing…") > before);
}

#[gpui_kit::test]
fn default_bar_keeps_results_state_and_failure_text(cx: &mut TestAppContext) {
    use crate::desktop::pages::PageId;
    use freshkube_ui::status::{Part, Segment};
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    let screen = cx.new(|_| SegmentScreen(None));
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.pages.map.insert(
                PageId::Etcd,
                crate::screens::ScreenHandle::new(screen.clone()),
            );
            view.navigate(Page::Etcd, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    for state in [
        "Tekton Results isn't forwarded yet · Forward (F)",
        "Tekton Results forward failed: No pods · The Service selects no pods · Forward (F)",
    ] {
        cx.update(|cx| {
            screen.update(cx, |screen, cx| {
                screen.0 = Some(Segment::new(
                    None::<&str>,
                    [
                        Part::new("6 changes"),
                        Part::new("2 not fully confirmed"),
                        Part::new(state).keep(),
                    ],
                ));
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let shown = freshkube_ui::status::shown("etcd-scope").unwrap();
            let prefix = if state.contains("failed") {
                "Tekton Results forward failed"
            } else {
                "Tekton Results isn't forwarded yet"
            };
            assert!(shown.contains(prefix), "{shown}");
        })
        .unwrap();
    }
}
