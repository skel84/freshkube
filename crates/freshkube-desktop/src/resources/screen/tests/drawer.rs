use super::*;

#[gpui_kit::test]
fn a_clicked_row_shows_its_details_in_the_drawer_over_the_list(cx: &mut TestAppContext) {
    // Here the page is the whole window: 1280 has room for the drawer
    // beside 280 of list, and the drawer takes a page under 600 whole.
    for (width, full) in [(1280., false), (560., true)] {
        let (_runtime, screen, handle) = mount_sized(cx, Some("homelab"), width);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("resource-drawer").is_none());
            let third = identity_at(&screen, 2, cx);
            window.within(row_id(&third)).click("name", cx);
            window.render_frame(cx);
            assert_eq!(shown(&screen, cx), Some(third.clone()));
            assert_eq!(
                window.find("detail-title").label(),
                Some(third.address().as_str())
            );
            // The list keeps its whole width under the drawer.
            let body = window.find("resource-split").bounds();
            let list = window.find("resource-list-area").bounds();
            assert_eq!(list.size.width, body.size.width);
            let drawn = crate::desktop::layout_check::assert_drawer(
                window,
                cx,
                "resource-split",
                "resource-drawer",
                "detail-inspector",
                "detail-title",
            );
            let page = body.size.width / crate::ui::dp_px(1., window);
            let fit = freshkube_ui::drawer::fit(freshkube_ui::drawer::WIDTH, page);
            assert_eq!(fit.full, full, "{width}: a page {page} wide");
            assert!(
                (drawn - fit.width).abs() <= 1.,
                "{width}: {drawn} for {fit:?}"
            );

            // With no filter to clear, Escape closes the drawer and drops
            // the selection it showed.
            window.press("escape", cx);
            window.render_frame(cx);
            assert!(window.try_find("resource-drawer").is_none());
            assert_eq!(shown(&screen, cx), None);
            assert_eq!(selected(&screen, cx), None);
        })
        .unwrap();
    }
}

/// A click on the list that isn't on a row closes the drawer; one on
/// another row swaps its object in the same drawer, without closing it.
#[gpui_kit::test]
fn a_click_beside_the_rows_closes_the_drawer_and_a_row_swaps_it(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount_window(cx, Some("homelab"), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first(&screen, window, cx);
        let pane = screen.read(cx).detail.clone();
        let second = identity_at(&screen, 1, cx);
        let closes = Rc::new(RefCell::new(0));
        let _closed = {
            let closes = closes.clone();
            cx.observe(&pane, move |pane, cx| {
                if pane.read(cx).target_identity().is_none() {
                    *closes.borrow_mut() += 1;
                }
            })
        };
        window.within(row_id(&second)).click("name", cx);
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), Some(second.clone()));
        assert_eq!(*closes.borrow(), 0, "the swap closed the drawer");
        assert!(window.try_find("resource-drawer").is_some());

        // A click inside the drawer leaves it open.
        window.click("detail-title", cx);
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), Some(second.clone()));

        // Under the last row, the list is empty space: a click there
        // closes the drawer, keeping the row selected for Enter.
        let list = window.find("resource-list-area").bounds();
        window.click_at(
            "resource-list-area",
            gpui_kit::point(px(40.), list.size.height - px(10.)),
            cx,
        );
        window.render_frame(cx);
        assert!(window.try_find("resource-drawer").is_none());
        assert_eq!(shown(&screen, cx), None);
        assert_eq!(selected(&screen, cx), Some(second.clone()));
        window.press("enter", cx);
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), Some(second));
    })
    .unwrap();
}

/// A control in the list, such as a group's Expand or the footer's Show
/// all, is no click beside the rows: the drawer stays open on its object.
#[gpui_kit::test]
fn the_lists_own_buttons_leave_the_drawer_open(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount_window(cx, Some("homelab"), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        problems(&screen, cx);
        open_first(&screen, window, cx);
        let first = shown(&screen, cx).expect("a row opened");
        let drawer = window.find("resource-drawer").bounds();
        for button in [
            "resource-group-healthy-toggle",
            "resource-group-healthy-toggle",
            "resource-show-all",
        ] {
            assert!(
                window.find(button).bounds().right() < drawer.left(),
                "{button}"
            );
            window.click(button, cx);
            window.render_frame(cx);
            assert_eq!(shown(&screen, cx), Some(first.clone()), "{button}");
        }
        // Show all did what it does.
        assert_eq!(screen.read(cx).list_view, ListView::All);
        // A column header sorts, and leaves the drawer open too.
        window.click(("resource-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), Some(first));
    })
    .unwrap();
}

/// Closed, the drawer keeps its row selected: the arrows move the
/// selection without opening it, Escape clears it, and only Enter or a
/// click opens it again.
#[gpui_kit::test]
fn a_closed_drawer_stays_closed_while_the_selection_moves(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount_window(cx, Some("homelab"), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first(&screen, window, cx);
        // A click under the rows closes the drawer and keeps the row.
        let list = window.find("resource-list-area").bounds();
        window.click_at(
            "resource-list-area",
            gpui_kit::point(px(40.), list.size.height - px(10.)),
            cx,
        );
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), None);
        assert_eq!(selected(&screen, cx), Some(identity_at(&screen, 0, cx)));
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(selected(&screen, cx), Some(identity_at(&screen, 1, cx)));
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(selected(&screen, cx), Some(identity_at(&screen, 2, cx)));
    })
    .unwrap();
    // The keyboard pause passes; nothing opens.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), None);
        assert!(window.try_find("resource-drawer").is_none());
        window.press("escape", cx);
        window.render_frame(cx);
        assert_eq!(selected(&screen, cx), None);
        window.press("down", cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), Some(identity_at(&screen, 0, cx)));
    })
    .unwrap();
}

