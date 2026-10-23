use super::*;

/// A pod's tabs are the inspector's: 28 high, at any text size, in one
/// strip under the heading.
#[gpui_kit::test]
fn every_tab_is_28_high_at_any_text_size(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update(|cx| crate::text_size::install(None, cx));
    let (pod, _) = crashing_pod();
    cx.update(|cx| open(&pane, &pod, Duration::ZERO, cx));
    cx.run_until_parked();
    for text_size in [13., 20.] {
        cx.update(|cx| crate::text_size::set(text_size, cx));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let strip = window.find("detail-inspector-tabs").bounds();
            let heading = window.find("detail-inspector-heading").bounds();
            assert!(strip.top() >= heading.bottom(), "{heading:?} {strip:?}");
            let tall = crate::ui::dp_px(28., window);
            for id in ["detail-tab-details", "detail-tab-yaml"] {
                let tab = window.find(id).bounds();
                assert!(
                    (tab.size.height - tall).abs() <= px(1.),
                    "{text_size} {id}: {tab:?}"
                );
                assert!(tab.top() >= strip.top() && tab.bottom() <= strip.bottom());
            }
        })
        .unwrap();
    }
}

/// The YAML tab's content starts where the heading does: its find field
/// is inset like it.
#[gpui_kit::test]
fn the_yaml_tab_is_inset_like_the_heading(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = crashing_pod();
    cx.update(|cx| open(&pane, &pod, Duration::ZERO, cx));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("detail-tab-yaml", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let inspector = window.find("detail-inspector").bounds();
        let find = window.find("detail-find").bounds();
        let inset = crate::ui::dp_px(freshkube_ui::page::PANE_PADDING, window);
        assert!(
            (find.left() - inspector.left() - inset).abs() <= px(1.),
            "{find:?}"
        );
    })
    .unwrap();
}

/// A narrow pane at text size 20 still fits both tabs, and the index over
/// Details wraps rather than cutting a section off.
#[gpui_kit::test]
fn a_narrow_pane_fits_its_tabs_and_wraps_the_index(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update(|cx| crate::text_size::install(None, cx));
    cx.update(|cx| crate::text_size::set(20., cx));
    cx.simulate_window_resize(handle, gpui_kit::size(px(320.), px(820.)));
    let (pod, _) = running_pod();
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        settle(window, cx);
        assert!(window.try_find("detail-inspector-tabs-later").is_none());
        let index = window.find("detail-index").bounds();
        for id in [
            "detail-jump-overview",
            "detail-jump-ports",
            "detail-jump-events",
        ] {
            let jump = window.find(id).bounds();
            assert!(
                jump.left() >= index.left() - px(0.5) && jump.right() <= index.right() + px(0.5),
                "{id}: {jump:?} outside {index:?}"
            );
        }
    })
    .unwrap();
}
