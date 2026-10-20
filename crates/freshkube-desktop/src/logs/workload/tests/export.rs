use super::*;

#[gpui_kit::test]
fn timestamps_hide_and_show_each_lines_time(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, pods(&api, "app=api"), cx);
        window.render_frame(cx);
        assert!(view.read(cx).columns().time);
        assert_eq!(
            window.find("workload-logs-timestamps").checked(),
            Some(true)
        );
        window.click("workload-logs-timestamps", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).columns().time);
        assert_eq!(
            window.find("workload-logs-timestamps").checked(),
            Some(false)
        );
        window.click("workload-logs-timestamps", cx);
        window.render_frame(cx);
        assert!(view.read(cx).columns().time);
    })
    .unwrap();
}

#[gpui_kit::test]
fn copy_under_a_pick_copies_only_that_pods_lines_led_by_their_tags(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    let picked = all[2].name.clone();
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        view.update(cx, |view, cx| view.pick_pod(Some(picked.clone()), cx));
        window.render_frame(cx);
        let (row, expected) = {
            let view = view.read(cx);
            let rows = view.visible_rows();
            let expected: Vec<String> = rows
                .iter()
                .map(|&ix| &view.retained()[ix])
                .filter(|entry| !entry.is_marker())
                .map(|entry| {
                    assert!(entry.service.as_str().starts_with(&format!("{picked}/")));
                    format!("{} {}", entry.service.as_str(), entry.selectable_text())
                })
                .collect();
            assert!(!expected.is_empty());
            let id = view.row_id(rows.len() - 1);
            (format!("log-line-{}-{id}", view.generation()), expected)
        };
        window.click(row, cx);
        window.press("secondary-a", cx);
        window.press("secondary-c", cx);
        let copied = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap();
        assert_eq!(copied, expected.join("\n"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_workload_file_holds_its_lines_as_copy_copies_them_led_by_their_tags(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.log");
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    let copied = cx
        .update_window(handle, |_, window, cx| {
            show_fed(&view, &api, cx);
            feed(&view, all.clone(), cx);
            window.render_frame(cx);
            let row = {
                let view = view.read(cx);
                let id = view.row_id(view.visible_rows().len() - 1);
                format!("log-line-{}-{id}", view.generation())
            };
            window.click(row, cx);
            window.press("secondary-a", cx);
            window.press("secondary-c", cx);
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .unwrap()
        })
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.click("logs-download", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(0, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    cx.run_until_parked();
    // Select all takes the last lines the selection cap allows: the file
    // ends with them, written as Copy writes them.
    let file = std::fs::read_to_string(&path).unwrap();
    assert!(file.len() > copied.len());
    assert!(file.ends_with(&format!("\n{copied}\n")));
    // Every line leads with the pod and container that wrote it.
    let pods: Vec<String> = all.iter().map(|pod| format!("{}/", pod.name)).collect();
    for line in file.lines() {
        assert!(
            pods.iter().any(|pod| line.starts_with(pod.as_str())),
            "{line}"
        );
    }
}

#[gpui_kit::test]
fn a_workload_download_is_named_by_the_workload_and_its_picked_pod(cx: &mut TestAppContext) {
    use crate::logs::{DownloadLines, LogSource};
    let (_runtime, view, _handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    cx.update(|cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        let workload = format!("{}-{}", api.namespace, api.name);
        let name = |lines, cx: &App| super::WorkloadLogs::download_name(view.read(cx), lines);
        assert_eq!(name(DownloadLines::Visible, cx), workload);
        assert_eq!(name(DownloadLines::Retained, cx), workload);
        view.update(cx, |view, cx| view.pick_pod(Some(all[0].name.clone()), cx));
        // Visible lines are the picked pod's; every retained line isn't.
        assert_eq!(
            name(DownloadLines::Visible, cx),
            format!("{workload}-{}", all[0].name)
        );
        assert_eq!(name(DownloadLines::Retained, cx), workload);
    });
}
