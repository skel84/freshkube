use super::*;

#[gpui_kit::test]
fn header_controls_share_the_title_row_at_both_widths(cx: &mut TestAppContext) {
    for width in [1280., 760.] {
        let (_runtime, _screen, handle) = mount_sized(cx, Some("homelab"), width);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let title = window.find("resource-title").bounds();
            let namespace = window.find("resource-namespace").bounds();
            let filter = window.find("resource-filter").bounds();
            let refresh = window.find("resource-refresh").bounds();
            for control in [namespace, filter, refresh] {
                assert!(control.left() >= px(0.), "{width}: {control:?}");
                assert!(control.right() <= px(width), "{width}: {control:?}");
            }
            assert!(title.right() <= filter.left());
            assert!(namespace.right() <= refresh.left());
            // Centred on one line, to layout's rounding.
            let level = |a: Bounds<Pixels>, b: Bounds<Pixels>| {
                (a.center().y - b.center().y).abs() < px(0.5)
            };
            assert!(level(namespace, refresh), "{namespace:?} {refresh:?}");
            assert!(filter.size.width >= px(120.), "{width}: {filter:?}");
            assert!(level(namespace, filter), "{namespace:?} {filter:?}");
            assert!(filter.right() <= namespace.left());
            assert!(window.find("resource-list").bounds().top() > refresh.bottom());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn pods_list_every_namespace_and_select_by_identity(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(screen.read(cx).store.len(), 22);
        assert!(window.find("resource-list").visible());
        // The glyph first, which doesn't sort; then the name, after its
        // namespace when listing every one, and its Logs button, which
        // doesn't sort either; then the containers, readiness, use,
        // restarts, owner, the node and age. IP stays left out.
        assert!(window.try_find(("resource-sort", 0usize)).is_none());
        assert!(window.try_find(("resource-sort", 2usize)).is_none());
        let labels: Vec<String> = (1..11usize)
            .filter(|ix| *ix != 2)
            .map(|ix| {
                window
                    .find(("resource-sort", ix))
                    .label()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(
            labels,
            [
                "Name",
                "Containers",
                "Ready",
                "CPU",
                "Memory",
                "Restarts",
                "Owner",
                "Node",
                "Age"
            ]
        );
        assert!(window.try_find(("resource-sort", 11usize)).is_none());

        let third = identity_at(&screen, 2, cx);
        window.within(row_id(&third)).click("name", cx);
        window.render_frame(cx);
        assert_eq!(selected(&screen, cx), Some(third.clone()));
        assert_eq!(window.find(row_id(&third)).selected(), Some(true));
        // A click focuses the list, so arrows move on from the row.
        window.press("down", cx);
        window.render_frame(cx);
        let fourth = identity_at(&screen, 3, cx);
        assert_eq!(selected(&screen, cx), Some(fourth.clone()));

        // Sorting moves rows, not the selection.
        window.click(("resource-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("resource-sort", 1usize)).label(),
            Some("Name, sorted ascending")
        );
        assert_eq!(selected(&screen, cx), Some(fourth.clone()));
        assert_eq!(window.find(row_id(&fourth)).selected(), Some(true));
        assert_ne!(identity_at(&screen, 3, cx), fourth);
        window.click(("resource-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("resource-sort", 1usize)).label(),
            Some("Name, sorted descending")
        );

        // Refresh runs ⌘R's action, which the shell handles by listing
        // again; mounted alone, the screen lists again itself.
        let refreshed = Rc::new(Cell::new(0));
        let count = refreshed.clone();
        cx.on_action(move |_: &freshkube_ui::page::Refresh, _| count.set(count.get() + 1));
        window.click("resource-refresh", cx);
        assert_eq!(refreshed.get(), 1);
        // Listing again finds the same object and selects it again.
        screen.update(cx, |screen, cx| screen.refresh(window, cx));
        window.render_frame(cx);
        assert_eq!(selected(&screen, cx), Some(fourth.clone()));
        assert_eq!(window.find(row_id(&fourth)).selected(), Some(true));

        window.press("end", cx);
        window.render_frame(cx);
        assert_eq!(selected(&screen, cx), Some(identity_at(&screen, 21, cx)));
        window.press("home", cx);
        window.render_frame(cx);
        assert_eq!(selected(&screen, cx), Some(identity_at(&screen, 0, cx)));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_namespace_narrows_namespaced_kinds_and_persists_across_kinds(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("resource-namespace").click("input", cx);
        window.render_frame(cx);
        window.input("payments", cx);
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let view = screen.read(cx);
        assert_eq!(view.namespace.as_deref(), Some("payments"));
        assert_eq!(view.store.len(), 6);
        assert!(
            view.store
                .entries()
                .iter()
                .all(|entry| entry.row().identity.namespace == "payments")
        );
        // One namespace isn't repeated before every name.
        assert!(!view.layout.namespaced);
        assert_eq!(
            window.find(("resource-sort", 3usize)).label(),
            Some("Containers")
        );

        // Cluster-scoped kinds have no namespace picker and list all.
        screen.update(cx, |screen, cx| screen.set_kind(kind("nodes"), window, cx));
        window.render_frame(cx);
        assert!(window.try_find("resource-namespace").is_none());
        assert!(!screen.read(cx).store.is_empty());
        assert!(window.find("resource-list").visible());

        // Back on a namespaced kind, the namespace still applies.
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("deployments.apps"), window, cx)
        });
        window.render_frame(cx);
        assert!(window.find("resource-namespace").visible());
        assert_eq!(screen.read(cx).store.len(), 3);
        // A glyph from the replicas ready, then the name without its
        // namespace.
        assert!(!screen.read(cx).layout.namespaced);
        assert_eq!(window.find(("resource-sort", 1usize)).label(), Some("Name"));
        assert_eq!(
            window.find(("resource-sort", 2usize)).label(),
            Some("Ready")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_filter_narrows_rows_and_escape_clears_it(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    // Typing reaches the screen through the input's change event, which
    // is delivered once the update ends.
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    step(cx, &|window, cx| {
        screen.update(cx, |screen, cx| screen.focus(window, cx));
    });
    step(cx, &|window, cx| window.press("/", cx));
    step(cx, &|window, cx| window.input("coredns", cx));
    step(cx, &|window, cx| {
        assert_eq!(screen.read(cx).projection.len(), 2);
        assert_eq!(screen.read(cx).store.len(), 22);
        window.input("-nothing-", cx);
    });
    step(cx, &|window, cx| {
        assert!(window.find("resource-empty").visible());
        // Enter hands the keyboard back to the list; Escape clears.
        window.press("enter", cx);
    });
    step(cx, &|window, cx| window.press("escape", cx));
    step(cx, &|window, cx| {
        assert_eq!(screen.read(cx).projection.len(), 22);
        assert!(window.try_find("resource-empty").is_none());
    });
}

#[gpui_kit::test]
fn object_links_keep_matching_namespace_and_filter_but_reveal_hidden_objects(
    cx: &mut TestAppContext,
) {
    let (_runtime, screen, handle) = mount(cx, Some("prod-fra"));
    cx.update_window(handle, |_, window, cx| {
        let identity = screen
            .read(cx)
            .identity_named("payments", "worker-5d7c9-7rr9b")
            .unwrap_or_else(|| {
                screen
                    .read(cx)
                    .store
                    .entries()
                    .iter()
                    .find(|entry| entry.row().identity.namespace == "payments")
                    .unwrap()
                    .row()
                    .identity
                    .clone()
            });
        screen.update(cx, |screen, cx| {
            screen.set_namespace(Some("payments".into()), window, cx);
            screen.set_filter(&identity.name, window, cx);
            screen.open_identity_on(identity.clone(), crate::Tab::Overview, window, cx);
        });
        assert_eq!(screen.read(cx).namespace.as_deref(), Some("payments"));
        assert_eq!(screen.read(cx).query.read(cx).value(), identity.name);
        assert_eq!(shown(&screen, cx), Some(identity.clone()));
        screen.update(cx, |screen, cx| {
            screen.set_namespace(Some("batch".into()), window, cx);
            screen.set_filter("does-not-match", window, cx);
            screen.open_identity_on(identity.clone(), crate::Tab::Logs, window, cx);
        });
        assert!(screen.read(cx).namespace.is_none());
        assert!(screen.read(cx).query.read(cx).value().is_empty());
        assert_eq!(shown(&screen, cx), Some(identity));
        // Logs open in the dock; the pane stays on its tab.
        assert_eq!(screen.read(cx).detail.read(cx).tab(), crate::Tab::Overview);
        let missing = ResourceIdentity {
            name: "absent-from-list".into(),
            uid: "missing-uid".into(),
            ..identity_at(&screen, 0, cx)
        };
        screen.update(cx, |screen, cx| {
            screen.open_identity_on(missing.clone(), crate::Tab::Overview, window, cx)
        });
        assert_eq!(shown(&screen, cx), Some(missing));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_sideways_scroll_keeps_each_name_in_view_once(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount_sized(cx, Some("homelab"), 640.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let rows: Vec<_> = (0..3)
            .map(|ix| row_id(&identity_at(&screen, ix, cx)))
            .collect();
        let left = |window: &mut gpui_kit::Window, row: &gpui_kit::ElementId| {
            window.within(row.clone()).find("name").bounds().left()
        };
        let names: Vec<_> = rows.iter().map(|row| left(window, row)).collect();
        let first = identity_at(&screen, 0, cx);
        let logs = gpui_kit::SharedString::from(format!("pod-row-logs-{}", first.uid));
        let button = window.find(logs.clone()).bounds().left();
        let header = window.find(("resource-sort", 1usize)).bounds().left();
        // Column 3 starts at the pinned run's edge (glyph, name, Logs),
        // where its label stays once scrolled; column 5 starts clear of
        // the scroll's 120.
        let edge = window.find(("resource-sort", 3usize)).bounds().left();
        let later = window.find(("resource-sort", 5usize)).bounds().left();
        assert!(later - edge > px(120.), "{:?}", later - edge);
        window.scroll(
            "resource-table-scroll",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(-120.), px(0.))),
            cx,
        );
        window.render_frame(cx);
        let moved = later - window.find(("resource-sort", 5usize)).bounds().left();
        assert!((f32::from(moved) - 120.).abs() <= 1.5, "{moved:?}");
        // `find` fails on an id that resolves twice.
        assert!((window.find(("resource-sort", 1usize)).bounds().left() - header).abs() <= px(1.5));
        for (row, name) in rows.iter().zip(names) {
            assert!((left(window, row) - name).abs() <= px(1.5));
        }
        // A pod's Logs button is pinned with its name (#291).
        assert!((window.find(logs).bounds().left() - button).abs() <= px(1.5));
    })
    .unwrap();
}

#[gpui_kit::test]
fn fog_glyph_filters_and_column_choices_change_the_table(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("example"));
    let before = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            for control in ["resource-columns", "resource-refresh"] {
                assert!(window.find(control).visible(), "{control}");
                assert!(
                    window.find(control).bounds().bottom()
                        <= window.find("resource-list").bounds().top()
                );
            }
            window.click("resource-tally-healthy", cx);
            assert_eq!(
                screen.read(cx).projection.pod_filter(),
                Some(crate::projection::PodFilter::Healthy)
            );
            assert_eq!(
                screen.read(cx).projection.len(),
                screen.read(cx).projection.tally().healthy
            );
            window.click("resource-view-all", cx);
            assert!(screen.read(cx).projection.pod_filter().is_none());
            assert_eq!(
                screen.read(cx).projection.len(),
                screen.read(cx).store.len()
            );
            let before = screen.read(cx).layout.width;
            window.click("resource-columns", cx);
            window.render_frame(cx);
            // The first optional column is Containers; the Name and glyph
            // stay fixed.
            window.within("popup-menu").click(0usize, cx);
            assert!(
                screen
                    .read(cx)
                    .hidden_columns
                    .contains(&super::super::layout::ColumnSource::Containers)
            );
            assert!(screen.read(cx).layout.width < before);
            before
        })
        .unwrap();
    // Let the menu's deferred dismiss finish before opening it again.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.click("resource-columns", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
        assert_eq!(screen.read(cx).layout.width, before);
    })
    .unwrap();
}

#[gpui_kit::test]
fn namespaces_survive_a_kind_change_but_reject_a_source_round_trip(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            let request = screen.namespace_read.start(Some(source("homelab").id));
            screen.set_kind(kind("deployments.apps"), window, cx);
            assert!(screen.finish_namespaces(&request, Ok(vec!["current".into()]), window, cx));
            assert_eq!(screen.namespaces, ["current"]);
            screen.set_source(Some(source("prod-fra")), window, cx);
            screen.set_source(Some(source("homelab")), window, cx);
            let current = screen.namespaces.clone();
            assert!(!screen.finish_namespaces(&request, Ok(vec!["obsolete".into()]), window, cx));
            assert_eq!(screen.namespaces, current);
            let request = screen.namespace_read.start(Some(source("homelab").id));
            assert!(screen.finish_namespaces(&request, Ok(vec!["fresh".into()]), window, cx));
            assert_eq!(screen.namespaces, ["fresh"]);
        });
    })
    .unwrap();
}

