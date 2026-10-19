use super::*;

/// The labels of the markers a chart draws.
fn chart_markers(
    cx: &mut TestAppContext,
    page: &Entity<MonitoringPage>,
    title: &str,
) -> Vec<String> {
    cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        let slot = board
            .slots
            .iter()
            .find(|slot| slot.spec.title == title)
            .unwrap();
        slot.view
            .read(cx)
            .marker_labels()
            .into_iter()
            .map(|label| label.to_string())
            .collect()
    })
}

fn choose(cx: &mut TestAppContext, page: &Entity<MonitoringPage>, variable: &str, value: &str) {
    let index = cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        let id = format!("monitoring-variable-{variable}");
        let control = board
            .controls
            .iter()
            .find(|control| control.id.as_ref() == id)
            .unwrap();
        assert!(
            control
                .options
                .iter()
                .any(|option| option.as_ref() == value)
        );
        control.index
    });
    cx.update(|cx| page.update(cx, |page, cx| page.set_variable(index, value.into(), cx)));
    cx.run_until_parked();
}

/// A chart drawing all its series keeps them through a refresh, and goes
/// back to the highest peaks when a variable or the time range changes.
#[gpui_kit::test]
fn show_all_series_lasts_until_the_page_asks_for_something_else(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let panel = cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        let slot = board.slots.iter().find(|slot| slot.spec.title == CPU);
        slot.unwrap().view.clone()
    });
    let all = |cx: &mut TestAppContext| cx.read(|cx| panel.read(cx).shows_all_series());
    let show_all = |cx: &mut TestAppContext| {
        cx.update(|cx| panel.update(cx, |panel, cx| panel.show_all_series(true, cx)));
        assert!(all(cx));
    };

    show_all(cx);
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    cx.run_until_parked();
    frame(cx, handle);
    assert!(all(cx), "a refresh dropped Show all");

    cx.update(|cx| page.update(cx, |page, cx| page.set_range(3600, cx)));
    cx.run_until_parked();
    assert!(!all(cx), "a new range kept Show all");

    show_all(cx);
    choose(cx, &page, "node", "worker-2");
    assert!(!all(cx), "a new variable kept Show all");
}

#[gpui_kit::test]
fn every_chart_draws_the_example_deploys_and_node_events(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    assert!(shown(cx, handle, "monitoring-markers-deploys"));
    assert!(shown(cx, handle, "monitoring-markers-nodes"));
    assert!(!shown(cx, handle, "monitoring-markers-unavailable"));
    let markers = chart_markers(cx, &page, CPU);
    assert_eq!(
        markers,
        [
            "cp-2 rebooted",
            "cp-2 Ready",
            "deploy/coredns → 1.11.3",
            "deploy/checkout → revision 14",
            "deploy/kube-state-metrics → 2.13.0",
            "deploy/api → 1.8.2",
            "worker-2 NotReady",
        ]
    );
    // Every timeseries draws them; a stat has no time axis to draw them on.
    assert_eq!(chart_markers(cx, &page, "Memory usage by node"), markers);
    assert!(chart_markers(cx, &page, "Ready nodes").is_empty());

    // A shorter range keeps those within it.
    cx.update(|cx| page.update(cx, |page, cx| page.set_range(3600, cx)));
    cx.run_until_parked();
    assert_eq!(
        chart_markers(cx, &page, CPU),
        ["deploy/api → 1.8.2", "worker-2 NotReady"]
    );
}

#[gpui_kit::test]
fn the_annotation_toggles_hide_and_show_each_kind(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let click = |cx: &mut TestAppContext, id: &'static str| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    click(cx, "monitoring-markers-deploys");
    let markers = chart_markers(cx, &page, CPU);
    assert_eq!(
        markers,
        ["cp-2 rebooted", "cp-2 Ready", "worker-2 NotReady"]
    );
    click(cx, "monitoring-markers-nodes");
    assert!(chart_markers(cx, &page, CPU).is_empty());
    click(cx, "monitoring-markers-deploys");
    click(cx, "monitoring-markers-nodes");
    assert_eq!(chart_markers(cx, &page, CPU).len(), 7);

    // A refresh keeps the choice.
    click(cx, "monitoring-markers-deploys");
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    cx.run_until_parked();
    assert_eq!(chart_markers(cx, &page, CPU).len(), 3);
}

#[gpui_kit::test]
fn markers_follow_the_namespace_and_node_variables(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    choose(cx, &page, "namespace", "payments");
    assert_eq!(
        chart_markers(cx, &page, CPU),
        [
            "cp-2 rebooted",
            "cp-2 Ready",
            "deploy/checkout → revision 14",
            "deploy/api → 1.8.2",
            "worker-2 NotReady",
        ]
    );
    choose(cx, &page, "node", "worker-2");
    assert_eq!(
        chart_markers(cx, &page, CPU),
        [
            "deploy/checkout → revision 14",
            "deploy/api → 1.8.2",
            "worker-2 NotReady",
        ]
    );
    choose(cx, &page, "namespace", "All");
    choose(cx, &page, "node", "All");
    assert_eq!(chart_markers(cx, &page, CPU).len(), 7);
}

#[gpui_kit::test]
fn another_context_forgets_the_markers_read(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    assert_eq!(chart_markers(cx, &page, CPU).len(), 7);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_source(None, cx);
            assert!(page.markers.all.is_empty());
            assert!(page.markers.shown.is_empty());
        })
    });
}
