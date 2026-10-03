//! Small shared building blocks for the Freshkube screens.
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon,
    empty::{
        EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
    },
    h_flex,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Bounds, Canvas, DefiniteLength, Div, ElementId, FontWeight, Hsla, PathBuilder,
    Pixels, Rems, SharedString, TestSupportExt, Window, canvas, div, fill, point, px, relative,
    rems, size, transparent_black,
};

use crate::palette::palette;
use crate::presentation::{Health, MemoryLevel};

/// Monospace face for resource names, hostnames, addresses, numbers and
/// logs. The interface face, Figtree, is the theme's `font.family`.
pub(crate) const MONO_FONT: &str = "IBM Plex Mono";
/// Weight of page titles and figures.
pub(crate) const TITLE_WEIGHT: FontWeight = FontWeight::BLACK;
/// Weight of section headings and uppercase captions.
pub(crate) const HEADING_WEIGHT: FontWeight = FontWeight::BOLD;

/// A page's title.
pub(crate) fn page_title(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(dp(20.))
        .line_height(dp(28.))
        .font_weight(TITLE_WEIGHT)
        .child(text.into())
}

/// A quiet cross-reference. Dots are quads so a table adds no separate
/// full-window path pass for each underline.
pub(crate) fn reference(text: impl Into<SharedString>, p: &crate::palette::Palette) -> Div {
    let color = p.muted;
    let accent = p.accent;
    div()
        .relative()
        .min_w_0()
        .font_family(MONO_FONT)
        .text_size(dp(12.5))
        .text_color(p.ink_2)
        .hover(move |style| style.text_color(accent))
        .child(div().min_w_0().truncate().child(text.into()))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let mut x = bounds.left();
                    while x < bounds.right() {
                        window.paint_quad(fill(
                            Bounds::new(point(x, bounds.top()), size(px(1.), px(1.))),
                            color,
                        ));
                        x += dp_px(3., window);
                    }
                },
            )
            .absolute()
            .bottom_0()
            .left_0()
            .w_full()
            .h(px(1.)),
        )
}

/// The theme's base text size at the default text size, in pixels. `dp`
/// lengths are pixels at this size.
pub(crate) const BASE_TEXT: f32 = 13.;

/// A length of `n` pixels at the default text size, scaling with the text
/// size the user chooses (`crate::text_size`). Size text, rows, padding and
/// widths with it; borders, hairlines and corner radii stay in `px`, as
/// Kit's do.
pub(crate) fn dp(n: f32) -> Rems {
    rems(n / BASE_TEXT)
}

