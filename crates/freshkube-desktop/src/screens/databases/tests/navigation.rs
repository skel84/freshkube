use super::*;

#[gpui_kit::test]
fn inventory_views_keyboard_inspector_and_resource_link_use_object_identity(
    cx: &mut TestAppContext,
) {
    let (_runtime, page, window) = mount(cx, 1280.);
    let links = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = links.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(
            &page,
            move |_, event: &crate::resources::ResourceLink, _| {
                seen.borrow_mut().push(event.clone())
            },
        )
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let id = row_id(&page, "orders-db", cx);
        window.click(id, cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("databases-detail-title").label(),
            Some("orders-db")
        );
        window.press("o", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let links = links.borrow();
    let crate::resources::ResourceLink::Object(kind, object, _) = &links[0] else {
        panic!("object link")
    };
    assert_eq!(kind.group, "postgresql.cnpg.io");
    assert_eq!(object.uid, "invented-orders-db");
    assert_eq!(object.connection.as_deref(), Some("invented-access"));
    cx.update_window(window.into(), |_, window, cx| {
        window.click("databases-view-backups", cx);
        window.render_frame(cx);
        assert_eq!(page.read(cx).display.rows.len(), 4);
        window.press("down", cx);
        window.render_frame(cx);
        assert!(page.read(cx).selected.is_some());
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("databases-detail").is_none());
        window.click("databases-view-schedules", cx);
        window.render_frame(cx);
        assert_eq!(page.read(cx).display.rows.len(), 2);
    })
    .unwrap();
}

#[gpui_kit::test]
fn loading_and_not_connected_keep_the_page_key_context(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280.);
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| {
            page.loader.reset();
            page.loader.start(page.identity().unwrap());
            cx.notify();
        });
        window.render_frame(cx);
        window.find("databases-list");
        page.update(cx, |page, cx| {
            page.set_page_source(PageSource::default(), window, cx)
        });
        window.render_frame(cx);
        window.find("databases-title");
    })
    .unwrap();
}

#[gpui_kit::test]
fn shell_shortcut_opens_databases_in_the_storage_column(cx: &mut TestAppContext) {
    let (_runtime, window, _pilot) = crate::desktop::tests::fixture(cx, 1280., 880.);
    cx.update_window(window, |_, window, cx| {
        window.press("secondary-shift-d", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.find("databases-title");
        window.find("nav-databases");
    })
    .unwrap();
}

#[gpui_kit::test]
fn escape_passes_on_only_when_no_inspector_is_selected(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280.);
    let count = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen = count.clone();
    cx.update(|cx| {
        cx.on_action(move |_: &super::Close, _| seen.set(seen.get() + 1));
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let id = row_id(&page, "orders-db", cx);
        window.click(id, cx);
        page.update(cx, |page, cx| page.focus(window, cx));
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(count.get(), 0);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(count.get(), 1);
}

#[gpui_kit::test]
fn collapsed_storage_keeps_databases_reachable_and_selected(cx: &mut TestAppContext) {
    for (width, text) in [(760., 13.), (1280., 20.)] {
        let (_runtime, window, _pilot) = crate::desktop::tests::fixture(cx, width, 880.);
        cx.update_window(window, |_, window, cx| {
            crate::text_size::set(text, cx);
            window.press("secondary-shift-d", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let column = window.find("nav-column").bounds();
            assert!(column.size.width < px(90.), "Storage is collapsed");
            assert_eq!(window.find("nav-databases").checked(), Some(true));
            window.click("nav-k8s-persistentvolumeclaims", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_ne!(window.find("nav-databases").checked(), Some(true));
            window.click("nav-databases", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.find("databases-title");
            assert_eq!(window.find("nav-databases").checked(), Some(true));
        })
        .unwrap();
    }
}
