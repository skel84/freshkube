use super::*;
use crate::DebugOptions;

#[gpui_kit::test]
fn nodes_matches_the_shared_table_layout_at_default_and_large_text(cx: &mut TestAppContext) {
    use crate::desktop::layout_check::{self, TablePage};
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let before = pilot.read(cx).selected_node.clone();
        for text in [14., 20.] {
            crate::text_size::set(text, cx);
            window.render_frame(cx);
            layout_check::assert_table_page(
                window,
                cx,
                &TablePage {
                    page: "nodes-page",
                    title: "nodes-title",
                    title_text: "Nodes",
                    table: "nodes-table-scroll",
                    list: "nodes-list",
                },
            );
            assert_eq!(pilot.read(cx).selected_node, before);
            assert!(!pilot.read(cx).node_workspace.read(cx).open);
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn nodes_columns_preserve_row_identity(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        crate::desktop::tests::expand_healthy_nodes(window, cx);
        assert_eq!(
            window.find("node-talos-cp-fra1-01").bounds().size.height,
            crate::ui::dp_px(26., window)
        );
        window.click("nodes-columns", cx);
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace.read(cx);
        let before_columns = nodes.columns.len();
        let services_menu = nodes
            .menu_columns
            .iter()
            .position(|(field, _)| *field == super::super::table::Field::Services)
            .unwrap();
        assert!(window.try_find(("nodes-sort", 8usize)).is_some());
        window.within("popup-menu").click(services_menu, cx);
        window.render_frame(cx);
        // The menu stays open for another column; the toolbar stays while a
        // node is open, so close it before clicking past it.
        window.click("nodes-columns", cx);
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
        assert!(window.try_find(("nodes-sort", 8usize)).is_none());
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .read(cx)
                .hidden_columns
                .contains(&super::super::table::Field::Services)
        );
        assert_eq!(
            pilot.read(cx).node_workspace.read(cx).columns.len(),
            before_columns - 1
        );
        window.click("node-talos-cp-fra1-01", cx);
        window.render_frame(cx);
        let key = pilot.read(cx).node_workspace.read(cx).selected.clone();
        assert_eq!(
            pilot.read(cx).selected_node.as_deref(),
            Some("talos-cp-fra1-01")
        );
        window.click("node-close", cx);
        window.render_frame(cx);
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        assert_eq!(window.find("node-talos-cp-fra1-01").selected(), Some(true));
        window.click("nodes-view-table", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.read(cx).selected, key);
        assert_eq!(window.find("node-talos-cp-fra1-01").selected(), Some(true));
        assert!(window.try_find(("nodes-sort", 8usize)).is_none());
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .read(cx)
                .hidden_columns
                .contains(&super::super::table::Field::Services)
        );
        assert_eq!(
            pilot.read(cx).node_workspace.read(cx).columns.len(),
            before_columns - 1
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn minimum_nodes_table_keeps_shared_padding_and_scaled_rows(cx: &mut TestAppContext) {
    use crate::desktop::layout_check::{self, TablePage};
    let (_runtime, handle, _pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        crate::text_size::set(20., cx);
        crate::desktop::tests::settle_header(window, cx);
        layout_check::assert_table_page(
            window,
            cx,
            &TablePage {
                page: "nodes-page",
                title: "nodes-title",
                title_text: "Nodes",
                table: "nodes-table-scroll",
                list: "nodes-list",
            },
        );
        for id in [
            "nodes-view-cards",
            "nodes-view-table",
            "nodes-filter",
            "nodes-tally-failing",
            "nodes-tally-warning",
            "nodes-tally-unknown",
            "nodes-tally-healthy",
            "nodes-columns",
            "nodes-refresh",
        ] {
            let control = window.find(id);
            assert!(control.visible(), "{id} is clipped");
            assert!(
                control.bounds().right() <= window.find("nodes-page").bounds().right(),
                "{id} extends beyond the page"
            );
        }
        // The chips take their own row, which leaves room for every control.
        assert!(window.try_find("nodes-more").is_none());
        assert!(
            window.find("nodes-chips").bounds().top()
                >= window.find("nodes-toolbar").bounds().bottom()
        );
    })
    .unwrap();
}

/// Scrolled sideways, a node's glyph and name stay at the table's left edge
/// while the other columns pass under them, and each id still finds one
/// element: the name is moved, never copied. A pinned name still opens its
/// node, and the selection keeps its key.
#[gpui_kit::test]
fn a_sideways_scroll_keeps_each_node_name_in_view_once(cx: &mut TestAppContext) {
    use gpui_kit::{ElementId, ScrollDelta, point, px};
    // Narrow enough to scroll sideways, tall enough for several rows.
    let (_runtime, handle, pilot) = fixture(cx, 760., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        // At text size 20 the healthy group's toggle is below the fold.
        crate::desktop::tests::expand_healthy_nodes(window, cx);
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        let rows: Vec<(ElementId, String)> = (pilot.read(cx).node_workspace.read(cx).rows.iter())
            .map(|row| (row.id.clone().into(), row.name.to_string()))
            .collect();
        let shown: Vec<_> = (rows.into_iter())
            .filter(|(id, _)| window.try_find(id.clone()).is_some())
            .collect();
        assert!(shown.len() >= 2, "{shown:?}");
        let left = |window: &mut gpui_kit::Window, row: &ElementId| {
            window
                .within(row.clone())
                .find("node-row-name")
                .bounds()
                .left()
        };
        let names: Vec<_> = shown.iter().map(|(row, _)| left(window, row)).collect();
        let header = window.find(("nodes-sort", 1usize)).bounds().left();
        // Column 2 starts at the pinned run's edge, where its label stays
        // once scrolled; column 3 starts clear of the scroll's 120.
        let edge = window.find(("nodes-sort", 2usize)).bounds().left();
        let role = window.find(("nodes-sort", 3usize)).bounds().left();
        assert!(role - edge > px(120.), "{:?}", role - edge);
        window.scroll(
            "nodes-table-scroll",
            ScrollDelta::Pixels(point(px(-120.), px(0.))),
            cx,
        );
        window.render_frame(cx);
        let moved = role - window.find(("nodes-sort", 3usize)).bounds().left();
        assert!((f32::from(moved) - 120.).abs() <= 1.5, "{moved:?}");
        // `find` fails on an id that resolves twice.
        assert!((window.find(("nodes-sort", 1usize)).bounds().left() - header).abs() <= px(1.5));
        for ((row, _), name) in shown.iter().zip(names) {
            assert!((left(window, row) - name).abs() <= px(1.5));
        }
        let (row, name) = shown[0].clone();
        window.within(row).click("node-row-name", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.read(cx).open);
        assert_eq!(pilot.read(cx).selected_node.as_deref(), Some(name.as_str()));
    })
    .unwrap();
}

