use std::cell::RefCell;
use std::rc::Rc;

use freshkube_core::monitoring::{
    ExampleSource, QueryError,
    builtin::BUILTINS,
    model::{Dashboard, PanelSpec, data::QueryContext, time::TimeWindow},
};
use gpui_kit::component::{Root, Theme, ThemeMode, v_flex};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Context, Entity, IntoElement, ParentElement, Render,
    StyleRefinement, Styled, TestAppContext, Window, div, point, px, size,
};

use super::*;
use crate::desktop::probe;
use freshkube_ui::table::TableSource;

const END: i64 = 1_700_003_600;

fn window_range() -> TimeWindow {
    TimeWindow::new(END, 6 * 3600, 72)
}

/// The panels of a page, one under another.
struct Host {
    panels: Vec<Entity<PanelView>>,
    width: f32,
    height: f32,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .children(self.panels.iter().map(|panel| {
                div()
                    .relative()
                    .flex_none()
                    .w(px(self.width))
                    .h(px(self.height))
                    .child(panel.clone().cached(StyleRefinement::default().size_full()))
                    .children(panel.read(cx).cursor_overlay())
            }))
    }
}

/// The built-in Cluster dashboard's panels, not yet answered.
fn cluster() -> (Dashboard, Vec<Rc<PanelSpec>>) {
    let dashboard = Dashboard::parse(BUILTINS[0].json).unwrap();
    let specs = dashboard.panels.iter().cloned().map(Rc::new).collect();
    (dashboard, specs)
}

fn mount(
    cx: &mut TestAppContext,
    specs: Vec<Rc<PanelSpec>>,
) -> (AnyWindowHandle, Vec<Entity<PanelView>>) {
    mount_at(cx, specs, 640.)
}

/// Mounts the panels `width` wide.
fn mount_at(
    cx: &mut TestAppContext,
    specs: Vec<Rc<PanelSpec>>,
    width: f32,
) -> (AnyWindowHandle, Vec<Entity<PanelView>>) {
    mount_sized(cx, specs, width, 320.)
}

/// Panels `width` wide and `height` high, one under another.
fn mount_sized(
    cx: &mut TestAppContext,
    specs: Vec<Rc<PanelSpec>>,
    width: f32,
    height: f32,
) -> (AnyWindowHandle, Vec<Entity<PanelView>>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.set_reduce_motion(true);
    });
    let mut panels = Vec::new();
    let total = height * specs.len() as f32;
    let window = cx.open_window(size(px(700.), px(total)), |window, cx| {
        panels = specs
            .into_iter()
            .enumerate()
            .map(|(index, spec)| cx.new(|_| PanelView::new(index.to_string(), spec)))
            .collect();
        let host = cx.new(|_| Host {
            panels: panels.clone(),
            width,
            height,
        });
        Root::new(host, window, cx)
    });
    cx.run_until_parked();
    (window.into(), panels)
}

/// Answers every panel from example data.
fn answer(cx: &mut TestAppContext, dashboard: &Dashboard, panels: &[Entity<PanelView>]) {
    let source = ExampleSource::new(BUILTINS[0].uid);
    let (variables, _) = source
        .resolve_variables(&dashboard.variables, &[], window_range())
        .unwrap();
    let context = QueryContext {
        window: window_range(),
        variables: variables.context_values(),
    };
    for panel in panels {
        cx.update(|cx| {
            panel.update(cx, |panel, cx| {
                let result = source
                    .query_panel(&panel.spec.clone(), &context, &variables)
                    .unwrap();
                panel.set_result(result, window_range(), cx);
            })
        });
    }
}

