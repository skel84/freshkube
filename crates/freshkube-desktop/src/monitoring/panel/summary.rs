//! The bodies of panels that aren't plots: stats, bar lists, tables, text,
//! and the loading, empty and failed states. Everything they show was
//! derived when the answer arrived.
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Bounds, FontWeight, Hsla, PathBuilder, Pixels, SharedString, TestSupportExt,
    canvas, div, fill, point, px, relative, size,
};

use crate::monitoring::colors::{Ink, Tier};
use crate::monitoring::derive::{BarRow, Spark, Stat, TableData};
use crate::palette::palette;
use crate::ui::{self, dp, dp_px};

fn tone(tier: Tier) -> ui::Tone {
    match tier {
        Tier::Warn => ui::Tone::Warn,
        Tier::Crit => ui::Tone::Crit,
    }
}

/// A quiet line in the middle of the card, with a status glyph when it
/// carries one.
pub(super) fn message(
    id: SharedString,
    tone: Option<ui::Tone>,
    text: SharedString,
    cx: &App,
) -> AnyElement {
    let p = palette(cx);
    h_flex()
        .id(id)
        .size_full()
        .justify_center()
        .gap(dp(6.))
        .px(dp(16.))
        .text_size(dp(12.))
        .text_color(if tone.is_some() { p.ink_2 } else { p.muted })
        .children(tone.and_then(|tone| ui::status_glyph(tone, cx)))
        .child(div().min_w_0().whitespace_normal().child(text))
        .test_support()
        .into_any_element()
}

pub(super) fn loading(_: &App) -> AnyElement {
    v_flex()
        .size_full()
        .gap(dp(8.))
        .px(dp(14.))
        .pt(dp(6.))
        .pb(dp(12.))
        .child(ui::skeleton(relative(0.4), dp(14.)))
        .child(ui::skeleton(relative(1.), dp(48.)))
        .into_any_element()
}

pub(super) fn stats(stats: &[Stat], cx: &App) -> AnyElement {
    if let [stat] = stats {
        return v_flex()
            .size_full()
            .gap(dp(4.))
            .px(dp(14.))
            .pb(dp(10.))
            .child(stat_value(stat, dp(22.), cx))
            .children(stat.gauge.map(|gauge| bar(gauge, stat.tier, cx)))
            .children(stat.spark.clone().map(|spark| sparkline(spark, cx)))
            .into_any_element();
    }
    let p = palette(cx);
    h_flex()
        .size_full()
        .flex_wrap()
        .content_start()
        .gap_x(dp(20.))
        .gap_y(dp(8.))
        .px(dp(14.))
        .pb(dp(10.))
        .children(stats.iter().map(|stat| {
            v_flex()
                .min_w(dp(96.))
                .gap(dp(2.))
                .children(stat.name.clone().map(|name| {
                    div()
                        .max_w(dp(220.))
                        .truncate()
                        .font_family(ui::MONO_FONT)
                        .text_size(dp(11.5))
                        .text_color(p.muted)
                        .child(name)
                }))
                .child(stat_value(stat, dp(18.), cx))
                .children(stat.gauge.map(|gauge| bar(gauge, stat.tier, cx)))
                .children(stat.spark.clone().map(|spark| sparkline(spark, cx)))
        }))
        .into_any_element()
}

fn stat_value(stat: &Stat, text_size: gpui_kit::Rems, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    h_flex()
        .gap(dp(8.))
        .child(
            div()
                .text_size(text_size)
                .font_weight(ui::TITLE_WEIGHT)
                .text_color(p.ink)
                .whitespace_nowrap()
                .child(stat.value.clone()),
        )
        .children(stat.note.clone().map(|note| {
            match stat.tier {
                Some(tier) => ui::tag(tone(tier), None, note, cx).into_any_element(),
                None => div()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child(note)
                    .into_any_element(),
            }
        }))
}

/// A gauge's fill, or a bar list's.
fn bar(fraction: f32, tier: Option<Tier>, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    let color = tier.map_or(p.accent, |tier| tier.color(cx));
    div()
        .flex_none()
        .w_full()
        .h(dp(6.))
        .rounded(px(3.))
        .bg(p.track)
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(relative(fraction.clamp(0., 1.)))
                .rounded(px(3.))
                .bg(color),
        )
}

