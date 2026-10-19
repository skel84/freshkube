use super::*;

#[gpui_kit::test]
fn the_page_takes_the_shared_frame_and_its_last_panel_ends_at_the_padding(cx: &mut TestAppContext) {
    use freshkube_ui::layout_check::{PageFrame, assert_page_frame_from};
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let slots = slot_count(cx, &page);
    cx.update_window(handle, |_, window, cx| {
        // The header leads with the breadcrumb's "Dashboards".
        assert_page_frame_from(
            window,
            cx,
            &PageFrame {
                page: "monitoring-page",
                title: "monitoring-title",
                title_text: "Cluster",
                content: "monitoring-grid",
            },
            "monitoring-dashboards",
        );
        // The grid draws the gap between panels, so the rightmost one ends
        // where the grid does, not a gap short of it.
        let grid = window.find("monitoring-grid").bounds();
        let right = (0..slots)
            .filter_map(|index| {
                window.try_find(SharedString::from(format!("monitoring-panel-{index}")))
            })
            .map(|panel| panel.bounds().right())
            .fold(
                px(0.),
                |right, edge| if edge > right { edge } else { right },
            );
        assert!(
            (right - grid.right()).abs() < px(0.5),
            "the last panel ends at {right:?}, the grid at {:?}",
            grid.right()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_narrow_header_folds_the_controls_beside_the_breadcrumb(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    // The page's width at 760 × 560 and 20 px, beside the rail and column.
    cx.simulate_window_resize(handle, size(px(560.), px(560.)));
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        freshkube_ui::layout_check::settle_header(window, cx);
        let toolbar = window.find("monitoring-toolbar").bounds();
        let title = window.find("monitoring-title").bounds();
        let more = window.find("monitoring-more").bounds();
        // The controls fold into "…" on the title's row instead of taking a
        // row of their own or running over the title.
        assert!(window.try_find("monitoring-range").is_none());
        assert!(window.try_find("monitoring-auto-refresh").is_none());
        assert!(
            more.bottom() <= toolbar.bottom(),
            "{more:?} under {toolbar:?}"
        );
        assert!(
            more.right() <= toolbar.right() + px(1.),
            "{more:?} past {toolbar:?}"
        );
        assert!(title.right() < more.left(), "{title:?} meets {more:?}");
        // Where the data comes from is on the meta line, not the toolbar.
        let scope = window.find("monitoring-scope").bounds();
        let source = window.find("monitoring-source").bounds();
        assert!(scope.contains(&source.center()), "{source:?} off {scope:?}");
    })
    .unwrap();
    // Wide, nothing folds and the controls share the title's row.
    cx.simulate_window_resize(handle, size(px(1200.), px(900.)));
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(13., cx);
        freshkube_ui::layout_check::settle_header(window, cx);
        assert!(window.try_find("monitoring-more").is_none());
        let toolbar = window.find("monitoring-toolbar").bounds();
        let title = window.find("monitoring-title").bounds();
        let refresh = window.find("monitoring-refresh").bounds();
        assert!(
            refresh.bottom() <= toolbar.bottom(),
            "{refresh:?} under {toolbar:?}"
        );
        assert!(
            title.right() < refresh.left(),
            "{title:?} meets {refresh:?}"
        );
        assert!(
            window
                .find("monitoring-scope")
                .bounds()
                .contains(&window.find("monitoring-source").bounds().center())
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_folded_time_range_picks_a_range_as_its_picker_does(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let (ranges, current) = cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        (board.ranges.clone(), board.span)
    });
    let (index, (span, _)) = ranges
        .iter()
        .enumerate()
        .find(|(_, (span, _))| *span != current)
        .expect("another range");
    cx.simulate_window_resize(handle, size(px(560.), px(560.)));
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        freshkube_ui::layout_check::settle_header(window, cx);
        window.click("monitoring-more", cx);
        window.render_frame(cx);
        // Time range, Refresh, Auto-refresh: the controls' order.
        window.within("popup-menu").click(0usize, cx);
        window.render_frame(cx);
        window
            .within("submenu")
            .within("popup-menu")
            .click(index, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| page.read(cx).board.as_ref().unwrap().span),
        *span
    );
}

/// The variables and the annotation toggles are the header's second row,
/// under the toolbar and above the meta line, at the toolbar's control
/// height.
#[gpui_kit::test]
fn the_variables_are_the_headers_second_row(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    cx.update_window(handle, |_, window, cx| {
        freshkube_ui::layout_check::settle_header(window, cx);
        let toolbar = window.find("monitoring-toolbar").bounds();
        let row = window.find("monitoring-secondary").bounds();
        let meta = window.find("monitoring-scope").bounds();
        assert!(
            row.top() >= toolbar.bottom() - px(0.5),
            "{row:?} over {toolbar:?}"
        );
        assert!(
            meta.top() >= row.bottom() - px(0.5),
            "{meta:?} over {row:?}"
        );
        let height = crate::ui::dp_px(crate::ui::CONTROL_HEIGHT, window);
        for id in [
            "monitoring-variable-node",
            "monitoring-variable-namespace",
            "monitoring-markers-deploys",
            "monitoring-markers-nodes",
        ] {
            let bounds = window.find(id).bounds();
            assert!(
                row.contains(&bounds.center()),
                "{id} {bounds:?} outside {row:?}"
            );
            assert!(
                (bounds.size.height - height).abs() < px(0.5),
                "{id} is {:?} high, not {height:?}",
                bounds.size.height
            );
        }
        assert_eq!(
            window.find("monitoring-variable-node").label(),
            Some("node: All")
        );
    })
    .unwrap();
}

/// Auto-refresh says what it does: off, or its interval.
#[gpui_kit::test]
fn auto_refresh_names_itself_and_its_interval(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let label = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, _| {
            window
                .find("monitoring-auto-refresh")
                .label()
                .map(str::to_owned)
        })
        .unwrap()
    };
    assert_eq!(label(cx).as_deref(), Some("Auto-refresh off"));
    cx.update(|cx| page.update(cx, |page, cx| page.set_refresh(Some(30), cx)));
    frame(cx, handle);
    assert_eq!(label(cx).as_deref(), Some("Auto-refresh 30s"));
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(false, cx)));
}