/// Draws until the header stops asking for frames: it folds its controls
/// from what its parts measured on the frame before.
fn settle(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    for _ in 0..4 {
        cx.run_until_parked();
        let asked = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.simulate_next_frame(cx)
            })
            .unwrap();
        if asked == 0 {
            return;
        }
    }
    panic!("the header keeps moving");
}

#[gpui_kit::test]
fn the_folded_controls_do_what_the_controls_do(cx: &mut TestAppContext) {
    // 316 dp inside the toolbar's insets: the title, filter and "…" fit,
    // and nothing else.
    let (_runtime, screen, handle) = mount_sized(cx, Some("homelab"), 340.);
    settle(handle, cx);
    let payments = cx
        .update_window(handle, |_, window, cx| {
            assert!(
                window.try_find("resource-namespace").is_none(),
                "not folded"
            );
            assert!(window.try_find("resource-refresh").is_none());
            // Every folded control at its default: no dot.
            assert!(window.try_find("resource-more-dot").is_none());
            assert_eq!(window.find("resource-more").label(), Some("More"));
            let names = &screen.read(cx).namespaces;
            // All namespaces comes first.
            names.iter().position(|name| name == "payments").unwrap() + 1
        })
        .unwrap();
    // Refresh runs ⌘R's action, which the shell handles; mounted alone,
    // the screen has no shell, so the app catches it.
    let refreshed = Rc::new(Cell::new(0));
    let count = refreshed.clone();
    cx.update(|cx| {
        cx.on_action(move |_: &freshkube_ui::page::Refresh, _| count.set(count.get() + 1));
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("resource-more", cx);
        window.render_frame(cx);
        // Namespace, Columns, Refresh.
        window.within("popup-menu").click(2usize, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(refreshed.get(), 1);
    // A namespace from the menu narrows the list as the picker does.
    cx.update_window(handle, |_, window, cx| {
        window.click("resource-more", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
        window.render_frame(cx);
        window
            .within("submenu")
            .within("popup-menu")
            .click(payments, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let view = screen.read(cx);
        assert_eq!(view.namespace.as_deref(), Some("payments"));
        assert_eq!(view.store.len(), 6);
        assert_eq!(
            view.namespace_select.read(cx).selected_value(),
            Some(&Some("payments".to_owned()))
        );
        // The "…" says the folded picker narrows the list.
        window.find("resource-more-dot");
        assert_eq!(
            window.find("resource-more").label(),
            Some("More · Namespace payments")
        );
    })
    .unwrap();
    // A column from the menu hides as the Columns menu hides it.
    let hidden = screen.read_with(cx, |screen, _| screen.hidden_columns.len());
    cx.update_window(handle, |_, window, cx| {
        window.click("resource-more", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(1usize, cx);
        window.render_frame(cx);
        window
            .within("submenu")
            .within("popup-menu")
            .click(0usize, cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_ne!(
        screen.read_with(cx, |screen, _| screen.hidden_columns.len()),
        hidden
    );
    cx.update_window(handle, |_, window, _| {
        assert_eq!(
            window.find("resource-more").label(),
            Some("More · Namespace payments · 1 column hidden")
        );
    })
    .unwrap();
    // Both back to their defaults: All namespaces, every column.
    for (control, item) in [(0usize, 0usize), (1, 0)] {
        cx.update_window(handle, |_, window, cx| {
            window.click("resource-more", cx);
            window.render_frame(cx);
            window.within("popup-menu").click(control, cx);
            window.render_frame(cx);
            window
                .within("submenu")
                .within("popup-menu")
                .click(item, cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(screen.read(cx).namespace, None);
        assert!(window.try_find("resource-more-dot").is_none());
        assert_eq!(window.find("resource-more").label(), Some("More"));
    })
    .unwrap();
}
