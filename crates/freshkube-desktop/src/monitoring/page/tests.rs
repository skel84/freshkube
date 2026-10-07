use freshkube_core::cluster_source::{ClusterAccess, ClusterSource, KubeClientSource};
use freshkube_core::monitoring::{
    Candidate, Discovery, ErrorKind, PrometheusService, QueryError, Rank, Tried,
};
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Pixels, Point, ScrollDelta, SharedString,
    TestAppContext, point, px, size,
};

use super::*;
use crate::desktop::probe;

const NOW: i64 = 1_700_003_600;

fn example_source() -> ClusterSource {
    ClusterSource {
        id: "example".into(),
        context: "prod-fra".into(),
        access: ClusterAccess::Example,
    }
}

/// The page alone in a window, its clock fixed and its source example data.
fn mount(
    cx: &mut TestAppContext,
    source: Option<ClusterSource>,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<MonitoringPage>,
) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (handle, page) = mount_with(cx, &runtime, source, None, None);
    (runtime, handle, page)
}

fn mount_with(
    cx: &mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
    source: Option<ClusterSource>,
    preferences: Option<&std::path::Path>,
    secrets: Option<crate::secrets::Secrets>,
) -> (AnyWindowHandle, Entity<MonitoringPage>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.set_reduce_motion(true);
    });
    let mut page = None;
    let window = cx.open_window(size(px(1200.), px(900.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view =
                MonitoringPage::new(runtime.handle().clone(), preferences, secrets.clone(), cx);
            view.now = || NOW;
            view.set_source(source, cx);
            view
        });
        page = Some(view.clone());
        // Uncached, as the shell holds it.
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (window.into(), page.unwrap())
}

fn frame(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

/// The wheel over the top of the grid, where it shows on a short page whose
/// grid runs below the window; `TestWindowExt::scroll` aims at its centre.
fn wheel(window: &mut gpui_kit::Window, delta: ScrollDelta, cx: &mut gpui_kit::App) {
    window.render_frame(cx);
    let grid = window.find("monitoring-grid").bounds();
    let position = point(grid.center().x, grid.top() + px(20.));
    window.dispatch_event(
        gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers: Default::default(),
        }),
        cx,
    );
    window.dispatch_event(
        gpui_kit::PlatformInput::ScrollWheel(gpui_kit::ScrollWheelEvent {
            position,
            delta,
            ..Default::default()
        }),
        cx,
    );
    window.render_frame(cx);
}

fn shown(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) -> bool {
    let id = SharedString::from(id.to_owned());
    cx.update_window(handle, move |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

fn show(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<MonitoringPage>) {
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(true, cx)));
    cx.run_until_parked();
    frame(cx, handle);
}

