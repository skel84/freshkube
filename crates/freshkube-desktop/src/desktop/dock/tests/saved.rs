use super::*;

/// A drag of the dock's edge writes its height once, under `dock.height`;
/// the next run opens at it, and a window too short for it shows less
/// without writing.
#[gpui_kit::test]
fn the_dock_height_is_written_once_and_survives_a_restart(cx: &mut TestAppContext) {
    let directory = std::env::temp_dir().join(format!(
        "freshkube-dock-height-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let preferences = directory.join("preferences.json");
    let options = || crate::GpuiOptions::fixture().with_preferences(Some(preferences.clone()));
    let saved = || {
        std::fs::read_to_string(directory.join("navigation.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|value| value["dock"]["height"].as_f64())
    };
    let before = freshkube_probe::probe::count("navigation.size.save");
    let (_runtime, handle, pilot) = crate::desktop::tests::mount(cx, options(), 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    cx.update_window(handle, |_, window, cx| {
        // A drag across many frames.
        for height in (300..=420).step_by(4) {
            dock.update(cx, |dock, cx| dock.resize(height as f32, window, cx));
        }
    })
    .unwrap();
    cx.run_until_parked();
    let sizes_saved = || freshkube_probe::probe::count("navigation.size.save") - before;
    assert_eq!(saved(), None, "nothing is written while it moves");
    assert_eq!(sizes_saved(), 0);
    cx.executor()
        .advance_clock(freshkube_ui::split_size::SAVE_DELAY * 2);
    cx.run_until_parked();
    assert_eq!(saved(), Some(420.));
    assert_eq!(sizes_saved(), 1, "the drag writes once");

    // The next run opens at it, in a window too short to show it all.
    let before = freshkube_probe::probe::count("navigation.size.save");
    let (_runtime, handle, pilot) = crate::desktop::tests::mount(cx, options(), 1280., 480.);
    let sizes_saved = || freshkube_probe::probe::count("navigation.size.save") - before;
    let dock = super::dock(&pilot, cx);
    assert_eq!(cx.update(|cx| dock.read(cx).height()), 420.);
    let pods = running_pods(&pilot, cx);
    open_logs(handle, &pilot, "pods", &pods[0], cx);
    draw(handle, cx);
    cx.update_window(handle, |_, window, cx| {
        let shown = dock.read(cx).shown_height(window, cx).unwrap();
        assert!(shown < crate::ui::dp_px(420., window), "{shown:?}");
    })
    .unwrap();
    cx.executor()
        .advance_clock(freshkube_ui::split_size::SAVE_DELAY * 2);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| dock.read(cx).height()), 420.);
    assert_eq!(saved(), Some(420.));
    assert_eq!(sizes_saved(), 0, "showing it shorter writes nothing");
    let _ = std::fs::remove_dir_all(&directory);
}

#[gpui_kit::test]
fn saved_tabs_come_back_for_their_context_and_read_once_shown(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    let context = cx.update(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let saved_tab = |pod: &ResourceIdentity, context: &str| SavedTab {
        kind: "pods".into(),
        context: context.into(),
        namespace: pod.namespace.clone(),
        name: pod.name.clone(),
        container: None,
        previous: false,
        all_containers: false,
        shell: false,
    };
    let saved = SavedDock {
        open: false,
        maximized: false,
        // The second of this context's tabs, counted among all of them.
        selected: Some(2),
        tabs: vec![
            saved_tab(&pods[2], "another-context"),
            saved_tab(&pods[0], &context),
            saved_tab(&pods[1], &context),
        ],
    };
    // As the app opens: the saved dock waits for the first connection.
    cx.update_window(handle, |_, window, cx| {
        let source = pilot.read(cx).source().kube;
        dock.update(cx, |dock, cx| {
            dock.source = None;
            dock.height.release(260., cx);
            dock.open = saved.open;
            dock.restore = Some(saved);
            dock.set_source(source, None, window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert_eq!(
        titles(&dock, cx),
        [
            format!("Pod {}", pods[0].name),
            format!("Pod {}", pods[1].name)
        ]
    );
    // Minimized, nothing reads.
    let started = |cx: &mut TestAppContext| -> Vec<bool> {
        cx.update(|cx| dock.read(cx).tabs.iter().map(|tab| tab.started).collect())
    };
    assert_eq!(started(cx), [false, false]);
    let first = pod_view(&dock, 0, cx);
    assert!(!cx.update(|cx| first.read(cx).streaming()));

    // Opening shows the selected tab, which reads; the other waits.
    cx.update_window(handle, |_, window, cx| window.click("dock-minimize", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(started(cx), [false, true]);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("dock-tab-0", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(started(cx), [true, true]);
    assert_eq!(cx.update(|cx| dock.read(cx).height()), 260.);
}

#[test]
fn the_saved_dock_reads_back_what_it_wrote_and_fills_in_what_is_missing() {
    let saved = SavedDock {
        open: false,
        maximized: true,
        selected: Some(1),
        tabs: vec![SavedTab {
            kind: "pods".into(),
            context: "lab".into(),
            namespace: "payments".into(),
            name: "api-0".into(),
            container: Some("api".into()),
            previous: true,
            all_containers: false,
            shell: false,
        }],
    };
    let json = serde_json::to_string(&saved).unwrap();
    assert_eq!(serde_json::from_str::<SavedDock>(&json).unwrap(), saved);
    // An empty dock, or one from a build that saved more, still reads; the
    // height beside it is a split's size, read apart (`dock.height`).
    let empty: SavedDock = serde_json::from_str("{}").unwrap();
    assert_eq!((empty.open, empty.tabs.len()), (true, 0));
    let older: SavedDock =
        serde_json::from_str(r#"{"hidden":true,"height":120,"open":false}"#).unwrap();
    assert!(!older.open);
}

#[gpui_kit::test]
fn a_restored_shell_tab_comes_back_idle(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    pick_shell(handle, &pilot, &pods[0], cx);
    let container = {
        let view = shell_view(&dock, 0, cx);
        cx.update(|cx| view.read(cx).container().unwrap().to_owned())
    };
    let context = cx.update(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let saved = SavedDock {
        open: true,
        maximized: false,
        selected: Some(0),
        tabs: vec![SavedTab {
            kind: "pods".into(),
            context,
            namespace: pods[0].namespace.clone(),
            name: pods[0].name.clone(),
            container: Some(container.clone()),
            previous: false,
            all_containers: false,
            shell: true,
        }],
    };
    // What the running tab saves is what comes back.
    let written = cx.update(|cx| dock.read(cx).saved(cx));
    assert_eq!(written.tabs, saved.tabs);
    cx.update_window(handle, |_, window, cx| {
        let source = pilot.read(cx).source().kube;
        dock.update(cx, |dock, cx| {
            dock.set_source(None, None, window, cx);
            dock.restore = Some(saved);
            dock.set_source(source, None, window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert_eq!(titles(&dock, cx), [format!("Shell {}", pods[0].name)]);
    // Shown and selected, it runs nothing until Start.
    assert_eq!(shell_state(&dock, 0, cx), ShellState::Idle);
    assert!(cx.read(shell::running_anywhere).is_empty());
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(window.find("pod-shell-start").label(), Some("Start"));
        window.click("pod-shell-start", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(shell_state(&dock, 0, cx), ShellState::Running);
    let view = shell_view(&dock, 0, cx);
    assert_eq!(
        cx.update(|cx| view.read(cx).container().map(str::to_owned)),
        Some(container)
    );
}

/// A tab saved by a build before All containers has no `all_containers`
/// key: it loads unchanged, on its container, and saving it again adds no
/// key while the pick is one container.
#[test]
fn a_tab_saved_before_all_containers_loads_on_its_container() {
    let json = r#"{"open":true,"maximized":false,"selected":0,"tabs":[{"kind":"pods","context":"lab","namespace":"web","name":"gateway-0","container":"gateway","previous":true}]}"#;
    // That build kept the height in the same object; it reads apart now.
    let earlier = json.replacen('{', r#"{"height":260.0,"#, 1);
    let saved: SavedDock = serde_json::from_str(&earlier).unwrap();
    let tab = &saved.tabs[0];
    assert_eq!(tab.container.as_deref(), Some("gateway"));
    assert!(tab.previous);
    assert!(!tab.all_containers);
    assert_eq!(serde_json::to_string(&saved).unwrap(), json);

    let all = SavedTab {
        all_containers: true,
        previous: false,
        ..tab.clone()
    };
    let written = serde_json::to_string(&all).unwrap();
    assert!(written.contains(r#""all_containers":true"#), "{written}");
    assert_eq!(serde_json::from_str::<SavedTab>(&written).unwrap(), all);
}

#[gpui_kit::test]
fn a_tab_saved_on_all_containers_comes_back_on_all_of_them(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let gateway = objects(
        &pilot,
        "pods",
        |cells, name| cells[2] == "Running" && name.starts_with("gateway-"),
        cx,
    )
    .remove(0);
    let context = cx.update(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let saved = SavedDock {
        open: true,
        maximized: false,
        selected: Some(0),
        tabs: vec![SavedTab {
            kind: "pods".into(),
            context,
            namespace: gateway.namespace.clone(),
            name: gateway.name.clone(),
            container: Some("gateway".into()),
            previous: false,
            all_containers: true,
            shell: false,
        }],
    };
    cx.update_window(handle, |_, window, cx| {
        let source = pilot.read(cx).source().kube;
        dock.update(cx, |dock, cx| {
            dock.source = None;
            dock.restore = Some(saved.clone());
            dock.set_source(source, None, window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    cx.run_until_parked();
    let view = pod_view(&dock, 0, cx);
    cx.update(|cx| {
        let view = view.read(cx);
        assert!(view.shows_all());
        assert!(view.streaming());
        assert!(view.columns().source);
    });
    // Saved again, it keeps the pick.
    let again = cx.update(|cx| dock.read(cx).saved(cx));
    assert!(again.tabs[0].all_containers);
    assert_eq!(again.tabs[0].container.as_deref(), Some("gateway"));
}

/// What the app writes to `navigation.json` for a given state, byte for
/// byte: the example `freshkube-prefs` round-trips too. It is driven
/// through the file's own setters, so a change to what they write shows
/// here as well as in that example.
#[gpui_kit::test]
fn navigation_json_is_what_the_setters_write(cx: &mut TestAppContext) {
    use crate::navigation_file::{DOCK_HEIGHT, DRAWER_WIDTH, NavigationFile};
    use freshkube_ui::inspector::width_key;
    use freshkube_ui::split_size::SizeStore as _;

    let directory = tempfile::tempdir().unwrap();
    let file = NavigationFile::open(Some(&directory.path().join("preferences.json")));
    let dock = SavedDock {
        open: true,
        maximized: false,
        selected: None,
        tabs: vec![SavedTab {
            kind: "pods".into(),
            context: "lab".into(),
            namespace: "payments".into(),
            name: "api-0".into(),
            container: Some("api".into()),
            previous: false,
            all_containers: false,
            shell: false,
        }],
    };
    cx.update(|cx| {
        file.set_collapsed(true, cx);
        file.set_dock(&dock, cx);
        file.save(DOCK_HEIGHT, 320., cx);
        file.save(DRAWER_WIDTH, 480., cx);
        file.save(width_key("nodes"), 360., cx);
        file.set_active_cluster("cluster-a", cx);
    });
    cx.run_until_parked();
    let written = std::fs::read_to_string(directory.path().join("navigation.json")).unwrap();
    assert_eq!(
        written,
        include_str!("../../../../../freshkube-prefs/src/examples/navigation.json")
    );
}