fn frame(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn shown(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) -> bool {
    let id = id.to_owned();
    cx.update_window(handle, move |_, window, cx| {
        window.render_frame(cx);
        window.try_find(gpui_kit::SharedString::from(id)).is_some()
    })
    .unwrap()
}

fn index_of(specs: &[Rc<PanelSpec>], title: &str) -> usize {
    specs.iter().position(|spec| spec.title == title).unwrap()
}

#[gpui_kit::test]
fn every_kind_in_the_cluster_dashboard_draws_from_example_data(cx: &mut TestAppContext) {
    let (dashboard, specs) = cluster();
    let (handle, panels) = mount(cx, specs.clone());
    for index in 0..specs.len() {
        assert!(shown(
            cx,
            handle,
            &format!("monitoring-panel-{index}-title")
        ));
        assert!(!shown(
            cx,
            handle,
            &format!("monitoring-panel-{index}-failed")
        ));
    }
    answer(cx, &dashboard, &panels);
    let cpu = index_of(&specs, "CPU usage by node");
    assert!(shown(cx, handle, &format!("monitoring-panel-{cpu}-plot")));
    assert!(shown(
        cx,
        handle,
        &format!("monitoring-panel-{cpu}-legend-0")
    ));
    let pods = index_of(&specs, "Top pods by CPU");
    assert!(shown(cx, handle, &format!("monitoring-panel-{pods}-bars")));
    let alerts = index_of(&specs, "Firing alerts");
    assert!(shown(
        cx,
        handle,
        &format!("monitoring-panel-{alerts}-table-list")
    ));
    for (index, panel) in panels.iter().enumerate() {
        cx.read(|cx| {
            let panel = panel.read(cx);
            assert_eq!(panel.state, State::Ready, "{index}");
            assert!(
                !matches!(
                    panel.data.as_ref().map(|data| &data.body),
                    Some(Body::NoData | Body::NotDrawn(_))
                ),
                "{}",
                panel.spec.title
            );
        });
    }
    // The latency panel gives its unit to the title.
    let latency = index_of(&specs, "API server latency");
    cx.read(|cx| {
        let data = panels[latency].read(cx).data.clone().unwrap();
        assert_eq!(data.unit.as_deref(), Some("ms"));
    });
}

#[gpui_kit::test]
fn a_failure_shows_in_the_card_until_an_answer_then_marks_it_stale(cx: &mut TestAppContext) {
    let (dashboard, specs) = cluster();
    let cpu = index_of(&specs, "CPU usage by node");
    let (handle, panels) = mount(cx, specs);
    let failed = format!("monitoring-panel-{cpu}-failed");
    let stale = format!("monitoring-panel-{cpu}-stale");
    let plot: gpui_kit::SharedString = format!("monitoring-panel-{cpu}-plot").into();
    let error = QueryError::refused();
    cx.update(|cx| panels[cpu].update(cx, |panel, cx| panel.set_error(&error, cx)));
    assert!(shown(cx, handle, &failed));
    assert!(!shown(cx, handle, &plot));

    answer(cx, &dashboard, &panels);
    assert!(!shown(cx, handle, &failed));
    assert!(shown(cx, handle, &plot));

    // A later failure keeps the chart and marks it.
    cx.update(|cx| panels[cpu].update(cx, |panel, cx| panel.set_error(&error, cx)));
    assert!(shown(cx, handle, &plot));
    assert!(shown(cx, handle, &stale));
    assert!(!shown(cx, handle, &failed));
}

#[gpui_kit::test]
fn the_legend_fades_other_series_on_hover_and_a_click_keeps_one(cx: &mut TestAppContext) {
    let (dashboard, specs) = cluster();
    let cpu = index_of(&specs, "CPU usage by node");
    let (handle, panels) = mount(cx, specs);
    answer(cx, &dashboard, &panels);
    frame(cx, handle);
    let focus = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            let panel = panels[cpu].read(cx);
            let plot = panel.plot.as_ref().unwrap().read(cx).focus;
            (panel.focus(), plot)
        })
    };
    let paths = probe::count("monitoring-path");
    let row: gpui_kit::SharedString = format!("monitoring-panel-{cpu}-legend-1").into();
    cx.update_window(handle, |_, window, cx| window.hover(row.clone(), cx))
        .unwrap();
    assert_eq!(focus(cx), (Some(1), Some(1)));

    let title: gpui_kit::SharedString = format!("monitoring-panel-{cpu}-title").into();
    cx.update_window(handle, |_, window, cx| window.hover(title.clone(), cx))
        .unwrap();
    assert_eq!(focus(cx), (None, None));

    cx.update_window(handle, |_, window, cx| window.click(row.clone(), cx))
        .unwrap();
    cx.update_window(handle, |_, window, cx| window.hover(title.clone(), cx))
        .unwrap();
    assert_eq!(focus(cx), (Some(1), Some(1)));

    // Fading draws the same shapes in another opacity; only the focused
    // series is built again, its line and area alone, to draw on top.
    assert!(probe::count("monitoring-path") <= paths + 2);

    // Clicking it again lets go.
    cx.update_window(handle, |_, window, cx| window.click(row.clone(), cx))
        .unwrap();
    cx.update_window(handle, |_, window, cx| window.hover(title.clone(), cx))
        .unwrap();
    assert_eq!(focus(cx), (None, None));
}

#[gpui_kit::test]
fn the_cursor_reads_out_values_and_tells_the_page_without_rebuilding_paths(
    cx: &mut TestAppContext,
) {
    let (dashboard, specs) = cluster();
    let cpu = index_of(&specs, "CPU usage by node");
    let memory = index_of(&specs, "Memory usage by node");
    let (handle, panels) = mount(cx, specs);
    answer(cx, &dashboard, &panels);
    cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    let _subscription = cx.update(|cx| {
        let events = events.clone();
        cx.subscribe(&panels[cpu], move |_, event: &PanelEvent, _| {
            events.borrow_mut().push(event.clone())
        })
    });
    let plot: gpui_kit::SharedString = format!("monitoring-panel-{cpu}-plot").into();
    let title: gpui_kit::SharedString = format!("monitoring-panel-{cpu}-title").into();
    // Real moves and real frames: `hover` draws with the view caches off.
    let move_to = |cx: &mut TestAppContext, id: gpui_kit::SharedString, dx: f32| {
        cx.update_window(handle, |_, window, cx| {
            let bounds = window.find(id).bounds();
            window.dispatch_event(
                gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                    position: bounds.center() + gpui_kit::point(px(dx), px(0.)),
                    pressed_button: None,
                    modifiers: Default::default(),
                }),
                cx,
            );
            window.draw(cx).clear(cx);
        })
        .unwrap();
    };
    move_to(cx, title.clone(), 0.);
    let counts = || {
        [
            "monitoring-panel",
            "monitoring-plot",
            "monitoring-plot-paint",
            "monitoring-legend",
            "monitoring-path",
        ]
        .map(probe::count)
    };
    let before = counts();
    let readout: gpui_kit::SharedString = format!("monitoring-panel-{cpu}-readout").into();
    let readout_right = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, _| {
            window.find(readout.clone()).bounds().right()
        })
        .unwrap()
    };
    move_to(cx, plot.clone(), 60.);
    let near = readout_right(cx);
    move_to(cx, plot.clone(), 100.);
    let far = readout_right(cx);
    move_to(cx, plot.clone(), 40.);
    // Only the cursor's own view draws: the panel, its plot and its legend
    // stay cached, and no shape is built.
    assert_eq!(counts(), before, "a pointer move drew the panel again");
    assert!(far > near, "the readout stayed at {near:?}");
    let cursor = cx.read(|cx| panels[cpu].read(cx).cursor.clone()).unwrap();
    assert!(!cursor.rows.is_empty());
    assert!(!cursor.time.is_empty());
    assert!(shown(cx, handle, &readout));
    let time = match events.borrow().last() {
        Some(PanelEvent::Cursor(Some(time))) => *time,
        other => panic!("{other:?}"),
    };

    // The page passes it on: the other chart's crosshair falls on the same
    // sample, and that chart shows no readout.
    let shared = cx
        .read(|cx| panels[memory].read(cx).crosshair(time, cx))
        .unwrap();
    let chart = cx.read(|cx| panels[memory].read(cx).chart()).unwrap();
    assert_eq!(shared.x, chart.xs[cursor.index]);
    assert!(!shown(
        cx,
        handle,
        &format!("monitoring-panel-{memory}-readout")
    ));

    // Leaving the plot clears it and says so.
    cx.update_window(handle, |_, window, cx| window.hover(title.clone(), cx))
        .unwrap();
    assert!(cx.read(|cx| panels[cpu].read(cx).cursor.is_none()));
    assert!(!shown(cx, handle, &readout));
    assert!(matches!(
        events.borrow().last(),
        Some(PanelEvent::Cursor(None))
    ));
}