/// The slots answered, by title.
fn ready(cx: &mut TestAppContext, page: &Entity<MonitoringPage>) -> Vec<String> {
    cx.read(|cx| {
        let page = page.read(cx);
        page.board
            .as_ref()
            .map(|board| {
                board
                    .slots
                    .iter()
                    .filter(|slot| slot.view.read(cx).is_ready())
                    .map(|slot| slot.spec.title.clone())
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// Where slot `n` is drawn: its panel, or the empty card in its place.
fn slot_bounds(window: &mut gpui_kit::Window, n: usize) -> Bounds<Pixels> {
    window
        .try_find(SharedString::from(format!("monitoring-panel-{n}")))
        .unwrap_or_else(|| window.find(format!("monitoring-placeholder-{n}")))
        .bounds()
}

/// Opens the stress run's 30 panels from a folder of its own, which the
/// caller removes.
fn open_thirty(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    page: &Entity<MonitoringPage>,
) -> std::path::PathBuf {
    let folder = std::env::temp_dir().join(format!(
        "freshkube-monitoring-thirty-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("thirty.json"),
        include_str!("../../bin/stress/dashboards/thirty.json"),
    )
    .unwrap();
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.saved.folder = Some(folder.clone());
            page.read_folder(cx);
        })
    });
    cx.run_until_parked();
    let id = cx.read(|cx| match &page.read(cx).catalog.folder {
        FolderState::Read { entries, .. } => entries[0].id.clone(),
        _ => panic!("the folder wasn't read"),
    });
    show(cx, handle, page);
    cx.update(|cx| page.update(cx, |page, cx| page.open(id, cx)));
    cx.run_until_parked();
    // The header folds from what it measured on the frame before; a page
    // that never stops asking for frames is the caller's to find.
    cx.update_window(handle, |_, window, cx| {
        for _ in 0..4 {
            window.render_frame(cx);
            window.simulate_next_frame(cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(slot_count(cx, page), 30);
    folder
}

fn slot_count(cx: &mut TestAppContext, page: &Entity<MonitoringPage>) -> usize {
    cx.read(|cx| {
        page.read(cx)
            .board
            .as_ref()
            .map_or(0, |board| board.slots.len())
    })
}

#[gpui_kit::test]
fn a_hidden_page_reads_nothing_and_showing_answers_the_panels_in_view(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    cx.read(|cx| {
        let page = page.read(cx);
        assert!(page.board.is_none(), "a hidden page opened a dashboard");
        assert!(matches!(page.connection, Connection::None));
    });

    show(cx, handle, &page);
    assert!(shown(cx, handle, "monitoring-title"));
    assert!(shown(cx, handle, "monitoring-variable-node"));
    assert!(shown(cx, handle, "monitoring-variable-namespace"));
    let answered = ready(cx, &page);
    assert!(answered.contains(&"Ready nodes".to_owned()), "{answered:?}");
    assert!(
        answered.contains(&"CPU usage by node".to_owned()),
        "{answered:?}"
    );
    assert_eq!(answered.len(), slot_count(cx, &page));
}

#[gpui_kit::test]
fn the_page_takes_the_shared_frame_and_its_last_panel_ends_at_the_padding(cx: &mut TestAppContext) {
    use crate::desktop::layout_check::{PageFrame, assert_page_frame_from};
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let slots = slot_count(cx, &page);
    cx.update_window(handle, |_, window, cx| {
        // The header leads with the breadcrumb's "Dashboards".
        assert_page_frame_from(
            window,
            cx,
            &PageFrame {
                page: "monitoring-page",
                title: "monitoring-title",
                title_text: "Cluster",
                content: "monitoring-grid",
            },
            "monitoring-dashboards",
        );
        // The grid draws the gap between panels, so the rightmost one ends
        // where the grid does, not a gap short of it.
        let grid = window.find("monitoring-grid").bounds();
        let right = (0..slots)
            .filter_map(|index| {
                window.try_find(SharedString::from(format!("monitoring-panel-{index}")))
            })
            .map(|panel| panel.bounds().right())
            .fold(
                px(0.),
                |right, edge| if edge > right { edge } else { right },
            );
        assert!(
            (right - grid.right()).abs() < px(0.5),
            "the last panel ends at {right:?}, the grid at {:?}",
            grid.right()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_narrow_header_folds_the_controls_beside_the_breadcrumb(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    // The page's width at 760 × 560 and 20 px, beside the rail and column.
    cx.simulate_window_resize(handle, size(px(560.), px(560.)));
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        crate::desktop::tests::settle_header(window, cx);
        let toolbar = window.find("monitoring-toolbar").bounds();
        let title = window.find("monitoring-title").bounds();
        let more = window.find("monitoring-more").bounds();
        // The controls fold into "…" on the title's row instead of taking a
        // row of their own or running over the title.
        assert!(window.try_find("monitoring-range").is_none());
        assert!(window.try_find("monitoring-auto-refresh").is_none());
        assert!(
            more.bottom() <= toolbar.bottom(),
            "{more:?} under {toolbar:?}"
        );
        assert!(
            more.right() <= toolbar.right() + px(1.),
            "{more:?} past {toolbar:?}"
        );
        assert!(title.right() < more.left(), "{title:?} meets {more:?}");
        // Where the data comes from is on the meta line, not the toolbar.
        let scope = window.find("monitoring-scope").bounds();
        let source = window.find("monitoring-source").bounds();
        assert!(scope.contains(&source.center()), "{source:?} off {scope:?}");
    })
    .unwrap();
    // Wide, nothing folds and the controls share the title's row.
    cx.simulate_window_resize(handle, size(px(1200.), px(900.)));
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(13., cx);
        crate::desktop::tests::settle_header(window, cx);
        assert!(window.try_find("monitoring-more").is_none());
        let toolbar = window.find("monitoring-toolbar").bounds();
        let title = window.find("monitoring-title").bounds();
        let refresh = window.find("monitoring-refresh").bounds();
        assert!(
            refresh.bottom() <= toolbar.bottom(),
            "{refresh:?} under {toolbar:?}"
        );
        assert!(
            title.right() < refresh.left(),
            "{title:?} meets {refresh:?}"
        );
        assert!(
            window
                .find("monitoring-scope")
                .bounds()
                .contains(&window.find("monitoring-source").bounds().center())
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_folded_time_range_picks_a_range_as_its_picker_does(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let (ranges, current) = cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        (board.ranges.clone(), board.span)
    });
    let (index, (span, _)) = ranges
        .iter()
        .enumerate()
        .find(|(_, (span, _))| *span != current)
        .expect("another range");
    cx.simulate_window_resize(handle, size(px(560.), px(560.)));
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        crate::desktop::tests::settle_header(window, cx);
        window.click("monitoring-more", cx);
        window.render_frame(cx);
        // Time range, Refresh, Auto-refresh: the controls' order.
        window.within("popup-menu").click(0usize, cx);
        window.render_frame(cx);
        window
            .within("submenu")
            .within("popup-menu")
            .click(index, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| page.read(cx).board.as_ref().unwrap().span),
        *span
    );
}

/// The variables and the annotation toggles are the header's second row,
/// under the toolbar and above the meta line, at the toolbar's control
/// height.
#[gpui_kit::test]
fn the_variables_are_the_headers_second_row(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    cx.update_window(handle, |_, window, cx| {
        crate::desktop::tests::settle_header(window, cx);
        let toolbar = window.find("monitoring-toolbar").bounds();
        let row = window.find("monitoring-secondary").bounds();
        let meta = window.find("monitoring-scope").bounds();
        assert!(
            row.top() >= toolbar.bottom() - px(0.5),
            "{row:?} over {toolbar:?}"
        );
        assert!(
            meta.top() >= row.bottom() - px(0.5),
            "{meta:?} over {row:?}"
        );
        let height = crate::ui::dp_px(crate::ui::CONTROL_HEIGHT, window);
        for id in [
            "monitoring-variable-node",
            "monitoring-variable-namespace",
            "monitoring-markers-deploys",
            "monitoring-markers-nodes",
        ] {
            let bounds = window.find(id).bounds();
            assert!(
                row.contains(&bounds.center()),
                "{id} {bounds:?} outside {row:?}"
            );
            assert!(
                (bounds.size.height - height).abs() < px(0.5),
                "{id} is {:?} high, not {height:?}",
                bounds.size.height
            );
        }
        assert_eq!(
            window.find("monitoring-variable-node").label(),
            Some("node: All")
        );
    })
    .unwrap();
}

/// Auto-refresh says what it does: off, or its interval.
#[gpui_kit::test]
fn auto_refresh_names_itself_and_its_interval(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let label = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, _| {
            window
                .find("monitoring-auto-refresh")
                .label()
                .map(str::to_owned)
        })
        .unwrap()
    };
    assert_eq!(label(cx).as_deref(), Some("Auto-refresh off"));
    cx.update(|cx| page.update(cx, |page, cx| page.set_refresh(Some(30), cx)));
    frame(cx, handle);
    assert_eq!(label(cx).as_deref(), Some("Auto-refresh 30s"));
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(false, cx)));
}

#[gpui_kit::test]
fn the_annotations_stay_inside_a_narrow_page(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    // 760 × 560 at 20 px with the column open leaves the page about 410 px.
    cx.simulate_window_resize(handle, size(px(410.), px(560.)));
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        crate::desktop::tests::settle_header(window, cx);
        let page = window.find("monitoring-page").bounds();
        for id in [
            "monitoring-annotations",
            "monitoring-markers-deploys",
            "monitoring-markers-nodes",
        ] {
            let bounds = window.find(id).bounds();
            assert!(
                bounds.right() <= page.right() + px(0.5),
                "{id} {bounds:?} past {page:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_meta_line_counts_the_panels_and_says_when_they_answered(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    assert!(shown(cx, handle, "monitoring-scope"));
    let slots = slot_count(cx, &page);
    let time = super::board::clock(NOW).unwrap();
    let meta = |cx: &mut TestAppContext| -> SharedString {
        cx.read(|cx| page.read(cx).board.as_ref().unwrap().meta.clone())
    };
    assert_eq!(meta(cx), format!("{slots} panels · {time}").as_str());

    cx.update(|cx| {
        page.update(cx, |page, _| {
            let board = page.board.as_mut().unwrap();
            board.slots[0].failed = true;
            assert!(board.derive_meta(), "a failure changes the meta line");
            assert!(!board.derive_meta(), "nothing changed the second time");
        })
    });
    assert_eq!(
        meta(cx),
        format!("{slots} panels · 1 failed · {time}").as_str()
    );
}

#[gpui_kit::test]
fn without_a_source_the_page_says_it_is_not_connected(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, None);
    show(cx, handle, &page);
    assert!(shown(cx, handle, "monitoring-page"));
    cx.read(|cx| assert!(matches!(page.read(cx).connection, Connection::None)));
    assert!(ready(cx, &page).is_empty());
    assert!(!shown(cx, handle, "monitoring-grid"));
}

#[gpui_kit::test]
fn discovery_states_show_what_was_looked_for_and_offer_retry(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let service = PrometheusService::new("monitoring", "metrics", 9090);
    let missing = Discovery::Missing {
        candidates: vec![Candidate {
            service: service.clone(),
            rank: Rank::Named,
        }],
        tried: vec![Tried {
            service: PrometheusService::new("monitoring", "prometheus", 9090),
            error: QueryError::new(ErrorKind::NotFound, "Not found"),
        }],
    };
    cx.update(|cx| page.update(cx, |page, cx| page.discovered(Ok(missing), cx)));
    assert!(shown(cx, handle, "monitoring-candidate-0"));
    assert!(shown(cx, handle, "monitoring-retry"));
    assert!(!shown(cx, handle, "monitoring-grid"));

    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.discovered(Err(QueryError::refused()), cx)
        })
    });
    assert!(matches!(
        cx.read(|cx| page.read(cx).connection_name()),
        "refused"
    ));
    assert!(shown(cx, handle, "monitoring-retry"));

    let failed = QueryError::new(ErrorKind::Unavailable, "Service unavailable");
    cx.update(|cx| page.update(cx, |page, cx| page.discovered(Err(failed), cx)));
    assert_eq!(cx.read(|cx| page.read(cx).connection_name()), "failed");

    // Try again starts over: the example source answers at once.
    cx.update_window(handle, |_, window, cx| window.click("monitoring-retry", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, handle);
    assert_eq!(cx.read(|cx| page.read(cx).connection_name()), "example");
    assert!(shown(cx, handle, "monitoring-grid"));
}

#[gpui_kit::test]
fn a_variable_change_reads_the_variables_and_panels_again(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let (index, before, choice) = cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        let control = board
            .controls
            .iter()
            .find(|control| control.id.as_ref() == "monitoring-variable-node")
            .unwrap();
        (
            control.index,
            control.value.clone(),
            control.options[1].clone(),
        )
    });
    assert_eq!(before.as_ref(), "All");
    let generation = cx.read(|cx| page.read(cx).generation);
    cx.update(|cx| page.update(cx, |page, cx| page.set_variable(index, choice.clone(), cx)));
    cx.run_until_parked();
    frame(cx, handle);
    cx.read(|cx| {
        let page = page.read(cx);
        assert!(page.generation > generation);
        let board = page.board.as_ref().unwrap();
        let control = board.controls.iter().find(|c| c.index == index).unwrap();
        assert_eq!(control.value, choice);
        assert_eq!(control.options[0].as_ref(), "All");
        assert_eq!(
            control
                .options
                .iter()
                .filter(|o| o.as_ref() == "All")
                .count(),
            1
        );
    });
    assert_eq!(ready(cx, &page).len(), slot_count(cx, &page));
}

