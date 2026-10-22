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

/// The shell's keys on macOS, where `secondary` is Command.
#[cfg(target_os = "macos")]
const SHELL_KEYS: [&str; 27] = [
    "alt-cmd-i inspector::ToggleInspector ",
    "cmd-+ text_size::LargerText ",
    "cmd-= text_size::LargerText ",
    "cmd-- text_size::SmallerText ",
    "cmd-0 text_size::DefaultText ",
    "cmd-r pilot::Refresh Freshkube",
    "cmd-b pilot::ToggleColumn Freshkube",
    "cmd-1 pilot::ShowOverview Freshkube",
    "cmd-2 pilot::ShowNodes Freshkube",
    "cmd-3 pilot::ShowNamespaces Freshkube",
    "cmd-4 pilot::ShowEvents Freshkube",
    "cmd-shift-d pilot::ShowDatabases Freshkube",
    "cmd-5 pilot::ShowHealth Freshkube",
    "cmd-6 pilot::ShowEtcd Freshkube",
    "cmd-7 pilot::ShowSystemServices Freshkube",
    "cmd-8 pilot::ShowSecurity Freshkube",
    "cmd-9 pilot::ShowLifecycle Freshkube",
    // The Tab character, which GPUI parses as no key.
    "ctrl- pilot::NextScreen MenuBarOnly",
    "ctrl-shift- pilot::PreviousScreen MenuBarOnly",
    "ctrl-tab pilot::NextScreen Freshkube",
    "ctrl-shift-tab pilot::PreviousScreen Freshkube",
    "alt-up pilot::PreviousContext Freshkube",
    "alt-down pilot::NextContext Freshkube",
    "cmd-k pilot::GoToKind Freshkube",
    "ctrl-. dock::NextDockTab Freshkube",
    "ctrl-, dock::PreviousDockTab Freshkube",
    "shift-escape dock::MinimizeDock Freshkube",
];

/// The shell's keys on Linux and Windows, where `secondary` is Control
/// and Kit's inspector takes Control-Shift-I.
#[cfg(not(target_os = "macos"))]
const SHELL_KEYS: [&str; 27] = [
    "ctrl-shift-i inspector::ToggleInspector ",
    "ctrl-+ text_size::LargerText ",
    "ctrl-= text_size::LargerText ",
    "ctrl-- text_size::SmallerText ",
    "ctrl-0 text_size::DefaultText ",
    "ctrl-r pilot::Refresh Freshkube",
    "ctrl-b pilot::ToggleColumn Freshkube",
    "ctrl-1 pilot::ShowOverview Freshkube",
    "ctrl-2 pilot::ShowNodes Freshkube",
    "ctrl-3 pilot::ShowNamespaces Freshkube",
    "ctrl-4 pilot::ShowEvents Freshkube",
    "ctrl-shift-d pilot::ShowDatabases Freshkube",
    "ctrl-5 pilot::ShowHealth Freshkube",
    "ctrl-6 pilot::ShowEtcd Freshkube",
    "ctrl-7 pilot::ShowSystemServices Freshkube",
    "ctrl-8 pilot::ShowSecurity Freshkube",
    "ctrl-9 pilot::ShowLifecycle Freshkube",
    // The Tab character, which GPUI parses as no key.
    "ctrl- pilot::NextScreen MenuBarOnly",
    "ctrl-shift- pilot::PreviousScreen MenuBarOnly",
    "ctrl-tab pilot::NextScreen Freshkube",
    "ctrl-shift-tab pilot::PreviousScreen Freshkube",
    "alt-up pilot::PreviousContext Freshkube",
    "alt-down pilot::NextContext Freshkube",
    "ctrl-k pilot::GoToKind Freshkube",
    "ctrl-. dock::NextDockTab Freshkube",
    "ctrl-, dock::PreviousDockTab Freshkube",
    "shift-escape dock::MinimizeDock Freshkube",
];

/// The shell's keys and the actions its root answers, as they stood before
/// #580 moved each to its owner. The bindings are those in the shell's and
/// the menu bar's contexts and the context-free ones, in the order they
/// were added: a key's later binding wins, and macOS's bar shows an
/// action's first. The actions are those the shell answers with the
/// keyboard on it, by page and by the menu the window has.
#[gpui_kit::test]
fn the_shell_binds_the_keys_and_actions_it_did(cx: &mut TestAppContext) {
    use gpui_kit::AsKeystroke;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let bindings: Vec<String> = cx.update(|cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        keymap
            .bindings()
            .filter_map(|binding| {
                let context = binding.predicate().map(|context| context.to_string());
                let shell = matches!(
                    context.as_deref(),
                    None | Some(freshkube_ui::page::SHELL_CONTEXT)
                        | Some(crate::desktop::MENU_BAR_ONLY)
                );
                shell.then(|| {
                    let keys: Vec<String> = binding
                        .keystrokes()
                        .iter()
                        .map(|key| key.as_keystroke().unparse())
                        .collect();
                    format!(
                        "{} {} {}",
                        keys.join(" "),
                        binding.action().name(),
                        context.unwrap_or_default()
                    )
                })
            })
            .collect()
    });
    assert_eq!(bindings, SHELL_KEYS);
    let answered = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            view.read(cx).focus.clone().focus(window, cx);
            window.render_frame(cx);
            let mut names: Vec<&'static str> = window
                .available_actions(cx)
                .iter()
                .map(|action| action.name())
                .collect();
            names.sort_unstable();
            names
        })
        .unwrap()
    };
    // The window starts with macOS's bar on every platform, so the actions
    // are the same wherever the test runs; the menu button comes last.
    cx.update(|cx| {
        view.update(cx, |pilot, _| {
            pilot.chrome.menu_platform = freshkube_ui::platform::Platform::MacOs
        })
    });
    let mut expected = vec![
        "app::About",
        "app::OpenClustersSettings",
        "app::OpenCorootSettings",
        "app::OpenDestinationsSettings",
        "app::OpenSettings",
        "app::OpenSourcesSettings",
        "input::Copy",
        "inspector::ToggleInspector",
        "pilot::GoToKind",
        "pilot::NextContext",
        "pilot::NextScreen",
        "pilot::PreviousContext",
        "pilot::PreviousScreen",
        "pilot::Refresh",
        "pilot::ShowDatabases",
        "pilot::ShowEtcd",
        "pilot::ShowEvents",
        "pilot::ShowHealth",
        "pilot::ShowLifecycle",
        "pilot::ShowNamespaces",
        "pilot::ShowNodes",
        "pilot::ShowOverview",
        "pilot::ShowSecurity",
        "pilot::ShowSystemServices",
        "pilot::UseDarkAppearance",
        "pilot::UseLightAppearance",
        "pilot::UseSystemAppearance",
        "root::Tab",
        "root::TabPrev",
        "text_size::DefaultText",
        "text_size::LargerText",
        "text_size::SmallerText",
    ];
    // Overview has no column.
    assert_eq!(answered(cx), expected);
    // Resources has one, so ToggleColumn joins.
    cx.update_window(handle, |_, window, cx| open_kind(window, cx, "pods"))
        .unwrap();
    expected.push("pilot::ToggleColumn");
    expected.sort_unstable();
    assert_eq!(answered(cx), expected);
    // The menu button answers OpenAppMenu.
    cx.update(|cx| {
        view.update(cx, |pilot, _| {
            pilot.chrome.menu_platform = freshkube_ui::platform::Platform::Linux
        })
    });
    expected.push("app::OpenAppMenu");
    expected.sort_unstable();
    assert_eq!(answered(cx), expected);
}
