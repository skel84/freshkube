//! A fixture check for the panels before the Monitoring page draws them:
//! the first built-in dashboard on Grafana's grid, from example data.
//! `FRESHKUBE_PAGE=monitoring-panels` with `--fixture` opens it.
use std::rc::Rc;

use freshkube_core::monitoring::{
    ExampleSource,
    builtin::BUILTINS,
    model::{Dashboard, data::QueryContext, layout, time::TimeWindow},
};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Entity, IntoElement, Render, StyleRefinement, Subscription, TestSupportExt, Window,
    div, relative,
};

use super::panel::{PanelEvent, PanelView};
use crate::palette::palette;
use crate::ui::{self, dp};

pub(crate) struct Gallery {
    title: gpui_kit::SharedString,
    /// Each panel with its place: columns from the left and wide, and grid
    /// rows from the top and high.
    panels: Vec<(Entity<PanelView>, [u32; 4])>,
    height: u32,
    _subscriptions: Vec<Subscription>,
}

impl Gallery {
    /// The dashboard over the six hours to `end`, in Unix seconds.
    pub(crate) fn new(end: i64, cx: &mut Context<Self>) -> Self {
        let builtin = BUILTINS[0];
        let dashboard = Dashboard::parse(builtin.json).expect("built-in dashboards parse");
        let window = TimeWindow::new(end, 6 * 3600, 72);
        let source = ExampleSource::new(builtin.uid);
        let (variables, _) = source
            .resolve_variables(&dashboard.variables, &[], window)
            .unwrap_or_default();
        let context = QueryContext {
            window,
            variables: variables.context_values(),
        };
        let mut panels = Vec::new();
        let mut subscriptions = Vec::new();
        let mut height = 0;
        for section in &dashboard.sections {
            for placed in &section.panels {
                let spec = Rc::new(dashboard.panels[placed.key].clone());
                let result = source.query_panel(&spec, &context, &variables);
                let panel = cx.new(|cx| {
                    let mut panel = PanelView::new(placed.key.to_string(), spec);
                    match result {
                        Ok(result) => panel.set_result(result, window, cx),
                        Err(error) => panel.set_error(&error, cx),
                    }
                    panel
                });
                subscriptions.push(cx.subscribe(&panel, Self::on_panel));
                let pos = placed.pos;
                height = height.max(pos.y + pos.h);
                panels.push((panel, [pos.x, pos.w, pos.y, pos.h]));
            }
        }
        Self {
            title: dashboard.title.into(),
            panels,
            height,
            _subscriptions: subscriptions,
        }
    }

    /// One chart's cursor shows on the others at the same time.
    fn on_panel(&mut self, from: Entity<PanelView>, event: &PanelEvent, cx: &mut Context<Self>) {
        let PanelEvent::Cursor(time) = event;
        for (panel, _) in &self.panels {
            if *panel != from {
                panel.update(cx, |panel, cx| panel.show_cursor(*time, cx));
            }
        }
    }
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let (row, gap) = (layout::ROW_HEIGHT, layout::GAP);
        let columns = layout::COLUMNS as f32;
        v_flex()
            .id("monitoring-gallery")
            .size_full()
            .overflow_y_scroll()
            .bg(p.surface_2)
            .p(dp(16.))
            .gap(dp(12.))
            .child(ui::page_title(self.title.clone()))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .w_full()
                    .h(dp(self.height as f32 * (row + gap)))
                    .children(self.panels.iter().map(|(panel, [x, w, y, h])| {
                        div()
                            .absolute()
                            .left(relative(*x as f32 / columns))
                            .w(relative(*w as f32 / columns))
                            .top(dp(*y as f32 * (row + gap)))
                            .h(dp(*h as f32 * (row + gap)))
                            .pr(dp(gap))
                            .pb(dp(gap))
                            .child(panel.clone().cached(StyleRefinement::default().size_full()))
                    })),
            )
            .test_support()
    }
}