#[gpui_kit::test]
fn a_time_range_change_asks_every_panel_over_the_new_window(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let (generation, span) = cx.read(|cx| {
        let page = page.read(cx);
        (page.generation, page.board.as_ref().unwrap().span)
    });
    assert_eq!(span, 6 * 3600);
    cx.update(|cx| page.update(cx, |page, cx| page.set_range(3600, cx)));
    cx.run_until_parked();
    frame(cx, handle);
    cx.read(|cx| {
        let page = page.read(cx);
        let board = page.board.as_ref().unwrap();
        assert_eq!(page.generation, generation + 1);
        assert_eq!(board.range_label.as_ref(), "Last 1 hour");
        assert_eq!(board.window.unwrap().span, 3600);
        assert!(
            board
                .slots
                .iter()
                .filter(|slot| slot.view.read(cx).is_ready())
                .all(|slot| slot.asked == Some(page.generation))
        );
    });
}

#[gpui_kit::test]
fn the_cursor_on_one_chart_shows_on_the_others_and_redraws_no_panel(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let (plot, others) = cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        let cpu = board
            .slots
            .iter()
            .position(|slot| slot.spec.title == "CPU usage by node")
            .unwrap();
        let others: Vec<SharedString> = board
            .slots
            .iter()
            .enumerate()
            .filter(|(index, slot)| *index != cpu && slot.view.read(cx).is_chart())
            .map(|(_, slot)| slot.view.read(cx).part("crosshair"))
            .collect();
        (board.slots[cpu].view.read(cx).part("plot"), others)
    });
    assert!(others.len() >= 2, "{others:?}");
    // Real moves and real frames, with the view caches on.
    cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    let move_to = |cx: &mut TestAppContext, at: fn(Bounds<Pixels>) -> Point<Pixels>| {
        let plot = plot.clone();
        cx.update_window(handle, |_, window, cx| {
            let bounds = window.find(plot).bounds();
            window.dispatch_event(
                gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                    position: at(bounds),
                    pressed_button: None,
                    modifiers: Default::default(),
                }),
                cx,
            );
            window.draw(cx).clear(cx);
        })
        .unwrap();
    };
    move_to(cx, |plot| plot.center());
    let counts = || {
        [
            probe::count("monitoring-panel"),
            probe::count("monitoring-page"),
            probe::count("monitoring-derive"),
        ]
    };
    let [panels, pages, derived] = counts();
    move_to(cx, |plot| plot.center() + point(px(40.), px(0.)));
    move_to(cx, |plot| plot.center() + point(px(80.), px(0.)));
    // No panel draws again, not even the one under the pointer: its cursor
    // is a view of its own. The page draws with the shell's frames to place
    // the crosshairs, from what it derived before: a move derives nothing.
    let [panels_now, pages_now, derived_now] = counts();
    assert_eq!(panels_now, panels, "a move redrew a panel");
    assert!(pages_now > pages);
    assert_eq!(derived_now, derived, "a move derived");
    for id in &others {
        assert!(shown(cx, handle, id), "no crosshair on {id}");
    }

    // Leaving the plot, for its card's title, takes them away.
    move_to(cx, |plot| point(plot.center().x, plot.top() - px(16.)));
    for id in &others {
        assert!(!shown(cx, handle, id), "{id} kept its crosshair");
    }
}

