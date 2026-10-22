use super::*;

#[gpui_kit::test]
fn theme_toggle_preserves_fixture_state_on_every_screen(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(Theme::global(cx).mode, ThemeMode::Light);
        pick_target(window, cx, 4);
        open_node_tab(window, cx, super::super::nodes::NodeTab::Services);
        window.render_frame(cx);
        window.within("services-region").click("containerd", cx);
    })
    .unwrap();
    cx.run_until_parked();
    for (nav, page) in [
        ("nav-overview", Page::Overview),
        ("node-tab-services", Page::Nodes),
        ("node-tab-logs", Page::Nodes),
    ] {
        cx.update_window(handle, |_, window, cx| {
            match nav {
                "node-tab-services" => {
                    open_node_tab(window, cx, super::super::nodes::NodeTab::Services)
                }
                "node-tab-logs" => open_node_tab(window, cx, super::super::nodes::NodeTab::Logs),
                _ => window.click(nav, cx),
            };
            window.render_frame(cx);
            let pilot = view.read(cx);
            assert_eq!(pilot.target.selected_node.as_deref(), Some(DEGRADED_NODE));
            assert_eq!(
                pilot.pages.selected_service(cx).as_deref(),
                Some("containerd")
            );
            let context = pilot.applied.context.clone();
            let node = pilot.target.selected_node.clone();
            let service = pilot.pages.selected_service(cx).clone();
            let epoch = pilot.epoch;
            let config_revision = pilot.configuration_read.revision();
            let overview_success = pilot.overview.last_successful();
            let services_success = pilot.target.services.last_successful();
            let overview_name = pilot.overview.data().unwrap().name.clone();
            let node_count = pilot.target.nodes.len();
            let service_count = pilot.target.services.data().unwrap().len();
            let logs = pilot.pages.logs.clone();
            let light_background = cx.theme().background;
            assert_eq!(window.find("theme-toggle").label(), Some("Dark mode"));
            if nav == "node-tab-logs" {
                assert_eq!(
                    window.find("logs-collection").label(),
                    Some("Start collecting")
                );
            }
            window.click("theme-toggle", cx);
            window.render_frame(cx);
            assert_eq!(Theme::global(cx).mode, ThemeMode::Dark);
            assert_ne!(cx.theme().background, light_background);
            assert_eq!(window.find("theme-toggle").label(), Some("Light mode"));
            // Kit buttons deliberately do not steal focus on pointer down.
            // Reach this control through the production keyboard tab order.
            for _ in 0..96 {
                if window.find("theme-toggle").focused() == Some(true) {
                    break;
                }
                window.press("tab", cx);
            }
            assert_eq!(
                window.find("theme-toggle").focused(),
                Some(true),
                "theme toggle must be keyboard reachable on {page:?}"
            );
            window.press("space", cx);
            window.render_frame(cx);
            assert_eq!(Theme::global(cx).mode, ThemeMode::Light);
            assert_eq!(cx.theme().background, light_background);
            assert_eq!(window.find("theme-toggle").label(), Some("Dark mode"));
            assert_eq!(window.find("theme-toggle").focused(), Some(true));
            let pilot = view.read(cx);
            assert_eq!(pilot.navigation.page, page);
            assert_eq!(pilot.applied.context, context);
            assert_eq!(pilot.target.selected_node, node);
            assert_eq!(pilot.pages.selected_service(cx), service);
            assert_eq!(pilot.epoch, epoch);
            assert_eq!(pilot.configuration_read.revision(), config_revision);
            assert_eq!(pilot.overview.last_successful(), overview_success);
            assert_eq!(pilot.target.services.last_successful(), services_success);
            assert_eq!(pilot.overview.data().unwrap().name, overview_name);
            assert_eq!(pilot.target.nodes.len(), node_count);
            assert_eq!(pilot.target.services.data().unwrap().len(), service_count);
            assert_eq!(pilot.pages.logs, logs);
            if nav == "node-tab-logs" {
                assert!(window.find("logs-viewport").visible());
                assert_eq!(
                    window.find("logs-collection").label(),
                    Some("Start collecting")
                );
            }
        })
        .unwrap();
    }
}

