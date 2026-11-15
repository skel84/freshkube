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
        entry: String::new(),
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
        seen: Default::default(),
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
        seen: Default::default(),
        open: false,
        maximized: true,
        selected: Some(1),
        tabs: vec![SavedTab {
            entry: String::new(),
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
        seen: Default::default(),
        open: true,
        maximized: false,
        selected: Some(0),
        tabs: vec![SavedTab {
            entry: String::new(),
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
            // A dock that opens with no tab, as a first launch does.
            dock.tabs.clear();
            dock.selected = None;
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
        entry: String::new(),
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
        seen: Default::default(),
        open: true,
        maximized: false,
        selected: Some(0),
        tabs: vec![SavedTab {
            entry: String::new(),
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
        seen: Default::default(),
        open: true,
        maximized: false,
        selected: None,
        tabs: vec![SavedTab {
            entry: String::new(),
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

/// Opens the dock as the app does: the saved dock waits for the first
/// connection, which is this window's context.
fn open_with(
    saved: &SavedDock,
    handle: AnyWindowHandle,
    pilot: &Entity<Pilot>,
    dock: &Entity<Dock>,
    cx: &mut TestAppContext,
) {
    cx.update_window(handle, |_, window, cx| {
        let source = pilot.read(cx).source().kube;
        dock.update(cx, |dock, cx| {
            dock.tabs.clear();
            dock.selected = None;
            dock.set_source(None, None, window, cx);
            dock.open = saved.open;
            dock.restore = Some(saved.clone());
            dock.set_source(source, None, window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
}

fn log_tab(pod: &ResourceIdentity, context: &str) -> SavedTab {
    SavedTab {
        entry: String::new(),
        kind: "pods".into(),
        context: context.into(),
        namespace: pod.namespace.clone(),
        name: pod.name.clone(),
        container: None,
        previous: false,
        all_containers: false,
        shell: false,
    }
}

/// #1044: a dock holds tabs of several clusters. Restoring on one of them
/// shows only its tabs, and saving again keeps the others as they were, with
/// the selection on the tab it was on, so they come back with their cluster.
#[gpui_kit::test]
fn tabs_of_another_context_survive_a_restore_and_the_next_save(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    let context = cx.update(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let saved = SavedDock {
        seen: Default::default(),
        open: true,
        maximized: false,
        selected: Some(1),
        tabs: vec![
            log_tab(&pods[2], "another-context"),
            log_tab(&pods[0], &context),
            log_tab(&pods[1], "third-context"),
        ],
    };
    open_with(&saved, handle, &pilot, &dock, cx);
    assert_eq!(titles(&dock, cx), [format!("Pod {}", pods[0].name)]);
    let again = cx.update(|cx| dock.read(cx).saved(cx));
    let mut names: Vec<_> = again
        .tabs
        .iter()
        .map(|tab| (tab.context.as_str(), tab.name.as_str()))
        .collect();
    names.sort();
    let mut expected = vec![
        ("another-context", pods[2].name.as_str()),
        (context.as_str(), pods[0].name.as_str()),
        ("third-context", pods[1].name.as_str()),
    ];
    expected.sort();
    assert_eq!(names, expected, "no context's tab is dropped by a save");
    let selected = again.selected.map(|ix| &again.tabs[ix]);
    assert_eq!(
        selected.map(|tab| tab.context.as_str()),
        Some(context.as_str())
    );
    // Saved to the file and read again, they are still there.
    let json = serde_json::to_string(&again).unwrap();
    assert_eq!(serde_json::from_str::<SavedDock>(&json).unwrap(), again);
}

/// A tab of another context comes back only when that context is shown, and
/// then idle: nothing read for a cluster the person had not opened.
#[gpui_kit::test]
fn tabs_of_another_context_come_back_idle_when_it_is_shown(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    let context = cx.update(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let other = cx.update(|cx| {
        let pilot = pilot.read(cx);
        pilot
            .contexts
            .iter()
            .find(|name| Some(*name) != pilot.applied.context.as_ref())
            .cloned()
            .unwrap()
    });
    let saved = SavedDock {
        seen: Default::default(),
        open: true,
        maximized: false,
        selected: Some(0),
        tabs: vec![log_tab(&pods[0], &context), log_tab(&pods[1], &other)],
    };
    open_with(&saved, handle, &pilot, &dock, cx);
    assert_eq!(titles(&dock, cx).len(), 1);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.select_context(other.clone(), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert_eq!(titles(&dock, cx).len(), 2, "{:?}", titles(&dock, cx));
    // The first tab is still the shown one; the other has read nothing.
    let started: Vec<bool> =
        cx.update(|cx| dock.read(cx).tabs.iter().map(|tab| tab.started).collect());
    assert_eq!(started.len(), 2);
    assert!(
        !started[1],
        "a restored tab of another context reads nothing"
    );
    let again = cx.update(|cx| dock.read(cx).saved(cx));
    assert_eq!(again.tabs.len(), 2);
}

/// A file an older build wrote has no `entry` on a tab and no `seen`: it still
/// reads, a tab without an entry matches by its context's name alone, and
/// reading and writing it back adds neither key.
#[test]
fn a_file_saved_by_an_older_build_still_reads() {
    let json = r#"{"open":true,"maximized":false,"selected":1,"tabs":[{"kind":"pods","context":"dev-fra","namespace":"web","name":"gateway-0"},{"kind":"pods","context":"prod-ams","namespace":"web","name":"gateway-1"}]}"#;
    let saved: SavedDock = serde_json::from_str(json).unwrap();
    assert_eq!(saved.selected, Some(1));
    assert_eq!(saved.tabs.len(), 2);
    assert!(saved.tabs.iter().all(|tab| tab.entry.is_empty()));
    assert!(saved.seen.is_empty());
    assert_eq!(serde_json::to_string(&saved).unwrap(), json);
    // A file written now, with an entry and the days, reads back as it was.
    let mut now = saved.clone();
    now.tabs[1].entry = "prod-ams-entry".into();
    now.seen.insert("prod-ams".into(), 20_000);
    let written = serde_json::to_string(&now).unwrap();
    assert!(written.contains(r#""entry":"prod-ams-entry""#), "{written}");
    assert!(
        written.contains(r#""seen":{"prod-ams":20000}"#),
        "{written}"
    );
    assert_eq!(serde_json::from_str::<SavedDock>(&written).unwrap(), now);
}

fn tab_in(pod: &str, context: &str, entry: &str) -> SavedTab {
    SavedTab {
        entry: entry.into(),
        kind: "pods".into(),
        context: context.into(),
        namespace: "web".into(),
        name: pod.into(),
        container: None,
        previous: false,
        all_containers: false,
        shell: false,
    }
}

/// Selects another context in the window, as the person does, and lets it load.
fn show_context(
    handle: AnyWindowHandle,
    pilot: &Entity<Pilot>,
    context: &str,
    cx: &mut TestAppContext,
) {
    let context = context.to_owned();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.select_context(context, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
}

fn other_context(pilot: &Entity<Pilot>, cx: &mut TestAppContext) -> String {
    cx.update(|cx| {
        let pilot = pilot.read(cx);
        pilot
            .contexts
            .iter()
            .find(|name| Some(*name) != pilot.applied.context.as_ref())
            .cloned()
            .unwrap()
    })
}

/// A parked shell tab of another context comes back idle when that context is
/// shown, and starts nothing: no exec until Start.
#[gpui_kit::test]
fn a_parked_shell_tab_comes_back_idle_on_a_switch(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    let context = cx.update(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let other = other_context(&pilot, cx);
    let mut shell = log_tab(&pods[1], &other);
    shell.shell = true;
    shell.container = Some("app".into());
    let saved = SavedDock {
        open: true,
        maximized: false,
        selected: Some(0),
        tabs: vec![log_tab(&pods[0], &context), shell],
        seen: Default::default(),
    };
    open_with(&saved, handle, &pilot, &dock, cx);
    assert_eq!(titles(&dock, cx).len(), 1);
    show_context(handle, &pilot, &other, cx);
    assert_eq!(titles(&dock, cx).len(), 2, "{:?}", titles(&dock, cx));
    assert_eq!(shell_state(&dock, 1, cx), ShellState::Idle);
    assert!(cx.read(shell::running_anywhere).is_empty(), "no exec");
}

/// A selection that was on a parked tab survives a save, until another tab is
/// selected.
#[gpui_kit::test]
fn a_selection_on_a_parked_tab_survives_saved_until_another_is_selected(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let pods = running_pods(&pilot, cx);
    let context = cx.update(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let saved = SavedDock {
        open: true,
        maximized: false,
        // The parked tab of another context was the selected one.
        selected: Some(0),
        tabs: vec![
            log_tab(&pods[2], "another-context"),
            log_tab(&pods[0], &context),
            log_tab(&pods[1], &context),
        ],
        seen: Default::default(),
    };
    open_with(&saved, handle, &pilot, &dock, cx);
    let again = cx.update(|cx| dock.read(cx).saved(cx));
    let selected = again.selected.map(|ix| again.tabs[ix].context.clone());
    assert_eq!(selected.as_deref(), Some("another-context"));
    // Selecting a tab of the shown context is what the person chose last.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("dock-tab-1", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let again = cx.update(|cx| dock.read(cx).saved(cx));
    let selected = again.selected.map(|ix| again.tabs[ix].clone());
    assert_eq!(
        selected.map(|tab| (tab.context, tab.name)),
        Some((context, pods[1].name.clone()))
    );
}

/// A full dock keeps another context's parked tabs parked, and they open once
/// there is room.
#[gpui_kit::test]
fn a_full_dock_keeps_the_parked_tabs_and_opens_them_when_there_is_room(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let context = cx.update(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let other = other_context(&pilot, cx);
    let mut tabs: Vec<SavedTab> = (0..MAX_LOG_TABS)
        .map(|ix| tab_in(&format!("pod-{ix}"), &context, ""))
        .collect();
    tabs.push(tab_in("waiting-0", &other, ""));
    let saved = SavedDock {
        open: true,
        maximized: false,
        selected: Some(0),
        tabs,
        seen: Default::default(),
    };
    open_with(&saved, handle, &pilot, &dock, cx);
    assert_eq!(titles(&dock, cx).len(), MAX_LOG_TABS);
    // Showing the other context has no room for its tab: it stays parked and
    // is still in the file.
    show_context(handle, &pilot, &other, cx);
    assert_eq!(titles(&dock, cx).len(), MAX_LOG_TABS);
    let kept = cx.update(|cx| dock.read(cx).saved(cx));
    assert!(kept.tabs.iter().any(|tab| tab.name == "waiting-0"));
    // A tab closes, and the next time its context is shown it opens.
    cx.update_window(handle, |_, window, cx| {
        let id = dock.read(cx).tabs[0].id;
        dock.update(cx, |dock, cx| dock.close_tab(id, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let source = pilot.read(cx).source().kube;
        dock.update(cx, |dock, cx| dock.set_source(source, None, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let names = cx.update(|cx| {
        dock.read(cx)
            .tabs
            .iter()
            .map(|tab| tab.target.identity.name.clone())
            .collect::<Vec<_>>()
    });
    assert!(names.contains(&"waiting-0".to_owned()), "{names:?}");
}

/// A tab reopens only on the entry it was saved on: the same context name
/// under another entry stays parked.
#[gpui_kit::test]
fn a_reused_context_name_under_another_entry_stays_parked(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let context = cx.update(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let saved = SavedDock {
        open: true,
        maximized: false,
        selected: Some(0),
        tabs: vec![
            tab_in("mine-0", &context, "entry-a"),
            tab_in("theirs-0", &context, "entry-b"),
            tab_in("older-0", &context, ""),
        ],
        seen: Default::default(),
    };
    cx.update(|cx| dock.update(cx, |dock, _| dock.shown_entry = Some("entry-a".into())));
    open_with(&saved, handle, &pilot, &dock, cx);
    let names = cx.update(|cx| {
        dock.read(cx)
            .tabs
            .iter()
            .map(|tab| tab.target.identity.name.clone())
            .collect::<Vec<_>>()
    });
    // The entry's own tab and the older tab without one (a name match) open;
    // the other entry's is kept as saved.
    assert_eq!(names, ["mine-0", "older-0"]);
    let again = cx.update(|cx| dock.read(cx).saved(cx));
    let theirs = again
        .tabs
        .iter()
        .find(|tab| tab.name == "theirs-0")
        .unwrap();
    assert_eq!(theirs.entry, "entry-b");
    let mine = again.tabs.iter().find(|tab| tab.name == "mine-0").unwrap();
    assert_eq!(mine.entry, "entry-a");
}

fn tabs_named(names: &[(&str, &str, bool)]) -> Vec<SavedTab> {
    names
        .iter()
        .map(|(name, context, shell)| {
            let mut tab = tab_in(name, context, "");
            tab.shell = *shell;
            tab
        })
        .collect()
}

/// The bounds: 8 log and 8 shell tabs per context, 64 in all, the earlier
/// ones first, with the selection following its tab.
#[test]
fn the_saved_tabs_are_bounded_per_context_and_in_all() {
    let seen = Default::default();
    // 10 log tabs of one context and 9 shells: 8 and 8 stay.
    let mut names: Vec<(String, &str, bool)> = (0..10)
        .map(|ix| (format!("log-{ix}"), "lab", false))
        .collect();
    names.extend((0..9).map(|ix| (format!("shell-{ix}"), "lab", true)));
    let tabs = tabs_named(
        &names
            .iter()
            .map(|(name, context, shell)| (name.as_str(), *context, *shell))
            .collect::<Vec<_>>(),
    );
    let (kept, selected) = bounded(tabs, 0, Some(9), &seen, 20_000);
    assert_eq!(kept.len(), MAX_LOG_TABS + MAX_SHELL_TABS);
    assert_eq!(kept.iter().filter(|tab| tab.shell).count(), MAX_SHELL_TABS);
    // The tab at position 9 (log-9) went, and with it the selection.
    assert_eq!(selected, None);
    // A selection on a tab that stays follows it when earlier ones went.
    let tabs = tabs_named(&[("a", "lab", false), ("b", "lab", false)]);
    assert_eq!(bounded(tabs, 0, Some(1), &seen, 0).1, Some(1));

    // 9 contexts of 8 log tabs each: 64 in all.
    let many: Vec<SavedTab> = (0..9)
        .flat_map(|context| {
            (0..8).map(move |ix| tab_in(&format!("p{ix}"), &format!("ctx-{context}"), ""))
        })
        .collect();
    let (kept, _) = bounded(many, 0, None, &seen, 0);
    assert_eq!(kept.len(), MAX_SAVED_TABS);
    // The earlier contexts keep all of theirs.
    assert_eq!(kept.iter().filter(|tab| tab.context == "ctx-0").count(), 8);
    assert_eq!(kept.iter().filter(|tab| tab.context == "ctx-8").count(), 0);
}

/// A parked context unseen for more than 30 days loses its tabs; an open tab
/// is never dropped for age, and a context with no day yet starts its clock.
#[test]
fn a_parked_context_unseen_for_30_days_loses_its_tabs() {
    let today = 20_000;
    let seen: std::collections::BTreeMap<String, u32> = [
        ("old".to_owned(), today - PARKED_DAYS - 1),
        ("edge".to_owned(), today - PARKED_DAYS),
    ]
    .into();
    let tabs = tabs_named(&[
        ("open-old", "old", false),
        ("parked-old", "old", false),
        ("parked-edge", "edge", false),
        ("parked-new", "never-dated", false),
    ]);
    let (kept, _) = bounded(tabs, 1, None, &seen, today);
    let names: Vec<_> = kept.iter().map(|tab| tab.name.as_str()).collect();
    assert_eq!(names, ["open-old", "parked-edge", "parked-new"]);
}

/// On load, a parked context unseen for 30 days is dropped and, on save, the
/// days are written for every context that is kept.
#[gpui_kit::test]
fn the_days_bound_parked_tabs_on_load_and_on_save(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let dock = dock(&pilot, cx);
    let context = cx.update(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let today = 20_000;
    super::super::saved::TODAY.with(|day| day.set(Some(today)));
    let saved = SavedDock {
        open: true,
        maximized: false,
        selected: Some(0),
        tabs: vec![
            tab_in("shown-0", &context, ""),
            tab_in("gone-0", "removed-from-setup", ""),
            tab_in("recent-0", "recently-used", ""),
            tab_in("undated-0", "older-file", ""),
        ],
        seen: [
            ("removed-from-setup".to_owned(), today - 31),
            ("recently-used".to_owned(), today - 3),
        ]
        .into(),
    };
    open_with(&saved, handle, &pilot, &dock, cx);
    let again = cx.update(|cx| dock.read(cx).saved(cx));
    let names: Vec<_> = again.tabs.iter().map(|tab| tab.name.as_str()).collect();
    assert_eq!(names, ["shown-0", "recent-0", "undated-0"]);
    // The days are kept for the parked, today for the shown and the undated.
    assert_eq!(again.seen.get("recently-used"), Some(&(today - 3)));
    assert_eq!(again.seen.get(&context), Some(&today));
    assert_eq!(again.seen.get("older-file"), Some(&today));
    assert!(!again.seen.contains_key("removed-from-setup"));
    // Thirty-one days later with nothing shown for them, and saved again:
    super::super::saved::TODAY.with(|day| day.set(Some(today + 31)));
    let later = cx.update(|cx| dock.read(cx).saved(cx));
    let names: Vec<_> = later.tabs.iter().map(|tab| tab.name.as_str()).collect();
    assert_eq!(names, ["shown-0"]);
    super::super::saved::TODAY.with(|day| day.set(None));
}