#[gpui_kit::test]
fn only_panels_in_reach_draw_and_one_scrolled_in_takes_its_card_place(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    let folder = open_thirty(cx, handle, &page);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Each slot is its panel or an empty card, never both or neither.
        let drawn: Vec<bool> = (0..30)
            .map(|n| {
                let panel = window
                    .try_find(SharedString::from(format!("monitoring-panel-{n}")))
                    .is_some();
                let card = window
                    .try_find(SharedString::from(format!("monitoring-placeholder-{n}")))
                    .is_some();
                assert_ne!(panel, card, "slot {n}");
                panel
            })
            .collect();
        assert!(drawn[0], "the first panel isn't drawn");
        let far = drawn
            .iter()
            .position(|drawn| !drawn)
            .expect("every panel is drawn");
        let grid = window.find("monitoring-grid").bounds();
        let card = window
            .find(format!("monitoring-placeholder-{far}"))
            .bounds();
        let first = slot_bounds(window, 0);
        // Scrolled until the card's top meets the grid's: drawn in the same
        // frame, at the card's place and size, with nothing else moved.
        let by = card.top() - grid.top();
        wheel(window, ScrollDelta::Pixels(point(px(0.), -by)), cx);
        let panel = window.find(format!("monitoring-panel-{far}")).bounds();
        assert_eq!(panel.size, card.size);
        assert!(
            (panel.top() - grid.top()).abs() < px(0.5),
            "{panel:?} after scrolling {by:?} to {grid:?}"
        );
        assert!(
            (slot_bounds(window, 0).top() - (first.top() - by)).abs() < px(0.5),
            "the first slot moved from {first:?} to {:?}",
            slot_bounds(window, 0)
        );
        assert!(
            window.try_find("monitoring-panel-0").is_none(),
            "the first panel is still drawn far above the view"
        );
    })
    .unwrap();
    std::fs::remove_dir_all(&folder).unwrap();
}

