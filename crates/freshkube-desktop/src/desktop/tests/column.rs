use super::*;

/// The column draws again when what it reads from a page changes, and only
/// while it shows that page's area: System services' badge on Control
/// plane, and the namespace a Workloads column picks.
#[gpui_kit::test]
fn the_column_redraws_for_the_pages_it_reads(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    let (services, resources) = cx.update(|cx| {
        let pilot = view.read(cx);
        (
            pilot.pages.system_services.clone(),
            pilot.pages.resources.clone(),
        )
    });
    let services_notify =
        |_: &mut gpui_kit::Window, cx: &mut gpui_kit::App| services.update(cx, |_, cx| cx.notify());
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-control-plane", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        chrome_redrawn(cx, handle, services_notify),
        ["chrome.column"]
    );
    cx.update_window(handle, |_, window, cx| open_kind(window, cx, "pods"))
        .unwrap();
    cx.run_until_parked();
    assert!(chrome_redrawn(cx, handle, services_notify).is_empty());
    let resources_notify = resources.clone();
    assert!(
        chrome_redrawn(cx, handle, |_, cx| resources_notify
            .update(cx, |_, cx| cx.notify()))
        .is_empty(),
        "a watch event leaves the column"
    );
    assert_eq!(
        chrome_redrawn(cx, handle, |window, cx| resources
            .update(cx, |resources, cx| {
                resources.set_namespace(Some("payments".into()), window, cx)
            })),
        ["chrome.column"]
    );
}

/// The column's rows, and its section labels, are the table's rows: 26 dp
/// at any text size.
#[gpui_kit::test]
fn the_column_rows_are_the_tables_height(cx: &mut TestAppContext) {
    // Wide enough to keep the column expanded at 20.
    let (_runtime, handle, view) = fixture(cx, 2000., 1000.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.open_builtin("pods", window, cx));
        for (text_size, height) in [(13., 26.), (20., 40.)] {
            crate::text_size::set(text_size, cx);
            window.render_frame(cx);
            for id in [
                "nav-health",
                "nav-k8s-pods",
                "nav-k8s-deployments.apps",
                "nav-namespaces",
                "nav-all-namespaces",
            ] {
                assert_eq!(
                    window.find(id).bounds().size.height,
                    px(height),
                    "{id} at {text_size}"
                );
            }
        }
    })
    .unwrap();
}

/// The column takes the keyboard as the table does: the arrows, Home and
/// End move over its rows, passing its labels, and Enter opens the row;
/// Escape hands the keyboard back to the page.
#[gpui_kit::test]
fn the_column_moves_and_opens_with_the_keyboard(cx: &mut TestAppContext) {
    use super::super::shell::ColumnKey;
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.open_builtin("pods", window, cx));
        window.render_frame(cx);
        // The page's row is the selected one, and only it.
        assert_eq!(window.find("nav-k8s-pods").selected(), Some(true));
        assert_ne!(window.find("nav-health").selected(), Some(true));
        assert_ne!(
            window.find("nav-k8s-deployments.apps").selected(),
            Some(true)
        );
        let focus = view.read(cx).navigation.column_list.focus_handle().clone();
        window.focus(&focus, cx);
        window.render_frame(cx);
        let cursor = |cx: &gpui_kit::App| view.read(cx).navigation.column_list.cursor().cloned();
        // The keyboard starts on the page's row.
        assert_eq!(cursor(cx), Some(ColumnKey::Kind("pods")));
        window.press("down", cx);
        assert_eq!(cursor(cx), Some(ColumnKey::Kind("deployments.apps")));
        assert_eq!(
            view.read(cx).navigation.resource_kind.key(),
            "pods",
            "only Enter opens"
        );
        window.press("up", cx);
        window.press("up", cx);
        assert_eq!(cursor(cx), Some(ColumnKey::Page(Page::Health)));
        window.press("up", cx);
        assert_eq!(
            cursor(cx),
            Some(ColumnKey::Page(Page::Health)),
            "the first stays"
        );
        // End passes the Namespaces label to the last namespace.
        window.press("end", cx);
        let last = view
            .read(cx)
            .navigation
            .column_list
            .lines()
            .iter()
            .rev()
            .find_map(|line| line.row())
            .map(|row| row.key().clone());
        assert!(matches!(last, Some(ColumnKey::Namespace(Some(_)))));
        assert_eq!(cursor(cx), last);
        window.press("home", cx);
        assert_eq!(cursor(cx), Some(ColumnKey::Page(Page::Health)));
        window.press("down", cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert_eq!(view.read(cx).navigation.page, Page::Resources);
        assert_eq!(
            view.read(cx).navigation.resource_kind.key(),
            "deployments.apps"
        );
        window.render_frame(cx);
        assert_eq!(
            window.find("nav-k8s-deployments.apps").selected(),
            Some(true)
        );
        assert_ne!(window.find("nav-k8s-pods").selected(), Some(true));
        assert!(
            !focus.is_focused(window),
            "Enter hands the keyboard to the page"
        );
        // Escape hands it back without opening anything.
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("down", cx);
        window.press("escape", cx);
        assert!(!focus.is_focused(window));
        assert_eq!(
            view.read(cx).navigation.resource_kind.key(),
            "deployments.apps"
        );
    })
    .unwrap();
}

