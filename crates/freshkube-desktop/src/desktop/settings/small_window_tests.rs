//! Regressions for a 760×560 window at the largest text size.
use super::form;
use crate::desktop::{layout_check, testing::mount};
use crate::{DebugOptions, GpuiOptions};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Bounds, ScrollDelta, TestAppContext, Window, point, px};

fn options(page: &str, text: f32) -> GpuiOptions {
    GpuiOptions::fixture().with_debug(DebugOptions {
        page: Some(page.into()),
        text_size: Some(text),
        ..DebugOptions::default()
    })
}

pub(super) fn assert_dialog(window: &mut Window, cx: &mut App, action: &'static str) {
    window.render_frame(cx);
    let surface = window.within("dialog").find(0usize).bounds();
    let viewport = Bounds::new(point(px(0.), px(0.)), window.viewport_size());
    assert!(
        surface.is_contained_within(&viewport),
        "{surface:?} outside {viewport:?}"
    );
    let button = window.find(action).bounds();
    assert!(
        button.is_contained_within(&surface),
        "{action}: {button:?} outside {surface:?}"
    );
    assert!(button.size.height > px(0.));
}

#[gpui_kit::test]
fn small_window_dialogs_keep_their_actions_in_view(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = mount(cx, options("settings-clusters", 20.), 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        let page = pilot.read(cx).settings_page.clone();
        // Example forms may be opened for layout checks without enabling saves.
        form::open(page.clone(), window, cx);
        assert_dialog(window, cx, "settings-form-save");
        window.click("settings-form-cancel", cx);
        window.render_frame(cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn small_window_stacked_inspector_keeps_the_table_header_in_place(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = mount(cx, options("settings-clusters", 20.), 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        layout_check::assert_table_page(
            window,
            cx,
            &layout_check::TablePage {
                page: "settings-page",
                title: "settings-title",
                title_text: "Clusters",
                table: "settings-table-scroll",
                list: "settings-list",
            },
        );
        window.click("settings-cluster-core-fra", cx);
        for _ in 0..6 {
            window.render_frame(cx);
        }
        layout_check::assert_inspector(
            window,
            cx,
            "settings-split",
            "settings-table",
            "settings-detail",
            "settings-detail-title",
        );
        let fixed = [
            "settings-title",
            "settings-kubeconfig",
            "settings-table-scroll",
        ];
        let before = fixed.map(|id| window.find(id).bounds());
        let column_header = window.find(("settings-sort", 1usize)).bounds();
        let first = window.find("settings-identity-heading").bounds();
        let body = window.find("settings-detail-body").bounds();
        assert!(body.bottom() <= window.find("settings-page").bounds().bottom() + px(1.));
        window.scroll(
            "settings-detail-body",
            ScrollDelta::Pixels(point(px(0.), px(-4000.))),
            cx,
        );
        window.render_frame(cx);
        assert!(window.find("settings-identity-heading").bounds().top() < first.top());
        assert_eq!(
            window.find(("settings-sort", 1usize)).bounds(),
            column_header
        );
        for (id, before) in fixed.into_iter().zip(before) {
            assert_eq!(window.find(id).bounds(), before, "{id} moved");
        }
        assert_eq!(
            pilot.read(cx).settings_page.read(cx).page_scroll.offset().y,
            px(0.)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn small_window_rail_opens_at_the_top_at_every_text_size(cx: &mut TestAppContext) {
    for text in crate::text_size::STEPS {
        let (_runtime, handle, pilot) = mount(cx, options("settings-clusters", text), 760., 560.);
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(text, cx);
            for _ in 0..4 {
                window.render_frame(cx);
            }
            let rail = window.find("nav-rail").bounds();
            let first = window.find("nav-overview").bounds();
            assert!(
                first.is_contained_within(&rail),
                "text {text}: {first:?} outside {rail:?}"
            );
            assert_eq!(pilot.read(cx).navigation.rail_scroll.offset().y, px(0.));
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn small_window_long_cells_keep_their_column_width(cx: &mut TestAppContext) {
    use freshkube_core::workspace::{self, Loaded};
    use freshkube_ui::table;
    let (_runtime, handle, pilot) = mount(cx, options("settings-clusters", 20.), 760., 560.);
    let path = std::env::temp_dir()
        .join("a-very-long-directory-name".repeat(12))
        .join("talosconfig");
    let loaded = Loaded::Workspace(workspace::parse(serde_json::json!({
        "version": 1,
        "clusters": [{"id": "prod", "role": "environment", "context": "prod", "talosconfig": path}],
        "destinations": [{"server": "https://a-very-long-cluster-name.region-one.zone-two.example.test:6443/a-long-path", "entry": "prod"}],
    }).to_string().as_bytes()).unwrap());
    cx.update_window(handle, |_, _, cx| {
        pilot.read(cx).settings_page.clone().update(cx, |page, cx| {
            page.set_workspace(&loaded, true, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        let cell = window
            .find("settings-cluster-prod-talosconfig-cell")
            .bounds();
        assert!((cell.size.width - crate::ui::dp_px(240., window)).abs() <= px(1.));
        let cut = table::word_cut_shown("settings-cluster-prod-talosconfig").unwrap();
        assert!(cut.ends_with('…'), "{cut}");
        assert!(cut.len() < path.to_string_lossy().len());
        pilot.update(cx, |pilot, cx| {
            pilot.show_settings(super::SettingsSection::Destinations, window, cx)
        });
        window.render_frame(cx);
        layout_check::assert_table_page(
            window,
            cx,
            &layout_check::TablePage {
                page: "settings-page",
                title: "settings-destinations-title",
                title_text: "Argo CD destinations",
                table: "settings-destinations-table-scroll",
                list: "settings-destinations-list",
            },
        );
        let destination = window
            .find("settings-destination-0-destination-cell")
            .bounds();
        let next = window.find("settings-destination-0-by-cell").bounds();
        assert!(
            (destination.size.width - crate::ui::dp_px(360., window)).abs() <= px(1.),
            "{destination:?}"
        );
        assert!((destination.right() - next.left()).abs() <= px(1.));
        assert!(
            table::word_cut_shown("settings-destination-0-destination")
                .unwrap()
                .ends_with('…')
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn small_window_sources_headers_ellipsis_and_full_tooltip(cx: &mut TestAppContext) {
    use freshkube_ui::table;
    use gpui_kit::InputEvent as _;

    let (_runtime, handle, pilot) = mount(cx, options("settings-general", 13.), 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("general-text-size", 20usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(crate::text_size::current(cx), 20.);
        pilot.update(cx, |pilot, cx| {
            pilot.show_settings(super::SettingsSection::Sources, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        layout_check::assert_table_page(
            window,
            cx,
            &layout_check::TablePage {
                page: "settings-sources",
                title: "settings-sources-title",
                title_text: "Delivery sources",
                table: "settings-sources-table-scroll",
                list: "settings-sources-list",
            },
        );
    })
    .unwrap();
    for (ix, full) in [(2usize, "Entry"), (3, "Chosen by")] {
        let tooltip_draws = freshkube_ui::tooltip::drawn(full);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.rem_size(), px(20.));
            let shown = table::word_cut_shown(format!("settings-sources-sort-{ix}-label"))
                .expect("header painted");
            assert!(shown.ends_with('…'), "{full}: {shown:?}");
            assert_ne!(shown.as_ref(), full);
            let cell = window.find(("settings-sources-sort", ix)).bounds();
            window.dispatch_event(
                gpui_kit::MouseMoveEvent {
                    position: cell.center(),
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
        })
        .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(600));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find("follow-tooltip-text");
            assert!(freshkube_ui::tooltip::drawn(full) > tooltip_draws, "{full}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
async fn a_saved_banner_keeps_the_stacked_panes_and_added_row_visible(cx: &mut TestAppContext) {
    use super::edit_tests::{saved, write};
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let clusters: Vec<_> = (0..12).map(|i| serde_json::json!({
        "id": format!("cluster-{i}"), "role": "environment", "context": format!("example-{i}")
    })).collect();
    write(
        directory.path(),
        &serde_json::json!({"version": 1, "clusters": clusters}).to_string(),
    );
    let options = GpuiOptions::new(Some(directory.path().join("missing")), None, 100)
        .with_preferences(Some(directory.path().join("preferences.json")))
        .with_debug(DebugOptions::default().with_page("settings-clusters"));
    let (_runtime, handle, pilot) = mount(cx, options, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        window.click("settings-cluster-cluster-0", cx);
        pilot
            .read(cx)
            .settings_page
            .clone()
            .update(cx, |page, cx| page.open_form(window, cx));
        window.render_frame(cx);
        window.click("settings-form-context", cx);
        window.input("new-cluster", cx);
        window.click("settings-form-save", cx);
    })
    .unwrap();
    saved(cx, handle, &pilot).await;
    cx.run_until_parked();
    for _ in 0..8 {
        cx.update_window(handle, |_, window, cx| {
            window.simulate_next_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-saved").visible());
        let table = window.find("settings-table").bounds();
        let pane = window.find("settings-detail").bounds();
        assert!(
            table.size.height + px(1.) >= crate::ui::dp_px(54., window),
            "{table:?}"
        );
        assert!(
            pane.size.height + px(1.) >= crate::ui::dp_px(80., window),
            "{pane:?}"
        );
        assert!(pane.bottom() <= window.find("settings-page").bounds().bottom() + px(1.));
        let row = window.find("settings-cluster-new-cluster").bounds();
        let list = window.find("settings-list").bounds();
        assert!(
            row.top() >= list.top() - px(1.) && row.bottom() <= list.bottom() + px(1.),
            "{row:?} in {list:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_long_destination_save_caps_the_banner_and_reveals_the_new_row(cx: &mut TestAppContext) {
    use super::edit_tests::{saved, write};
    use crate::debug_options::Theme;
    cx.executor().allow_parking();
    for (width, height, text, theme) in [
        (760., 560., 20., Theme::Light),
        (760., 560., 20., Theme::Dark),
        (1280., 880., 13., Theme::Light),
        (1280., 880., 13., Theme::Dark),
        (1280., 880., 20., Theme::Light),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let destinations: Vec<_> = (0..12)
            .map(|i| serde_json::json!({"name": format!("destination-{i}"), "entry": "prod"}))
            .collect();
        write(
            directory.path(),
            &serde_json::json!({
                "version": 1,
                "clusters": [{"id": "prod", "role": "environment", "context": "example-prod"}],
                "destinations": destinations,
            })
            .to_string(),
        );
        let options = GpuiOptions::new(Some(directory.path().join("missing")), None, 100)
            .with_preferences(Some(directory.path().join("preferences.json")))
            .with_debug(
                DebugOptions::default()
                    .with_page("settings-destinations")
                    .with_theme(theme),
            );
        let (_runtime, handle, pilot) = mount(cx, options, width, height);
        let url = format!(
            "https://{}.example.test:6443/a-long-path",
            "long-cluster-name.region-one.zone-two.".repeat(5)
        );
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(text, cx);
            window.render_frame(cx);
            window.click("settings-destination-0", cx);
            window.press("a", cx);
            window.render_frame(cx);
            window.click("settings-destination-value", cx);
            window.input(&url, cx);
            window.click("settings-destination-save", cx);
        })
        .unwrap();
        saved(cx, handle, &pilot).await;
        cx.run_until_parked();
        for _ in 0..8 {
            cx.update_window(handle, |_, window, cx| {
                window.simulate_next_frame(cx);
            })
            .unwrap();
            cx.run_until_parked();
        }
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let page = pilot.read(cx).settings_page.clone();
            let banners = window.find("settings-banners").bounds();
            let banner = window.find("settings-saved").bounds();
            let cap = crate::ui::dp_px(48., window);
            let scroll = page.read(cx).banners_scroll.clone();
            if freshkube_ui::page::is_short(window) {
                assert!(
                    banners.size.height <= cap + px(1.),
                    "{banners:?} exceeds {cap:?}"
                );
                assert!(
                    banner.size.height > banners.size.height,
                    "the save must overflow"
                );
                assert!(scroll.max_offset().y > px(0.));
                window.scroll(
                    "settings-banners",
                    ScrollDelta::Pixels(point(px(0.), px(-4000.))),
                    cx,
                );
                window.render_frame(cx);
                assert!(scroll.offset().y < px(0.), "the capped banner must scroll");
                assert_eq!(window.find("settings-banners").bounds(), banners);
                assert!(window.find("settings-saved").bounds().top() < banner.top());
            } else {
                assert!(
                    banners.size.height > cap,
                    "a tall window must show the full banner"
                );
                assert!(
                    banner.top() >= banners.top() - px(1.)
                        && banner.bottom() <= banners.bottom() + px(1.),
                    "{banner:?} outside {banners:?}"
                );
                assert_eq!(scroll.max_offset().y, px(0.));
            }
            let row = window.find("settings-destination-12").bounds();
            let list = window.find("settings-destinations-list").bounds();
            let page_bounds = window.find("settings-page").bounds();
            assert!(
                row.top() >= list.top() - px(1.) && row.bottom() <= list.bottom() + px(1.),
                "{row:?} outside {list:?}"
            );
            assert!(
                row.top() >= page_bounds.top() && row.bottom() <= page_bounds.bottom() + px(1.),
                "{row:?} outside page {page_bounds:?}"
            );
            let list = page.read(cx).destinations.read(cx);
            assert_eq!(
                list.selected(),
                Some(&freshkube_core::workspace::Key::Server(url.clone()))
            );
        })
        .unwrap();
    }
}