#[gpui_kit::test]
fn with_the_panels_in_reach_answered_the_page_asks_for_no_frame(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    // A loading panel pulses only with motion on.
    cx.update(|cx| cx.set_reduce_motion(false));
    let folder = open_thirty(cx, handle, &page);
    let generation = cx.read(|cx| page.read(cx).generation);
    let ready = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let board = page.read(cx).board.as_ref().unwrap();
            // What is drawn is what was asked, and the rest is still loading.
            for (n, slot) in board.slots.iter().enumerate() {
                let drawn = window
                    .try_find(SharedString::from(format!("monitoring-panel-{n}")))
                    .is_some();
                assert_eq!(drawn, slot.asked == Some(generation), "slot {n}");
                assert_eq!(drawn, slot.view.read(cx).is_ready(), "slot {n}");
            }
            board
                .slots
                .iter()
                .filter(|slot| slot.asked.is_some())
                .count()
        })
        .unwrap();
    assert!((1..30).contains(&ready), "{ready} of 30 answered");
    cx.update_window(handle, |_, window, cx| {
        // The header folds from what it measured on the frame before.
        for _ in 0..4 {
            window.draw(cx).clear(cx);
            window.simulate_next_frame(cx);
        }
        let panels = probe::count("monitoring-panel");
        for _ in 0..3 {
            window.draw(cx).clear(cx);
            assert_eq!(window.simulate_next_frame(cx), 0, "a frame is asked for");
        }
        assert_eq!(probe::count("monitoring-panel"), panels);
    })
    .unwrap();
    std::fs::remove_dir_all(&folder).unwrap();
}

#[gpui_kit::test]
fn hiding_mid_request_drops_the_reads_and_showing_asks_again(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    // Show and hide before any read can answer.
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_visible(true, cx);
            assert!(page.board.as_ref().unwrap().resolving.is_some());
            page.set_visible(false, cx);
        })
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let page = page.read(cx);
        let board = page.board.as_ref().unwrap();
        assert!(board.resolving.is_none());
        assert!(board.variables.is_none(), "an answer arrived after hiding");
        assert!(page.refresh_task.is_none());
    });
    assert!(ready(cx, &page).is_empty());

    // Panels asked, then hidden: their reads go and nothing lands.
    show(cx, handle, &page);
    let before = cx.read(|cx| page.read(cx).generation);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.ask_again(cx);
            page.set_visible(false, cx);
            let board = page.board.as_ref().unwrap();
            assert!(board.slots.iter().all(|slot| slot.request.is_none()));
            assert!(board.slots.iter().all(|slot| slot.asked.is_none()));
        })
    });
    cx.run_until_parked();
    assert!(cx.read(|cx| page.read(cx).generation) > before);

    show(cx, handle, &page);
    assert_eq!(ready(cx, &page).len(), slot_count(cx, &page));
    cx.read(|cx| {
        let page = page.read(cx);
        let generation = page.generation;
        assert!(
            page.board
                .as_ref()
                .unwrap()
                .slots
                .iter()
                .all(|slot| slot.asked == Some(generation))
        );
    });
}

#[gpui_kit::test]
fn a_folded_row_shows_its_count_and_unfolding_asks_its_panels(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.open(EntryId::Builtin("freshkube-workloads"), cx)
        })
    });
    cx.run_until_parked();
    frame(cx, handle);
    let network: Vec<String> = vec!["Received by pod".into(), "Sent by pod".into()];
    let answered = ready(cx, &page);
    assert!(answered.contains(&"CPU by pod".to_owned()), "{answered:?}");
    assert!(network.iter().all(|title| !answered.contains(title)));
    let row = cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        let (section, header) = board
            .rows
            .iter()
            .enumerate()
            .find_map(|(index, row)| {
                row.as_ref()
                    .filter(|row| row.title.as_ref() == "Network")
                    .map(|row| (index, row))
            })
            .unwrap();
        assert!(board.is_collapsed(section));
        assert_eq!(header.count.as_ref(), "2 panels");
        header.id.clone()
    });
    cx.update_window(handle, |_, window, cx| window.click(row, cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, handle);
    let answered = ready(cx, &page);
    assert!(
        network.iter().all(|title| answered.contains(title)),
        "{answered:?}"
    );
}

