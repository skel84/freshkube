//! Small shared building blocks for the Freshkube screens.
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon, Selectable,
    button::{Button, ButtonVariants},
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
    Pixels, Rems, Role, SharedString, TestSupportExt, Window, canvas, div, fill, point, px, rems,
    size, svg, transparent_black,
};

use crate::palette::{Palette, palette};

/// Monospace face for resource names, hostnames, addresses, numbers and
/// logs. The interface face, Figtree, is the theme's `font.family`.
pub const MONO_FONT: &str = "IBM Plex Mono";
/// Weight of page titles and figures.
pub const TITLE_WEIGHT: FontWeight = FontWeight::BLACK;
/// Weight of section headings and uppercase captions.
pub const HEADING_WEIGHT: FontWeight = FontWeight::BOLD;

/// A page's title.
pub fn page_title(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(dp(20.))
        .line_height(dp(28.))
        .font_weight(TITLE_WEIGHT)
        .child(text.into())
}

/// Weight of a toolbar's label, the page title on a page with a toolbar.
pub const LABEL_WEIGHT: FontWeight = FontWeight::SEMIBOLD;
/// The height of a control in a toolbar: a button, field, select or segment.
/// Kit's small controls are 1.5 rem, 19.5 at 13 px, so a caller sizes each
/// one with `.h(dp(CONTROL_HEIGHT))`, or `.size(…)` for an icon button.
pub const CONTROL_HEIGHT: f32 = 24.;

/// A toolbar's leading label: the page's title, 13 semibold in `ink`.
pub fn toolbar_label(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .flex_none()
        .text_size(dp(13.))
        .line_height(dp(18.))
        .font_weight(LABEL_WEIGHT)
        .text_color(palette(cx).ink)
        .whitespace_nowrap()
        .child(text.into())
}