/// `dp(n)` in pixels, for APIs and arithmetic that take `Pixels`.
pub(crate) fn dp_px(n: f32, window: &Window) -> Pixels {
    window.rem_size() * (n / BASE_TEXT)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Tone {
    Good,
    Warn,
    Crit,
    #[default]
    Unknown,
    Accent,
    Outline,
}

pub(crate) fn health_tone(health: Health) -> Tone {
    match health {
        Health::Healthy => Tone::Good,
        Health::Unhealthy => Tone::Crit,
        Health::Unknown => Tone::Unknown,
    }
}

pub(crate) fn memory_tone(level: MemoryLevel) -> Option<(Tone, &'static str)> {
    match level {
        MemoryLevel::Normal => None,
        MemoryLevel::High => Some((Tone::Warn, "High")),
        MemoryLevel::Critical => Some((Tone::Crit, "Critical")),
    }
}

/// A compact status pill; the label always carries the meaning, never color
/// alone. A status tone leads with its [`status_glyph`]; `icon` marks only
/// Accent and Outline tags, which carry no status.
pub(crate) fn tag(
    tone: Tone,
    icon: Option<IconName>,
    text: impl Into<SharedString>,
    cx: &App,
) -> Div {
    let p = palette(cx);
    let (bg, fg) = match tone {
        Tone::Good => (p.good_soft, p.good_ink),
        Tone::Warn => (p.warn_soft, p.warn_ink),
        Tone::Crit => (p.crit_soft, p.crit_ink),
        Tone::Unknown => (p.unk_soft, p.unk_ink),
        Tone::Accent => (p.accent_soft, p.accent),
        Tone::Outline => (transparent_black(), p.muted),
    };
    h_flex()
        .flex_none()
        .gap(dp(5.))
        .h(dp(20.))
        .px(dp(8.))
        .rounded_full()
        .bg(bg)
        .text_color(fg)
        .text_size(dp(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap()
        .when(tone == Tone::Outline, |this| {
            this.border_1()
                .border_color(p.line_strong)
                .font_weight(FontWeight::MEDIUM)
        })
        .map(|this| match status_glyph(tone, cx) {
            Some(glyph) => this.child(glyph),
            None => this.when_some(icon, |this, icon| {
                this.child(Icon::new(icon).size(dp(13.)).text_color(fg))
            }),
        })
        .child(text.into())
}

/// The status language of docs/DESIGN.md, shape and color together: ● OK,
/// outlined ▲ warning, ◆ critical, ○ pending or unknown. Accent and Outline
/// carry no status and have no glyph.
pub(crate) fn status_glyph(tone: Tone, cx: &App) -> Option<AnyElement> {
    let p = palette(cx);
    let shape = match tone {
        Tone::Good => div()
            .size(dp(8.))
            .rounded_full()
            .bg(p.good)
            .into_any_element(),
        Tone::Warn => triangle(p.warn_ink).size(dp(10.)).into_any_element(),
        Tone::Crit => diamond(p.crit).size(dp(9.)).into_any_element(),
        Tone::Unknown => div()
            .size(dp(8.))
            .rounded_full()
            .border(px(1.5))
            .border_color(p.unk_ink)
            .into_any_element(),
        Tone::Accent | Tone::Outline => return None,
    };
    Some(
        h_flex()
            .flex_none()
            .size(dp(10.))
            .items_center()
            .justify_center()
            .child(shape)
            .into_any_element(),
    )
}

/// A standalone [`status_glyph`] with a tooltip that says what it means.
pub(crate) fn status_mark(
    id: impl Into<ElementId>,
    tone: Tone,
    tooltip: impl Into<SharedString>,
    cx: &App,
) -> AnyElement {
    let tooltip = tooltip.into();
    div()
        .id(id)
        .flex_none()
        .children(status_glyph(tone, cx))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .test_support()
        .into_any_element()
}

/// The status mark for a health, with its label as the tooltip.
pub(crate) fn health_mark(id: impl Into<ElementId>, health: Health, cx: &App) -> AnyElement {
    let label = crate::presentation::health_text(&health);
    status_mark(id, health_tone(health), label, cx)
}

fn diamond(color: Hsla) -> Canvas<()> {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            let center = bounds.center();
            let r = bounds.size.width / 2.;
            let mut path = PathBuilder::fill();
            path.move_to(point(center.x, center.y - r));
            path.line_to(point(center.x + r, center.y));
            path.line_to(point(center.x, center.y + r));
            path.line_to(point(center.x - r, center.y));
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
}

fn triangle(color: Hsla) -> Canvas<()> {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            // Inset by half the stroke so the outline stays inside the box.
            let inset = px(1.);
            let (left, right) = (bounds.left() + inset, bounds.right() - inset);
            let (top, bottom) = (bounds.top() + inset, bounds.bottom() - inset);
            let mut path = PathBuilder::stroke(px(1.5));
            path.move_to(point(bounds.center().x, top));
            path.line_to(point(right, bottom));
            path.line_to(point(left, bottom));
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
}

/// Uppercase caption used for section, column and field labels.
pub(crate) fn caption(text: &str, cx: &App) -> Div {
    div()
        .font_weight(HEADING_WEIGHT)
        .text_size(dp(11.))
        .text_color(palette(cx).muted)
        .whitespace_nowrap()
        .child(text.to_uppercase())
}

/// A small keycap hint such as `⌘1`.
pub(crate) fn keycap(text: impl Into<SharedString>, cx: &App) -> Div {
    let p = palette(cx);
    div()
        .flex_none()
        .h(dp(18.))
        .px(dp(5.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .border_1()
        .border_b_2()
        .border_color(p.line_strong)
        .bg(p.surface)
        .font_family(MONO_FONT)
        .text_size(dp(10.5))
        .text_color(p.muted)
        .child(text.into())
}

/// The platform's primary modifier glyph.
pub(crate) fn modifier() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl "
    }
}

pub(crate) fn meter(percent: f64, level: MemoryLevel, cx: &App) -> Div {
    let p = palette(cx);
    let fill_color = match level {
        MemoryLevel::Normal => p.accent,
        MemoryLevel::High => p.warn,
        MemoryLevel::Critical => p.crit,
    };
    div()
        .w_full()
        .h(dp(6.))
        .rounded(px(3.))
        .bg(p.track)
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(relative((percent / 100.).clamp(0., 1.) as f32))
                .rounded(px(3.))
                .bg(fill_color),
        )
}

/// Load over time against the core count. The dashed line marks "all cores
/// busy" when every sample fits beneath it.
pub(crate) fn sparkline(samples: Vec<f64>, cores: Option<usize>, cx: &App) -> Canvas<()> {
    let p = palette(cx);
    let (accent, guide, ring) = (p.accent, p.faint, p.surface);
    let peak = samples.iter().copied().fold(0., f64::max);
    let cores = cores.map(|cores| cores as f64);
    let ceiling = cores.unwrap_or(0.).max(peak).max(0.01);
    let capped = cores.is_some_and(|cores| peak <= cores);
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            let left = bounds.origin.x;
            let right = bounds.right();
            let top = bounds.origin.y + px(2.);
            let bottom = bounds.bottom() - px(1.);
            let height = bottom - top;
            if capped {
                let mut line = PathBuilder::stroke(px(1.));
                line = line.dash_array(&[px(3.), px(3.)]);
                line.move_to(point(left, top));
                line.line_to(point(right, top));
                if let Ok(path) = line.build() {
                    window.paint_path(path, guide.opacity(0.7));
                }
            }
            if samples.is_empty() {
                return;
            }
            let span = (samples.len().max(2) - 1) as f32;
            let at = |ix: usize, value: f64| {
                let x = if samples.len() == 1 {
                    right
                } else {
                    left + (right - left) * (ix as f32 / span)
                };
                point(x, bottom - height * (value / ceiling).clamp(0., 1.) as f32)
            };
            if samples.len() > 1 {
                let mut area = PathBuilder::fill();
                let mut line = PathBuilder::stroke(px(1.5));
                area.move_to(point(left, bottom));
                for (ix, value) in samples.iter().enumerate() {
                    let pt = at(ix, *value);
                    if ix == 0 {
                        line.move_to(pt);
                    } else {
                        line.line_to(pt);
                    }
                    area.line_to(pt);
                }
                area.line_to(point(right, bottom));
                area.close();
                if let Ok(path) = area.build() {
                    window.paint_path(path, accent.opacity(0.14));
                }
                if let Ok(path) = line.build() {
                    window.paint_path(path, accent);
                }
            }
            let last = at(samples.len() - 1, samples[samples.len() - 1]);
            let ring_size = dp_px(9., window);
            let dot = Bounds::centered_at(last, size(ring_size, ring_size));
            window.paint_quad(fill(dot, ring).corner_radii(ring_size / 2.));
            let dot_size = dp_px(6., window);
            let dot = Bounds::centered_at(last, size(dot_size, dot_size));
            window.paint_quad(fill(dot, accent).corner_radii(dot_size / 2.));
        },
    )
    .w_full()
    .h(dp(32.))
}