#[gpui_kit::test]
fn a_file_that_is_not_a_dashboard_shows_why(cx: &mut TestAppContext) {
    let folder = std::env::temp_dir().join(format!(
        "freshkube-monitoring-folder-{}",
        std::process::id()
    ));
    _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("broken.json"), "{ not json").unwrap();
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.saved.folder = Some(folder.clone());
            page.read_folder(cx);
        })
    });
    cx.run_until_parked();
    let id = cx.read(|cx| match &page.read(cx).catalog.folder {
        FolderState::Read { entries, .. } => entries[0].id.clone(),
        _ => panic!("the folder wasn't read"),
    });
    show(cx, handle, &page);
    cx.update(|cx| page.update(cx, |page, cx| page.open(id, cx)));
    cx.run_until_parked();
    frame(cx, handle);
    cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        assert!(board.error.is_some());
        assert!(board.slots.is_empty());
    });
    assert!(!shown(cx, handle, "monitoring-grid"));
    assert!(!shown(cx, handle, "monitoring-range"));
    std::fs::remove_dir_all(&folder).unwrap();
}

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

const CPU: &str = "CPU usage by node";

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

#[gpui_kit::test]
fn history_reads_where_the_page_found_or_remembers_prometheus(cx: &mut TestAppContext) {
    use crate::monitoring::history::HistoryKind;
    let (_runtime, _handle, page) = mount(cx, Some(example_source()));
    let kind = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            page.read(cx).history().map(|history| match history.kind {
                HistoryKind::Example => "example".to_owned(),
                HistoryKind::Ready(prometheus) => {
                    format!("ready {}", prometheus.endpoint().label())
                }
                HistoryKind::Remembered { service, .. } => {
                    format!("remembered {}", service.label())
                }
            })
        })
    };
    // Example data answers without the page ever showing.
    assert_eq!(kind(cx).as_deref(), Some("example"));

    // A live context with nothing remembered has none until the page finds one.
    let live = live_source();
    cx.update(|cx| page.update(cx, |page, cx| page.set_source(Some(live.clone()), cx)));
    assert_eq!(kind(cx), None);

    let service = PrometheusService::new("monitoring", "prometheus-operated", 9090);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.saved
                .services
                .insert("prod-ams".into(), service.clone());
            page.set_source(None, cx);
            page.set_source(Some(live), cx);
        })
    });
    assert_eq!(
        kind(cx).as_deref(),
        Some("remembered monitoring/prometheus-operated:9090")
    );

    // Looked for and not found: none, so the panes show nothing.
    let missing = Discovery::Missing {
        candidates: Vec::new(),
        tried: Vec::new(),
    };
    cx.update(|cx| page.update(cx, |page, cx| page.discovered(Ok(missing), cx)));
    assert_eq!(kind(cx), None);
}

/// A live cluster whose client never builds, as a kubeconfig without
/// files: nothing in these tests reaches the cluster.
struct NoClient;

impl KubeClientSource for NoClient {
    fn client(&self) -> futures::future::BoxFuture<'_, Result<kube::Client, String>> {
        Box::pin(async { Err("No kubeconfig".into()) })
    }
}

fn live_source() -> ClusterSource {
    ClusterSource {
        id: "live".into(),
        context: "prod-ams".into(),
        access: ClusterAccess::Live(std::sync::Arc::new(NoClient)),
    }
}

/// A Prometheus API on loopback that wants `Bearer s3cret` and answers
/// only the confirming query.
fn metrics_server(runtime: &tokio::runtime::Runtime) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let address = listener.local_addr().unwrap();
    runtime.spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buffer = vec![0; 16 * 1024];
                let mut read = 0;
                while !buffer[..read].windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buffer[read..]).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => read += n,
                    }
                }
                let head = String::from_utf8_lossy(&buffer[..read]).to_lowercase();
                let target = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                let (status, body) = if !head.contains("authorization: bearer s3cret") {
                    (401, "unauthorized".to_owned())
                } else if target == "/vm/api/v1/query?query=1" {
                    (
                        200,
                        r#"{"status":"success","data":{"resultType":"scalar","result":[1,"1"]}}"#
                            .to_owned(),
                    )
                } else {
                    (404, "not found".to_owned())
                };
                let answer = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(answer.as_bytes()).await;
            });
        }
    });
    format!("http://{address}/vm")
}

fn test_said(cx: &mut TestAppContext, page: &Entity<MonitoringPage>) -> Option<String> {
    cx.read(|cx| match &page.read(cx).form.as_ref()?.test {
        source::Test::Passed(message) => Some(format!("passed: {message}")),
        source::Test::Failed(message) => Some(format!("failed: {message}")),
        _ => None,
    })
}

async fn tested(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<MonitoringPage>) {
    let observed = page.clone();
    cx.wait_for(handle, Duration::from_secs(5), move |_, cx| {
        !matches!(
            observed.read(cx).form.as_ref().map(|form| &form.test),
            Some(source::Test::Testing(_))
        )
    })
    .await;
}

