//! A timeseries legend: inline under a few series, else rows with a value
//! per heading, in two columns where they fit. Hovering a row fades the
//! other lines; a click keeps that series in front until clicked again.
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Div, Stateful, TestSupportExt, div, px, relative};

use super::PanelView;
use crate::monitoring::derive::{Chart, LegendMode, LegendRow};
use crate::palette::palette;
use crate::ui::{self, dp};

pub(super) fn legend(view: &PanelView, chart: &Chart, cx: &mut Context<PanelView>) -> AnyElement {
    match chart.legend.mode {
        LegendMode::Hidden => div().into_any_element(),
        LegendMode::Inline => inline(view, chart, cx),
        LegendMode::Table => table(view, chart, cx),
    }
}

fn inline(view: &PanelView, chart: &Chart, cx: &mut Context<PanelView>) -> AnyElement {
    let p = palette(cx);
    h_flex()
        .flex_none()
        .flex_wrap()
        .gap_x(dp(16.))
        .gap_y(dp(2.))
        .px(dp(16.))
        .pt(dp(4.))
        .pb(dp(10.))
        .text_size(dp(11.5))
        .children(chart.legend.rows.iter().map(|row| {
            entry(view, chart, row, cx)
                .gap(dp(6.))
                .child(div().text_color(p.muted).child(row.name.clone()))
                .when_some(row.values.first(), |this, value| {
                    this.child(
                        div()
                            .font_family(ui::MONO_FONT)
                            .text_size(dp(12.))
                            .text_color(p.ink)
                            .child(value.clone()),
                    )
                })
                .when_some(row.stale.clone(), |this, at| this.child(stale(at, cx)))
                .test_support()
        }))
        .into_any_element()
}

fn table(view: &PanelView, chart: &Chart, cx: &mut Context<PanelView>) -> AnyElement {
    let p = palette(cx);
    let legend = &chart.legend;
    let headings = h_flex()
        .justify_end()
        .gap(dp(18.))
        .px(dp(16.))
        .pt(dp(2.))
        .text_size(dp(10.5))
        .text_color(p.muted)
        .children(legend.headings.iter().enumerate().map(|(index, heading)| {
            div()
                .when(index > 0, |this| this.w(dp(36.)).text_right())
                .child(heading.clone())
        }));
    let mut rows = Vec::new();
    for row in &legend.rows {
        rows.push(table_row(view, chart, row, cx).into_any_element());
    }
    v_flex()
        .flex_none()
        .max_h(relative(0.5))
        .child(headings)
        .child(
            v_flex()
                .id(view.element_id("legend"))
                .min_h_0()
                .overflow_y_scroll()
                .restrict_scroll_to_axis()
                .gap(dp(2.))
                .px(dp(12.))
                .pt(dp(4.))
                .pb(dp(10.))
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_x(dp(18.))
                        .gap_y(dp(2.))
                        .children(rows),
                )
                .when(legend.more > 0, |this| {
                    this.child(
                        div()
                            .px(dp(4.))
                            .text_size(dp(11.))
                            .text_color(p.muted)
                            .child(format!("and {} more", legend.more)),
                    )
                }),
        )
        .into_any_element()
}

fn table_row(
    view: &PanelView,
    chart: &Chart,
    row: &LegendRow,
    cx: &mut Context<PanelView>,
) -> impl IntoElement {
    let p = palette(cx);
    let mut values = row.values.iter();
    // Two columns where both fit; one in a narrow panel, so names show.
    entry(view, chart, row, cx)
        .flex_grow(1.)
        .flex_basis(dp(160.))
        .min_w_0()
        .gap(dp(8.))
        .h(dp(20.))
        .px(dp(4.))
        .rounded(px(5.))
        .hover(|this| this.bg(p.hover))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(ui::MONO_FONT)
                .text_size(dp(11.5))
                .text_color(p.ink_2)
                .child(row.name.clone()),
        )
        .when_some(values.next(), |this, value| {
            this.child(
                h_flex()
                    .flex_none()
                    .gap(dp(4.))
                    .font_family(ui::MONO_FONT)
                    .text_size(dp(12.))
                    .text_color(p.ink)
                    .child(value.clone())
                    .when_some(row.stale.clone(), |this, at| this.child(stale(at, cx))),
            )
        })
        .children(values.map(|value| {
            div()
                .flex_none()
                .w(dp(44.))
                .text_right()
                .font_family(ui::MONO_FONT)
                .text_size(dp(12.))
                .text_color(p.muted)
                .child(value.clone())
        }))
        .test_support()
}

/// A row's swatch, hover and click, shared by both modes.
fn entry(
    view: &PanelView,
    chart: &Chart,
    row: &LegendRow,
    cx: &mut Context<PanelView>,
) -> Stateful<Div> {
    let p = palette(cx);
    let series = row.series;
    let color = chart.series[series].ink.color(view.focus() == Some(series));
    let picked = view.picked == Some(series);
    h_flex()
        .id(view.element_id(&format!("legend-{series}")))
        .flex_none()
        .cursor_pointer()
        .when(picked, |this| this.bg(p.hover).rounded(px(5.)))
        .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
            if *hovered {
                view.set_hovered(Some(series), cx);
            } else if view.hovered == Some(series) {
                view.set_hovered(None, cx);
            }
        }))
        .on_click(cx.listener(move |view, _, _, cx| view.toggle_picked(series, cx)))
        .child(
            div()
                .flex_none()
                .w(dp(14.))
                .h(px(2.))
                .rounded(px(1.))
                .bg(color),
        )
}

/// The time of a series' last value, when it stopped before the window's
/// end.
fn stale(at: gpui_kit::SharedString, cx: &Context<PanelView>) -> impl IntoElement {
    div().text_color(palette(cx).muted).child(format!("({at})"))
}