/// Remaining time until the next automatic refresh, drawn as a ring.
pub(crate) fn countdown_ring(remaining: f32, visible: bool, cx: &App) -> Canvas<()> {
    let p = palette(cx);
    let (track, progress) = (p.line, p.accent);
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            if !visible {
                return;
            }
            let center = bounds.center();
            let radius = bounds.size.width.min(bounds.size.height) / 2. - px(1.5);
            let arc = |from: f32, to: f32| {
                let mut path = PathBuilder::stroke(px(2.));
                let steps = ((to - from).abs() * 48.).ceil().max(1.) as usize;
                for step in 0..=steps {
                    let t = from + (to - from) * step as f32 / steps as f32;
                    let angle = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                    let pt = point(
                        center.x + radius * angle.cos(),
                        center.y + radius * angle.sin(),
                    );
                    if step == 0 {
                        path.move_to(pt);
                    } else {
                        path.line_to(pt);
                    }
                }
                path.build().ok()
            };
            if let Some(path) = arc(0., 1.) {
                window.paint_path(path, track);
            }
            let remaining = remaining.clamp(0., 1.);
            if remaining > 0.
                && let Some(path) = arc(0., remaining)
            {
                window.paint_path(path, progress);
            }
        },
    )
    .size(dp(28.))
}