/// Grey history, and the last stretch in the first series colour. Status
/// belongs to the stat's glyph, never to its timeseries.
fn sparkline(spark: Spark, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    let (history, recent) = (p.faint, Ink::Slot(0).color(false));
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            let count = spark.ys.len();
            if count < 2 {
                return;
            }
            let inset = px(2.);
            let (left, top) = (bounds.origin.x + px(1.), bounds.origin.y + inset);
            let width = bounds.size.width - px(2.) - dp_px(3., window);
            let height = bounds.size.height - inset * 2.;
            let at = |index: usize| {
                point(
                    left + width * (index as f32 / (count - 1) as f32),
                    top + height * (1. - spark.ys[index]),
                )
            };
            let stroke = |from: usize, to: usize, color: Hsla, window: &mut gpui_kit::Window| {
                let mut path = PathBuilder::stroke(px(1.5));
                let mut drawing = false;
                for index in from..=to {
                    if !spark.ys[index].is_finite() {
                        drawing = false;
                        continue;
                    }
                    if drawing {
                        path.line_to(at(index));
                    } else {
                        path.move_to(at(index));
                        drawing = true;
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            };
            stroke(0, spark.tail, history, window);
            stroke(spark.tail, count - 1, recent, window);
            if let Some(last) = spark.ys.iter().rposition(|y| y.is_finite()) {
                let r = dp_px(3., window);
                window.paint_quad(
                    fill(Bounds::centered_at(at(last), size(r * 2., r * 2.)), recent)
                        .corner_radii(r),
                );
            }
        },
    )
    .flex_none()
    .w_full()
    .max_w(dp(156.))
    .h(dp(24.))
}

pub(super) fn bars(id: SharedString, rows: &[BarRow], cx: &App) -> AnyElement {
    let p = palette(cx);
    v_flex()
        .id(id)
        .size_full()
        .overflow_y_scroll()
        .px(dp(14.))
        .pt(dp(2.))
        .pb(dp(8.))
        .children(rows.iter().map(|row| {
            h_flex()
                .flex_none()
                .h(dp(26.))
                .gap(dp(10.))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .font_family(ui::MONO_FONT)
                        .text_size(dp(11.5))
                        .children(row.prefix.clone().map(|prefix| {
                            div()
                                .flex_none()
                                .whitespace_nowrap()
                                .text_color(p.muted)
                                .child(prefix)
                        }))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(p.ink_2)
                                .child(row.name.clone()),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .w(dp(90.))
                        .child(bar(row.fraction, row.tier, cx)),
                )
                .child(
                    div()
                        .flex_none()
                        .w(dp(48.))
                        .text_right()
                        .whitespace_nowrap()
                        .font_family(ui::MONO_FONT)
                        .text_size(dp(12.))
                        .text_color(if row.missing { p.muted } else { p.ink })
                        .child(row.value.clone()),
                )
        }))
        .test_support()
        .into_any_element()
}

pub(super) fn table(id: SharedString, table: &TableData, cx: &App) -> AnyElement {
    let p = palette(cx);
    let cell = |numeric: bool| {
        div()
            .flex_1()
            .min_w(dp(64.))
            .truncate()
            .when(numeric, |this| this.text_right().font_family(ui::MONO_FONT))
    };
    let header = h_flex()
        .flex_none()
        .h(dp(28.))
        .gap(dp(12.))
        .px(dp(14.))
        .children(table.columns.iter().map(|column| {
            cell(false)
                .when(column.numeric, |this| this.text_right())
                .text_size(dp(11.5))
                .font_weight(FontWeight::BOLD)
                .text_color(p.muted)
                .child(column.name.clone())
        }));
    let rows = table.rows.iter().map(|row| {
        h_flex()
            .flex_none()
            .h(dp(28.))
            .gap(dp(12.))
            .px(dp(14.))
            .border_t_1()
            .border_color(p.line)
            .text_size(dp(12.))
            .text_color(p.ink_2)
            .children(
                row.iter()
                    .zip(&table.columns)
                    .map(|(value, column)| cell(column.numeric).child(value.clone())),
            )
    });
    v_flex()
        .size_full()
        .child(header)
        .child(
            v_flex()
                .id(id)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .children(rows)
                .when(table.total > table.rows.len(), |this| {
                    this.child(
                        div()
                            .flex_none()
                            .px(dp(14.))
                            .py(dp(6.))
                            .border_t_1()
                            .border_color(p.line)
                            .text_size(dp(11.5))
                            .text_color(p.muted)
                            .child(format!(
                                "Showing {} of {} rows",
                                table.rows.len(),
                                table.total
                            )),
                    )
                })
                .test_support(),
        )
        .into_any_element()
}

pub(super) fn text(text: SharedString, cx: &App) -> AnyElement {
    let p = palette(cx);
    div()
        .size_full()
        .px(dp(14.))
        .pb(dp(10.))
        .text_size(dp(12.5))
        .text_color(p.ink_2)
        .whitespace_normal()
        .child(text)
        .into_any_element()
}