#[test]
fn the_nearest_sample_wins() {
    let xs = [0., 0.25, 0.5, 1.];
    assert_eq!(cursor::nearest_x(&xs, 0.3), Some(1));
    assert_eq!(cursor::nearest_x(&xs, 0.8), Some(3));
    assert_eq!(cursor::nearest_x(&[], 0.5), None);
    let times = [10., 20., 30.];
    assert_eq!(cursor::nearest_time(&times, 24.), Some(1));
    assert_eq!(cursor::nearest_time(&times, 26.), Some(2));
    assert_eq!(cursor::nearest_time(&times, 31.), None);
}

#[gpui_kit::test]
fn a_click_on_the_info_icon_copies_the_promql_as_sent(cx: &mut TestAppContext) {
    let (dashboard, specs) = cluster();
    let cpu = index_of(&specs, "CPU usage by node");
    let (handle, panels) = mount(cx, specs);
    answer(cx, &dashboard, &panels);
    frame(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click(format!("monitoring-panel-{cpu}-query"), cx)
    })
    .unwrap();
    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap();
    assert!(copied.contains("node_cpu_seconds_total"), "{copied}");
    assert!(!copied.contains("$node"), "{copied}");
}

#[gpui_kit::test]
fn markers_on_the_window_draw_and_the_readout_names_the_one_under_the_pointer(
    cx: &mut TestAppContext,
) {
    use freshkube_core::monitoring::markers::{Marker, MarkerKind};
    let (dashboard, specs) = cluster();
    let cpu = index_of(&specs, "CPU usage by node");
    let stat = index_of(&specs, "Ready nodes");
    let (handle, panels) = mount(cx, specs);
    answer(cx, &dashboard, &panels);
    let marker = |kind, at: i64, label: &str| Marker {
        kind,
        at,
        namespace: None,
        node: None,
        label: label.into(),
    };
    let markers: Rc<[Marker]> = Rc::from(vec![
        marker(MarkerKind::Deploy, END - 7 * 3600, "deploy/old → 3"),
        marker(MarkerKind::Deploy, END - 3 * 3600, "deploy/api → 1.8.2"),
        marker(MarkerKind::NodeNotReady, END - 600, "worker-2 NotReady"),
    ]);
    for panel in &panels {
        let markers = markers.clone();
        cx.update(|cx| panel.update(cx, |panel, cx| panel.set_markers(markers, cx)));
    }
    let labels = cx.read(|cx| panels[cpu].read(cx).marker_labels());
    assert_eq!(labels, ["deploy/api → 1.8.2", "worker-2 NotReady"]);
    assert!(cx.read(|cx| panels[stat].read(cx).marker_labels().is_empty()));

    cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    let geometry = cx.read(|cx| panels[cpu].read(cx).geometry.get());
    let move_to = |cx: &mut TestAppContext, fraction: f32| {
        cx.update_window(handle, |_, window, cx| {
            let position = geometry.origin
                + gpui_kit::point(
                    geometry.left + geometry.width * fraction,
                    geometry.top + geometry.height / 2.,
                );
            window.dispatch_event(
                gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Default::default(),
                }),
                cx,
            );
            window.draw(cx).clear(cx);
        })
        .unwrap();
    };
    let marker_at = |cx: &mut TestAppContext| {
        cx.read(|cx| panels[cpu].read(cx).cursor.clone())
            .and_then(|cursor| cursor.marker)
    };
    // Three hours before the end of six is halfway.
    move_to(cx, 0.5);
    assert_eq!(marker_at(cx), Some(0));
    assert!(shown(
        cx,
        handle,
        &format!("monitoring-panel-{cpu}-readout")
    ));
    move_to(cx, 0.25);
    assert_eq!(marker_at(cx), None);

    // A new answer over a later window places them again.
    let later = TimeWindow::new(END + 4 * 3600, 6 * 3600, 72);
    let source = ExampleSource::new(BUILTINS[0].uid);
    let (variables, _) = source
        .resolve_variables(&dashboard.variables, &[], later)
        .unwrap();
    let context = QueryContext {
        window: later,
        variables: variables.context_values(),
    };
    cx.update(|cx| {
        panels[cpu].update(cx, |panel, cx| {
            let result = source
                .query_panel(&panel.spec.clone(), &context, &variables)
                .unwrap();
            panel.set_result(result, later, cx);
        })
    });
    let labels = cx.read(|cx| panels[cpu].read(cx).marker_labels());
    assert_eq!(labels, ["worker-2 NotReady"]);
}

/// A chart of forty pods, pod n's values about 3n, mounted unanswered, and
/// its answer.
fn crowded(
    cx: &mut TestAppContext,
) -> (
    AnyWindowHandle,
    Vec<Entity<PanelView>>,
    freshkube_core::monitoring::PanelResult,
) {
    crowded_at(cx, 640.)
}