#[gpui_kit::test]
fn default_columns_fit_without_sideways_scroll_when_healthy_is_folded_or_expanded(
    cx: &mut TestAppContext,
) {
    use freshkube_ui::table::TableColumn;
    use gpui_kit::{SharedString, TextRun, font, px};
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-nodes", cx);
        for text in [13., 14.] {
            crate::text_size::set(text, cx);
            window.render_frame(cx);
            for expanded in [false, true] {
                if pilot.read(cx).node_workspace.read(cx).healthy_collapsed() == expanded {
                    window.click("nodes-healthy-toggle", cx);
                    window.render_frame(cx);
                }
                let viewport = window.find("nodes-table-scroll").bounds();
                let last = window.find(("nodes-sort", 8usize)).bounds();
                assert!(last.right() <= viewport.right() + px(1.),
                    "Services overflows at text {text}, expanded={expanded}: {last:?} in {viewport:?}");
                let nodes = &pilot.read(cx).node_workspace.read(cx);
                assert!(crate::ui::dp_px(nodes.table_width, window) <= viewport.size.width,
                    "column width {} exceeds viewport {:?}", nodes.table_width, viewport);
                let name = window.find(("nodes-sort", 1usize)).bounds();
                for row in nodes.rows.iter() {
                    let resources = nodes.resource_cells.get(&row.key).unwrap();
                    for (label, value) in [("CPU", resources.cpu.text.as_ref()), ("Memory", resources.memory.text.as_ref())] {
                        let column = nodes.columns.iter().find(|column| column.label() == label).unwrap();
                        assert_eq!(column.width(), 124.);
                        let run = TextRun {len: value.len(), font: font(crate::ui::MONO_FONT),
                            color: Default::default(), background_color: None, underline: None, strikethrough: None};
                        let shaped = window.text_system().shape_line(SharedString::from(value.to_owned()),
                            crate::ui::dp_px(12.5, window), &[run], None).width;
                        assert!(shaped <= crate::ui::dp_px(48., window),
                            "{label} truncates {value} at text {text}");
                    }
                    if row.name.len() <= 16 {
                        assert!(crate::ui::dp_px(row.name.len() as f32 * 7.5 + 24., window) <= name.size.width);
                    }
                }
                let row = nodes.rows.iter().find(|row| row.name == "talos-wk-fra1-02").unwrap();
                let status = window.find((row.id.clone(), super::super::table::Field::Services as usize));
                assert!(status.visible());
                assert_eq!(status.role(), Some(gpui_kit::Role::Status));
                assert_eq!(status.label(), Some(row.service_status.label.as_ref()));
                assert!(status.bounds().right() <= viewport.right() + px(1.));
                assert_eq!(row.table_load.split('·').count(), 3);
            }
        }
    }).unwrap();
}