/// The drawer's bounds come from the list's width as last drawn. When the
/// window narrows, the list measures itself while drawing and asks for one
/// more frame, which fits the drawer to it without any input, and then
/// asks for no more.
#[gpui_kit::test]
fn the_measured_width_settles_without_input(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount_window(cx, Some("homelab"), 1280., 880.);
    let left = |window: &mut gpui_kit::Window| {
        let body = window.find("resource-split").bounds();
        let drawer = window.find("resource-drawer").bounds();
        (drawer.left() - body.left()) / crate::ui::dp_px(1., window)
    };
    cx.update_window(handle, |_, window, cx| {
        open_first(&screen, window, cx);
        while window.simulate_next_frame(cx) > 0 {
            window.render_frame(cx);
        }
    })
    .unwrap();
    cx.simulate_window_resize(handle, size(px(800.), px(880.)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The list measured its new width and asked for a frame.
        assert_eq!(window.simulate_next_frame(cx), 1, "no frame asked for");
        window.render_frame(cx);
        assert!(
            left(window) >= freshkube_ui::drawer::LIST_KEEPS - 1.,
            "{}",
            left(window)
        );
        // Settled: the next draw asks for nothing.
        assert_eq!(window.simulate_next_frame(cx), 0);
    })
    .unwrap();
}

/// The page's toolbar is above the drawer, not under it: typing a filter
/// keeps the drawer open.
#[gpui_kit::test]
fn the_drawer_starts_under_the_page_toolbar(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount_window(cx, Some("homelab"), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first(&screen, window, cx);
        let drawer = window.find("resource-drawer").bounds();
        let filter = window.find("resource-filter").bounds();
        assert!(drawer.top() >= filter.bottom(), "{filter:?} {drawer:?}");
        window.click("resource-filter", cx);
        window.render_frame(cx);
        assert!(window.try_find("resource-drawer").is_some());
    })
    .unwrap();
}

fn open_first(
    screen: &Entity<ResourcesScreen>,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
) {
    window.render_frame(cx);
    let first = identity_at(screen, 0, cx);
    window.within(row_id(&first)).click("name", cx);
    window.render_frame(cx);
}

/// The drawer's width is the page's, for every kind: a drag of its left
/// edge sets it within its bounds, and it comes back when the app opens
/// again.
#[gpui_kit::test]
fn the_drawer_width_follows_its_edge_and_survives_reopening(cx: &mut TestAppContext) {
    use crate::navigation_file::NavigationFile;
    use freshkube_ui::split_size::SizeStore as _;
    let directory = std::env::temp_dir().join(format!(
        "freshkube-resources-width-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let preferences = directory.join("preferences.json");
    cx.update(|cx| NavigationFile::open(Some(&preferences)).install(cx));
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        open_first(&screen, window, cx);
        let drawer = window.find("resource-drawer").bounds();
        // Drag the left edge so the drawer is 560 dp wide.
        let from = gpui_kit::point(drawer.left() + px(2.), drawer.center().y);
        let to = drawer.right() - crate::ui::dp_px(560., window);
        window.drag(from, gpui_kit::point(to, from.y), cx);
        window.render_frame(cx);
        let width = window.find("resource-drawer").bounds().size.width;
        let expected = crate::ui::dp_px(560., window);
        assert!(
            (width - expected).abs() <= px(2.),
            "{width:?}, {expected:?}"
        );
        // Dragged past its bounds, it stops at them.
        screen.update(cx, |screen, cx| screen.resize_drawer(50., window, cx));
        assert_eq!(
            screen.read(cx).drawer.size(),
            freshkube_ui::drawer::MIN_WIDTH
        );
        screen.update(cx, |screen, cx| screen.resize_drawer(560., window, cx));
    })
    .unwrap();
    cx.executor()
        .advance_clock(freshkube_ui::split_size::SAVE_DELAY * 2);
    cx.run_until_parked();
    let reopened = NavigationFile::open(Some(&preferences));
    assert_eq!(
        reopened.size(crate::navigation_file::DRAWER_WIDTH),
        Some(560.)
    );
    assert_eq!(
        reopened.size(freshkube_ui::inspector::width_key("resources")),
        None
    );

    cx.update(|cx| reopened.install(cx));
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        open_first(&screen, window, cx);
        let width = window.find("resource-drawer").bounds().size.width;
        let expected = crate::ui::dp_px(560., window);
        assert!(
            (width - expected).abs() <= px(1.),
            "{width:?}, {expected:?}"
        );
    })
    .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