/// [`crowded`], `width` wide.
fn crowded_at(
    cx: &mut TestAppContext,
    width: f32,
) -> (
    AnyWindowHandle,
    Vec<Entity<PanelView>>,
    freshkube_core::monitoring::PanelResult,
) {
    use freshkube_core::monitoring::model::data::{Frame, Series};
    let dashboard = Dashboard::parse(
        r#"{
          "title": "Crowded",
          "panels": [{
            "type": "timeseries", "title": "Goroutines",
            "gridPos": {"x": 0, "y": 0, "w": 24, "h": 8},
            "fieldConfig": {"defaults": {"custom": {"fillOpacity": 10}}},
            "targets": [{"refId": "A", "expr": "go_goroutines", "legendFormat": "{{pod}}"}]
          }]
        }"#,
    )
    .unwrap();
    let specs: Vec<Rc<PanelSpec>> = dashboard.panels.iter().cloned().map(Rc::new).collect();
    let (handle, panels) = mount_at(cx, specs, width);
    let times: Vec<f64> = (0..72).map(|n| (END - 6 * 3600 + n * 300) as f64).collect();
    let series = (0..40)
        .map(|pod| Series {
            name: format!("app-{pod}"),
            query: "A".into(),
            field: None,
            labels: vec![("pod".into(), format!("app-{pod}"))],
            values: (0..72).map(|n| (pod * 3 + n % 7) as f64).collect(),
        })
        .collect();
    let result = freshkube_core::monitoring::PanelResult {
        frame: Frame { times, series },
        warnings: Vec::new(),
        expressions: Vec::new(),
    };
    (handle, panels, result)
}

/// Thirty of forty series past the two colours: the grey lines draw as one
/// path and the grey areas as another, not one of each per series.
#[gpui_kit::test]
fn a_crowded_chart_draws_lines_of_one_look_as_one_path(cx: &mut TestAppContext) {
    let (handle, panels, result) = crowded(cx);
    let before = probe::count("monitoring-path");
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.set_result(result, window_range(), cx)));
    frame(cx, handle);
    let series = cx.read(|cx| panels[0].read(cx).chart().unwrap().series.len());
    assert_eq!(series, derive::MOST_SERIES);
    // Two coloured series and the grey rest, each a line and an area.
    assert_eq!(probe::count("monitoring-path") - before, 3 * 2);
}

/// A group of grey lines past GPUI's 65,536 vertices a path still draws:
/// it is built in chunks joined into one path, where it once failed to
/// build and drew nothing.
#[gpui_kit::test]
fn a_group_past_the_vertex_limit_still_draws(cx: &mut TestAppContext) {
    use freshkube_core::monitoring::model::data::{Frame, Series};
    let dashboard = Dashboard::parse(
        r#"{
          "title": "Dense",
          "panels": [{
            "type": "timeseries", "title": "Dense",
            "gridPos": {"x": 0, "y": 0, "w": 24, "h": 8},
            "targets": [{"refId": "A", "expr": "go_goroutines", "legendFormat": "{{pod}}"}]
          }]
        }"#,
    )
    .unwrap();
    let specs: Vec<Rc<PanelSpec>> = dashboard.panels.iter().cloned().map(Rc::new).collect();
    let (handle, panels) = mount(cx, specs);
    // The 600 samples a panel asks for by default.
    let times: Vec<f64> = (0..600).map(|n| (END - 6 * 3600 + n * 36) as f64).collect();
    // Seventy jagged series: the 68 grey ones pass the limit as one path.
    let series = (0..70)
        .map(|pod| Series {
            name: format!("app-{pod}"),
            query: "A".into(),
            field: None,
            labels: vec![("pod".into(), format!("app-{pod}"))],
            values: (0..600).map(|n| ((n * 7 + pod * 13) % 17) as f64).collect(),
        })
        .collect();
    let result = freshkube_core::monitoring::PanelResult {
        frame: Frame { times, series },
        warnings: Vec::new(),
        expressions: Vec::new(),
    };
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.set_result(result, window_range(), cx)));
    frame(cx, handle);
    let before = probe::count("monitoring-path-painted");
    frame(cx, handle);
    // One frame paints the two coloured lines and the grey ones as one.
    assert_eq!(probe::count("monitoring-path-painted") - before, 3);
}