/// A skeleton block of a fixed size.
pub(crate) fn skeleton(
    width: impl Into<DefiniteLength>,
    height: impl Into<DefiniteLength>,
) -> gpui_kit::component::skeleton::Skeleton {
    let width: DefiniteLength = width.into();
    let height: DefiniteLength = height.into();
    gpui_kit::component::skeleton::Skeleton::new()
        .w(width)
        .h(height)
        .rounded(px(5.))
}

/// A centered empty state with an icon, a title, optional detail and actions.
pub(crate) fn empty_state(
    icon: IconName,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    error: Option<String>,
    actions: Vec<AnyElement>,
    cx: &App,
) -> Div {
    let p = palette(cx);
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .p_6()
        .child(
            gpui_kit::component::empty::Empty::new()
                .max_w(dp(480.))
                .header(
                    EmptyHeader::new()
                        .media(
                            EmptyMedia::new()
                                .with_variant(EmptyMediaVariant::Icon)
                                .child(Icon::new(icon).size(dp(20.))),
                        )
                        .title(
                            EmptyTitle::new()
                                .font_weight(HEADING_WEIGHT)
                                .text_size(dp(19.))
                                .text_color(p.ink)
                                .child(title.into()),
                        )
                        .description(EmptyDescription::new().child(description.into())),
                )
                .when_some(error, |this, error| {
                    this.child(
                        div()
                            .mt_2()
                            .px_2p5()
                            .py_1p5()
                            .rounded(px(6.))
                            .bg(p.crit_soft)
                            .text_color(p.crit_ink)
                            .font_family(MONO_FONT)
                            .text_size(dp(12.))
                            .child(error),
                    )
                })
                .when(!actions.is_empty(), |this| {
                    this.content(EmptyContent::new().child(h_flex().gap_2().children(actions)))
                }),
        )
}

/// Accent-free inline warning banner with an optional action.
pub(crate) fn warning_banner(
    lead: Option<SharedString>,
    body: impl Into<SharedString>,
    action: Option<AnyElement>,
    cx: &App,
) -> Div {
    let p = palette(cx);
    h_flex()
        .items_start()
        .gap_2p5()
        .px_3()
        .py_2p5()
        .rounded(px(8.))
        .border_1()
        .border_color(p.warn_line)
        .bg(p.warn_soft)
        .text_size(dp(12.5))
        .text_color(p.ink)
        .child(
            Icon::new(IconName::TriangleAlert)
                .size(dp(16.))
                .text_color(p.warn_ink)
                .mt(dp(1.)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .when_some(lead, |this, lead| {
                    this.child(div().font_weight(FontWeight::SEMIBOLD).child(lead))
                })
                .child(body.into()),
        )
        .children(action)
}

/// Wall-clock time of day, as shown next to refresh results.
pub(crate) fn clock(time: std::time::SystemTime) -> String {
    let time: chrono::DateTime<chrono::Local> = time.into();
    time.format("%H:%M:%S").to_string()
}

pub(crate) fn transparent() -> Hsla {
    transparent_black()
}