/// A window too narrow for the width the user gave shows the drawer
/// narrower and writes nothing: a wider window gets the width back.
#[gpui_kit::test]
fn a_narrow_window_bounds_the_drawer_without_saving(cx: &mut TestAppContext) {
    use freshkube_ui::split_size::{MemorySizes, SizeStore as _, set_store};
    let sizes = std::rc::Rc::new(MemorySizes::with(
        crate::navigation_file::DRAWER_WIDTH,
        900.,
    ));
    cx.update(|cx| set_store(sizes.clone(), cx));
    let (_runtime, screen, handle) = mount_window(cx, Some("homelab"), 900., 700.);
    cx.update_window(handle, |_, window, cx| {
        open_first(&screen, window, cx);
        let width = window.find("resource-drawer").bounds().size.width;
        assert!(width < crate::ui::dp_px(900., window), "{width:?}");
    })
    .unwrap();
    cx.executor()
        .advance_clock(freshkube_ui::split_size::SAVE_DELAY * 2);
    cx.run_until_parked();
    assert_eq!(sizes.saves(), 0);
    assert_eq!(sizes.size(crate::navigation_file::DRAWER_WIDTH), Some(900.));
    cx.update(|cx| assert_eq!(screen.read(cx).drawer.size(), 900.));
}

#[gpui_kit::test]
fn arrow_keys_show_the_next_row_once_the_keyboard_pauses(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let first = identity_at(&screen, 0, cx);
        window.within(row_id(&first)).click("name", cx);
        window.render_frame(cx);
        assert!(window.try_find("detail-state").is_none());

        window.press("down", cx);
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(shown(&screen, cx), Some(identity_at(&screen, 2, cx)));
        assert_eq!(window.find("detail-state").label(), Some("Reading"));
        assert!(window.find("resource-drawer").visible());
    })
    .unwrap();
    cx.executor().advance_clock(KEYBOARD_PAUSE);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("detail-state").is_none());
        assert!(window.find("detail-overview").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_deleted_row_marks_its_details_and_offers_the_new_object(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let successor = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let first = identity_at(&screen, 0, cx);
            let row = screen.read(cx).store.get(&first).cloned().unwrap();
            window.within(row_id(&first)).click("name", cx);
            deliver(&screen, vec![ResourceEvent::Delete(first.clone())], cx);
            window.render_frame(cx);
            assert!(window.find("detail-deleted").visible());
            assert_eq!(window.find("detail-state").label(), Some("Deleted"));
            assert!(window.try_find("detail-open-recreated").is_none());

            // The same name again, with a new UID: another object.
            let successor = ResourceIdentity {
                uid: "recreated".into(),
                ..first.clone()
            };
            let recreated = ResourceRow {
                identity: successor.clone(),
                ..row
            };
            deliver(&screen, vec![ResourceEvent::Upsert(recreated)], cx);
            window.render_frame(cx);
            assert_eq!(shown(&screen, cx), Some(first));
            window.click("detail-open-recreated", cx);
            successor
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        assert_eq!(selected(&screen, cx), Some(successor.clone()));
        assert_eq!(shown(&screen, cx), Some(successor));
    })
    .unwrap();
}

#[gpui_kit::test]
fn another_kind_namespace_or_connection_closes_the_details(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        let open_first = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            window.render_frame(cx);
            window
                .within(row_id(&identity_at(&screen, 0, cx)))
                .click("name", cx);
            window.render_frame(cx);
            assert!(window.find("resource-detail").visible());
        };
        open_first(window, cx);
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("deployments.apps"), window, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("resource-detail").is_none());
        assert_eq!(shown(&screen, cx), None);

        open_first(window, cx);
        screen.update(cx, |screen, cx| {
            screen.set_namespace(Some("payments".into()), window, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("resource-detail").is_none());

        // Another snapshot of the same access session keeps it open.
        open_first(window, cx);
        screen.update(cx, |screen, cx| {
            screen.set_source(Some(source("homelab")), window, cx)
        });
        window.render_frame(cx);
        assert!(window.find("resource-detail").visible());
        screen.update(cx, |screen, cx| {
            screen.set_source(Some(source("staging-eu")), window, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("resource-detail").is_none());
        assert_eq!(shown(&screen, cx), None);
    })
    .unwrap();
}