async fn settled(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<MonitoringPage>) {
    let observed = page.clone();
    cx.wait_for(handle, Duration::from_secs(5), move |_, cx| {
        !matches!(
            observed.read(cx).connection,
            Connection::Looking { .. } | Connection::None
        )
    })
    .await;
}

#[gpui_kit::test]
async fn a_url_chosen_in_settings_is_tested_saved_and_read_with_its_token_from_the_store(
    cx: &mut TestAppContext,
) {
    use crate::secrets::{MemoryStore, Secrets};
    cx.executor().allow_parking();
    let directory = std::env::temp_dir().join(format!(
        "freshkube-monitoring-source-{}-{}",
        std::process::id(),
        line!()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let preferences = directory.join("preferences.json");
    let store = std::sync::Arc::new(MemoryStore::default());
    let secrets: Secrets = store.clone();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let base = metrics_server(&runtime);
    let (handle, page) = mount_with(
        cx,
        &runtime,
        Some(live_source()),
        Some(&preferences),
        Some(secrets.clone()),
    );

    // Test checks the form as typed: a wrong token is refused.
    let fill = |cx: &mut TestAppContext, token: &'static str| {
        let url = base.clone();
        cx.update_window(handle, |_, window, cx| {
            page.update(cx, |page, cx| {
                page.source_form(window, cx);
                page.set_mode(source::Mode::Url, cx);
                let form = page.form.as_ref().unwrap();
                form.url
                    .update(cx, |input, cx| input.set_value(url, window, cx));
                form.token
                    .update(cx, |input, cx| input.set_value(token, window, cx));
                page.test_source(cx);
            })
        })
        .unwrap();
    };
    fill(cx, "wrong");
    tested(cx, handle, &page).await;
    let said = test_said(cx, &page).unwrap();
    assert!(said.starts_with("failed: The server refused"), "{said}");
    assert!(!said.contains("wrong"), "{said}");
    fill(cx, "s3cret");
    tested(cx, handle, &page).await;
    assert_eq!(
        test_said(cx, &page).unwrap(),
        format!("passed: Prometheus API answered at {base}")
    );
    // Testing saved nothing.
    assert!(store.keys.lock().unwrap().is_empty());
    cx.read(|cx| assert!(page.read(cx).saved.choices.is_empty()));

    // Save keeps the URL in monitoring.json and the token in the store,
    // then the page reads from it.
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(true, cx)));
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| page.save_source(window, cx))
    })
    .unwrap();
    settled(cx, handle, &page).await;
    let url = format!("{base}/");
    cx.read(|cx| {
        let page = page.read(cx);
        assert_eq!(page.connection_name(), "ready");
        let history = page.history().unwrap();
        assert!(matches!(
            history.kind,
            crate::monitoring::history::HistoryKind::Ready(ref prometheus)
                if prometheus.endpoint().label() == base
        ));
    });
    let account = crate::monitoring::store::token_account(&url);
    cx.wait_for(handle, Duration::from_secs(5), {
        let store = store.clone();
        let account = account.clone();
        move |_, _| store.keys.lock().unwrap().contains_key(&account)
    })
    .await;
    assert_eq!(
        store.keys.lock().unwrap().get(&account).map(String::as_str),
        Some("s3cret")
    );
    let file = directory.join("monitoring.json");
    cx.wait_for(handle, Duration::from_secs(5), {
        let file = file.clone();
        move |_, _| std::fs::read_to_string(&file).is_ok_and(|text| text.contains("sources"))
    })
    .await;
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(written.contains(&url), "{written}");
    assert!(!written.contains("s3cret"), "{written}");

    // The next launch reads the token from the store.
    let (relaunched_handle, relaunched) = mount_with(
        cx,
        &runtime,
        Some(live_source()),
        Some(&preferences),
        Some(secrets),
    );
    cx.update(|cx| relaunched.update(cx, |page, cx| page.set_visible(true, cx)));
    settled(cx, relaunched_handle, &relaunched).await;
    cx.read(|cx| assert_eq!(relaunched.read(cx).connection_name(), "ready"));

    // Forget drops the token, and the server refuses the page.
    cx.update_window(relaunched_handle, |_, window, cx| {
        relaunched.update(cx, |page, cx| {
            page.source_form(window, cx);
            page.forget_token(window, cx);
        })
    })
    .unwrap();
    settled(cx, relaunched_handle, &relaunched).await;
    cx.read(|cx| assert_eq!(relaunched.read(cx).connection_name(), "refused"));
    cx.wait_for(relaunched_handle, Duration::from_secs(5), {
        let store = store.clone();
        move |_, _| store.keys.lock().unwrap().is_empty()
    })
    .await;
    assert!(store.keys.lock().unwrap().is_empty());
    std::fs::remove_dir_all(&directory).unwrap();
}