/// A column that folds or goes hands the keyboard to the page: a list
/// that isn't drawn takes no keys, so the page's would stop working.
#[gpui_kit::test]
fn the_column_hands_the_keyboard_to_the_page_when_it_folds(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    let focus = cx.update(|cx| view.read(cx).navigation.column_list.focus_handle().clone());
    let focus_column = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.focus(&focus, cx);
            window.render_frame(cx);
            assert!(focus.is_focused(window));
        })
        .unwrap();
    };
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.open_builtin("pods", window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    // Folded with ⌘B.
    focus_column(cx);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-b", cx);
        window.render_frame(cx);
        assert!(view.read(cx).navigation.column_collapsed(window));
        assert!(!focus.is_focused(window));
        assert!(window.focused(cx).is_some(), "the page has the keyboard");
        window.press("secondary-b", cx);
        window.render_frame(cx);
    })
    .unwrap();
    // Folded by a narrower window.
    focus_column(cx);
    cx.simulate_window_resize(handle, size(px(760.), px(560.)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(view.read(cx).navigation.column_collapsed(window));
        assert!(!focus.is_focused(window));
        assert!(window.focused(cx).is_some());
    })
    .unwrap();
    cx.simulate_window_resize(handle, size(px(1280.), px(880.)));
    cx.run_until_parked();
    // Gone with an area that has no column.
    focus_column(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.show_area(Area::Events, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(!view.read(cx).navigation.area.has_column());
        assert!(!focus.is_focused(window));
        assert!(window.focused(cx).is_some());
    })
    .unwrap();
}

/// All namespaces is the row shown while Pods lists every namespace, and
/// past the rows the column lists, a menu offers every namespace.
#[gpui_kit::test]
fn the_column_reaches_every_namespace(cx: &mut TestAppContext) {
    use super::super::shell::ColumnKey;
    use freshkube_ui::source_list::Line;
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.open_builtin("pods", window, cx));
        window.render_frame(cx);
        assert_eq!(window.find("nav-all-namespaces").selected(), Some(true));
        let names: Vec<SharedString> = (0..24)
            .map(|ix| SharedString::from(format!("team-{ix:02}")))
            .collect();
        view.update(cx, |pilot, cx| {
            pilot.navigation.column_state.namespaces = names
                .iter()
                .map(|name| (name.clone(), "1".into()))
                .collect();
            pilot.navigation.column_changed();
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("nav-namespace-team-19").is_some());
        assert!(window.try_find("nav-namespace-team-20").is_none());
        assert_eq!(
            window.find("nav-namespaces-more").label(),
            Some("4 more namespaces…")
        );
        let pilot = view.read(cx);
        let Some(Line::Menu(menu)) = pilot
            .navigation
            .column_list
            .lines()
            .iter()
            .find(|line| line.id().as_ref() == "nav-namespaces-more")
        else {
            panic!("no namespace menu");
        };
        let keys: Vec<_> = menu.entries().iter().map(|(_, key)| key.clone()).collect();
        assert_eq!(keys.len(), 25);
        assert_eq!(keys[0], ColumnKey::Namespace(None));
        assert_eq!(keys[24], ColumnKey::Namespace(Some(names[23].clone())));
    })
    .unwrap();
}

