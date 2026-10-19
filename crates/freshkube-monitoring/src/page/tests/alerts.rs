use super::*;

/// The Firing alerts table's view, once every panel has answered.
fn alerts_table(
    cx: &mut TestAppContext,
    page: &Entity<MonitoringPage>,
) -> Entity<crate::panel::TableView> {
    cx.update(|cx| page.update(cx, |page, cx| page.answer_example_now(cx)));
    cx.run_until_parked();
    cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        let slot = board
            .slots
            .iter()
            .find(|slot| slot.spec.title == "Firing alerts")
            .unwrap();
        slot.view.read(cx).table().expect("a table view")
    })
}

#[gpui_kit::test]
fn another_dashboard_or_source_starts_each_table_again(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let first = alerts_table(cx, &page);
    cx.update(|cx| first.update(cx, |table, cx| table.show_all(cx)));
    // A refresh answers the same table, which keeps its scroll and Show all.
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    cx.run_until_parked();
    assert_eq!(alerts_table(cx, &page).entity_id(), first.entity_id());
    assert!(cx.read(|cx| first.read(cx).shows_all()));
    // Another dashboard and back: a new table, with nothing carried over.
    for id in ["freshkube-workloads", "freshkube-cluster"] {
        cx.update(|cx| page.update(cx, |page, cx| page.open(EntryId::Builtin(id), cx)));
        cx.run_until_parked();
    }
    frame(cx, handle);
    let second = alerts_table(cx, &page);
    assert_ne!(second.entity_id(), first.entity_id());
    assert!(!cx.read(|cx| second.read(cx).shows_all()));
    // Another source: a new table again.
    let other = ClusterSource {
        id: "example-2".into(),
        ..example_source()
    };
    cx.update(|cx| page.update(cx, |page, cx| page.set_source(Some(other), cx)));
    cx.run_until_parked();
    frame(cx, handle);
    assert_ne!(alerts_table(cx, &page).entity_id(), second.entity_id());
}

#[gpui_kit::test]
fn a_table_keeps_its_rows_in_its_card_when_the_page_and_table_scroll(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    // 760 × 560 at 20 px with the column folded.
    cx.simulate_window_resize(handle, size(px(600.), px(420.)));
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        freshkube_ui::layout_check::settle_header(window, cx);
    })
    .unwrap();
    let table = alerts_table(cx, &page);
    let (list, row) = cx.read(|cx| table.read(cx).test_ids());
    let down = |delta: f32| ScrollDelta::Pixels(point(px(0.), px(-delta)));
    cx.update_window(handle, |_, window, cx| {
        // The page to its end, then the wheel goes on over the table, as a
        // trackpad's does once the page stops.
        wheel(window, down(40_000.), cx);
        freshkube_ui::layout_check::settle_header(window, cx);
        for _ in 0..3 {
            window.scroll(list.clone(), down(200.), cx);
            window.simulate_next_frame(cx);
        }
        window.scroll(
            list.clone(),
            ScrollDelta::Pixels(point(px(0.), px(200.))),
            cx,
        );
        window.simulate_next_frame(cx);
        let list = window.find(list.clone()).bounds();
        let row = window.find(row.clone()).bounds();
        // Scrolled, the first row may start above the list; never below
        // its top, which would leave the rows' room blank.
        assert!(
            row.top() <= list.top() + px(0.5),
            "the first row {row:?} starts below its list {list:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_firing_alerts_table_takes_the_shared_rows(cx: &mut TestAppContext) {
    use freshkube_ui::layout_check::{Table, assert_table};
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let table = alerts_table(cx, &page);
    let (list, _) = cx.read(|cx| table.read(cx).test_ids());
    let scroll = list.replace("-list", "-table-scroll");
    let table = Table {
        table: Some(Box::leak(scroll.into_boxed_str())),
        list: Box::leak(list.to_string().into_boxed_str()),
    };
    for text_size in [crate::ui::BASE_TEXT, 20.] {
        cx.update_window(handle, |_, _, cx| crate::text_size::set(text_size, cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            // Bring the panel into view, wherever the grid puts it; at 20 px
            // the window is short and the page scrolls instead of the grid.
            let top = window.find(table.list).bounds().top();
            window.scroll(
                "monitoring-page",
                ScrollDelta::Pixels(point(px(0.), px(200.) - top)),
                cx,
            );
            let rows = assert_table(window, cx, &table);
            assert!(rows.header.is_some(), "{rows:#?}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn held_example_data_leaves_the_table_on_its_loading_rows_with_their_motion(
    cx: &mut TestAppContext,
) {
    let held = ClusterSource {
        access: ClusterAccess::Example(Hold {
            monitoring: true,
            ..Hold::NONE
        }),
        ..example_source()
    };
    let (_runtime, handle, page) = mount(cx, Some(held));
    show(cx, handle, &page);
    assert!(ready(cx, &page).is_empty());
    let key = cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        board
            .slots
            .iter()
            .find(|slot| slot.spec.title == "Firing alerts")
            .map(|slot| slot.spec.key)
            .unwrap()
    });
    let loading = format!("monitoring-panel-{key}-table-loading");
    let motions = |cx: &mut TestAppContext| {
        let before = probe::count("table.loading-motion");
        frame(cx, handle);
        probe::count("table.loading-motion") - before
    };
    assert!(shown(cx, handle, &loading));
    // The page mounts the motion beside the panel, as the cursor's overlay.
    assert_eq!(motions(cx), 1);

    // Once the table answers, the motion goes with its rows.
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            // The same example source, no longer held, keeps the board.
            page.set_source(Some(example_source()), cx);
            page.answer_example_now(cx)
        })
    });
    cx.run_until_parked();
    assert!(!shown(cx, handle, &loading));
    assert_eq!(motions(cx), 0);
}
