use super::*;

#[gpui_kit::test]
fn a_short_window_scrolls_the_frame_to_the_node_log_catalog_and_multiline_errors(
    cx: &mut TestAppContext,
) {
    short_window_node_log(cx, None);
}

#[gpui_kit::test]
fn a_short_window_at_the_largest_text_keeps_the_node_log_in_its_body(cx: &mut TestAppContext) {
    short_window_node_log(cx, Some(20.));
}

/// The node log's notices, viewport and status in a 760 × 560 window, at
/// the default text size or `text_size`.
fn short_window_node_log(cx: &mut TestAppContext, text_size: Option<f32>) {
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        let events = (0..30)
            .map(|ix| {
                freshkube_core::logs::LogEvent::new(
                    format!("service-{ix:02}-long-catalog-name"),
                    format!("level=info fixture catalog service {ix}"),
                )
            })
            .collect();
        let failures = (0..30)
            .map(|ix| {
                (
                    freshkube_core::logs::ServiceId::new(format!(
                        "service-{ix:02}-long-catalog-name"
                    )),
                    format!(
                        "Service {ix}: connection unavailable\n{}\nRetry on the next manual refresh",
                        "Multiline diagnostic context 中文 ".repeat(8)
                    ),
                )
            })
            .collect();
        let logs = view.read(cx).logs.clone();
        logs.update(cx, |logs, cx| {
            logs.set_fixture(events, window, cx);
            logs.set_fixture_failures(failures, cx);
        });
        window.render_frame(cx);
        // Reach the same retained log view through the node pane.
        open_node_tab(window, cx, super::super::nodes::NodeTab::Logs);
        if let Some(text_size) = text_size {
            crate::text_size::set(text_size, cx);
        }
        // The log's width, then its toolbar and the body around it, settle.
        // A frame draws only when one was asked for, as on screen, so the
        // body's height reaches it only through its own redraw.
        window.render_frame(cx);
        let mut asked = 0;
        for _ in 0..8 {
            let frames = window.simulate_next_frame(cx);
            if frames == 0 {
                break;
            }
            asked += frames;
            window.render_frame(cx);
        }
        assert!(asked > 0, "the body never asked to draw again");
        assert_eq!(window.simulate_next_frame(cx), 0, "frames never settled");
        // The frame scrolled to the pane when it opened, at the text size
        // of the time.
        let frame = view.read(cx).node_workspace.read(cx).page_scroll.clone();
        frame.scroll_to_bottom();
        window.render_frame(cx);
        // 560 px is short at the default text size, so the Logs body
        // scrolls inside the node's inspector. The log keeps its notices
        // above a usable viewport inside its panel, and the status bar's log
        // status stays below the pane.
        let panel = window.find("logs-panel").bounds();
        let viewport = window.find("logs-viewport").bounds();
        let notices = window.find("logs-notices").bounds();
        assert!(viewport.top() >= panel.top());
        assert!(viewport.left() >= panel.left());
        assert!(viewport.right() <= panel.right());
        assert!(viewport.bottom() <= panel.bottom());
        assert!(viewport.size.height >= window.rem_size() * 6.);
        assert!(viewport.right() <= px(760.));
        assert!(notices.size.height >= window.rem_size());
        assert!(notices.bottom() <= viewport.top());
        // At the largest text the toolbar alone outgrows the body's room,
        // and the body scrolls to the rest of it.
        if text_size.is_none() {
            assert!(window.find("logs-collection").visible());
        }
        // Scrolling the body brings the notices into view, then the whole
        // viewport, above the status bar's log status.
        let body = view.read(cx).node_workspace.read(cx).logs_scroll.clone();
        let content = window.find("node-logs-scroll").bounds();
        let by = (content.bottom() - notices.bottom()).min(px(0.));
        body.set_offset(point(px(0.), body.offset().y + by));
        window.render_frame(cx);
        assert!(window.find("logs-notices").visible());
        body.set_offset(point(px(0.), -body.max_offset().y));
        window.render_frame(cx);
        let viewport = window.find("logs-viewport").bounds();
        let shown = window.find("node-logs-scroll").bounds();
        let status = window.find("logs-status");
        assert!(status.visible());
        assert!(status.bounds().top() >= viewport.bottom());
        assert!(status.bounds().bottom() <= px(560.));
        assert!(viewport.top() >= shown.top(), "{viewport:?} {shown:?}");
        assert!(viewport.bottom() <= shown.bottom(), "{viewport:?} {shown:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn text_size_steps_by_shortcut_and_settings_and_survives_appearance(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let rem = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        window.render_frame(cx);
        window.rem_size()
    };
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(rem(window, cx), px(13.));
        window.press("secondary-=", cx);
        assert_eq!(rem(window, cx), px(14.));
        for _ in 0..3 {
            window.press("secondary-=", cx);
        }
        // The largest step holds.
        assert_eq!(rem(window, cx), px(20.));
        assert!((Theme::global(cx).mono_font_size - px(20. * 12.5 / 13.)).abs() < px(0.001));
        window.press("secondary-0", cx);
        assert_eq!(rem(window, cx), px(13.));
        window.press("secondary--", cx);
        window.press("secondary--", cx);
        assert_eq!(rem(window, cx), px(12.));

        window.click("settings", cx);
        window.render_frame(cx);
        window.click(("text-size", 18usize), cx);
        assert_eq!(rem(window, cx), px(18.));
        // Kit reloads the theme's font size with its colors.
        view.update(cx, |view, cx| {
            view.set_appearance(super::super::Appearance::Dark, window, cx)
        });
        assert_eq!(Theme::global(cx).mode, ThemeMode::Dark);
        assert_eq!(rem(window, cx), px(18.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_shell_scales_with_the_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        let measure = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            window.render_frame(cx);
            let sidebar = window.find("nav-rail").bounds().size.width;
            let nav = window.find("nav-overview").bounds().size.height;
            (sidebar, nav)
        };
        let (sidebar, nav) = measure(window, cx);
        for _ in 0..4 {
            window.press("secondary-=", cx);
        }
        let (larger_sidebar, larger_nav) = measure(window, cx);
        let ratio = 20. / 13.;
        assert!(
            (larger_sidebar / sidebar - ratio).abs() < 0.01,
            "{larger_sidebar:?}"
        );
        assert!(
            (larger_nav / nav - ratio).abs() < 0.1,
            "{nav:?} → {larger_nav:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_resources_toolbar_folds_rather_than_clip_at_a_large_size(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        area(window, cx, "nav-k8s-group-workloads");
        let fit = |window: &mut gpui_kit::Window| {
            let ids = ["resource-filter", "resource-namespace", "resource-refresh"];
            let row = window.find("resource-toolbar").bounds();
            let height = crate::ui::dp_px(38., window);
            assert!((row.size.height - height).abs() < px(0.5), "{row:?}");
            for id in ids.into_iter().chain(["resource-more"]) {
                let Some(control) = window.try_find(id) else {
                    continue;
                };
                let bounds = control.bounds();
                assert!(
                    bounds.right() <= px(760.),
                    "{id} ends at {:?}",
                    bounds.right()
                );
                assert!(bounds.size.width > px(0.), "{id}");
                assert!(
                    bounds.top() >= row.top() && bounds.bottom() <= row.bottom(),
                    "{id}"
                );
            }
        };
        settle_header(window, cx);
        fit(window);
        for _ in 0..4 {
            window.press("secondary-=", cx);
        }
        settle_header(window, cx);
        fit(window);
        assert!(
            window.try_find("resource-more").is_some(),
            "a full row folds its controls"
        );
        assert!(
            window.find("resource-list").bounds().size.height >= crate::ui::dp_px(68., window),
            "The toolbar and legend must leave at least two rows of list space"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_resources_page_scales_with_the_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        area(window, cx, "nav-k8s-group-workloads");
        let namespace = window.find("resource-namespace").bounds().size.width;
        assert_eq!(namespace, px(132.));
        window.press("secondary--", cx);
        window.render_frame(cx);
        let smaller = window.find("resource-namespace").bounds().size.width;
        assert!(
            (smaller / namespace - 12. / 13.).abs() < 0.01,
            "{smaller:?}"
        );
    })
    .unwrap();
}

/// The table page every other table page is measured against.
const PODS: super::super::layout_check::TablePage = super::super::layout_check::TablePage {
    page: "resources-page",
    title: "resource-title",
    title_text: "Pods",
    table: "resource-table-scroll",
    list: "resource-list",
};

#[gpui_kit::test]
fn pods_is_a_table_page_at_every_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            view.update(cx, |view, cx| view.open_builtin("pods", window, cx));
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            // Example pods include a failing one, so Problems shows groups.
            let layout = super::super::layout_check::assert_table_page(window, cx, &PODS);
            assert!(layout.group.is_some(), "{layout:#?}");
        })
        .unwrap();
    }
}