/// A page's own redraws leave the column as it was drawn, and the
/// keyboard moving in the column draws the column alone.
#[gpui_kit::test]
fn the_column_draws_for_itself_alone(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.open_builtin("pods", window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let (resources, health) = cx.update(|cx| {
        let pilot = view.read(cx);
        (pilot.pages.resources.clone(), pilot.workloads())
    });
    assert!(
        chrome_redrawn(cx, handle, |_, cx| resources
            .update(cx, |_, cx| cx.notify()))
        .is_empty()
    );
    assert!(chrome_redrawn(cx, handle, |_, cx| health.update(cx, |_, cx| cx.notify())).is_empty());
    let focus = cx.update(|cx| view.read(cx).navigation.column_list.focus_handle().clone());
    cx.update_window(handle, |_, window, cx| window.focus(&focus, cx))
        .unwrap();
    cx.run_until_parked();
    // Not `window.press`, which draws whole frames and so passes every
    // cache.
    assert_eq!(
        chrome_redrawn(cx, handle, |window, cx| focus.dispatch_action(
            &freshkube_ui::source_list::SelectNext,
            window,
            cx
        )),
        ["chrome.column"]
    );
    assert_eq!(
        cx.update(|cx| view.read(cx).navigation.column_list.cursor().cloned()),
        Some(super::super::shell::ColumnKey::Kind("deployments.apps"))
    );
}

/// Monitoring and Observability each draw the column in their own area,
/// and only there. Custom Resources draws it through the shell, which
/// observes it.
#[gpui_kit::test]
fn the_column_redraws_for_its_areas_page(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    type Notify = Box<dyn Fn(&mut gpui_kit::App)>;
    let (custom, pages): (Notify, Vec<(&str, Notify)>) = cx.update(|cx| {
        let pilot = view.read(cx);
        let (custom, monitoring) = (pilot.pages.custom.clone(), pilot.pages.monitoring.clone());
        let observability = pilot.pages.observability.clone();
        (
            Box::new(move |cx: &mut gpui_kit::App| custom.update(cx, |_, cx| cx.notify()))
                as Notify,
            vec![
                (
                    "nav-monitoring",
                    Box::new(move |cx: &mut gpui_kit::App| {
                        monitoring.update(cx, |_, cx| cx.notify())
                    }) as Notify,
                ),
                (
                    "nav-observability",
                    Box::new(move |cx: &mut gpui_kit::App| {
                        observability.update(cx, |_, cx| cx.notify())
                    }),
                ),
            ],
        )
    });
    for (nav, _) in &pages {
        cx.update_window(handle, |_, window, cx| window.click(*nav, cx))
            .unwrap();
        cx.run_until_parked();
        for (other, notify) in &pages {
            let redrawn = chrome_redrawn(cx, handle, |_, cx| notify(cx));
            if other == nav {
                assert_eq!(redrawn, ["chrome.column"], "{other} on {nav}");
            } else {
                assert!(redrawn.is_empty(), "{other} on {nav}: {redrawn:?}");
            }
        }
    }
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-k8s-group-custom", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(chrome_redrawn(cx, handle, |_, cx| custom(cx)).contains(&"chrome.column"));
}

#[gpui_kit::test]
fn fog_observability_navigation_range_and_sidebar_shortcut(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-observability", cx);
        assert_eq!(pilot.read(cx).navigation.page, Page::Observability);
        window.render_frame(cx);
        assert!(window.find("refresh").visible());
        window.click("nav-collapse", cx);
        assert!(pilot.read(cx).navigation.column_collapsed(window));
        window.click("nav-obs-traces", cx);
        assert_eq!(
            pilot.read(cx).pages.observability.read(cx).destination(),
            crate::observability::Destination::Traces
        );
        window.click("obs-time", cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert_eq!(pilot.read(cx).pages.observability.read(cx).hours(), 24);
        window.press("secondary-b", cx);
        assert!(!pilot.read(cx).navigation.column_collapsed(window));
    })
    .unwrap();
    // An expanded preference still yields to a narrow window, then returns.
    cx.simulate_window_resize(handle, size(px(760.), px(560.)));
    cx.update_window(handle, |_, window, cx| {
        assert!(pilot.read(cx).navigation.column_collapsed(window));
    })
    .unwrap();
    cx.simulate_window_resize(handle, size(px(1280.), px(880.)));
    cx.update_window(handle, |_, window, cx| {
        assert!(!pilot.read(cx).navigation.column_collapsed(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn monitoring_breadcrumb_opens_a_collapsed_dashboards_column(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Monitoring, window, cx));
        window.render_frame(cx);
        assert!(pilot.read(cx).navigation.column_collapsed(window));
        window.click("monitoring-dashboards", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(!pilot.read(cx).navigation.column_collapsed(window));
        window.render_frame(cx);
        // An open column stays open.
        window.click("monitoring-dashboards", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(!pilot.read(cx).navigation.column_collapsed(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn collapsed_observability_column_scrolls_its_active_item_into_view(cx: &mut TestAppContext) {
    use crate::observability::Destination;
    // At 20 px text a 560 high window can't show every destination.
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| {
            pilot.navigate(Page::Observability, window, cx)
        });
        window.render_frame(cx);
        assert!(pilot.read(cx).navigation.column_collapsed(window));
    })
    .unwrap();
    for destination in [
        Destination::Deployments,
        Destination::Applications,
        Destination::Profiling,
    ] {
        cx.update_window(handle, |_, window, cx| {
            let page = pilot.read(cx).pages.observability.clone();
            page.update(cx, |page, cx| page.open(destination, cx));
            // The reveal is asked for while drawing and applied by the next layout.
            window.render_frame(cx);
            window.render_frame(cx);
            let list = window.find("obs-navigation-scroll").bounds();
            let item = window
                .find(format!("nav-obs-{}", destination.slug()))
                .bounds();
            assert!(
                item.top() >= list.top() - px(0.5) && item.bottom() <= list.bottom() + px(0.5),
                "{destination:?}: {item:?} outside {list:?}"
            );
            // Nor does a fade over a cut edge dim it (#406).
            let fade = crate::ui::dp_px(28., window);
            let (top, bottom) =
                super::super::shell::cut_edges(&pilot.read(cx).navigation.obs_column_scroll);
            assert!(
                (!top || item.top() >= list.top() + fade - px(0.5))
                    && (!bottom || item.bottom() <= list.bottom() - fade + px(0.5)),
                "{destination:?}: {item:?} under a fade of {list:?}"
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn every_expanded_column_puts_its_caption_and_rows_in_one_place(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        // Each area's collapse button (beside its caption) and first row.
        let mut seen = Vec::new();
        for (page, first) in [
            (Page::Etcd, "nav-etcd"),
            (Page::Health, "nav-health"),
            (Page::Monitoring, "nav-collapse"),
            (Page::Observability, "nav-obs-applications"),
        ] {
            pilot.update(cx, |pilot, cx| pilot.navigate(page, window, cx));
            window.render_frame(cx);
            assert!(!pilot.read(cx).navigation.column_collapsed(window));
            let toggle = window.find("nav-collapse").bounds();
            let row = window.find(first).bounds();
            seen.push((page, toggle, row));
        }
        let (_, toggle, row) = seen[0];
        for (page, other, other_row) in &seen[1..] {
            assert_eq!(*other, toggle, "{page:?}'s caption moved");
            if *page != Page::Monitoring {
                assert_eq!(other_row.origin, row.origin, "{page:?}'s first row moved");
                assert_eq!(
                    other_row.size.width, row.size.width,
                    "{page:?}'s rows are wider"
                );
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_rail_fits_without_scrolling_at_the_default_window_and_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(13., cx);
        for active in [Area::Overview, Area::Settings, Area::Overview] {
            window.render_frame(cx);
            window.click(active.id(), cx);
            window.render_frame(cx);
            window.render_frame(cx);
            assert_eq!(pilot.read(cx).navigation.area, active);
            let scroll = &pilot.read(cx).navigation.rail_scroll;
            assert_eq!(scroll.max_offset().y, px(0.));
            assert_eq!(scroll.offset().y, px(0.));
            let rail = window.find("nav-rail").bounds();
            for area in Area::RAIL.iter().flat_map(|section| section.iter()) {
                let button = window.find(area.id());
                let bounds = button.bounds();
                assert!(button.visible(), "{area:?} is hidden");
                assert_eq!(bounds.size, gpui_kit::size(px(44.), px(44.)));
                assert!((bounds.center().x - (rail.center().x - px(0.5))).abs() <= px(0.5));
                assert!(
                    bounds.top() >= rail.top() && bounds.bottom() <= rail.bottom(),
                    "{area:?}: {bounds:?} outside {rail:?}"
                );
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_scrolling_rail_keeps_the_selected_highlight_clear_of_the_scrollbar(cx: &mut TestAppContext) {
    use gpui_kit::component::scroll::Scrollbar;

    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    for (width, height, text) in [(760., 560., 13.), (1280., 880., 20.)] {
        cx.simulate_window_resize(handle, size(px(width), px(height)));
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(text, cx);
            for theme in [ThemeMode::Light, ThemeMode::Dark] {
                Theme::change(theme, Some(window), cx);
                for active in [Area::Overview, Area::Settings, Area::Overview] {
                    window.render_frame(cx);
                    window.scroll(
                        "nav-rail",
                        gpui_kit::ScrollDelta::Pixels(point(
                            px(0.),
                            px(if active == Area::Settings {
                                -2000.
                            } else {
                                2000.
                            }),
                        )),
                        cx,
                    );
                    window.click(active.id(), cx);
                    window.render_frame(cx);
                    window.render_frame(cx);
                    assert_eq!(pilot.read(cx).navigation.area, active);
                    let scroll = &pilot.read(cx).navigation.rail_scroll;
                    assert!(scroll.max_offset().y > px(0.));
                    let visible = scroll.bounds();
                    let button = window.find(active.id());
                    let bounds = button.bounds();
                    assert_eq!(button.selected(), Some(true));
                    assert!(button.visible());
                    assert!(bounds.is_contained_within(&visible));
                    assert!(
                        bounds.right() <= visible.right() - Scrollbar::width(),
                        "{active:?}: {bounds:?} overlaps the scrollbar in {visible:?}"
                    );
                    let painted = bounds.scale(window.scale_factor());
                    let highlight =
                        gpui_kit::solid_background(crate::palette::palette(cx).accent_soft);
                    let quads = window.painted_quads();
                    let fill = quads
                        .iter()
                        .find(|quad| quad.bounds == painted && quad.background == highlight)
                        .expect("the selected button paints its highlight");
                    assert!(painted.is_contained_within(&fill.content_mask.bounds));
                    assert!(fill.corner_radii.top_left.0 > 0.);
                    assert!(fill.corner_radii.top_right.0 > 0.);
                    assert!(fill.corner_radii.bottom_left.0 > 0.);
                    assert!(fill.corner_radii.bottom_right.0 > 0.);
                }
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_cut_rail_or_column_fades_at_the_edges_it_cuts(cx: &mut TestAppContext) {
    use super::super::shell::cut_edges;
    // At 20 px text a 560 high window shows neither the rail's areas nor
    // the collapsed column's destinations in full (#406).
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        window.render_frame(cx);
        let edges = |pilot: &Entity<Pilot>, cx: &gpui_kit::App| {
            let pilot = pilot.read(cx);
            (
                cut_edges(&pilot.navigation.rail_scroll),
                cut_edges(&pilot.navigation.obs_column_scroll),
            )
        };
        // Overview, at the top, leaves only the rail's bottom cut.
        assert_eq!(edges(&pilot, cx).0, (false, true));
        pilot.update(cx, |pilot, cx| {
            pilot.navigate(Page::Observability, window, cx)
        });
        window.render_frame(cx);
        window.render_frame(cx);
        // The column has scrolled to Applications, so it is cut at the
        // bottom; the rail shows Observability clear of its fades.
        let column = edges(&pilot, cx).1;
        assert!(column.1, "{column:?}");
        let rail = pilot.read(cx).navigation.rail_scroll.clone();
        assert_clear_of_fades(window, "nav-rail", "nav-observability", &rail);
        for _ in 0..30 {
            window.scroll(
                "nav-rail",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-40.))),
                cx,
            );
            window.render_frame(cx);
        }
        assert_eq!(edges(&pilot, cx).0, (true, false));
        window.scroll(
            "nav-rail",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(40.))),
            cx,
        );
        window.render_frame(cx);
        assert_eq!(edges(&pilot, cx).0, (true, true));
    })
    .unwrap();

    // With room for everything, nothing is cut.
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.navigate(Page::Observability, window, cx)
        });
        window.render_frame(cx);
        window.render_frame(cx);
        let pilot = pilot.read(cx);
        assert_eq!(cut_edges(&pilot.navigation.rail_scroll), (false, false));
        assert_eq!(
            cut_edges(pilot.navigation.column_list.scroll()),
            (false, false)
        );
        assert_eq!(
            cut_edges(&pilot.navigation.obs_column_scroll),
            (false, false)
        );
    })
    .unwrap();
}

/// Asserts that `item` shows inside the scrolling `list`, and that no fade
/// over a cut edge of it dims the item (#406).
fn assert_clear_of_fades(
    window: &mut gpui_kit::Window,
    list: &'static str,
    item: impl Into<SharedString>,
    scroll: &gpui_kit::ScrollHandle,
) {
    let item = item.into();
    let area = window.find(list).bounds();
    let bounds = window.find(item.clone()).bounds();
    let fade = crate::ui::dp_px(28., window);
    let (top, bottom) = super::super::shell::cut_edges(scroll);
    assert!(
        bounds.top() >= area.top() + if top { fade } else { px(0.) } - px(0.5)
            && bounds.bottom() <= area.bottom() - if bottom { fade } else { px(0.) } + px(0.5),
        "{item}: {bounds:?} under a fade or outside {area:?} (cut {top}, {bottom})"
    );
}

/// A reveal measures the layout it scrolls in, not the one before it: a
/// change of room or of the column's state between two reveals still
/// leaves the item in view and clear of the fades (#406).
#[gpui_kit::test]
fn a_reveal_follows_a_change_of_room_or_column(cx: &mut TestAppContext) {
    use crate::observability::Destination;
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    let settle = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        for _ in 0..3 {
            window.render_frame(cx);
        }
    };
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.navigate(Page::Observability, window, cx)
        });
        let page = pilot.read(cx).pages.observability.clone();
        page.update(cx, |page, cx| page.open(Destination::Deployments, cx));
        settle(window, cx);
        let scroll = pilot.read(cx).navigation.obs_column_scroll.clone();
        assert_clear_of_fades(
            window,
            "obs-navigation-scroll",
            "nav-obs-deployments",
            &scroll,
        );
        // Larger text cuts the column the item was revealed in.
        crate::text_size::set(20., cx);
        settle(window, cx);
        assert!(pilot.read(cx).navigation.column_collapsed(window));
        assert_ne!(super::super::shell::cut_edges(&scroll), (false, false));
        assert_clear_of_fades(
            window,
            "obs-navigation-scroll",
            "nav-obs-deployments",
            &scroll,
        );
        // Expanded, its rows are taller, in the source list.
        window.click("nav-collapse", cx);
        settle(window, cx);
        assert!(!pilot.read(cx).navigation.column_collapsed(window));
        let column = pilot.read(cx).navigation.column_list.scroll().clone();
        assert_clear_of_fades(window, "nav-column-list", "nav-obs-deployments", &column);
        // From a short column, a kind low in a long group; the column's
        // scroll is shared by every area.
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Etcd, window, cx));
        settle(window, cx);
        let key = "validatingadmissionpolicybindings.admissionregistration.k8s.io";
        pilot.update(cx, |pilot, cx| pilot.open_builtin(key, window, cx));
        settle(window, cx);
        assert_clear_of_fades(window, "nav-column-list", format!("nav-k8s-{key}"), &column);
        // And the rail shows Control plane, low in it, as Command-9 opens.
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Lifecycle, window, cx));
        settle(window, cx);
        let rail = pilot.read(cx).navigation.rail_scroll.clone();
        assert_clear_of_fades(window, "nav-rail", "nav-control-plane", &rail);
    })
    .unwrap();
}