/// Forty series draw the thirty highest until Show all, which draws every
/// one as before, the grey ones still sharing their paths; Top 30 returns.
/// A refresh keeps Show all, and a query that picks its series with topk
/// draws them all with no line to offer.
#[gpui_kit::test]
fn a_crowded_chart_draws_its_highest_peaks_until_show_all(cx: &mut TestAppContext) {
    let (handle, panels, result) = crowded(cx);
    let again = result.clone();
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.set_result(result, window_range(), cx)));
    let drawn = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            let chart = panels[0].read(cx).chart().unwrap();
            (chart.series.len(), chart.capped)
        })
    };
    assert!(shown(cx, handle, "monitoring-panel-0-cap"));
    assert_eq!(drawn(cx), (30, Some(derive::Capped { of: 40, all: false })));
    // The lowest pods are the ones left out.
    let first = cx.read(|cx| panels[0].read(cx).chart().unwrap().series[0].name.clone());
    assert_eq!(first, "app-10");

    let before = probe::count("monitoring-path");
    cx.update_window(handle, |_, window, cx| {
        window.click("monitoring-panel-0-cap-toggle", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(drawn(cx), (40, Some(derive::Capped { of: 40, all: true })));
    assert_eq!(probe::count("monitoring-path") - before, 3 * 2);

    // A refresh answers again and keeps every series.
    cx.update(|cx| {
        panels[0].update(cx, |panel, cx| {
            panel.set_result(again.clone(), window_range(), cx)
        })
    });
    assert_eq!(drawn(cx).0, 40);

    // Top 30.
    cx.update_window(handle, |_, window, cx| {
        window.click("monitoring-panel-0-cap-toggle", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(drawn(cx).0, 30);

    let mut picked = again;
    picked.expressions = vec![("A".into(), "topk(40, go_goroutines)".into())];
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.set_result(picked, window_range(), cx)));
    assert_eq!(drawn(cx), (40, None));
    assert!(!shown(cx, handle, "monitoring-panel-0-cap"));
}

/// Show all lasts until the page asks for something else.
#[gpui_kit::test]
fn asking_again_returns_a_chart_to_its_highest_peaks(cx: &mut TestAppContext) {
    let (handle, panels, result) = crowded(cx);
    let again = result.clone();
    cx.update(|cx| {
        panels[0].update(cx, |panel, cx| {
            panel.set_result(result, window_range(), cx);
            panel.show_all_series(true, cx);
        })
    });
    frame(cx, handle);
    cx.update(|cx| {
        panels[0].update(cx, |panel, cx| {
            panel.cap_again();
            panel.set_result(again, window_range(), cx);
        })
    });
    let drawn = cx.read(|cx| panels[0].read(cx).chart().unwrap().series.len());
    assert_eq!(drawn, 30);
}

/// Past what fits, the readout names the highest values and the picked
/// series, and stays inside the plot.
#[gpui_kit::test]
fn a_crowded_readout_ranks_the_highest_values_and_fits_the_plot(cx: &mut TestAppContext) {
    let (handle, panels, result) = crowded(cx);
    cx.update(|cx| {
        panels[0].update(cx, |panel, cx| {
            panel.set_result(result, window_range(), cx);
            panel.toggle_picked(2, cx);
        })
    });
    frame(cx, handle);
    let (plot, readout): (gpui_kit::SharedString, gpui_kit::SharedString) = (
        "monitoring-panel-0-plot".into(),
        "monitoring-panel-0-readout".into(),
    );
    cx.update_window(handle, |_, window, cx| {
        let bounds = window.find(plot.clone()).bounds();
        window.dispatch_event(
            gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                position: bounds.center(),
                pressed_button: None,
                modifiers: Default::default(),
            }),
            cx,
        );
        window.draw(cx).clear(cx);
    })
    .unwrap();
    let cursor = cx.read(|cx| panels[0].read(cx).cursor.clone()).unwrap();
    let series: Vec<usize> = cursor.rows.iter().map(|row| row.series).collect();
    // It ranks and counts only the thirty series drawn.
    let fit = series.len();
    assert!((3..=cursor::READOUT_ROWS).contains(&fit), "{series:?}");
    assert_eq!(cursor.more, 30 - fit);
    let mut expected: Vec<usize> = (0..fit - 1).map(|n| 29 - n).collect();
    expected.push(2);
    assert_eq!(series, expected);
    cx.update_window(handle, |_, window, _| {
        let (plot, readout) = (window.find(plot).bounds(), window.find(readout).bounds());
        assert!(readout.bottom() <= plot.bottom(), "{readout:?} in {plot:?}");
    })
    .unwrap();
}

/// In a narrow chart, a readout flipped left of the crosshair stays off the
/// value axis, keeps its time and values whole, and its "top N of M" never
/// spills out of its box.
#[gpui_kit::test]
fn a_narrow_readout_keeps_off_the_axis_and_inside_its_box(cx: &mut TestAppContext) {
    let (readout, plot_left) = narrow_readout(cx, 260.);
    // Layout rounds to the test window's half-pixel grid.
    assert!(
        readout.left() >= plot_left - px(0.5),
        "{readout:?} left of {plot_left:?}"
    );
}

/// Where the plot has no room for the time and values, the readout crosses
/// onto the axis rather than cut them off.
#[gpui_kit::test]
fn a_readout_too_wide_for_the_plot_keeps_its_time_and_values_whole(cx: &mut TestAppContext) {
    let (readout, plot_left) = narrow_readout(cx, 160.);
    assert!(readout.left() < plot_left, "{readout:?} fits after all");
}

/// Hovers three quarters across a crowded chart `width` wide and checks what
/// holds at any width: the readout is at least its time and values wide,
/// draws them whole and keeps its count inside. Returns the readout's bounds
/// and the plot's left edge.
fn narrow_readout(
    cx: &mut TestAppContext,
    width: f32,
) -> (gpui_kit::Bounds<gpui_kit::Pixels>, gpui_kit::Pixels) {
    let (handle, panels, result) = crowded_at(cx, width);
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.set_result(result, window_range(), cx)));
    frame(cx, handle);
    let geometry = cx.read(|cx| panels[0].read(cx).geometry.get());
    let plot_left = geometry.origin.x + geometry.left;
    cx.update_window(handle, |_, window, cx| {
        let position = point(
            plot_left + geometry.width * 0.75,
            geometry.origin.y + px(60.),
        );
        window.dispatch_event(
            gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Default::default(),
            }),
            cx,
        );
        window.draw(cx).clear(cx);
    })
    .unwrap();
    let cursor = cx.read(|cx| panels[0].read(cx).cursor.clone()).unwrap();
    assert!(cursor.flip && cursor.more > 0, "{cursor:?}");
    cx.update_window(handle, |_, window, _| {
        let readout = window.find("monitoring-panel-0-readout").bounds();
        assert!(readout.size.width >= cursor.least - px(0.5), "{readout:?}");
        // The time and every value are drawn whole, inside the box.
        let mut whole = vec!["monitoring-panel-0-readout-time".to_owned()];
        whole.extend(
            (0..cursor.rows.len()).map(|n| format!("monitoring-panel-0-readout-value-{n}")),
        );
        for id in whole {
            let text = window
                .find(gpui_kit::SharedString::from(id.clone()))
                .bounds();
            assert!(
                text.left() >= readout.left() && text.right() <= readout.right() + px(0.5),
                "{id}: {text:?} out of {readout:?}"
            );
        }
        // The count gives way to the time: it fits, or shrinks to nothing.
        let more = window.find("monitoring-panel-0-readout-more").bounds();
        assert!(
            more.size.width == px(0.) || more.right() <= readout.right(),
            "{more:?} out of {readout:?}"
        );
        (readout, plot_left)
    })
    .unwrap()
}

