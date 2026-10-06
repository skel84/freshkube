//! One dashboard panel in the Console look: a card with its title, the
//! PromQL behind it on hover, and a body for its kind. Display data comes
//! from `derive` when an answer arrives; `render` only reads it.
//!
//! A timeseries draws its plot in a cached child view ([`PlotView`]), so a
//! moving cursor or a hovered legend row redraws only the overlay above it.
//! A table is a child view too ([`TableView`]), which keeps its scroll.
mod cursor;
mod legend;
mod markers;
mod plot;
mod summary;
mod table;
#[cfg(test)]
mod tests;

use std::cell::Cell;
use std::rc::Rc;

use freshkube_core::monitoring::{
    PanelResult, QueryError,
    markers::Marker,
    model::{PanelSpec, time::TimeWindow},
};
use freshkube_ui::card::{CardHeader, ChartCard, StatCard};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Context, Entity, EventEmitter, IntoElement, Render, SharedString, StyleRefinement,
    TestSupportExt, Window, div,
};

use super::derive::{self, Body, PanelData};
use crate::ui::{self, dp};

pub(crate) use cursor::Cursor;
pub(crate) use markers::glyph as marker_glyph;
pub(crate) use plot::{Geometry, PlotView};
pub(crate) use table::TableView;

/// What a panel tells its page.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PanelEvent {
    /// The pointer moved over the plot to this sample time, or left it.
    Cursor(Option<f64>),
}

#[derive(Clone, Debug, PartialEq)]
enum State {
    /// Nothing asked yet, or the first answer is on its way.
    Loading,
    Ready,
    /// The query failed with no earlier answer to show.
    Failed(SharedString),
}

pub(crate) struct PanelView {
    id: SharedString,
    spec: Rc<PanelSpec>,
    state: State,
    data: Option<PanelData>,
    /// Why the answer shown is old: the refresh that failed.
    stale: Option<SharedString>,
    /// The PromQL sent, after interpolation, and anything the panel can't
    /// show, for the title's tooltip.
    about: SharedString,
    /// Just the PromQL, which a click on the info icon copies.
    promql: SharedString,
    plot: Option<Entity<PlotView>>,
    table: Option<Entity<TableView>>,
    /// The plot's inner rectangle, set when the plot paints.
    geometry: Rc<Cell<Geometry>>,
    /// The legend row under the pointer, and the one picked by a click.
    hovered: Option<usize>,
    picked: Option<usize>,
    cursor: Option<Cursor>,
    /// The page's markers, and those on this chart's window.
    markers: Rc<[Marker]>,
    placed: Rc<[markers::Placed]>,
}

impl EventEmitter<PanelEvent> for PanelView {}

impl PanelView {
    /// A panel for `spec`, waiting for its first answer. `id` tells it apart
    /// from the page's other panels, as `monitoring-panel-<id>`.
    pub(crate) fn new(id: impl Into<SharedString>, spec: Rc<PanelSpec>) -> Self {
        let about = about(&spec, &[], &[]);
        let promql = promql(&spec, &[]);
        Self {
            id: format!("monitoring-panel-{}", id.into()).into(),
            spec,
            state: State::Loading,
            data: None,
            stale: None,
            about,
            promql,
            plot: None,
            table: None,
            geometry: Rc::default(),
            hovered: None,
            picked: None,
            cursor: None,
            markers: Rc::from([]),
            placed: Rc::from([]),
        }
    }

    /// A new answer over `window`: derives what it shows and drops any
    /// stale mark.
    pub(crate) fn set_result(
        &mut self,
        result: PanelResult,
        window: TimeWindow,
        cx: &mut Context<Self>,
    ) {
        let data = derive::derive(&self.spec, result.frame, window);
        self.about = about(&self.spec, &result.expressions, &result.warnings);
        self.promql = promql(&self.spec, &result.expressions);
        self.plot = match &data.body {
            Body::Chart(chart) => Some(match self.plot.take() {
                Some(plot) => {
                    plot.update(cx, |plot, cx| plot.set_chart(chart.clone(), cx));
                    plot
                }
                None => {
                    let (chart, geometry) = (chart.clone(), self.geometry.clone());
                    let id = self.id.clone();
                    cx.new(|_| PlotView::new(id, chart, geometry))
                }
            }),
            _ => None,
        };
        self.table = match &data.body {
            Body::Table(rows) => Some(match self.table.take() {
                Some(table) => {
                    table.update(cx, |table, cx| {
                        table.set_data(rows.clone());
                        cx.notify();
                    });
                    table
                }
                None => {
                    let (id, title) = (self.id.clone(), self.spec.title.clone());
                    cx.new(|_| TableView::new(&id, title.into(), rows.clone()))
                }
            }),
            _ => None,
        };
        self.data = Some(data);
        self.state = State::Ready;
        self.stale = None;
        self.cursor = None;
        self.hovered = None;
        self.picked = None;
        self.place_markers(cx);
        cx.notify();
    }

    /// The deploys and node events to draw, already filtered by the page.
    pub(crate) fn set_markers(&mut self, markers: Rc<[Marker]>, cx: &mut Context<Self>) {
        if Rc::ptr_eq(&self.markers, &markers) {
            return;
        }
        self.markers = markers;
        self.place_markers(cx);
        cx.notify();
    }

    fn place_markers(&mut self, cx: &mut Context<Self>) {
        let placed = self
            .chart()
            .map(|chart| markers::place(&self.markers, &chart))
            .unwrap_or_else(|| Rc::from([]));
        if placed == self.placed {
            return;
        }
        self.placed = placed.clone();
        if let Some(plot) = &self.plot {
            plot.update(cx, |plot, cx| plot.set_markers(placed, cx));
        }
    }

