use super::*;

/// How far `section`'s top sits below the top of Details' scroller.
fn below_top(window: &gpui_kit::Window, section: &str) -> gpui_kit::Pixels {
    window
        .find(gpui_kit::SharedString::from(format!(
            "detail-section-{section}"
        )))
        .bounds()
        .top()
        - window.find("detail-details").bounds().top()
}

/// A jump into a pane that has never drawn lands its section at the top
/// of Details, not past it, though the new scroll handle knows no bounds
/// yet (#322 review).
#[gpui_kit::test]
fn a_jump_into_a_fresh_pane_lands_on_its_section(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount_sized(cx, size(px(560.), px(360.)));
    let (pod, _) = running_pod();
    cx.update_window(handle, |_, window, cx| {
        for section in [super::Tab::Ports, super::Tab::Events, super::Tab::Ports] {
            // Another object each time, so each jump meets a new handle.
            pane.update(cx, |pane, cx| pane.close(cx));
            open(&pane, &pod, Duration::ZERO, cx);
            pane.update(cx, |pane, cx| pane.set_tab(section, cx));
            settle(window, cx);
            let slug = match section {
                super::Tab::Ports => "ports",
                _ => "events",
            };
            let scroller = window.find("detail-details").bounds();
            let at_end = {
                let pane = pane.read(cx);
                let scroll = &pane.details_scroll;
                -scroll.offset().y >= scroll.max_offset().y - px(1.)
            };
            let below = below_top(window, slug);
            // Events, the last, may be too short to reach the top; then
            // Details stops at its end with Events in sight.
            if slug == "ports" {
                assert!(below.abs() <= px(1.), "{slug}: {below:?}");
            } else {
                assert!(
                    below.abs() <= px(1.) || (at_end && below < scroller.size.height),
                    "{slug}: {below:?}"
                );
            }
            assert_eq!(
                window
                    .find(gpui_kit::SharedString::from(format!("detail-jump-{slug}")))
                    .selected(),
                Some(true)
            );
        }
    })
    .unwrap();
}

/// A jump made while the document still loads keeps its section at the
/// top as the Overview above it grows.
#[gpui_kit::test]
fn a_jump_holds_its_section_while_the_document_arrives(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount_sized(cx, size(px(560.), px(360.)));
    let (pod, _) = running_pod();
    let delay = Duration::from_millis(400);
    let skeleton = cx
        .update_window(handle, |_, window, cx| {
            open(&pane, &pod, delay, cx);
            pane.update(cx, |pane, cx| pane.set_tab(super::Tab::Ports, cx));
            settle(window, cx);
            assert_eq!(read(&pane, cx), DocumentRead::Loading);
            let below = below_top(window, "ports");
            assert!(below.abs() <= px(1.), "{below:?}");
            window.find("detail-section-overview").bounds().size.height
        })
        .unwrap();
    cx.executor().advance_clock(delay);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        assert_ne!(read(&pane, cx), DocumentRead::Loading);
        let grown = window.find("detail-section-overview").bounds().size.height;
        assert!(grown > skeleton, "{skeleton:?} → {grown:?}");
        let below = below_top(window, "ports");
        assert!(below.abs() <= px(1.), "{below:?}");
        assert_eq!(window.find("detail-jump-ports").selected(), Some(true));

        // The wheel lets go, and the index follows the scroll again.
        window.scroll(
            "detail-details",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(5000.))),
            cx,
        );
        settle(window, cx);
        assert_eq!(window.find("detail-jump-overview").selected(), Some(true));
    })
    .unwrap();
}

/// Details is one page under an index: a jump brings its section to the
/// top and marks it, YAML and back keep the place, and scrolling marks the
/// section the scroll reaches.
#[gpui_kit::test]
fn the_index_jumps_through_details_and_follows_its_scroll(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = running_pod();
    let top = |window: &mut gpui_kit::Window, id: &'static str| {
        window.find(id).bounds().top() - window.find("detail-details").bounds().top()
    };
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        settle(window, cx);
        assert_eq!(window.find("detail-jump-overview").selected(), Some(true));
        assert!(window.try_find("detail-section-ports").is_some());
        let start = top(window, "detail-section-overview");

        // This pod's Ports and Events are shorter than the pane, so the
        // jump scrolls to the end and Ports stays marked.
        let before = top(window, "detail-section-ports");
        window.click("detail-jump-ports", cx);
        settle(window, cx);
        let jumped = top(window, "detail-section-ports");
        assert!(
            jumped < before && jumped >= px(-1.),
            "{before:?} → {jumped:?}"
        );
        assert!(window.find("detail-section-events").visible());
        assert_eq!(window.find("detail-jump-ports").selected(), Some(true));
        assert_eq!(window.find("detail-jump-overview").selected(), Some(false));

        window.click("detail-tab-yaml", cx);
        settle(window, cx);
        assert!(window.try_find("detail-details").is_none());
        window.click("detail-tab-details", cx);
        settle(window, cx);
        assert_eq!(top(window, "detail-section-ports"), jumped);
        assert_eq!(window.find("detail-jump-ports").selected(), Some(true));

        // Back to the top by the wheel, and the index follows.
        window.scroll(
            "detail-details",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(5000.))),
            cx,
        );
        settle(window, cx);
        assert_eq!(top(window, "detail-section-overview"), start);
        assert_eq!(window.find("detail-jump-overview").selected(), Some(true));

        // A pane asked to open on Events scrolls there.
        pane.update(cx, |pane, cx| pane.set_tab(super::Tab::Events, cx));
        settle(window, cx);
        assert_eq!(window.find("detail-tab-details").selected(), Some(true));
        assert_eq!(window.find("detail-jump-events").selected(), Some(true));
        assert!(window.find("detail-section-events").visible());
    })
    .unwrap();
}
