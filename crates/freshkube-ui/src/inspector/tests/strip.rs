use super::*;

/// An inspector `width` wide whose strip holds seven tabs, `active` shown.
struct Strip {
    strip: TabStrip,
    width: f32,
    active: usize,
    /// Wheel events that reached the page behind the inspector.
    page_wheels: usize,
}

const LABELS: [&str; 7] = [
    "Overview", "YAML", "Events", "Logs", "Shell", "Ports", "Metrics",
];

impl Render for Strip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row = self
            .strip
            .row("strip-row", self.active)
            .children(LABELS.iter().enumerate().map(|(ix, label)| {
                tab(
                    SharedString::from(format!("strip-tab-{ix}")),
                    *label,
                    ix == self.active,
                    cx,
                )
            }));
        div()
            .size_full()
            .on_scroll_wheel(cx.listener(|this, _, _, _| this.page_wheels += 1))
            .child(
                div().w(px(self.width)).h_full().child(
                    Inspector::new("inspector")
                        .heading(div().child("api-7d9"))
                        .tabs(&self.strip, row)
                        .content(div())
                        .render(cx),
                ),
            )
    }
}

fn open_strip(
    cx: &mut TestAppContext,
    width: f32,
    active: usize,
) -> (AnyWindowHandle, Entity<Strip>) {
    install(cx);
    let mut page = None;
    let handle = cx.open_window(size(px(1000.), px(400.)), |window, cx| {
        let view = cx.new(|_| Strip {
            strip: TabStrip::default(),
            width,
            active,
            page_wheels: 0,
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (handle.into(), page.unwrap())
}

fn strip_tab(window: &mut Window, ix: usize) -> Bounds<Pixels> {
    window
        .find(SharedString::from(format!("strip-tab-{ix}")))
        .bounds()
}

/// Whether tab `ix` shows whole in the row.
fn whole(window: &mut Window, ix: usize) -> bool {
    let row = window.find("strip-row").bounds();
    let item = strip_tab(window, ix);
    item.left() >= row.left() - px(0.5) && item.right() <= row.right() + px(0.5)
}

/// Whether tab `ix` shows whole and clear of the cut ends' fades.
fn clear(window: &mut Window, ix: usize) -> bool {
    let item = strip_tab(window, ix);
    whole(window, ix)
        && window
            .try_find("inspector-tabs-earlier")
            .is_none_or(|end| item.left() >= end.bounds().right() - px(0.5))
        && window
            .try_find("inspector-tabs-later")
            .is_none_or(|end| item.right() <= end.bounds().left() + px(0.5))
}

#[gpui_kit::test]
fn a_strip_with_room_shows_every_tab_and_no_chevron(cx: &mut TestAppContext) {
    let (handle, view) = open_strip(cx, 900., 0);
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        for ix in 0..LABELS.len() {
            assert!(whole(window, ix), "tab {ix} is cut");
        }
        assert!(window.try_find("inspector-tabs-earlier").is_none());
        assert!(window.try_find("inspector-tabs-later").is_none());
        assert_eq!(view.read(cx).strip.edges(), Edges::default());
    })
    .unwrap();
}

/// A strip learns it is cut while drawing, and asks for the frame that
/// shows its chevron itself: nothing else would draw one.
#[gpui_kit::test]
fn a_narrow_strip_marks_its_cut_end_without_input(cx: &mut TestAppContext) {
    let (handle, view) = open_strip(cx, 260., 0);
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        assert_eq!(
            view.read(cx).strip.edges(),
            Edges {
                earlier: false,
                later: true
            }
        );
        assert!(window.try_find("inspector-tabs-earlier").is_none());
        let tabs = window.find("inspector-tabs").bounds();
        let later = window.find("inspector-tabs-later").bounds();
        close("chevron at the edge", later.right(), tabs.right());
        close(
            "chevron as tall as a tab",
            later.size.height,
            dp_px(TAB_HEIGHT, window),
        );
        // The hairline under the strip stays drawn.
        assert!(
            later.bottom() <= tabs.bottom() - px(1.) + px(0.5),
            "{later:?} {tabs:?}"
        );
        assert!(whole(window, 0));
        assert!(!whole(window, LABELS.len() - 1));
    })
    .unwrap();
    // Unlike a test's frame, an app's frame only comes when asked for.
    view.update(cx, |strip, cx| {
        strip.strip = TabStrip::default();
        cx.notify();
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.simulate_next_frame(cx) > 0,
            "the strip asked for no frame"
        );
    })
    .unwrap();
}

