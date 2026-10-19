use super::*;

/// A row's tooltip, such as its name's, doesn't draw over the menu the
/// row opened, and shows again once the menu is gone.
#[gpui_kit::test]
fn no_row_tooltip_draws_while_its_menu_is_open(cx: &mut TestAppContext) {
    use freshkube_ui::tooltip::drawn_total;
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let row = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let row = row_id(&identity_at(&screen, 0, cx));
            window.within(row.clone()).hover("name", cx);
            row
        })
        .unwrap();
    let shown = |cx: &mut TestAppContext| {
        // Past the tooltip's show delay.
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(600));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let before = drawn_total();
            window.render_frame(cx);
            drawn_total() > before
        })
        .unwrap()
    };
    assert!(shown(cx), "the name's tooltip never showed");
    let items = open_menu(cx, handle, |window, cx| {
        window.within(row.clone()).right_click("name", cx)
    });
    assert!(!items.is_empty());
    // The pointer rests on the name, left of the menu that opened at its
    // middle.
    cx.update_window(handle, |_, window, cx| {
        use gpui_kit::{InputEvent, MouseMoveEvent, point, px};
        let name = window.within(row.clone()).find("name").bounds();
        let position = point(name.left() + px(4.), name.center().y);
        window.dispatch_event(
            MouseMoveEvent {
                position,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
    assert!(!shown(cx), "a tooltip drew over the open menu");
    // A click outside closes the menu; the pointer comes back to the name.
    cx.update_window(handle, |_, window, cx| {
        window.click("resource-filter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none(), "the menu stayed");
        window.within(row).hover("name", cx);
    })
    .unwrap();
    assert!(shown(cx), "the tooltip stayed hidden after the menu closed");
}

#[gpui_kit::test]
fn a_right_click_selects_its_row_before_the_menu_acts(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let (first, third) = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let first = identity_at(&screen, 0, cx);
            window.within(row_id(&first)).click("name", cx);
            (first, identity_at(&screen, 2, cx))
        })
        .unwrap();
    assert_eq!(cx.read(|cx| selected(&screen, cx)), Some(first.clone()));
    // Its name, clear of the drawer the first click opened.
    let row = row_id(&third);
    let items = open_menu(cx, handle, |window, cx| {
        window.within(row).right_click("name", cx)
    });
    // The menu is the row's own: it is selected as the menu opens.
    assert_eq!(cx.read(|cx| selected(&screen, cx)), Some(third.clone()));
    assert_eq!(
        items.first().cloned().flatten().as_deref(),
        Some("Open"),
        "{items:?}"
    );
    assert!(items.contains(&Some("Logs".into())), "{items:?}");
    cx.update_window(handle, |_, window, cx| {
        let ix = items
            .iter()
            .position(|item| item.as_deref() == Some("Mark"));
        window.within("popup-menu").click(ix.unwrap(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        let marked = &screen.read(cx).marked;
        assert!(marked.contains(&third) && !marked.contains(&first));
    });
}

/// Moves the selection to the first row, as a list change under an open
/// menu could.
fn select_first(screen: &Entity<ResourcesScreen>, cx: &mut gpui_kit::App) {
    screen.update(cx, |screen, cx| {
        screen.projection.select(&screen.store, Some(0));
        cx.notify();
    });
}

#[gpui_kit::test]
fn a_menu_acts_only_on_the_row_it_was_opened_for(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let third = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            identity_at(&screen, 2, cx)
        })
        .unwrap();
    let row = row_id(&third);
    let items = open_menu(cx, handle, |window, cx| {
        window.within(row.clone()).right_click("name", cx)
    });
    // Another row is selected while the menu is open: Mark marks neither.
    cx.update_window(handle, |_, window, cx| {
        select_first(&screen, cx);
        window.render_frame(cx);
        pick(window, &items, "Mark", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert!(screen.read(cx).marked.is_empty()));

    // A row's menu goes with its row.
    open_menu(cx, handle, |window, cx| {
        window.within(row).right_click("name", cx)
    });
    cx.update_window(handle, |_, window, cx| {
        deliver(&screen, vec![ResourceEvent::Delete(third.clone())], cx);
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
    })
    .unwrap();
    cx.run_until_parked();

    // A group's menu outlives the row it selected, and acts only on it.
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.not_ready = [("talos-home".to_owned(), None)].into();
            screen.set_list_view(ListView::Problems, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    let group = "resource-group-node:talos-home-line";
    let items = open_menu(cx, handle, |window, cx| window.right_click(group, cx));
    let first = cx.read(|cx| selected(&screen, cx)).unwrap();
    cx.update_window(handle, |_, window, cx| {
        select_first(&screen, cx);
        assert_ne!(selected(&screen, cx), Some(first.clone()));
        window.render_frame(cx);
        pick(window, &items, "Mark", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert!(screen.read(cx).marked.is_empty()));

    // With its row still selected, the same pick marks it, so the checks
    // above can't pass on a missed click.
    let first = cx.read(|cx| selected(&screen, cx)).unwrap();
    let row = row_id(&first);
    let items = open_menu(cx, handle, |window, cx| {
        window.within(row).right_click("name", cx)
    });
    cx.update_window(handle, |_, window, cx| pick(window, &items, "Mark", cx))
        .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert!(screen.read(cx).marked.contains(&first)));
}