    /// The markers on this chart, for the page's tests.
    #[cfg(test)]
    pub(crate) fn marker_labels(&self) -> Vec<SharedString> {
        self.placed
            .iter()
            .map(|marker| marker.label.clone())
            .collect()
    }

    /// A failed query: the last answer stays, marked stale, else the
    /// failure shows in the card.
    pub(crate) fn set_error(&mut self, error: &QueryError, cx: &mut Context<Self>) {
        let message: SharedString = error.to_string().into();
        if self.data.is_some() {
            self.stale = Some(message);
        } else {
            self.state = State::Failed(message);
        }
        cx.notify();
    }

    /// Whether an answer shows, for the page's tests.
    #[cfg(test)]
    pub(crate) fn is_ready(&self) -> bool {
        self.state == State::Ready
    }

    /// The table's view, for the page's tests.
    #[cfg(test)]
    pub(crate) fn table(&self) -> Option<Entity<TableView>> {
        self.table.clone()
    }

    /// Whether a crosshair shows, for the page's tests.
    #[cfg(test)]
    pub(crate) fn has_cursor(&self) -> bool {
        self.cursor.is_some()
    }

    fn focus(&self) -> Option<usize> {
        self.hovered.or(self.picked)
    }

    fn set_hovered(&mut self, series: Option<usize>, cx: &mut Context<Self>) {
        if self.hovered != series {
            self.hovered = series;
            self.focus_changed(cx);
        }
    }

    fn toggle_picked(&mut self, series: usize, cx: &mut Context<Self>) {
        self.picked = (self.picked != Some(series)).then_some(series);
        self.focus_changed(cx);
    }

    fn focus_changed(&mut self, cx: &mut Context<Self>) {
        let focus = self.focus();
        if let Some(plot) = &self.plot {
            plot.update(cx, |plot, cx| plot.set_focus(focus, cx));
        }
        cx.notify();
    }

    fn element_id(&self, part: &str) -> SharedString {
        format!("{}-{part}", self.id).into()
    }
}

/// Each query's PromQL as sent, after interpolation, or as written before
/// the first answer.
fn promql(spec: &PanelSpec, expressions: &[(String, String)]) -> SharedString {
    let lines: Vec<String> = if expressions.is_empty() {
        spec.queries
            .iter()
            .filter_map(|query| query.text.clone())
            .collect()
    } else {
        let several = expressions.len() > 1;
        expressions
            .iter()
            .map(|(id, expr)| {
                if several {
                    format!("{id}: {expr}")
                } else {
                    expr.clone()
                }
            })
            .collect()
    };
    lines.join("\n").into()
}

/// The title's tooltip: each query's PromQL, then any warnings and the
/// settings the panel doesn't draw.
fn about(spec: &PanelSpec, expressions: &[(String, String)], warnings: &[String]) -> SharedString {
    let mut lines: Vec<String> = Vec::new();
    let promql = promql(spec, expressions);
    if !promql.is_empty() {
        lines.push(promql.to_string());
    }
    lines.extend(warnings.iter().cloned());
    if !spec.ignored.is_empty() {
        lines.push(format!("Not drawn: {}", spec.ignored.join(", ")));
    }
    lines.join("\n").into()
}

impl Render for PanelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        crate::desktop::probe::hit("monitoring-panel");
        let _span = crate::perf::span("monitoring.panel_render");
        let stat = matches!(
            self.data.as_ref().map(|data| &data.body),
            Some(Body::Stats(_))
        );
        let header = CardHeader::new(self.id.clone(), self.spec.title.clone())
            .unit(self.data.as_ref().and_then(|data| data.unit.clone()))
            .about(self.about.clone())
            .copy(self.promql.clone(), "Click to copy the PromQL")
            .stale(self.stale.clone());
        let body = self.render_body(window, cx);
        if stat {
            StatCard::new(header).render(body, cx)
        } else {
            ChartCard::new(header).render(body, cx)
        }
    }
}

impl PanelView {
    fn render_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match (&self.state, &self.data) {
            (State::Failed(error), _) => summary::message(
                self.element_id("failed"),
                Some(ui::Tone::Crit),
                error.clone(),
                cx,
            ),
            (State::Loading, _) | (_, None) => summary::loading(cx),
            (State::Ready, Some(data)) => match data.body.clone() {
                Body::Chart(chart) => self.render_chart(&chart, window, cx),
                Body::Stats(stats) => summary::stats(&stats, cx),
                Body::Bars(rows) => summary::bars(self.element_id("bars"), &rows, cx),
                Body::Table(_) => self
                    .table
                    .clone()
                    .map_or_else(|| div().into_any_element(), IntoElement::into_any_element),
                Body::Text(text) => summary::text(text, cx),
                Body::NoData => {
                    summary::message(self.element_id("empty"), None, "No data".into(), cx)
                }
                Body::NotDrawn(kind) => summary::message(
                    self.element_id("not-drawn"),
                    None,
                    derive::not_drawn(&kind).into(),
                    cx,
                ),
            },
        }
    }

    fn render_chart(
        &mut self,
        chart: &Rc<derive::Chart>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(plot) = self.plot.clone() else {
            return div().into_any_element();
        };
        v_flex()
            .size_full()
            .child(
                div().flex_1().min_h(dp(64.)).px(dp(12.)).pt(dp(2.)).child(
                    div()
                        .id(self.element_id("plot"))
                        .relative()
                        .size_full()
                        .child(plot.cached(StyleRefinement::default().size_full()))
                        .child(self.render_cursor(chart, window, cx))
                        .test_support(),
                ),
            )
            .child(legend::legend(self, chart, cx))
            .into_any_element()
    }
}
