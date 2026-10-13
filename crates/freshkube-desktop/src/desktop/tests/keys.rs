use super::*;

#[gpui_kit::test]
fn named_keys_navigate_all_screens_and_configured_contexts(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for (key, page, kind) in [
            ("secondary-1", Page::Overview, None),
            ("secondary-2", Page::Nodes, None),
            ("secondary-3", Page::Resources, Some("namespaces")),
            ("secondary-4", Page::Resources, Some("events")),
            ("secondary-5", Page::Health, None),
            ("secondary-6", Page::Etcd, None),
            ("secondary-7", Page::SystemServices, None),
            ("secondary-8", Page::Security, None),
            ("secondary-9", Page::Lifecycle, None),
        ] {
            window.press(key, cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).navigation.page, page);
            if let Some(kind) = kind {
                assert_eq!(view.read(cx).navigation.resource_kind.key(), kind);
            }
        }
        window.press("secondary-1", cx);
        window.render_frame(cx);
        window.press("alt-down", cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("staging-eu"));
        window.press("alt-up", cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("prod-fra"));
        window.press("alt-up", cx);
        assert_eq!(
            view.read(cx).applied.context.as_deref(),
            Some("talos-production-frankfurt-equinix-fr5-baremetal-b7")
        );
        window.render_frame(cx);
        contexts(window, cx);
        assert!(window.find(("context", 0usize)).visible());
        assert_eq!(window.find(("context", 3usize)).selected(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn shell_keys_work_while_a_kind_shows_no_rows(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The example data has no config maps, so a message replaces the rows.
        let kind = crate::resources::example::kind("configmaps").unwrap();
        view.update(cx, |view, cx| view.open_kind(kind, window, cx));
        window.render_frame(cx);
        assert!(window.find("resource-not-in-example").visible());
        assert_eq!(window.find("resource-body").focused(), Some(true));
        window.press("secondary-2", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).navigation.page, Page::Nodes);
    })
    .unwrap();
}

#[gpui_kit::test]
fn another_context_hands_the_keyboard_back_to_the_list(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let kind = crate::resources::example::kind("pods").unwrap();
        view.update(cx, |view, cx| view.open_kind(kind, window, cx));
        window.render_frame(cx);
        window.press("alt-down", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("staging-eu"));
        assert_eq!(window.find("resource-body").focused(), Some(true));
        assert!(window.try_find("detail-close").is_none());
        window.press("down", cx);
    })
    .unwrap();
    // The arrow selected a row; with the drawer closed, only Enter opens
    // its details.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("detail-close").is_none());
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(window.find("detail-close").visible());
        // Clicking a context in the sidebar hands the keyboard back too.
        contexts(window, cx);
        window.click(("context", 2usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("homelab"));
        assert_eq!(window.find("resource-body").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_logs_page_puts_the_keyboard_on_its_lines(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        open_node_tab(window, cx, super::super::nodes::NodeTab::Logs);
        window.render_frame(cx);
        assert_eq!(view.read(cx).navigation.page, Page::Nodes);
        assert_eq!(window.find("logs-viewport").focused(), Some(true));
        // Command-F finds in the logs, and the shell's keys still work.
        window.press("secondary-f", cx);
        window.render_frame(cx);
        assert_eq!(window.find("logs-viewport").focused(), Some(false));
        window.press("secondary-1", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).navigation.page, Page::Overview);
    })
    .unwrap();
}

#[gpui_kit::test]
fn ctrl_tab_cycles_through_every_screen(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("secondary-9", cx);
        window.render_frame(cx);
        for expected in [
            Page::Operations,
            Page::Overview,
            Page::Nodes,
            Page::Resources,
            Page::Resources,
            Page::Health,
            Page::Resources,
            Page::Etcd,
            Page::SystemServices,
            Page::Security,
            Page::Lifecycle,
        ] {
            window.press("ctrl-tab", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).navigation.page, expected);
        }
        window.press("ctrl-shift-tab", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).navigation.page, Page::Security);
    })
    .unwrap();
}

#[gpui_kit::test]
fn command_k_goes_to_any_kind_from_any_page(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
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
        window.press("secondary-2", cx);
        window.press("secondary-k", cx);
    });
    step(cx, &|window, cx| {
        assert!(window.has_active_dialog(cx));
        // Kinds match by name or kubectl key.
        window.input("statefulsets", cx);
    });
    step(cx, &|window, cx| window.press("enter", cx));
    step(cx, &|window, cx| {
        assert!(!window.has_active_dialog(cx));
        assert_eq!(view.read(cx).navigation.page, Page::Resources);
        assert_eq!(
            view.read(cx).navigation.resource_kind.key(),
            "statefulsets.apps"
        );
        assert_eq!(window.find("page-title").label(), Some("StatefulSets"));
        assert_eq!(window.find("resource-body").focused(), Some(true));
        // Escape leaves the palette and hands the keyboard back.
        window.press("secondary-k", cx);
    });
    step(cx, &|window, cx| {
        assert!(window.has_active_dialog(cx));
        window.press("escape", cx);
    });
    step(cx, &|window, cx| {
        assert!(!window.has_active_dialog(cx));
        assert_eq!(window.find("resource-body").focused(), Some(true));
        assert_eq!(
            view.read(cx).navigation.resource_kind.key(),
            "statefulsets.apps"
        );
        // Custom kinds join once discovered.
        let kind = crate::resources::example::kind("certificates.cert-manager.io").unwrap();
        view.update(cx, |view, cx| view.open_kind(kind, window, cx));
    });
    step(cx, &|window, cx| {
        window.press("secondary-1", cx);
        window.press("secondary-k", cx);
    });
    step(cx, &|window, cx| window.input("certificates.cert", cx));
    step(cx, &|window, cx| window.press("enter", cx));
    step(cx, &|_, cx| {
        assert_eq!(view.read(cx).navigation.page, Page::Resources);
        assert_eq!(
            view.read(cx).navigation.resource_kind.key(),
            "certificates.cert-manager.io"
        );
    });
}