#[gpui_kit::test]
fn a_short_window_scrolls_the_frame_and_keeps_the_list_usable(cx: &mut TestAppContext) {
    use freshkube_ui::page::{PANE_PADDING_Y, SHORT_LIST_HEIGHT};
    use gpui_kit::{point, px};
    // At 560 high the header and a usable list fit; at 480 they don't.
    let (_runtime, handle, pilot) = fixture(cx, 760., 480.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        crate::text_size::set(20., cx);
        crate::desktop::tests::settle_header(window, cx);
        // The table's frame, header and legend run to the end of the
        // scrolled page; its hairlines sit outside the measured scroll.
        let least = crate::ui::dp_px(SHORT_LIST_HEIGHT, window) - px(2.);
        let scroll = pilot.read(cx).node_workspace.read(cx).page_scroll.clone();
        assert!(scroll.max_offset().y > px(0.), "the frame doesn't scroll");
        let end = window.find("nodes-page").bounds().bottom() + scroll.max_offset().y;
        let table = window.find("nodes-table-scroll").bounds();
        assert!(
            end - table.top() >= least,
            "the table is squeezed to {table:?}"
        );
        assert!(window.find("nodes-list").bounds().size.height > px(0.));

        // Opening a node scrolls the frame down to its inspector, and
        // closing it starts the frame at the top again.
        scroll.set_offset(point(px(0.), px(0.)));
        pilot.update(cx, |pilot, cx| {
            let key = pilot.node_workspace.read(cx).rows[0].key.clone();
            pilot.open_node(key, window, cx);
        });
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(
            scroll.max_offset().y > px(0.),
            "the open frame doesn't scroll"
        );
        assert_eq!(scroll.offset().y, -scroll.max_offset().y);
        scroll.set_offset(point(px(0.), -scroll.max_offset().y));
        pilot.update(cx, |pilot, cx| pilot.close_node(window, cx));
        window.render_frame(cx);
        assert_eq!(scroll.offset().y, px(0.));

        // The cards and their legend keep it too, inside their inset.
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        let end = window.find("nodes-page").bounds().bottom() + scroll.max_offset().y;
        let cards = window.find("nodes-cards").bounds();
        let inset = crate::ui::dp_px(2. * PANE_PADDING_Y, window);
        assert!(
            end - cards.top() >= least - inset,
            "the cards are squeezed to {cards:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_tall_window_keeps_the_frame_still_and_the_table_filling_it(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let scroll = pilot.read(cx).node_workspace.read(cx).page_scroll.clone();
        assert_eq!(scroll.max_offset().y, px(0.));
        // The legend sits under the rows, at the page's foot.
        let page = window.find("nodes-page").bounds();
        let table = window.find("nodes-table-scroll").bounds();
        let list = window.find("nodes-list").bounds();
        assert!(
            list.bottom() <= table.bottom() + px(0.5),
            "{list:?} in {table:?}"
        );
        assert!(
            page.bottom() - table.bottom() < crate::ui::dp_px(60., window),
            "{table:?} in {page:?}"
        );
    })
    .unwrap();
}

/// The log rows drawn at least in part inside `bounds`, by their ids: the
/// view's generation, then each shown row's id.
fn log_rows_within(
    window: &mut gpui_kit::Window,
    cx: &gpui_kit::App,
    logs: &gpui_kit::Entity<crate::logs::LogPanel>,
    bounds: gpui_kit::Bounds<gpui_kit::Pixels>,
) -> usize {
    use gpui_kit::SharedString;
    let logs = logs.read(cx);
    let ids: Vec<u64> = (logs.visible_rows().iter())
        .map(|&ix| logs.row_id(ix))
        .collect();
    let mut rows = 0;
    for generation in 0..16 {
        for id in &ids {
            let id = SharedString::from(format!("log-line-{generation}-{id}"));
            if window
                .try_find(id)
                .is_some_and(|row| row.bounds().intersects(&bounds))
            {
                rows += 1;
            }
        }
    }
    rows
}