/// A wheel over the rail redraws the rail, so its fades follow the scroll,
/// and none of the other cached parts.
#[gpui_kit::test]
fn a_rail_wheel_redraws_the_rail_alone(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
    })
    .unwrap();
    for _ in 0..5 {
        cx.update_window(handle, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
    // Not `window.scroll`, which draws whole frames around the wheel and
    // so passes every cache.
    let rail = cx
        .update_window(handle, |_, window, _| window.find("nav-rail").bounds())
        .unwrap();
    let before = chrome_counts();
    cx.update_window(handle, |_, window, cx| {
        use gpui_kit::InputEvent as _;
        window.dispatch_event(
            gpui_kit::ScrollWheelEvent {
                position: rail.center(),
                delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-40.))),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.simulate_next_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let after = chrome_counts();
    let scrolled =
        cx.read(|cx| super::super::shell::cut_edges(&pilot.read(cx).navigation.rail_scroll).0);
    assert!(scrolled, "the wheel scrolled the rail");
    assert!(
        after[1] > before[1],
        "the rail drew again: {before:?} → {after:?}"
    );
    assert_eq!(after[0], before[0], "the header drew again");
    assert_eq!(after[2], before[2], "the column drew again");
}

#[gpui_kit::test]
fn fog_narrow_sidebar_can_be_expanded_manually(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.navigate(Page::Observability, window, cx)
        });
        window.render_frame(cx);
        assert!(pilot.read(cx).navigation.column_collapsed(window));
        window.click("nav-collapse", cx);
        assert!(!pilot.read(cx).navigation.column_collapsed(window));
        window.click("nav-collapse", cx);
        assert!(pilot.read(cx).navigation.column_collapsed(window));
    })
    .unwrap();
}