/// The header is 44 dp with one row of controls centred in it, at the
/// default text size and the largest; the page starts under it.
#[gpui_kit::test]
fn header_is_one_row_of_controls_at_every_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 880.);
    for text_size in [13., 20.] {
        cx.update(|cx| crate::text_size::set(text_size, cx));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let dp = |n| crate::ui::dp_px(n, window);
            let close = |what: &str, got: gpui_kit::Pixels, want: gpui_kit::Pixels| {
                assert!(
                    (got - want).abs() < px(0.5),
                    "{what} at text {text_size}: {got:?}, not {want:?}"
                );
            };
            close(
                "the page's top",
                window.find("page-cell").bounds().top(),
                dp(44.),
            );
            close(
                "the page's bottom, over the status bar",
                window.find("page-cell").bounds().bottom(),
                px(880.) - dp(28.),
            );
            // Centred on the header less its hairline.
            let middle = (dp(44.) - px(1.)) / 2.;
            for (id, height) in [
                ("context-switcher", 28.),
                ("search-everything", 28.),
                ("refresh", 24.),
                ("theme-toggle", 24.),
                ("settings", 24.),
            ] {
                let bounds = window.find(id).bounds();
                close(id, bounds.size.height, dp(height));
                close(id, bounds.center().y, middle);
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn narrow_window_keeps_screens_and_actions_reachable(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("refresh").visible());
        assert!(window.find("theme-toggle").visible());
        assert!(window.find("theme-toggle").bounds().right() <= px(760.));
        assert!(window.find("context-switcher").bounds().right() <= px(760.));
        assert!(window.find("overview-cards").visible());
        assert!(window.find("overview-cards").bounds().right() <= px(760.));
        open_node_tab(window, cx, super::super::nodes::NodeTab::Services);
        window.render_frame(cx);
        assert!(window.find("theme-toggle").visible());
        assert!(window.find("service-filter").visible());
        assert!(window.find("services-region").bounds().size.height > px(0.));
        assert!(window.find("services-region").bounds().right() <= px(760.));
        assert!(window.find("services-region").bounds().top() < px(560.));
        open_node_tab(window, cx, super::super::nodes::NodeTab::Logs);
        window.render_frame(cx);
        assert!(window.find("theme-toggle").visible());
        assert!(window.find("theme-toggle").bounds().right() <= px(760.));
        assert!(window.find("theme-toggle").bounds().bottom() <= px(560.));
        window.click("theme-toggle", cx);
        assert_eq!(Theme::global(cx).mode, ThemeMode::Dark);
        assert_eq!(window.find("theme-toggle").label(), Some("Light mode"));
        for _ in 0..96 {
            if window.find("theme-toggle").focused() == Some(true) {
                break;
            }
            window.press("tab", cx);
        }
        assert_eq!(window.find("theme-toggle").focused(), Some(true));
        window.press("space", cx);
        assert_eq!(Theme::global(cx).mode, ThemeMode::Light);
    })
    .unwrap();
}

#[gpui_kit::test]
fn long_context_names_keep_both_ends_and_wrap_in_the_popover(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let full = "talos-production-frankfurt-equinix-fr5-baremetal-b7";
    for text_size in [14., 20.] {
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(text_size, cx);
            view.update(cx, |view, cx| view.select_context(full.into(), window, cx));
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let short = window
                .find("context-short-name")
                .label()
                .unwrap()
                .to_owned();
            assert!(short.contains('…'), "{short}");
            assert!(short.starts_with("talos-"), "{short}");
            assert!(short.ends_with("-b7"), "{short}");
            assert_eq!(window.find("context-switcher").label(), Some(full));
            contexts(window, cx);
            let row = window.find(("context", 3usize));
            assert_eq!(row.label(), Some(full));
            assert!(row.bounds().size.height >= crate::ui::dp_px(32., window));
            window.press("escape", cx);
        })
        .unwrap();
    }
}

/// The header has no section tabs (the rail covers navigation); Search
/// everything is centred at 320 dp, and the switcher, Refresh and Settings
/// stay.
#[gpui_kit::test]
fn header_has_no_section_tabs_and_centres_a_320_search(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for id in [
            "section-dashboard",
            "section-nodes",
            "section-workloads",
            "section-events",
            "section-observability",
        ] {
            assert!(window.try_find(id).is_none(), "{id} is still drawn");
        }
        let dp = |n| crate::ui::dp_px(n, window);
        let search = window.find("search-everything").bounds();
        assert!(
            (search.size.width - dp(320.)).abs() < px(0.5),
            "search is {:?} wide",
            search.size.width
        );
        assert!(
            (search.center().x - px(1280.) / 2.).abs() < dp(2.),
            "search is centred at {:?}",
            search.center().x
        );
        for id in ["context-switcher", "refresh", "settings"] {
            assert!(window.find(id).visible(), "{id} is missing");
        }
        assert!(window.find("context-switcher").bounds().right() < search.left());
        assert!(window.find("refresh").bounds().left() > search.right());
    })
    .unwrap();
}

/// At the narrowest window and the largest text, the search gives way
/// before anything overlaps.
#[gpui_kit::test]
fn narrow_header_shrinks_the_search_before_anything_overlaps(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 760., 560.);
    cx.update(|cx| crate::text_size::set(20., cx));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let search = window.find("search-everything").bounds();
        let switcher = window.find("context-switcher").bounds();
        let settings = window.find("settings").bounds();
        assert!(switcher.right() <= search.left(), "{switcher:?} {search:?}");
        assert!(search.right() <= settings.left(), "{search:?} {settings:?}");
        assert!(settings.right() <= px(760.));
    })
    .unwrap();
}
