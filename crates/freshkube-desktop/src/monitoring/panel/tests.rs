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
    StyleRefinement, Styled, TestAppContext, Window, div, px, size,
};

use super::*;
use crate::desktop::probe;

const END: i64 = 1_700_003_600;

fn window_range() -> TimeWindow {
    TimeWindow::new(END, 6 * 3600, 72)
}

/// The panels of a page, one under another.
struct Host {
    panels: Vec<Entity<PanelView>>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .children(self.panels.iter().map(|panel| {
                div()
                    .flex_none()
                    .w(px(640.))
                    .h(px(320.))
                    .child(panel.clone().cached(StyleRefinement::default().size_full()))
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
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.set_reduce_motion(true);
    });
    let mut panels = Vec::new();
    let height = 320. * specs.len() as f32;
    let window = cx.open_window(size(px(700.), px(height)), |window, cx| {
        panels = specs
            .into_iter()
            .enumerate()
            .map(|(index, spec)| cx.new(|_| PanelView::new(index.to_string(), spec)))
            .collect();
        let host = cx.new(|_| Host {
            panels: panels.clone(),
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
        &format!("monitoring-panel-{alerts}-table")
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
    let (panel_draws, paths) = (
        probe::count("monitoring-panel"),
        probe::count("monitoring-path"),
    );
    move_to(cx, plot.clone(), 0.);
    move_to(cx, plot.clone(), 40.);
    assert!(probe::count("monitoring-panel") > panel_draws);
    // The plot renders again inside its panel, but builds no shape.
    assert_eq!(
        probe::count("monitoring-path"),
        paths,
        "the cursor rebuilt a path"
    );
    let cursor = cx.read(|cx| panels[cpu].read(cx).cursor.clone()).unwrap();
    assert!(cursor.own);
    assert!(!cursor.rows.is_empty());
    assert!(!cursor.time.is_empty());
    assert!(shown(
        cx,
        handle,
        &format!("monitoring-panel-{cpu}-readout")
    ));
    let time = match events.borrow().last() {
        Some(PanelEvent::Cursor(Some(time))) => *time,
        other => panic!("{other:?}"),
    };

    // The page passes it on; the other chart shows a crosshair, no readout.
    cx.update(|cx| panels[memory].update(cx, |panel, cx| panel.show_cursor(Some(time), cx)));
    let shared = cx
        .read(|cx| panels[memory].read(cx).cursor.clone())
        .unwrap();
    assert!(!shared.own);
    assert_eq!(shared.index, cursor.index);
    assert!(!shown(
        cx,
        handle,
        &format!("monitoring-panel-{memory}-readout")
    ));

    // Leaving the plot clears it and says so.
    cx.update_window(handle, |_, window, cx| window.hover(title.clone(), cx))
        .unwrap();
    assert!(cx.read(|cx| panels[cpu].read(cx).cursor.is_none()));
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
    let (handle, panels) = mount(cx, specs);
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

/// Forty series past the six colours: the grey lines draw as one path and
/// the grey areas as another, not one of each per series.
#[gpui_kit::test]
fn a_crowded_chart_draws_lines_of_one_look_as_one_path(cx: &mut TestAppContext) {
    let (handle, panels, result) = crowded(cx);
    let before = probe::count("monitoring-path");
    cx.update(|cx| panels[0].update(cx, |panel, cx| panel.set_result(result, window_range(), cx)));
    frame(cx, handle);
    let series = cx.read(|cx| panels[0].read(cx).chart().unwrap().series.len());
    assert_eq!(series, 40);
    // Six coloured series and the grey rest, each a line and an area.
    assert_eq!(probe::count("monitoring-path") - before, 7 * 2);
}
