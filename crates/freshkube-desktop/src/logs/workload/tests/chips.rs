use super::*;

#[gpui_kit::test]
fn a_chip_hides_its_streams_lines_and_new_streams_show(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount(cx);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all[1..].to_vec(), cx);
        window.render_frame(cx);
        let hidden = format!("{}/api", all[1].name);
        window.click(format!("workload-logs-stream-{hidden}"), cx);
        window.render_frame(cx);
        assert_eq!(
            window
                .find(format!("workload-logs-stream-{hidden}"))
                .checked(),
            Some(false)
        );
        let shown_tags = |view: &Entity<WorkloadLogView>, cx: &App| {
            let view = view.read(cx);
            view.visible_rows()
                .iter()
                .map(|&ix| view.retained()[ix].service.as_str().to_owned())
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert!(!shown_tags(&view, cx).contains(&hidden));
        assert!(!shown_tags(&view, cx).is_empty());

        // A pod that joins shows at once.
        feed(&view, all.clone(), cx);
        window.render_frame(cx);
        let joined = format!("{}/api", all[0].name);
        assert!(shown_tags(&view, cx).contains(&joined));
        assert!(!shown_tags(&view, cx).contains(&hidden));

        window.click(format!("workload-logs-stream-{hidden}"), cx);
        window.render_frame(cx);
        assert!(shown_tags(&view, cx).contains(&hidden));
    })
    .unwrap();
}

/// Chips past the rows' room end in "+N", which lists every container
/// and shows or hides its lines like a chip.
#[gpui_kit::test]
fn chips_past_the_rows_end_in_more_which_lists_them_all(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount_sized(cx, 520., 820.);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all.clone(), cx);
        // A narrow panel: the chips fit its width on the next frame.
        window.render_frame(cx);
        window.render_frame(cx);
        let total = view.read(cx).source().chips.len();
        assert_eq!(total, all.len());
        let fits = view.read(cx).source().fits;
        assert!(fits < total, "{fits} of {total}");
        let shown = (0..total)
            .filter(|&ix| {
                let id = view.read(cx).source().chips[ix].id.clone();
                window.try_find(id).is_some()
            })
            .count();
        assert_eq!(shown, fits);
        let more = window
            .find("workload-logs-more")
            .label()
            .unwrap()
            .to_owned();
        assert!(more.contains(&format!("+{}", total - fits)), "{more}");

        window.click("workload-logs-more", cx);
        window.render_frame(cx);
        assert!(window.find("workload-logs-list").visible());
        let last = view.read(cx).source().chips[total - 1].clone();
        window.click(last.list_id.clone(), cx);
        window.render_frame(cx);
        assert!(view.read(cx).source().hidden.contains(&last.service));
        assert_eq!(window.find(last.list_id.clone()).checked(), Some(false));
    })
    .unwrap();
}

/// The list behind "+N" stays closed once dismissed, through new frames,
/// new chips and another workload; it opens only when asked.
#[gpui_kit::test]
fn the_list_of_containers_stays_closed_once_dismissed(cx: &mut TestAppContext) {
    let (_runtime, view, handle) = mount_sized(cx, 520., 820.);
    let api = deployment("api");
    let all = pods(&api, "app=api");
    let open = |view: &Entity<WorkloadLogView>, cx: &App| view.read(cx).source().more_open;
    cx.update_window(handle, |_, window, cx| {
        show_fed(&view, &api, cx);
        feed(&view, all[1..].to_vec(), cx);
        window.render_frame(cx);
        window.render_frame(cx);
        window.click("workload-logs-more", cx);
        window.render_frame(cx);
        assert!(open(&view, cx));
        assert!(window.find("workload-logs-list").visible());

        // Dismissed by a click outside it.
        window.click("workload-logs-status", cx);
        window.render_frame(cx);
        assert!(!open(&view, cx));
        assert!(window.try_find("workload-logs-list").is_none());
    })
    .unwrap();
    // New frames and new lines leave it closed.
    cx.executor().advance_clock(EXAMPLE_INTERVAL * 2);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(!open(&view, cx));
        assert!(window.try_find("workload-logs-list").is_none());

        // Open again, then a pod joins: another set of chips closes it.
        window.click("workload-logs-more", cx);
        window.render_frame(cx);
        assert!(open(&view, cx));
        feed(&view, all.clone(), cx);
        window.render_frame(cx);
        assert!(!open(&view, cx));
        assert!(window.try_find("workload-logs-list").is_none());

        // Open again, then another workload: closed.
        window.click("workload-logs-more", cx);
        window.render_frame(cx);
        assert!(open(&view, cx));
        view.update(cx, |view, cx| {
            view.show_workload(Some(deployment("ledger")), None, cx)
        });
        window.render_frame(cx);
        assert!(!open(&view, cx));
        assert!(window.try_find("workload-logs-list").is_none());
    })
    .unwrap();
}

#[test]
fn chips_fit_their_rows_and_leave_room_for_more() {
    use gpui_kit::px;
    let fit = |widths: &[f32], rows| {
        let widths: Vec<_> = widths.iter().map(|&width| px(width)).collect();
        crate::logs::chips_that_fit(&widths, px(30.), px(6.), px(100.), rows)
    };
    // All fit: no "+N" needed.
    assert_eq!(fit(&[40., 40.], 1), 2);
    assert_eq!(fit(&[40., 40., 40., 40.], 2), 4);
    // The third doesn't fit one row; "+N" (30) after the first leaves 76.
    assert_eq!(fit(&[40., 40., 40.], 1), 1);
    // Two rows of two, then "+N" takes the second's place in the last row.
    assert_eq!(fit(&[40., 40., 40., 40., 40.], 2), 3);
    // No rows: only "+N".
    assert_eq!(fit(&[40.], 0), 0);
    // A chip too wide for any row still takes one alone.
    assert_eq!(fit(&[150.], 1), 1);
}

#[test]
fn short_labels_drop_the_shared_prefix_up_to_a_dash() {
    let labels = |tags: &[&str]| {
        super::short_labels(tags.iter().map(|tag| ServiceId::new(*tag)))
            .into_values()
            .map(|label| label.to_string())
            .collect::<Vec<_>>()
    };
    // One ReplicaSet: the random suffix; one container name: no container.
    assert_eq!(
        labels(&["api-6c4f8d9f-bbbbg/api", "api-6c4f8d9f-bbbch/api"]),
        ["bbbbg", "bbbch"]
    );
    // A rollout keeps the template hash; a sidecar names the container.
    assert_eq!(
        labels(&["api-6c4f8d9f-bbbbg/api", "api-7d5e9a0b-cxk2p/proxy"]),
        ["6c4f8d9f-bbbbg/api", "7d5e9a0b-cxk2p/proxy"]
    );
    // A pod's own tag, from its markers, doesn't make the container show.
    assert_eq!(
        labels(&[
            "api-6c4f8d9f-bbbbg",
            "api-6c4f8d9f-bbbbg/api",
            "api-6c4f8d9f-bbbch/api"
        ]),
        ["bbbbg", "bbbbg", "bbbch"]
    );
    // StatefulSet ordinals, and a lone pod.
    assert_eq!(
        labels(&["db-0/db", "db-1/db", "db-10/db"]),
        ["0", "1", "10"]
    );
    assert_eq!(labels(&["report-28411/report"]), ["28411"]);
    assert_eq!(labels(&["solo/app"]), ["solo"]);
}