#[gpui_kit::test]
fn the_annotations_stay_inside_a_narrow_page(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    // 760 × 560 at 20 px with the column open leaves the page about 410 px.
    cx.simulate_window_resize(handle, size(px(410.), px(560.)));
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        freshkube_ui::layout_check::settle_header(window, cx);
        let page = window.find("monitoring-page").bounds();
        for id in [
            "monitoring-annotations",
            "monitoring-markers-deploys",
            "monitoring-markers-nodes",
        ] {
            let bounds = window.find(id).bounds();
            assert!(
                bounds.right() <= page.right() + px(0.5),
                "{id} {bounds:?} past {page:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_meta_line_counts_the_panels_and_says_when_they_answered(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    assert!(shown(cx, handle, "monitoring-scope"));
    let slots = slot_count(cx, &page);
    let time = super::super::board::clock(NOW).unwrap();
    let meta = |cx: &mut TestAppContext| -> SharedString {
        cx.read(|cx| page.read(cx).board.as_ref().unwrap().meta.clone())
    };
    assert_eq!(meta(cx), format!("{slots} panels · {time}").as_str());

    cx.update(|cx| {
        page.update(cx, |page, _| {
            let board = page.board.as_mut().unwrap();
            board.slots[0].failed = true;
            assert!(board.derive_meta(), "a failure changes the meta line");
            assert!(!board.derive_meta(), "nothing changed the second time");
        })
    });
    assert_eq!(
        meta(cx),
        format!("{slots} panels · 1 failed · {time}").as_str()
    );
}

#[gpui_kit::test]
fn a_short_window_scrolls_the_page_alone_header_and_all(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let down = |delta: f32| ScrollDelta::Pixels(point(px(0.), px(-delta)));
    let panels = cx.read(|cx| page.read(cx).board.as_ref().unwrap().slots.len());
    // Tall, the page stays put and the grid takes what the header leaves.
    cx.update_window(handle, |_, window, cx| {
        freshkube_ui::layout_check::settle_header(window, cx);
        let title = window.find("monitoring-title").bounds();
        window.scroll("monitoring-page", down(400.), cx);
        freshkube_ui::layout_check::settle_header(window, cx);
        assert_eq!(window.find("monitoring-title").bounds(), title);
        let frame = window.find("monitoring-page").bounds();
        let grid = window.find("monitoring-grid").bounds();
        assert!(grid.top() > title.bottom(), "{grid:?} over {title:?}");
        assert!(grid.bottom() <= frame.bottom(), "{grid:?} past {frame:?}");
    })
    .unwrap();
    // 760 × 560 at 20 px with the column folded: 273 dp, under 620.
    cx.simulate_window_resize(handle, size(px(600.), px(420.)));
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        freshkube_ui::layout_check::settle_header(window, cx);
        let unit = crate::ui::dp_px(1., window);
        let frame = window.find("monitoring-page").bounds();
        let grid = window.find("monitoring-grid").bounds();
        // The grid is all its panels high, inside the page that scrolls.
        assert!(
            grid.size.height > frame.size.height,
            "{grid:?} in {frame:?}"
        );
        // The wheel over the grid moves the header and the panels together:
        // the page scrolls and the grid doesn't scroll inside it.
        let title = window.find("monitoring-title").bounds();
        let panel = window.find("monitoring-panel-0").bounds();
        wheel(window, down(60.), cx);
        freshkube_ui::layout_check::settle_header(window, cx);
        let moved = title.top() - window.find("monitoring-title").bounds().top();
        let panel_moved = panel.top() - window.find("monitoring-panel-0").bounds().top();
        assert!(moved > px(0.), "the page didn't scroll");
        assert!(
            (moved - panel_moved).abs() < px(0.5),
            "the header moved {moved:?}, the panels {panel_moved:?}"
        );
        // To the page's end: the header gone and the last panel in it.
        wheel(window, down(40_000.), cx);
        freshkube_ui::layout_check::settle_header(window, cx);
        let title = window.find("monitoring-title").bounds();
        assert!(
            title.bottom() <= frame.top(),
            "{title:?} still in {frame:?}"
        );
        let last = (0..panels)
            .map(|n| slot_bounds(window, n).bottom())
            .fold(px(f32::MIN), |a, b| a.max(b));
        assert!(last <= frame.bottom() + px(0.5), "{last:?} past {frame:?}");
        assert!(
            last > frame.bottom() - unit * 40.,
            "{last:?} short of {frame:?}"
        );
        // The panels it asks for are the ones in the page's view.
        let viewport = page.read(cx).viewport.get();
        assert!(viewport.top > 0., "{viewport:?}");
        assert!(
            (viewport.height - frame.size.height / unit).abs() < 1.,
            "asks for {viewport:?}, the page is {frame:?}"
        );
    })
    .unwrap();
}
