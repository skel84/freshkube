use super::*;

/// Opens the Pod select and clicks its `item`th entry: All pods is 0, and
/// the pods follow a separator, from 2.
fn pick_pod(handle: AnyWindowHandle, item: usize, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("workload-logs-pod", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(item, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_pod_select_narrows_the_lines_and_chips_to_one_pod(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("workload-logs-pod").label(),
            Some("Pod: All pods")
        );
    })
    .unwrap();
    let before = cx.update(|cx| shown_tags(&view, cx));
    assert!(before.len() > 1, "{before:?}");
    // Newest first, as the menu lists them.
    let picked = view.read_with(cx, |view, _| view.source().pod_choices[1].clone());
    pick_pod(handle, 3, cx);
    cx.update_window(handle, |_, window, cx| {
        let source = view.read(cx).source();
        assert_eq!(source.pod.as_deref(), Some(picked.name.as_str()));
        assert_eq!(
            window.find("workload-logs-pod").label(),
            Some(format!("Pod: {}", picked.label).as_str())
        );
        let tags = shown_tags(&view, cx);
        assert!(!tags.is_empty());
        assert!(
            tags.iter()
                .all(|tag| tag.split('/').next() == Some(picked.name.as_str())),
            "{tags:?}"
        );
        // The chips are the picked pod's containers.
        assert!(source.chips.iter().all(|chip| {
            chip.service
                .as_str()
                .starts_with(&format!("{}/", picked.name))
        }));
        assert_eq!(source.chips.len(), 1);
        // Every stream reads on.
        assert_eq!(source.reads.streams.len(), all.len());
    })
    .unwrap();
    pick_pod(handle, 0, cx);
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).source().pod, None);
        assert_eq!(
            window.find("workload-logs-pod").label(),
            Some("Pod: All pods")
        );
        assert_eq!(view.read(cx).source().chips.len(), all.len());
        assert_eq!(shown_tags(&view, cx), before);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_picked_pod_that_leaves_keeps_its_pick_and_its_lines(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    let leaving = all[0].name.clone();
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        window.render_frame(cx);
        view.update(cx, |view, cx| view.pick_pod(Some(leaving.clone()), cx));
        window.render_frame(cx);
        let before = shown_tags(&view, cx);
        assert!(before.contains(&format!("{leaving}/api")));

        feed(&view, all[1..].to_vec(), cx);
        window.render_frame(cx);
        let source = view.read(cx).source();
        assert_eq!(source.pod.as_deref(), Some(leaving.as_str()));
        let gone = source.pod_choices.last().unwrap();
        assert_eq!(gone.name, leaving);
        assert!(gone.label.ends_with(" (gone)"), "{}", gone.label);
        assert!(
            window
                .find("workload-logs-pod")
                .label()
                .unwrap()
                .ends_with(" (gone)")
        );
        // What it wrote stays shown.
        assert!(shown_tags(&view, cx).contains(&format!("{leaving}/api")));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_pod_past_the_cap_says_it_is_not_read(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let mut all = pods(&api, "app=api");
    for pod in &mut all {
        let mut sidecar: Container = pod.containers.containers[0].clone();
        sidecar.name = "proxy".into();
        sidecar.role = ContainerRole::App;
        sidecar.state = ContainerState::Running(None);
        pod.containers.containers.push(sidecar);
    }
    let containers: usize = all
        .iter()
        .map(|pod| {
            pod.containers
                .containers
                .iter()
                .filter(|container| container.role == ContainerRole::App)
                .count()
        })
        .sum();
    assert!(containers > MAX_STREAMS, "{containers}");
    let oldest = all
        .iter()
        .min_by_key(|pod| pod.created)
        .unwrap()
        .name
        .clone();
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all, cx);
        window.render_frame(cx);
        let source = view.read(cx).source();
        let choice = source
            .pod_choices
            .iter()
            .find(|choice| choice.name == oldest)
            .unwrap();
        assert!(choice.label.ends_with(" · not read"), "{}", choice.label);
        assert!(
            source
                .pod_choices
                .iter()
                .filter(|choice| choice.label.ends_with(" · not read"))
                .all(|choice| !source.reads.streams.keys().any(|key| key.pod == choice.name))
        );
        view.update(cx, |view, cx| view.pick_pod(Some(oldest.clone()), cx));
        window.render_frame(cx);
        // The list says why it is empty, in the cap note's words, though
        // the other pods' lines are retained.
        assert!(lines(&view, cx) > 0);
        assert_eq!(
            window.find("logs-empty").label(),
            Some(
                format!(
                    "{oldest} isn't read. Reading {MAX_STREAMS} of {containers} containers, the newest pods first."
                )
                .as_str()
            )
        );

        // Hidden, nothing is read, so no pod is marked as left out.
        view.update(cx, |view, cx| view.set_active(false, cx));
        window.render_frame(cx);
        let source = view.read(cx).source();
        assert_eq!(source.capped, None);
        assert!(
            source
                .pod_choices
                .iter()
                .all(|choice| !choice.label.ends_with(" · not read")),
            "{:?}",
            source.pod_choices
        );
        assert_ne!(
            window.find("logs-empty").label().map(str::to_owned),
            Some(format!(
                "{oldest} isn't read. Reading {MAX_STREAMS} of {containers} containers, the newest pods first."
            ))
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_gone_pod_that_comes_back_under_its_name_is_read_again(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    let member = all[0].name.clone();
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        view.update(cx, |view, cx| view.pick_pod(Some(member.clone()), cx));
        feed(&view, all[1..].to_vec(), cx);
        window.render_frame(cx);
        assert!(view.read(cx).source().pod_label.ends_with(" (gone)"));

        // A StatefulSet's member comes back with its name.
        feed(&view, all.clone(), cx);
        window.render_frame(cx);
        let source = view.read(cx).source();
        assert_eq!(source.pod.as_deref(), Some(member.as_str()));
        assert!(
            !source.pod_label.ends_with(" (gone)"),
            "{}",
            source.pod_label
        );
        assert!(source.reads.streams.keys().any(|key| key.pod == member));
        assert!(
            source
                .chips
                .iter()
                .all(|chip| chip.service.as_str().starts_with(&format!("{member}/")))
        );
        assert_eq!(source.chips.len(), 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_pick_outlives_hide_and_show_and_another_workload_drops_it(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    let picked = all[1].name.clone();
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        view.update(cx, |view, cx| view.pick_pod(Some(picked.clone()), cx));
        window.render_frame(cx);
        view.update(cx, |view, cx| view.set_active(false, cx));
        window.render_frame(cx);
        assert_eq!(view.read(cx).source().pod.as_deref(), Some(picked.as_str()));
        view.update(cx, |view, cx| view.set_active(true, cx));
        feed(&view, all.clone(), cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).source().pod.as_deref(), Some(picked.as_str()));
        let tags = shown_tags(&view, cx);
        assert!(
            tags.iter()
                .all(|tag| tag.split('/').next() == Some(picked.as_str())),
            "{tags:?}"
        );

        // Another workload starts on All pods.
        view.update(cx, |view, cx| {
            view.show_workload(Some(deployment("ledger")), None, cx)
        });
        window.render_frame(cx);
        assert_eq!(view.read(cx).source().pod, None);
        assert_eq!(
            window.find("workload-logs-pod").label(),
            Some("Pod: All pods")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_count_line_counts_the_picked_pod_of_all(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    let count = all.len();
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        window.render_frame(cx);
        assert_eq!(
            status(window),
            format!("Following: {count} pods · {count} containers")
        );
        view.update(cx, |view, cx| view.pick_pod(Some(all[0].name.clone()), cx));
        window.render_frame(cx);
        // The tag stays the workload's: every stream reads on.
        assert_eq!(
            status(window),
            format!("Following: 1 of {count} pods · 1 of {count} containers")
        );
        feed(&view, all[1..].to_vec(), cx);
        window.render_frame(cx);
        assert_eq!(
            status(window),
            format!(
                "Following: 0 of {} pods · 0 of {} containers",
                count - 1,
                count - 1
            )
        );
    })
    .unwrap();
}