/// The arrows and Command-Shift-] move the active tab; the strip brings
/// it in clear of the fades, wherever it is.
#[gpui_kit::test]
fn the_active_tab_comes_into_view_when_the_keyboard_moves_to_it(cx: &mut TestAppContext) {
    let (handle, view) = open_strip(cx, 260., 0);
    cx.update_window(handle, |_, window, cx| settle(window, cx))
        .unwrap();
    for active in [5, LABELS.len() - 1, 2, 0] {
        view.update(cx, |strip, cx| {
            strip.active = active;
            cx.notify();
        });
        cx.update_window(handle, |_, window, cx| {
            settle(window, cx);
            assert!(clear(window, active), "tab {active} isn't in view");
        })
        .unwrap();
    }
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("inspector-tabs-earlier").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_active_tab_comes_back_into_view_after_a_resize(cx: &mut TestAppContext) {
    let (handle, view) = open_strip(cx, 900., 5);
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        assert!(clear(window, 5));
    })
    .unwrap();
    for width in [260., 200., 320.] {
        view.update(cx, |strip, cx| {
            strip.width = width;
            cx.notify();
        });
        cx.update_window(handle, |_, window, cx| {
            settle(window, cx);
            assert!(clear(window, 5), "tab 5 isn't in view at {width}");
            assert!(window.try_find("inspector-tabs-earlier").is_some());
        })
        .unwrap();
    }
}

/// A chevron brings in the first tab cut at its end, and leaves the active
/// tab as it was; clicking back reaches the first tab and drops the chevron.
#[gpui_kit::test]
fn a_chevron_brings_in_the_next_cut_tab(cx: &mut TestAppContext) {
    let (handle, view) = open_strip(cx, 260., 0);
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        let next = (0..LABELS.len())
            .find(|ix| !clear(window, *ix))
            .expect("a cut tab");
        window.click("inspector-tabs-later", cx);
        settle(window, cx);
        assert!(clear(window, next), "tab {next} isn't in view");
        assert!(window.try_find("inspector-tabs-earlier").is_some());
        for _ in 0..LABELS.len() {
            if window.try_find("inspector-tabs-earlier").is_none() {
                break;
            }
            window.click("inspector-tabs-earlier", cx);
            settle(window, cx);
        }
        assert!(window.try_find("inspector-tabs-earlier").is_none());
        assert!(whole(window, 0));
        assert_eq!(window.find("strip-tab-0").selected(), Some(true));
    })
    .unwrap();
    assert_eq!(view.read_with(cx, |strip, _| strip.active), 0);
}

/// A plain wheel scrolls the strip sideways, and it stays where it was
/// left until the active tab or the width changes.
#[gpui_kit::test]
fn a_wheel_scroll_stays_until_the_active_tab_changes(cx: &mut TestAppContext) {
    let (handle, view) = open_strip(cx, 260., 0);
    let left = cx
        .update_window(handle, |_, window, cx| {
            settle(window, cx);
            let left = strip_tab(window, 0).left();
            window.scroll(
                "strip-row",
                ScrollDelta::Pixels(point(px(0.), px(-80.))),
                cx,
            );
            settle(window, cx);
            assert!(
                strip_tab(window, 0).left() < left - px(40.),
                "the wheel moved nothing"
            );
            assert!(window.try_find("inspector-tabs-earlier").is_some());
            strip_tab(window, 0).left()
        })
        .unwrap();
    view.update(cx, |_, cx| cx.notify());
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        close("left as it was", strip_tab(window, 0).left(), left);
    })
    .unwrap();
}