#[gpui_kit::test]
fn the_form_says_why_it_cannot_be_saved(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(live_source()));
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.source_form(window, cx);
            page.set_mode(source::Mode::Service, cx);
            page.form
                .as_ref()
                .unwrap()
                .namespace
                .update(cx, |input, cx| input.set_value("vm", window, cx));
            page.save_source(window, cx);
            assert!(page.saved.choices.is_empty());
            page.set_mode(source::Mode::Url, cx);
            page.form.as_ref().unwrap().url.update(cx, |input, cx| {
                input.set_value("https://u:p@vm.lan", window, cx)
            });
            page.save_source(window, cx);
            assert!(page.saved.choices.is_empty());
        })
    })
    .unwrap();
    // A chosen Service is the only one tried, and history reads it.
    let service = PrometheusService::new("vm", "vmselect", 8481).with_path("select/0/prometheus");
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.saved.choices.insert(
                "prod-ams".into(),
                crate::monitoring::store::Choice::Service(service.clone()),
            );
            page.saved.services.insert(
                "prod-ams".into(),
                PrometheusService::new("monitoring", "prometheus-operated", 9090),
            );
            page.set_source(None, cx);
            page.set_source(Some(live_source()), cx);
        })
    });
    cx.read(|cx| {
        assert!(matches!(
            page.read(cx).history().unwrap().kind,
            crate::monitoring::history::HistoryKind::Remembered { service: ref s, .. } if *s == service
        ));
    });
}

/// The Firing alerts table's view, once every panel has answered.
fn alerts_table(
    cx: &mut TestAppContext,
    page: &Entity<MonitoringPage>,
) -> Entity<crate::monitoring::panel::TableView> {
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
        crate::desktop::tests::settle_header(window, cx);
    })
    .unwrap();
    let table = alerts_table(cx, &page);
    let (list, row) = cx.read(|cx| table.read(cx).test_ids());
    let down = |delta: f32| ScrollDelta::Pixels(point(px(0.), px(-delta)));
    cx.update_window(handle, |_, window, cx| {
        // The page to its end, then the wheel goes on over the table, as a
        // trackpad's does once the page stops.
        wheel(window, down(40_000.), cx);
        crate::desktop::tests::settle_header(window, cx);
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
    use crate::desktop::layout_check::{Table, assert_table};
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
fn a_short_window_scrolls_the_page_alone_header_and_all(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let down = |delta: f32| ScrollDelta::Pixels(point(px(0.), px(-delta)));
    let panels = cx.read(|cx| page.read(cx).board.as_ref().unwrap().slots.len());
    // Tall, the page stays put and the grid takes what the header leaves.
    cx.update_window(handle, |_, window, cx| {
        crate::desktop::tests::settle_header(window, cx);
        let title = window.find("monitoring-title").bounds();
        window.scroll("monitoring-page", down(400.), cx);
        crate::desktop::tests::settle_header(window, cx);
        assert_eq!(window.find("monitoring-title").bounds(), title);
        let frame = window.find("monitoring-page").bounds();
        let grid = window.find("monitoring-grid").bounds();
        assert!(grid.top() > title.bottom(), "{grid:?} over {title:?}");
        assert!(grid.bottom() <= frame.bottom(), "{grid:?} past {frame:?}");
    })
    .unwrap();
    // 760 × 560 at 20 px with the column folded: 273 dp, under 620.
    cx.simulate_window_resize(handle, size(px(600.), px(420.)));
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        crate::desktop::tests::settle_header(window, cx);
        let unit = crate::ui::dp_px(1., window);
        let frame = window.find("monitoring-page").bounds();
        let grid = window.find("monitoring-grid").bounds();
        // The grid is all its panels high, inside the page that scrolls.
        assert!(
            grid.size.height > frame.size.height,
            "{grid:?} in {frame:?}"
        );
        // The wheel over the grid moves the header and the panels together:
        // the page scrolls and the grid doesn't scroll inside it.
        let title = window.find("monitoring-title").bounds();
        let panel = window.find("monitoring-panel-0").bounds();
        wheel(window, down(60.), cx);
        crate::desktop::tests::settle_header(window, cx);
        let moved = title.top() - window.find("monitoring-title").bounds().top();
        let panel_moved = panel.top() - window.find("monitoring-panel-0").bounds().top();
        assert!(moved > px(0.), "the page didn't scroll");
        assert!(
            (moved - panel_moved).abs() < px(0.5),
            "the header moved {moved:?}, the panels {panel_moved:?}"
        );
        // To the page's end: the header gone and the last panel in it.
        wheel(window, down(40_000.), cx);
        crate::desktop::tests::settle_header(window, cx);
        let title = window.find("monitoring-title").bounds();
        assert!(
            title.bottom() <= frame.top(),
            "{title:?} still in {frame:?}"
        );
        let last = (0..panels)
            .map(|n| slot_bounds(window, n).bottom())
            .fold(px(f32::MIN), |a, b| a.max(b));
        assert!(last <= frame.bottom() + px(0.5), "{last:?} past {frame:?}");
        assert!(
            last > frame.bottom() - unit * 40.,
            "{last:?} short of {frame:?}"
        );
        // The panels it asks for are the ones in the page's view.
        let viewport = page.read(cx).viewport.get();
        assert!(viewport.top > 0., "{viewport:?}");
        assert!(
            (viewport.height - frame.size.height / unit).abs() < 1.,
            "asks for {viewport:?}, the page is {frame:?}"
        );
    })
    .unwrap();
}