/// Stacked under the table at 760 and text size 20, Talos' toolbar takes
/// most of the inspector (#233), so the Logs body scrolls inside it: first
/// the controls, then at least three log lines. Expand is the way to more.
#[gpui_kit::test]
fn stacked_node_logs_scroll_their_body_to_the_controls_then_the_lines(cx: &mut TestAppContext) {
    use gpui_kit::{point, px};
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        pilot.update(cx, |pilot, cx| {
            pilot.startup_selection(&DebugOptions::default().with_page("node-logs"), window, cx);
        });
        // The log view learns its width while drawing, then lays out again.
        for _ in 0..3 {
            window.render_frame(cx);
        }
        let table = window.find("nodes-table-scroll").bounds();
        let pane = window.find("node-inspector").bounds();
        assert!(
            pane.top() >= table.bottom() - px(1.),
            "{pane:?} under {table:?}"
        );
        let content = window.find("node-inspector-content").bounds();
        let body = pilot.read(cx).node_workspace.read(cx).logs_scroll.clone();
        assert!(
            body.max_offset().y > px(0.),
            "the stacked log body doesn't scroll"
        );

        // At the top, the toolbar's controls show whole in the inspector.
        let toolbar = window.find("logs-toolbar").bounds();
        for id in ["logs-collection", "logs-search", "logs-follow"] {
            let control = window.find(id).bounds();
            assert!(
                control.top() >= toolbar.top() && control.bottom() <= toolbar.bottom(),
                "{id} at {control:?} is clipped by the toolbar at {toolbar:?}"
            );
            let shown = window.find(id).bounds();
            if shown.bottom() > content.bottom() {
                // Lower controls come into view as the body scrolls.
                let by = content.bottom() - shown.bottom();
                body.set_offset(point(px(0.), body.offset().y + by));
                window.render_frame(cx);
            }
            let shown = window.find(id).bounds();
            assert!(
                shown.top() >= content.top() - px(0.5)
                    && shown.bottom() <= content.bottom() + px(0.5),
                "{id} at {shown:?} can't be scrolled into {content:?}"
            );
        }

        // Scrolled on to the log, at least three of its lines show.
        let viewport = window.find("logs-viewport").bounds();
        let by = (content.top() - viewport.top()).max(-body.max_offset().y - body.offset().y);
        body.set_offset(point(px(0.), body.offset().y + by));
        window.render_frame(cx);
        let logs = pilot.read(cx).logs.clone();
        // Its entries wrap, so count lines of text, 1.5 rem each.
        let shown = window.find("logs-viewport").bounds().intersect(&content);
        let lines = shown.size.height / (window.rem_size() * 1.5);
        assert!(lines >= 3., "{lines} log lines show in {content:?}");
        assert!(
            log_rows_within(window, cx, &logs, shown) > 0,
            "no log rows in {shown:?}"
        );

        // A wheel over the log scrolls the log, not the body or the frame.
        let frame = pilot.read(cx).node_workspace.read(cx).page_scroll.clone();
        let (scrolled, framed) = (body.offset().y, frame.offset().y);
        // At the part of the log that shows; its centre is below the fold.
        wheel(window, shown.center(), 120., cx);
        assert_eq!(
            body.offset().y,
            scrolled,
            "the log's wheel scrolled the body"
        );
        assert_eq!(
            frame.offset().y,
            framed,
            "the log's wheel scrolled the frame"
        );
        assert!(!logs.read(cx).following(), "the wheel didn't reach the log");

        // Another tab starts the body at the top again.
        pilot.update(cx, |pilot, cx| {
            pilot.show_node_tab(NodeTab::Overview, window, cx);
            pilot.show_node_tab(NodeTab::Logs, window, cx);
        });
        window.render_frame(cx);
        assert_eq!(body.offset().y, px(0.));

        // Expanded, the inspector fills the page and the log has more room.
        let stacked = content.size.height;
        assert!(frame.offset().y < px(0.), "the frame isn't scrolled down");
        window.click("node-expand", cx);
        for _ in 0..3 {
            window.render_frame(cx);
        }
        assert!(window.try_find("nodes-table-scroll").is_none());
        let expanded = window.find("node-inspector-content").bounds().size.height;
        assert!(
            expanded > stacked,
            "expanded {expanded:?}, stacked {stacked:?}"
        );
        // From the top of the frame, so its heading and Collapse show.
        assert_eq!(frame.offset().y, px(0.));
        let page = window.find("nodes-page").bounds();
        let title = window.find("node-pane-title").bounds();
        assert!(title.top() >= page.top(), "{title:?} above {page:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn node_logs_in_a_tall_window_keep_the_frame_still(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.startup_selection(&DebugOptions::default().with_page("node-logs"), window, cx);
        });
        for _ in 0..3 {
            window.render_frame(cx);
        }
        let frame = pilot.read(cx).node_workspace.read(cx).page_scroll.clone();
        assert_eq!(frame.max_offset().y, px(0.));
        // Nor does the log's body scroll inside the inspector.
        let body = pilot.read(cx).node_workspace.read(cx).logs_scroll.clone();
        assert_eq!(body.max_offset().y, px(0.));
        let pane_bounds = window.find("node-pane").bounds();
        let viewport = window.find("logs-viewport").bounds();
        assert!(
            viewport.bottom() <= pane_bounds.bottom() + px(0.5),
            "{viewport:?} in {pane_bounds:?}"
        );
    })
    .unwrap();
}

