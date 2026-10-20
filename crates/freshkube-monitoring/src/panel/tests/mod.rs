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
use crate::probe;
use freshkube_ui::table::TableSource;

mod crowded;
mod cursor;
mod info;
mod legend;
mod markers;
mod readout;
mod states;
mod table;

const END: i64 = 1_700_003_600;

pub(super) fn window_range() -> TimeWindow {
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
pub(super) fn cluster() -> (Dashboard, Vec<Rc<PanelSpec>>) {
    let dashboard = Dashboard::parse(BUILTINS[0].json).unwrap();
    let specs = dashboard.panels.iter().cloned().map(Rc::new).collect();
    (dashboard, specs)
}

pub(super) fn mount(
    cx: &mut TestAppContext,
    specs: Vec<Rc<PanelSpec>>,
) -> (AnyWindowHandle, Vec<Entity<PanelView>>) {
    mount_at(cx, specs, 640.)
}

/// Mounts the panels `width` wide.
pub(super) fn mount_at(
    cx: &mut TestAppContext,
    specs: Vec<Rc<PanelSpec>>,
    width: f32,
) -> (AnyWindowHandle, Vec<Entity<PanelView>>) {
    mount_sized(cx, specs, width, 320.)
}

/// Panels `width` wide and `height` high, one under another.
pub(super) fn mount_sized(
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
            .map(|(index, spec)| cx.new(|cx| PanelView::new(index.to_string(), spec, cx)))
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
pub(super) fn answer(cx: &mut TestAppContext, dashboard: &Dashboard, panels: &[Entity<PanelView>]) {
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

pub(super) fn frame(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

pub(super) fn shown(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) -> bool {
    let id = id.to_owned();
    cx.update_window(handle, move |_, window, cx| {
        window.render_frame(cx);
        window.try_find(gpui_kit::SharedString::from(id)).is_some()
    })
    .unwrap()
}

pub(super) fn index_of(specs: &[Rc<PanelSpec>], title: &str) -> usize {
    specs.iter().position(|spec| spec.title == title).unwrap()
}

/// A chart of forty pods, pod n's values about 3n, mounted unanswered, and
/// its answer.
pub(super) fn crowded(
    cx: &mut TestAppContext,
) -> (
    AnyWindowHandle,
    Vec<Entity<PanelView>>,
    freshkube_core::monitoring::PanelResult,
) {
    crowded_at(cx, 640.)
}

/// [`crowded`], `width` wide.
pub(super) fn crowded_at(
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

/// A narrow chart of five long-named series, answered at once.
pub(super) fn narrow_chart(
    cx: &mut TestAppContext,
    width: f32,
) -> (AnyWindowHandle, Entity<PanelView>) {
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