/// A series picked in the legend while the cursor shows is marked in the
/// readout at once, and the panel never draws for the pointer.
#[gpui_kit::test]
fn a_pick_under_the_cursor_shows_in_the_readout(cx: &mut TestAppContext) {
    let (handle, panels, result) = crowded(cx);
    let again = result.clone();
    cx.update(|cx| {
        panels[0].update(cx, |panel, cx| {
            panel.set_result(result, window_range(), cx);
        })
    });
    frame(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        let bounds = window.find("monitoring-panel-0-plot").bounds();
        window.dispatch_event(
            gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                position: bounds.center(),
                pressed_button: None,
                modifiers: Default::default(),
            }),
            cx,
        );
        window.draw(cx).clear(cx);
    })
    .unwrap();
    let overlay = cx.read(|cx| panels[0].read(cx).cursor_overlay()).unwrap();
    let (rows, focused) = cx.read(|cx| overlay.read(cx).named()).unwrap();
    assert_eq!(focused, None);
    let pick = rows[1];
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.toggle_picked(pick, cx)));
    assert_eq!(
        cx.read(|cx| overlay.read(cx).named()),
        Some((rows.clone(), Some(pick)))
    );
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.toggle_picked(pick, cx)));
    assert_eq!(cx.read(|cx| overlay.read(cx).named()), Some((rows, None)));

    // A new answer takes the cursor away, as before.
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.set_result(again, window_range(), cx)));
    assert_eq!(cx.read(|cx| overlay.read(cx).named()), None);
}

/// A table panel of `count` alerts, one per name, answered at once.
fn alert_table(cx: &mut TestAppContext, count: usize) -> (AnyWindowHandle, Entity<PanelView>) {
    use freshkube_core::monitoring::{
        PanelResult,
        model::data::{Frame, Series},
    };
    let dashboard = serde_json::json!({"title": "Test", "panels": [{
        "type": "table",
        "title": "Alerts",
        "gridPos": {"x": 0, "y": 0, "w": 12, "h": 8},
        "targets": [{"refId": "A", "expr": "ALERTS", "format": "table", "instant": true}],
        "transformations": [{"id": "organize", "options": {"excludeByName": {"Time": true}}}],
    }]});
    let spec = Dashboard::parse(&dashboard.to_string())
        .unwrap()
        .panels
        .remove(0);
    let (handle, panels) = mount(cx, vec![Rc::new(spec)]);
    let times: Vec<f64> = window_range()
        .times()
        .into_iter()
        .map(|t| t as f64)
        .collect();
    let series = (0..count)
        .map(|n| Series {
            name: "ALERTS".into(),
            query: "A".into(),
            field: None,
            labels: [
                ("alertname".into(), format!("alert-{n}")),
                (
                    "severity".into(),
                    if n % 2 == 0 { "critical" } else { "info" }.into(),
                ),
            ]
            .into_iter()
            .collect(),
            values: vec![1.; times.len()],
        })
        .collect();
    let panel = panels[0].clone();
    cx.update(|cx| {
        panel.update(cx, |panel, cx| {
            let result = PanelResult {
                frame: Frame { times, series },
                warnings: Vec::new(),
                expressions: Vec::new(),
            };
            panel.set_result(result, window_range(), cx);
        })
    });
    (handle, panel)
}

