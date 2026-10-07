//! The bodies of panels that aren't plots or tables: stats, bar lists, text,
//! and the loading, empty and failed states. Everything they show was
//! derived when the answer arrived.
use freshkube_ui::card::{self, Figure};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, SharedString, TestSupportExt, div, relative};

use crate::monitoring::colors::{Ink, Tier};
use crate::monitoring::derive::{BarRow, Stat};
use crate::palette::palette;
use crate::ui::{self, dp};

pub(super) fn tone(tier: Tier) -> ui::Tone {
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

/// The stats as a `StatCard`'s figures, borrowed from the derived data;
/// the sparkline's last stretch takes the first series colour.
pub(super) fn stats(stats: &[Stat], cx: &App) -> AnyElement {
    let recent = Ink::Slot(0).color(&palette(cx), false);
    let figures: Vec<Figure> = stats
        .iter()
        .map(|stat| Figure {
            name: stat.name.as_ref(),
            value: &stat.value,
            note: stat.note.as_ref(),
            tone: stat.tier.map(tone),
            gauge: stat.gauge,
            segments: None,
            detail: None,
            tinted: false,
            spark: stat.spark.as_ref().map(|spark| card::Spark {
                ys: &spark.ys,
                tail: spark.tail,
                recent,
            }),
        })
        .collect();
    card::figures(&figures, cx)
}

pub(super) fn bars(id: SharedString, rows: &[BarRow], cx: &App) -> AnyElement {
    let p = palette(cx);
    v_flex()
        .id(id)
        .size_full()
        .overflow_y_scroll()
        .restrict_scroll_to_axis()
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
                .child(div().flex_none().w(dp(90.)).child(card::gauge(
                    row.fraction,
                    row.tier.map(tone),
                    cx,
                )))
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