/// A page whose table isn't the whole page (a dashboard's table panel, a list
/// without column captions) checks its frame and its table apart.
#[gpui_kit::test]
fn the_frame_and_table_checks_measure_apart(cx: &mut TestAppContext) {
    use super::super::layout_check::{PageFrame, Table, assert_edge_frame, assert_table};
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.open_builtin("pods", window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let frame = PageFrame {
            page: PODS.page,
            title: PODS.title,
            title_text: PODS.title_text,
            content: PODS.table,
        };
        assert_edge_frame(window, cx, &frame);
        let rows = assert_table(
            window,
            cx,
            &Table {
                table: Some(PODS.table),
                list: PODS.list,
            },
        );
        assert!(rows.header.is_some(), "{rows:#?}");
        // A list without column captions skips the header.
        let rows = assert_table(
            window,
            cx,
            &Table {
                table: None,
                list: PODS.list,
            },
        );
        assert!(rows.header.is_none(), "{rows:#?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn search_everything_keeps_one_width_on_every_page(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    let mut widths = Vec::new();
    for page in [Page::Resources, Page::Observability, Page::Overview] {
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.navigate(page, window, cx));
            window.render_frame(cx);
            let width = window.find("search-everything").bounds().size.width;
            widths.push((page, width / crate::ui::dp_px(1., window)));
        })
        .unwrap();
    }
    for (page, width) in &widths {
        assert!(
            (*width - 240.).abs() < 0.5,
            "{page:?} shows Search everything as a {width} dp field, not the full one"
        );
    }
}

#[gpui_kit::test]
fn at_the_default_window_pods_folds_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1320., 860.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.open_builtin("pods", window, cx));
        settle_header(window, cx);
        assert!(window.try_find("resource-more").is_none());
        let row = window.find("resource-toolbar").bounds();
        assert_eq!(row.size.height, crate::ui::dp_px(38., window));
        // Namespace, Columns and Refresh, all on the toolbar's row.
        for ix in 0..3 {
            let slot = window.find(format!("resource-slot-{ix}")).bounds();
            assert!(
                slot.top() >= row.top() && slot.bottom() <= row.bottom(),
                "{ix}"
            );
        }
        let chips = window.find("resource-tally-healthy").bounds();
        assert!(chips.top() >= row.top() && chips.bottom() <= row.bottom());
    })
    .unwrap();
}