/// At 760 wide and the largest text, the cards are one to a row, as wide as
/// the list.
#[gpui_kit::test]
fn a_narrow_window_at_large_text_shows_one_card_to_a_row(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        crate::text_size::set(20., cx);
        crate::desktop::tests::settle_header(window, cx);
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        assert_eq!(super::super::cards::card_columns(window), 1);
        let list = window.find("nodes-cards").bounds();
        let first = pilot.read(cx).node_workspace.read(cx).lines[0];
        let first = pilot.read(cx).node_workspace.read(cx).rows[first]
            .id
            .clone();
        let card = window.find(first).bounds();
        assert_eq!(card.left(), list.left());
        assert_eq!(card.right(), list.right());
    })
    .unwrap();
}

/// Beside the inspector at 1280 the table is narrower than its columns: it
/// scrolls sideways with its header, and each row's glyph and name stay
/// at its left edge, drawn once.
#[gpui_kit::test]
fn the_table_beside_the_inspector_scrolls_sideways_with_its_names_pinned(cx: &mut TestAppContext) {
    use gpui_kit::{ElementId, ScrollDelta, point, px};
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first_node(window, cx);
        let table = window.find("nodes-table-scroll").bounds();
        let columns = pilot.read(cx).node_workspace.read(cx).table_width;
        assert!(
            crate::ui::dp_px(columns, window) > table.size.width,
            "the table fits its columns in {table:?}"
        );
        let rows: Vec<ElementId> = (pilot.read(cx).node_workspace.read(cx).rows.iter())
            .map(|row| row.id.clone().into())
            .filter(|id: &ElementId| window.try_find(id.clone()).is_some())
            .collect();
        assert!(rows.len() >= 2, "{rows:?}");
        let name = |window: &mut gpui_kit::Window, row: &ElementId| {
            window
                .within(row.clone())
                .find("node-row-name")
                .bounds()
                .left()
        };
        let names: Vec<_> = rows.iter().map(|row| name(window, row)).collect();
        let header = window.find(("nodes-sort", 1usize)).bounds().left();
        let role = window.find(("nodes-sort", 3usize)).bounds().left();
        window.scroll(
            "nodes-table-scroll",
            ScrollDelta::Pixels(point(px(-120.), px(0.))),
            cx,
        );
        window.render_frame(cx);
        let moved = role - window.find(("nodes-sort", 3usize)).bounds().left();
        assert!((f32::from(moved) - 120.).abs() <= 1.5, "{moved:?}");
        // `find` fails on an id that resolves twice.
        assert!((window.find(("nodes-sort", 1usize)).bounds().left() - header).abs() <= px(1.5));
        for (row, left) in rows.iter().zip(names) {
            assert!((name(window, row) - left).abs() <= px(1.5));
            assert!(name(window, row) >= table.left());
        }
        assert!(window.find("node-inspector").visible());
    })
    .unwrap();
}