/// A quiet cross-reference. Dots are quads so a table adds no separate
/// full-window path pass for each underline.
pub fn reference(text: impl Into<SharedString>, p: &crate::palette::Palette) -> Div {
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
pub const BASE_TEXT: f32 = 13.;

/// A length of `n` pixels at the default text size, scaling with the text
/// size the user chooses (`crate::text_size`). Size text, rows, padding and
/// widths with it; borders, hairlines and corner radii stay in `px`, as
/// Kit's do.
pub fn dp(n: f32) -> Rems {
    rems(n / BASE_TEXT)
}

/// `dp(n)` in pixels, for APIs and arithmetic that take `Pixels`.
pub fn dp_px(n: f32, window: &Window) -> Pixels {
    window.rem_size() * (n / BASE_TEXT)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    Good,
    Warn,
    Crit,
    /// A container that ran and stopped: drawn as a skull, counted with
    /// critical.
    Died,
    #[default]
    Unknown,
    /// Needs an integration before it can say more (docs/DESIGN.md).
    Integration,
    /// Something to know that isn't a fault, such as errors in an
    /// application's logs: a small blue dot.
    Info,
    Accent,
    Outline,
}

/// A compact status pill; the label always carries the meaning, never color
/// alone. A status tone leads with its [`status_glyph`]; `icon` marks only
/// Accent and Outline tags, which carry no status.
pub fn tag(tone: Tone, icon: Option<IconName>, text: impl Into<SharedString>, cx: &App) -> Div {
    let p = palette(cx);
    let (bg, fg) = match tone {
        Tone::Good => (p.good_soft, p.good_ink),
        Tone::Warn => (p.warn_soft, p.warn_ink),
        Tone::Crit | Tone::Died => (p.crit_soft, p.crit_ink),
        Tone::Unknown => (p.unk_soft, p.unk_ink),
        Tone::Integration => (p.integration.opacity(0.14), p.integration),
        Tone::Info | Tone::Accent => (p.accent_soft, p.accent),
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

/// The chosen option of a segmented control or chip row. Kit marks an
/// outline button's selection with a faint input tint that Fog's surfaces
/// hide; the design marks selection in blue, so the chosen option takes the
/// primary outline.
pub fn choice(button: Button, selected: bool) -> Button {
    let button = button.selected(selected);
    if selected { button.primary() } else { button }
}

/// One segment of a ghost segmented control on a `surface_2` track. Kit
/// marks a selected ghost button with `secondary.active`, which is the
/// track's own colour in Fog, so the chosen segment takes the accent tint.
pub fn segment(button: Button, selected: bool, cx: &App) -> Button {
    let p = palette(cx);
    button
        .ghost()
        .toggled(selected)
        .when(selected, |b| b.bg(p.accent_soft).text_color(p.accent))
}

/// The status language of docs/DESIGN.md, the G6 Round set: one round
/// silhouette whose inside carries the meaning. A dot in a halo is OK, a
/// half-filled ring a warning, a disc with a bar cut out critical, the
/// skull a container that died, a dashed ring pending or unknown, a ring
/// with a plus integration required, and a small blue dot information. Accent and Outline carry no
/// status and have no glyph. The glyph is decorative: whatever holds it
/// names the state, as a tag's text, a chip's label or [`status_mark`]'s
/// tooltip do.
pub fn status_glyph(tone: Tone, cx: &App) -> Option<AnyElement> {
    let (drawing, color) = glyph(tone, cx)?;
    Some(
        svg()
            .data(drawing)
            .flex_none()
            .size(dp(10.))
            .text_color(color)
            .into_any_element(),
    )
}

/// A standalone [`status_glyph`] with a tooltip that says what it means,
/// which is also its accessible label.
pub fn status_mark(
    id: impl Into<ElementId>,
    tone: Tone,
    tooltip: impl Into<SharedString>,
    cx: &App,
) -> AnyElement {
    let tooltip = tooltip.into();
    div()
        .id(id)
        .flex_none()
        .role(Role::Image)
        .aria_label(tooltip.clone())
        .children(status_glyph(tone, cx))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .test_support()
        .into_any_element()
}

/// A small dot on an icon's corner that says something there needs a look:
/// a rail area with problems, a destination with incidents. It isn't a
/// status glyph, which names a state beside its text; the caller places it
/// (absolute, at the corner) and the icon's tooltip says why. With `ring`,
/// it's drawn larger with a 2 px ring in that colour, the background it sits
/// on, which cuts it out of the icon beneath.
pub fn badge_dot(tone: Tone, ring: Option<Hsla>, cx: &App) -> Div {
    let p = palette(cx);
    let color = match tone {
        Tone::Crit | Tone::Died => p.crit,
        Tone::Warn => p.warn,
        Tone::Good => p.good,
        Tone::Info | Tone::Accent => p.accent,
        Tone::Unknown | Tone::Integration | Tone::Outline => p.muted,
    };
    let dot = div().flex_none().rounded_full().bg(color);
    match ring {
        Some(ring) => dot.size(dp(9.)).border_2().border_color(ring),
        None => dot.size(dp(7.)),
    }
}

/// A tone's drawing and its colour.
fn glyph(tone: Tone, cx: &App) -> Option<(&'static [u8], Hsla)> {
    let p = palette(cx);
    let color = match tone {
        Tone::Good => p.good,
        Tone::Warn => p.warn_ink,
        Tone::Crit | Tone::Died => p.crit,
        Tone::Unknown => p.unk_ink,
        Tone::Integration => p.integration,
        Tone::Info => p.accent,
        Tone::Accent | Tone::Outline => return None,
    };
    Some((drawing(tone)?, color))
}

/// A tone's G6 drawing: a single-colour SVG on a 16 grid that GPUI draws as
/// an alpha mask, so the halo keeps its transparency and the cut-outs show
/// whatever lies behind the glyph. The drawings are compiled in rather than
/// loaded as assets, so every window and test harness draws them, whatever
/// its asset source.
fn drawing(tone: Tone) -> Option<&'static [u8]> {
    Some(match tone {
        Tone::Good => include_bytes!("../assets/glyphs/ok.svg"),
        Tone::Warn => include_bytes!("../assets/glyphs/warning.svg"),
        Tone::Crit => include_bytes!("../assets/glyphs/critical.svg"),
        Tone::Died => include_bytes!("../assets/glyphs/died.svg"),
        Tone::Unknown => include_bytes!("../assets/glyphs/pending.svg"),
        Tone::Integration => include_bytes!("../assets/glyphs/integration.svg"),
        Tone::Info => include_bytes!("../assets/glyphs/info.svg"),
        Tone::Accent | Tone::Outline => return None,
    })
}

/// Uppercase caption used for section and field labels.
pub fn caption(text: &str, cx: &App) -> Div {
    div()
        .font_weight(HEADING_WEIGHT)
        .text_size(dp(11.))
        .text_color(palette(cx).muted)
        .whitespace_nowrap()
        .child(text.to_uppercase())
}

/// A table's column label, drawn as given: pages write it in sentence
/// case, and Kubernetes' printed columns keep their own.
pub fn column_label(text: &SharedString, cx: &App) -> Div {
    div()
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(dp(11.5))
        .text_color(palette(cx).muted)
        .whitespace_nowrap()
        .child(text.clone())
}

/// A small keycap hint such as `⌘1`.
pub fn keycap(text: impl Into<SharedString>, cx: &App) -> Div {
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
pub fn modifier() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl "
    }
}

/// Load over time against the core count. The dashed line marks "all cores
/// busy" when every sample fits beneath it.
pub fn sparkline(samples: Vec<f64>, cores: Option<usize>, cx: &App) -> Canvas<()> {
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
pub fn countdown_ring(remaining: f32, visible: bool, cx: &App) -> Canvas<()> {
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
pub fn skeleton(
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
pub fn empty_state(
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
pub fn warning_banner(
    lead: Option<SharedString>,
    body: impl Into<SharedString>,
    action: Option<AnyElement>,
    cx: &App,
) -> Div {
    banner(Tone::Warn, lead, body.into(), action, cx)
}

/// An inline banner with an optional action: Crit for a fault the page
/// exists to show, such as a lost quorum, Warn for everything else.
pub fn banner(
    tone: Tone,
    lead: Option<SharedString>,
    body: impl IntoElement,
    action: Option<AnyElement>,
    cx: &App,
) -> Div {
    let p = palette(cx);
    let (icon, ink, soft, line) = banner_look(tone, &p);
    h_flex()
        .items_start()
        .gap_2p5()
        .px_3()
        .py_2p5()
        .rounded(px(8.))
        .border_1()
        .border_color(line)
        .bg(soft)
        .text_size(dp(12.5))
        .text_color(p.ink)
        .child(Icon::new(icon).size(dp(16.)).text_color(ink).mt(dp(1.)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .when_some(lead, |this, lead| {
                    this.child(div().font_weight(FontWeight::SEMIBOLD).child(lead))
                })
                .child(body),
        )
        .children(action)
}

/// A banner's icon, and its ink, fill and border: critical for Crit and
/// Died, warning for any other tone.
fn banner_look(tone: Tone, p: &Palette) -> (IconName, Hsla, Hsla, Hsla) {
    match tone {
        Tone::Crit | Tone::Died => (IconName::CircleX, p.crit_ink, p.crit_soft, p.crit_line),
        _ => (
            IconName::TriangleAlert,
            p.warn_ink,
            p.warn_soft,
            p.warn_line,
        ),
    }
}

/// Wall-clock time of day, as shown next to refresh results.
pub fn clock(time: std::time::SystemTime) -> String {
    let time: chrono::DateTime<chrono::Local> = time.into();
    time.format("%H:%M:%S").to_string()
}

pub fn transparent() -> Hsla {
    transparent_black()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Context, Render, SvgRenderer, TestAppContext, size};

    use super::*;

    const STATUS: [(Tone, &str); 7] = [
        (Tone::Good, "Running"),
        (Tone::Warn, "Not ready"),
        (Tone::Crit, "ImagePullBackOff"),
        (Tone::Died, "CrashLoopBackOff"),
        (Tone::Unknown, "Pending"),
        (Tone::Integration, "Needs metrics-server"),
        (Tone::Info, "3 errors in the logs"),
    ];

    /// A tone's drawing rasterised as GPUI rasterises it for a 16 px glyph
    /// (twice over), as alpha by row.
    fn alpha(tone: Tone) -> Vec<Vec<u8>> {
        let image = SvgRenderer::new(Arc::new(()))
            .render_single_frame(drawing(tone).unwrap(), 1.)
            .unwrap();
        let width = image.size(0).width.0 as usize;
        let bgra = image.as_bytes(0).unwrap();
        bgra.chunks_exact(width * 4)
            .map(|row| {
                row.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|pixel| pixel[3])
                    .collect()
            })
            .collect()
    }

    /// The alpha at a point of the 16 grid.
    fn at(mask: &[Vec<u8>], x: f32, y: f32) -> u8 {
        let scale = mask.len() as f32 / 16.;
        mask[(y * scale) as usize][(x * scale) as usize]
    }

    #[test]
    fn every_status_tone_paints_a_glyph_of_its_own() {
        let masks: Vec<_> = STATUS.iter().map(|(tone, _)| alpha(*tone)).collect();
        for ((tone, _), mask) in STATUS.iter().zip(&masks) {
            assert_eq!(mask.len(), 32, "{tone:?}");
            assert!(
                mask.iter().flatten().filter(|a| **a > 128).count() > 20,
                "{tone:?} paints nothing"
            );
        }
        for (ix, a) in masks.iter().enumerate() {
            for b in &masks[ix + 1..] {
                assert_ne!(a, b, "two tones share a drawing");
            }
        }
        assert_eq!(drawing(Tone::Accent), None);
        assert_eq!(drawing(Tone::Outline), None);
    }

    #[test]
    fn the_drawings_have_their_shapes() {
        // OK: a solid dot in a faint halo.
        let ok = alpha(Tone::Good);
        assert_eq!(at(&ok, 8., 8.), 255);
        assert!((30..70).contains(&at(&ok, 8., 2.)), "{}", at(&ok, 8., 2.));
        // Warning: a ring, empty above and filled below.
        let warn = alpha(Tone::Warn);
        assert_eq!(at(&warn, 8., 6.), 0);
        assert_eq!(at(&warn, 8., 10.5), 255);
        // Critical: a disc with a bar cut out of it.
        let crit = alpha(Tone::Crit);
        assert_eq!(at(&crit, 8., 8.), 0);
        assert_eq!(at(&crit, 8., 4.), 255);
        // Died: a skull, its eyes cut out of the cranium.
        let died = alpha(Tone::Died);
        assert_eq!(at(&died, 5.6, 7.2), 0);
        assert_eq!(at(&died, 10.4, 7.2), 0);
        assert_eq!(at(&died, 8., 3.), 255);
        // Pending: a ring, open in the middle.
        assert_eq!(at(&alpha(Tone::Unknown), 8., 8.), 0);
        // Integration required: a ring with a plus.
        assert_eq!(at(&alpha(Tone::Integration), 8., 8.), 255);
        // Information: a small dot, nothing around it.
        let info = alpha(Tone::Info);
        assert_eq!(at(&info, 8., 8.), 255);
        assert_eq!(at(&info, 8., 3.), 0);
    }

    struct Marks;

    impl Render for Marks {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            h_flex().children(
                STATUS
                    .iter()
                    .enumerate()
                    .map(|(ix, (tone, label))| status_mark(("mark", ix), *tone, *label, cx)),
            )
        }
    }

    /// Each status mark draws a 10 dp glyph and names its state.
    fn assert_marks(cx: &mut TestAppContext, text: f32) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            crate::text_size::set(text, cx);
        });
        let handle = cx.open_window(size(px(400.), px(200.)), |window, cx| {
            let view = cx.new(|_| Marks);
            Root::new(view, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let side = dp_px(10., window);
            for (ix, (tone, label)) in STATUS.iter().enumerate() {
                let mark = window.find(ElementId::from(("mark", ix)));
                assert_eq!(mark.role(), Some(Role::Image), "{tone:?}");
                assert_eq!(mark.label(), Some(*label), "{tone:?}");
                let drawn = mark.bounds().size;
                assert!(
                    (drawn.width - side).abs() < px(0.5) && (drawn.height - side).abs() < px(0.5),
                    "{tone:?}: {drawn:?}, not {side:?}"
                );
            }
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn status_marks_are_10_dp_and_labelled(cx: &mut TestAppContext) {
        assert_marks(cx, BASE_TEXT);
    }

    #[gpui_kit::test]
    fn status_marks_follow_the_text_size(cx: &mut TestAppContext) {
        assert_marks(cx, 20.);
    }

    #[gpui_kit::test]
    fn a_critical_banner_draws_in_the_critical_colours(cx: &mut TestAppContext) {
        let p = cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            palette(cx)
        });
        for tone in [Tone::Crit, Tone::Died] {
            let (icon, ink, soft, line) = banner_look(tone, &p);
            assert!(matches!(icon, IconName::CircleX), "{tone:?}");
            assert_eq!((ink, soft, line), (p.crit_ink, p.crit_soft, p.crit_line));
        }
        for tone in [Tone::Warn, Tone::Unknown, Tone::Good] {
            let (icon, ink, soft, line) = banner_look(tone, &p);
            assert!(matches!(icon, IconName::TriangleAlert), "{tone:?}");
            assert_eq!((ink, soft, line), (p.warn_ink, p.warn_soft, p.warn_line));
        }
        assert_ne!(p.crit_line, p.warn_line);
    }
}