#[gpui_kit::test]
fn a_long_table_shows_its_first_rows_until_show_all(cx: &mut TestAppContext) {
    let (handle, panel) = alert_table(cx, 240);
    let table = cx.read(|cx| panel.read(cx).table()).expect("a table view");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // "Showing 100 of 240" and Show all.
        window.find("monitoring-panel-0-table-showing");
        window.find("monitoring-panel-0-table-show-all");
        assert_eq!(table.read(cx).line_count(), 100);
        // The severity leads each row with its glyph; the first is critical.
        assert!(table.read(cx).leads_with_glyph());
        window.click("monitoring-panel-0-table-show-all", cx);
        window.render_frame(cx);
        assert!(table.read(cx).shows_all());
        assert_eq!(table.read(cx).line_count(), 240);
        assert!(
            window
                .try_find("monitoring-panel-0-table-showing")
                .is_none()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn past_the_cap_the_table_says_how_many_it_left_out(cx: &mut TestAppContext) {
    // derive's MAX_ROWS, which its tests pin at 1,000.
    let max = 1_000;
    let (handle, panel) = alert_table(cx, max + 5);
    let table = cx.read(|cx| panel.read(cx).table()).expect("a table view");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("monitoring-panel-0-table-show-all", cx);
        window.render_frame(cx);
        assert!(table.read(cx).shows_all());
        // Every kept row shows; "Showing 1000 of 1005" stays, without
        // Show all.
        assert_eq!(table.read(cx).line_count(), max);
        window.find("monitoring-panel-0-table-showing");
        assert!(
            window
                .try_find("monitoring-panel-0-table-show-all")
                .is_none()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_new_answer_keeps_the_tables_show_all(cx: &mut TestAppContext) {
    let (handle, panel) = alert_table(cx, 240);
    let table = cx.read(|cx| panel.read(cx).table()).expect("a table view");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("monitoring-panel-0-table-show-all", cx);
    })
    .unwrap();
    // A refresh on the same dashboard answers the same panel again.
    let data = cx.read(|cx| panel.read(cx).data.clone().unwrap());
    let Body::Table(rows) = data.body else {
        panic!("not a table")
    };
    cx.update(|cx| table.update(cx, |table, _| table.set_data(rows)));
    assert!(cx.read(|cx| table.read(cx).shows_all()));
    frame(cx, handle);
}

/// A timeseries of `count` series `width` wide, its legend a table of last
/// and max, answered at once. The series in `stopped` end before the
/// window does, at the value the others end at.
fn legend_panel(
    cx: &mut TestAppContext,
    count: usize,
    width: f32,
    stopped: &[usize],
) -> (AnyWindowHandle, Entity<PanelView>) {
    let names: Vec<String> = (0..count).map(|n| format!("pod-{n}")).collect();
    named_panel(cx, &names, width, 320., stopped)
}

/// A timeseries panel `width` by `height` answered with a series per name.
fn named_panel(
    cx: &mut TestAppContext,
    names: &[String],
    width: f32,
    height: f32,
    stopped: &[usize],
) -> (AnyWindowHandle, Entity<PanelView>) {
    use freshkube_core::monitoring::{
        PanelResult,
        model::data::{Frame, Series},
    };
    let dashboard = serde_json::json!({"title": "Test", "panels": [{
        "type": "timeseries",
        "title": "Memory",
        "gridPos": {"x": 0, "y": 0, "w": 12, "h": 8},
        "targets": [{"refId": "A", "expr": "memory", "legendFormat": "{{pod}}"}],
    }]});
    let spec = Dashboard::parse(&dashboard.to_string())
        .unwrap()
        .panels
        .remove(0);
    let (handle, panels) = mount_sized(cx, vec![Rc::new(spec)], width, height);
    let times: Vec<f64> = window_range()
        .times()
        .into_iter()
        .map(|t| t as f64)
        .collect();
    let series = names
        .iter()
        .enumerate()
        .map(|(n, name)| {
            let mut values = vec![5.; times.len()];
            if stopped.contains(&n) {
                let end = values.len();
                values[end - 3..].fill(f64::NAN);
            }
            Series {
                name: name.clone(),
                query: "A".into(),
                field: None,
                labels: vec![("pod".into(), name.clone())],
                values,
            }
        })
        .collect();
    let panel = panels[0].clone();
    cx.update(|cx| {
        panel.update(cx, |panel, cx| {
            let result = PanelResult {
                frame: Frame { times, series },
                warnings: Vec::new(),
                expressions: Vec::new(),
            };
            panel.set_result(result, window_range(), cx);
        })
    });
    (handle, panel)
}

/// One wheel event at `position`, as the start of a gesture.
fn wheel(
    window: &mut Window,
    position: gpui_kit::Point<gpui_kit::Pixels>,
    y: f32,
    cx: &mut gpui_kit::App,
) {
    use gpui_kit::{InputEvent, MouseMoveEvent, ScrollDelta, ScrollWheelEvent, TouchPhase};
    window.dispatch_event(
        MouseMoveEvent {
            position,
            ..Default::default()
        }
        .to_platform_input(),
        cx,
    );
    window.dispatch_event(
        ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.), px(y))),
            touch_phase: TouchPhase::Started,
            ..Default::default()
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

fn bounds_of(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    ids: &[String],
) -> Vec<gpui_kit::Bounds<gpui_kit::Pixels>> {
    let ids = ids.to_vec();
    cx.update_window(handle, move |_, window, cx| {
        window.render_frame(cx);
        ids.into_iter()
            .map(|id| window.find(gpui_kit::SharedString::from(id)).bounds())
            .collect()
    })
    .unwrap()
}

fn legend_rows(count: usize) -> Vec<String> {
    (0..count)
        .map(|n| format!("monitoring-panel-0-legend-{n}"))
        .collect()
}

/// Eleven series: the eleventh row takes half the width, as the ten above
/// it do, not the whole line.
#[gpui_kit::test]
fn an_odd_last_legend_row_keeps_half_the_width(cx: &mut TestAppContext) {
    let (handle, _panel) = legend_panel(cx, 11, 640., &[]);
    let rows = bounds_of(cx, handle, &legend_rows(11));
    let (first, second, last) = (rows[0], rows[1], rows[10]);
    assert!(second.left() > first.right(), "two columns: {rows:?}");
    assert_eq!(second.top(), first.top());
    for row in &rows {
        assert!(
            (row.size.width - first.size.width).abs() < px(0.5),
            "{rows:?}"
        );
    }
    assert_eq!(last.left(), first.left());
}

/// Where two rows don't fit, each takes the line, the odd last one too.
/// Seven series, so the short names still take the table.
#[gpui_kit::test]
fn a_narrow_legend_takes_one_column(cx: &mut TestAppContext) {
    let (handle, _panel) = legend_panel(cx, 7, 300., &[]);
    let rows = bounds_of(cx, handle, &legend_rows(7));
    for pair in rows.windows(2) {
        assert_eq!(pair[1].left(), pair[0].left());
        assert!(pair[1].top() > pair[0].top(), "{rows:?}");
        assert_eq!(pair[1].size.width, pair[0].size.width);
    }
    // Nothing but the padding beside the rows.
    let card = bounds_of(cx, handle, &["monitoring-panel-0".into()])[0];
    assert!(
        rows[0].size.width > card.size.width - px(40.),
        "{rows:?} in {card:?}"
    );
}

/// A legend that scrolls keeps its bottom padding: its rows end above the
/// card's edge, not at it.
#[gpui_kit::test]
fn a_long_legend_stops_short_of_the_cards_edge(cx: &mut TestAppContext) {
    let (handle, _panel) = legend_panel(cx, 30, 640., &[]);
    let found = bounds_of(
        cx,
        handle,
        &[
            "monitoring-panel-0".into(),
            "monitoring-panel-0-legend".into(),
        ],
    );
    let (card, legend) = (found[0], found[1]);
    assert!(
        card.bottom() - legend.bottom() >= px(10.),
        "{legend:?} in {card:?}"
    );
}

/// Six 16-character names on a narrow, short panel list inline one a line,
/// more than half the panel: the legend stops at half, above the card's
/// padding, and scrolls to its last line.
#[gpui_kit::test]
fn a_narrow_inline_legend_scrolls_to_its_last_line(cx: &mut TestAppContext) {
    let names: Vec<String> = (0..6).map(|n| format!("worker-pool-ab-{n}")).collect();
    assert_eq!(names[0].chars().count(), 16);
    let (handle, panel) = named_panel(cx, &names, 250., 200., &[]);
    cx.update(|cx| {
        assert_eq!(
            panel.read(cx).chart().unwrap().legend.mode,
            crate::monitoring::derive::LegendMode::Inline
        )
    });
    let ids = [
        "monitoring-panel-0".to_string(),
        "monitoring-panel-0-plot".into(),
        "monitoring-panel-0-legend".into(),
        "monitoring-panel-0-legend-5".into(),
    ];
    let found = bounds_of(cx, handle, &ids);
    let (card, plot, legend, last) = (found[0], found[1], found[2], found[3]);
    assert!(
        plot.size.height >= legend.size.height,
        "{plot:?} {legend:?}"
    );
    assert!(
        card.bottom() - legend.bottom() >= px(10.),
        "{legend:?} in {card:?}"
    );
    assert!(
        last.bottom() > legend.bottom(),
        "fits unscrolled: {last:?} {legend:?}"
    );
    cx.update_window(handle, |_, window, cx| {
        wheel(window, legend.center(), -400., cx)
    })
    .unwrap();
    let last = bounds_of(cx, handle, &ids[3..])[0];
    assert!(
        last.bottom() <= legend.bottom(),
        "{last:?} below {legend:?}"
    );
    assert!(last.top() >= legend.top(), "{last:?} above {legend:?}");
}

/// A series that stopped early shows its last value muted, with the time in
/// the row's tooltip: its value takes no more room than any other. Seven
/// series, so the legend is a table.
#[gpui_kit::test]
fn a_stopped_series_keeps_its_value_as_wide_as_the_others(cx: &mut TestAppContext) {
    let (handle, panel) = legend_panel(cx, 7, 640., &[0]);
    let stale = cx.read(|cx| {
        let chart = panel.read(cx).chart().unwrap();
        (
            chart.legend.rows[0].stale.clone(),
            chart.legend.rows[1].stale.clone(),
        )
    });
    assert!(stale.0.unwrap().starts_with("Last value at "));
    assert_eq!(stale.1, None);
    let values = bounds_of(
        cx,
        handle,
        &[
            "monitoring-panel-0-legend-0-last".into(),
            "monitoring-panel-0-legend-1-last".into(),
        ],
    );
    assert_eq!(values[0].size.width, values[1].size.width, "{values:?}");
}

/// A narrow chart of five long-named series, answered at once.
fn narrow_chart(cx: &mut TestAppContext, width: f32) -> (AnyWindowHandle, Entity<PanelView>) {
    use freshkube_core::monitoring::{
        PanelResult,
        model::data::{Frame, Series},
    };
    let dashboard = serde_json::json!({"title": "Test", "panels": [{
        "type": "timeseries",
        "title": "Memory",
        "gridPos": {"x": 0, "y": 0, "w": 8, "h": 8},
        "targets": [{"refId": "A", "expr": "memory", "legendFormat": "{{pod}}"}],
    }]});
    let spec = Dashboard::parse(&dashboard.to_string())
        .unwrap()
        .panels
        .remove(0);
    let (handle, panels) = mount_at(cx, vec![Rc::new(spec)], width);
    let times: Vec<f64> = window_range()
        .times()
        .into_iter()
        .map(|t| t as f64)
        .collect();
    let series = (0..5)
        .map(|n| {
            let name = format!("storage-system/instance-manager-{n}-5d8b7c9f4-lwz8r");
            Series {
                name: name.clone(),
                query: "A".into(),
                field: None,
                labels: vec![("pod".into(), name)],
                values: vec![1000. * (n + 1) as f64; times.len()],
            }
        })
        .collect();
    let panel = panels[0].clone();
    cx.update(|cx| {
        panel.update(cx, |panel, cx| {
            let result = PanelResult {
                frame: Frame { times, series },
                warnings: Vec::new(),
                expressions: Vec::new(),
            };
            panel.set_result(result, window_range(), cx);
        })
    });
    frame(cx, handle);
    (handle, panel)
}

/// In a narrow chart, long names truncate so the readout stays inside the
/// plot on either side of the crosshair: just right of the middle it sits
/// to the left, just left of it to the right.
#[gpui_kit::test]
fn a_narrow_readout_stays_inside_the_plot_on_either_side(cx: &mut TestAppContext) {
    let (handle, panel) = narrow_chart(cx, 360.);
    let (plot, readout): (gpui_kit::SharedString, gpui_kit::SharedString) = (
        "monitoring-panel-0-plot".into(),
        "monitoring-panel-0-readout".into(),
    );
    for fraction in [0.62, 0.38, 0.9, 0.1] {
        let (plot, readout) = (plot.clone(), readout.clone());
        cx.update_window(handle, |_, window, cx| {
            let bounds = window.find(plot.clone()).bounds();
            window.dispatch_event(
                gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                    position: gpui_kit::point(
                        bounds.left() + bounds.size.width * fraction,
                        bounds.center().y,
                    ),
                    pressed_button: None,
                    modifiers: Default::default(),
                }),
                cx,
            );
            window.draw(cx).clear(cx);
            let (plot, readout) = (window.find(plot).bounds(), window.find(readout).bounds());
            assert!(
                readout.left() >= plot.left() && readout.right() <= plot.right(),
                "at {fraction}: {readout:?} in {plot:?}"
            );
        })
        .unwrap();
        assert!(cx.read(|cx| panel.read(cx).cursor.is_some()));
    }
}