/// Each chevron click shows a tab that wasn't showing: the row moves by
/// more than a tab's padding, or that end stops being cut. A tab whose
/// padding alone lies under the fade doesn't count as cut.
#[gpui_kit::test]
fn every_chevron_click_shows_another_tab(cx: &mut TestAppContext) {
    for width in (230..=420).step_by(7) {
        let (handle, _) = open_strip(cx, width as f32, 0);
        cx.update_window(handle, |_, window, cx| {
            settle(window, cx);
            let padding = dp_px(super::tabs::TAB_PADDING, window);
            for (end, ahead) in [
                ("inspector-tabs-later", -1.),
                ("inspector-tabs-earlier", 1.),
            ] {
                for _ in 0..=LABELS.len() {
                    if window.try_find(end).is_none() {
                        break;
                    }
                    let before = strip_tab(window, 0).left();
                    window.click(end, cx);
                    settle(window, cx);
                    let moved = (strip_tab(window, 0).left() - before) * ahead;
                    assert!(
                        moved > padding || window.try_find(end).is_none(),
                        "{end} at {width} moved the row {moved:?}"
                    );
                }
                assert!(
                    window.try_find(end).is_none(),
                    "{end} at {width} never ends"
                );
            }
        })
        .unwrap();
    }
}

/// A cut strip keeps the wheel that scrolls it, so the page behind doesn't
/// scroll as well; a strip with room lets it through.
#[gpui_kit::test]
fn a_cut_strip_keeps_its_wheel_from_the_page(cx: &mut TestAppContext) {
    for (width, reaches) in [(260., 0), (900., 1)] {
        let (handle, view) = open_strip(cx, width, 0);
        cx.update_window(handle, |_, window, cx| {
            settle(window, cx);
            window.scroll(
                "strip-row",
                ScrollDelta::Pixels(point(px(0.), px(-80.))),
                cx,
            );
        })
        .unwrap();
        assert_eq!(
            view.read_with(cx, |strip, _| strip.page_wheels),
            reaches,
            "at {width}"
        );
    }
}

/// › pages forward: the first tab cut at the right moves to the left, clear
/// of the ‹ fade, so every tab that fits after it comes in. ‹ mirrors it.
#[gpui_kit::test]
fn a_chevron_pages_the_strip(cx: &mut TestAppContext) {
    for width in [230., 260., 300.] {
        let (handle, _) = open_strip(cx, width, 0);
        cx.update_window(handle, |_, window, cx| {
            settle(window, cx);
            let padding = dp_px(super::tabs::TAB_PADDING, window);
            let edge = window.find("inspector-tabs-later").bounds().left();
            let next = (0..LABELS.len())
                .find(|ix| strip_tab(window, *ix).right() > edge + padding + px(0.5))
                .expect("a cut tab");
            window.click("inspector-tabs-later", cx);
            settle(window, cx);
            let earlier = window.find("inspector-tabs-earlier").bounds().right();
            if window.try_find("inspector-tabs-later").is_some() {
                close("paged forward", strip_tab(window, next).left(), earlier);
            } else {
                assert!(whole(window, LABELS.len() - 1), "at {width}");
            }

            let prev = (0..LABELS.len())
                .rev()
                .find(|ix| strip_tab(window, *ix).left() < earlier - padding - px(0.5))
                .expect("a tab cut at the left");
            window.click("inspector-tabs-earlier", cx);
            settle(window, cx);
            if let Some(later) = window.try_find("inspector-tabs-later") {
                let later = later.bounds().left();
                if window.try_find("inspector-tabs-earlier").is_some() {
                    close("paged back", strip_tab(window, prev).right(), later);
                }
            }
            assert!(clear(window, prev), "tab {prev} at {width}");
        })
        .unwrap();
    }
}
