//! A timeseries legend: inline under a few series, else rows with a value
//! per heading, in two columns where they fit. Hovering a row fades the
//! other lines; a click keeps that series in front until clicked again. A
//! series that stopped before the window's end shows its last value muted,
//! with the time in the row's tooltip. A chart with more series than the
//! cap ends with a line saying how many it draws, and a way to draw all.
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Div, Stateful, TestSupportExt, div, px, relative};

use super::PanelView;
use crate::monitoring::derive::{Capped, Chart, LegendMode, LegendRow, MOST_SERIES};
use crate::palette::palette;
use crate::ui::{self, dp};

pub(super) fn legend(view: &PanelView, chart: &Chart, cx: &mut Context<PanelView>) -> AnyElement {
    #[cfg(test)]
    crate::desktop::probe::hit("monitoring-legend");
    match chart.legend.mode {
        LegendMode::Hidden => match chart.capped {
            Some(capped) => div()
                .px(dp(12.))
                .pb(dp(10.))
                .child(cap_line(view, capped, cx))
                .into_any_element(),
            None => div().into_any_element(),
        },
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
                            .id(view.element_id(&format!("legend-{}-last", row.series)))
                            .font_family(ui::MONO_FONT)
                            .text_size(dp(12.))
                            .text_color(last_color(row, cx))
                            .child(value.clone())
                            .test_support(),
                    )
                })
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
                .when(index > 0, |this| this.w(dp(legend.column)).text_right())
                .child(heading.clone())
        }));
    let mut rows = Vec::new();
    for row in &legend.rows {
        rows.push(table_row(view, chart, row, cx).into_any_element());
    }
    // An odd last row shares its line with an empty one, so it keeps half
    // the width as every other row does; in one column it takes no room.
    if legend.rows.len() % 2 == 1 {
        rows.push(half(div()).into_any_element());
    }
    v_flex()
        .flex_none()
        .max_h(relative(0.5))
        // Outside the scroll, so rows that overflow stop short of the
        // card's edge.
        .pb(dp(10.))
        .child(headings)
        .child(
            v_flex()
                .id(view.element_id("legend"))
                .min_h_0()
                .overflow_y_scroll()
                .restrict_scroll_to_axis()
                .gap(dp(2.))
                .px(dp(12.))
                .pt(dp(3.))
                .child(h_flex().flex_wrap().gap_x(dp(18.)).children(rows))
                .when(legend.more > 0, |this| {
                    this.child(
                        div()
                            .px(dp(4.))
                            .text_size(dp(11.))
                            .text_color(p.muted)
                            .child(format!("and {} more", legend.more)),
                    )
                })
                .test_support(),
        )
        .when_some(chart.capped, |this, capped| {
            this.child(
                div()
                    .px(dp(12.))
                    .pt(dp(2.))
                    .child(cap_line(view, capped, cx)),
            )
        })
        .into_any_element()
}

/// How many series a capped chart draws, and the switch between the
/// highest peaks and all of them.
fn cap_line(view: &PanelView, capped: Capped, cx: &mut Context<PanelView>) -> impl IntoElement {
    let p = palette(cx);
    let (said, action) = if capped.all {
        (
            format!("Drawing all {}", capped.of),
            format!("Top {MOST_SERIES}"),
        )
    } else {
        (
            format!(
                "Drawing {MOST_SERIES} of {}, highest peaks first",
                capped.of
            ),
            "Show all".into(),
        )
    };
    let all = !capped.all;
    h_flex()
        .id(view.element_id("cap"))
        .gap(dp(6.))
        .px(dp(4.))
        .text_size(dp(11.))
        .text_color(p.muted)
        .child(said)
        .child("·")
        .child(
            Button::new(view.element_id("cap-toggle"))
                .link()
                .xsmall()
                .label(action)
                .on_click(cx.listener(move |view, _, _, cx| view.show_all_series(all, cx))),
        )
        .test_support()
}

fn table_row(
    view: &PanelView,
    chart: &Chart,
    row: &LegendRow,
    cx: &mut Context<PanelView>,
) -> impl IntoElement {
    let p = palette(cx);
    let mut values = row.values.iter();
    half(entry(view, chart, row, cx))
        .gap(dp(8.))
        .h(dp(20.))
        // The rows' spacing, which the empty row beside an odd last one
        // doesn't take.
        .my(dp(1.))
        .px(dp(4.))
        .rounded(px(3.))
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
                div()
                    .id(view.element_id(&format!("legend-{}-last", row.series)))
                    .flex_none()
                    .whitespace_nowrap()
                    .font_family(ui::MONO_FONT)
                    .text_size(dp(12.))
                    .text_color(last_color(row, cx))
                    .child(value.clone())
                    .test_support(),
            )
        })
        .children(values.map(|value| {
            div()
                .flex_none()
                .w(dp(chart.legend.column))
                .whitespace_nowrap()
                .text_right()
                .font_family(ui::MONO_FONT)
                .text_size(dp(12.))
                .text_color(p.muted)
                .child(value.clone())
        }))
        .test_support()
}

/// Half a line where two rows of at least 160 fit, else the whole line.
fn half<E: Styled>(element: E) -> E {
    element
        .flex_grow(1.)
        .flex_shrink_0()
        .flex_basis(relative(0.4))
        .min_w(dp(160.))
}

/// A stopped series' last value is muted, as last-known values are.
fn last_color(row: &LegendRow, cx: &Context<PanelView>) -> gpui_kit::Hsla {
    let p = palette(cx);
    if row.stale.is_some() { p.muted } else { p.ink }
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
        .when(picked, |this| this.bg(p.hover).rounded(px(3.)))
        .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
            if *hovered {
                view.set_hovered(Some(series), cx);
            } else if view.hovered == Some(series) {
                view.set_hovered(None, cx);
            }
        }))
        .on_click(cx.listener(move |view, _, _, cx| view.toggle_picked(series, cx)))
        .when_some(row.stale.clone(), |this, tip| {
            this.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        })
        .child(
            div()
                .flex_none()
                .w(dp(14.))
                .h(px(2.))
                .rounded(px(3.))
                .bg(color),
        )
}
