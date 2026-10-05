//! The dashboard cards (docs/DESIGN.md#components): `StatCard`, a muted
//! title over one figure or a wrap of named ones, and `ChartCard`, a title
//! over a plot the caller draws. Both share a header with the unit, the
//! query behind an info mark and the stale mark. They take plain data, so a
//! page derives its figures when its data changes and only hands them over
//! here.
use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::{Icon, h_flex, tooltip::Tooltip, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Bounds, ClipboardItem, Div, FontWeight, Hsla, PathBuilder, Pixels, Rems,
    SharedString, Stateful, TestSupportExt, canvas, div, fill, point, px, relative, size,
};

use crate::page;
use crate::palette::palette;
use crate::ui::{self, Tone, dp, dp_px};

/// A card's header: its title, the unit its figures or axis leave out, the
/// query behind them and why the answer shown is old. Its parts' ids are
/// `<id>-title`, `-query` and `-stale`.
pub struct CardHeader {
    id: SharedString,
    title: SharedString,
    unit: Option<SharedString>,
    about: SharedString,
    /// What a click on the info mark copies, and the line saying so.
    copy: Option<(SharedString, SharedString)>,
    stale: Option<SharedString>,
}

impl CardHeader {
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            unit: None,
            about: SharedString::default(),
            copy: None,
            stale: None,
        }
    }

    /// The unit, after the title.
    pub fn unit(mut self, unit: Option<SharedString>) -> Self {
        self.unit = unit;
        self
    }

    /// The info mark's tooltip, such as the query and anything the card
    /// can't show. No mark when it's empty.
    pub fn about(mut self, about: SharedString) -> Self {
        self.about = about;
        self
    }

    /// What a click on the info mark copies, with the tooltip's line saying
    /// so; nothing when `text` is empty.
    pub fn copy(mut self, text: SharedString, hint: impl Into<SharedString>) -> Self {
        self.copy = (!text.is_empty()).then(|| (text, hint.into()));
        self
    }

    /// Why the answer shown is old, as the stale mark at the right.
    pub fn stale(mut self, stale: Option<SharedString>) -> Self {
        self.stale = stale;
        self
    }

    fn part(&self, part: &str) -> SharedString {
        format!("{}-{part}", self.id).into()
    }

    fn render(self, stat: bool, cx: &App) -> Div {
        let p = palette(cx);
        let (title_id, query_id, stale_id) =
            (self.part("title"), self.part("query"), self.part("stale"));
        let about = self.about;
        let (copy, hint) = self.copy.unzip();
        h_flex()
            .flex_none()
            .gap(dp(8.))
            .h(dp(if stat { 28. } else { 32. }))
            .pl(dp(if stat { 14. } else { 12. }))
            .pr(dp(8.))
            .pt(dp(if stat { 4. } else { 0. }))
            .child(
                div()
                    .id(title_id)
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::BOLD)
                    .text_size(dp(if stat { 12. } else { 13. }))
                    .text_color(if stat { p.muted } else { p.ink })
                    .child(self.title)
                    .test_support(),
            )
            .when_some(self.unit, |this, unit| {
                this.child(
                    div()
                        .flex_none()
                        .text_size(dp(12.))
                        .text_color(p.muted)
                        .child(unit),
                )
            })
            .when(!about.is_empty(), |this| {
                this.child(
                    div()
                        .id(query_id)
                        .flex_none()
                        .child(Icon::new(IconName::Info).size(dp(13.)).text_color(p.faint))
                        .when_some(copy, |this, copy| {
                            this.cursor_pointer().on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.to_string()))
                            })
                        })
                        .tooltip(move |window, cx| {
                            let (about, hint) = (about.clone(), hint.clone());
                            Tooltip::element(move |_, cx| {
                                v_flex()
                                    .max_w(dp(420.))
                                    .gap(dp(6.))
                                    .child(
                                        div()
                                            .font_family(ui::MONO_FONT)
                                            .text_size(dp(11.5))
                                            .whitespace_normal()
                                            .child(about.clone()),
                                    )
                                    .children(hint.clone().map(|hint| {
                                        div()
                                            .text_size(dp(11.5))
                                            .text_color(palette(cx).muted)
                                            .child(hint)
                                    }))
                            })
                            .build(window, cx)
                        })
                        .test_support(),
                )
            })
            .child(div().flex_1())
            .when_some(self.stale, |this, error| {
                this.child(ui::status_mark(
                    stale_id,
                    Tone::Warn,
                    format!("Showing the last answer; the refresh failed: {error}"),
                    cx,
                ))
            })
    }
}

/// The card both kinds share: it fills its grid cell, with the header
/// above the body.
fn frame(header: CardHeader, stat: bool, body: AnyElement, cx: &App) -> Observed<Stateful<Div>> {
    let id = header.id.clone();
    page::card(cx)
        .id(id)
        .size_full()
        .overflow_hidden()
        .child(header.render(stat, cx))
        .child(div().flex_1().min_h_0().flex().flex_col().child(body))
        .test_support()
}

/// A card of figures: a muted 12 bold title over [`figures`], or over a
/// state while there are none yet.
pub struct StatCard {
    header: CardHeader,
}

impl StatCard {
    pub fn new(header: CardHeader) -> Self {
        Self { header }
    }

    pub fn render(self, body: impl IntoElement, cx: &App) -> Observed<Stateful<Div>> {
        frame(self.header, true, body.into_any_element(), cx)
    }
}

/// A card for a chart: a 13 bold title over the plot and legend the caller
/// draws, or over a state.
pub struct ChartCard {
    header: CardHeader,
}

impl ChartCard {
    pub fn new(header: CardHeader) -> Self {
        Self { header }
    }

    pub fn render(self, body: impl IntoElement, cx: &App) -> Observed<Stateful<Div>> {
        frame(self.header, false, body.into_any_element(), cx)
    }
}

/// One figure on a `StatCard`, borrowed from the page's derived data, so
/// handing it over while drawing copies nothing.
#[derive(Clone, Copy, Debug)]
pub struct Figure<'a> {
    /// The series' name, when the card shows several.
    pub name: Option<&'a SharedString>,
    pub value: &'a SharedString,
    /// Why the value carries a status, such as "above 90%": a tag in the
    /// tone when there is one, muted text otherwise.
    pub note: Option<&'a SharedString>,
    pub tone: Option<Tone>,
    /// A gauge's fill, 0 to 1, in the tone or the accent.
    pub gauge: Option<f32>,
    /// A meter of parts, one per item in its tone, such as a cluster's
    /// nodes, drawn where a gauge goes.
    pub segments: Option<&'a [Tone]>,
    pub spark: Option<Spark<'a>>,
    /// A muted line under the figure, wrapping, such as what it counts.
    pub detail: Option<&'a SharedString>,
    /// The value in its tone's text colour rather than in ink, for a card
    /// whose figure is the status itself.
    pub tinted: bool,
}

/// A figure's sparkline: grey history, then the last stretch in `recent`.
#[derive(Clone, Copy, Debug)]
pub struct Spark<'a> {
    /// Per sample, 0 to 1 across the series' own range; NaN at a gap.
    pub ys: &'a [f32],
    /// The first sample of the last stretch.
    pub tail: usize,
    pub recent: Hsla,
}

/// A `StatCard`'s body: one large figure, or a wrap of named ones.
pub fn figures(figures: &[Figure<'_>], cx: &App) -> AnyElement {
    if let [figure] = figures {
        return v_flex()
            .size_full()
            .gap(dp(4.))
            .px(dp(14.))
            .pb(dp(10.))
            .child(value(figure, dp(22.), cx))
            .children(figure.gauge.map(|fill| gauge(fill, figure.tone, cx)))
            .children(figure.segments.map(|parts| segments(parts, cx)))
            .children(figure.spark.map(|spark| sparkline(spark, cx)))
            .children(figure.detail.map(|line| detail(line, cx)))
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
        .children(figures.iter().map(|figure| {
            v_flex()
                .min_w(dp(96.))
                .gap(dp(2.))
                .children(figure.name.map(|name| {
                    div()
                        .max_w(dp(220.))
                        .truncate()
                        .font_family(ui::MONO_FONT)
                        .text_size(dp(11.5))
                        .text_color(p.muted)
                        .child(name.clone())
                }))
                .child(value(figure, dp(18.), cx))
                .children(figure.gauge.map(|fill| gauge(fill, figure.tone, cx)))
                .children(figure.segments.map(|parts| segments(parts, cx)))
                .children(figure.spark.map(|spark| sparkline(spark, cx)))
                .children(figure.detail.map(|line| detail(line, cx)))
        }))
        .into_any_element()
}

fn value(figure: &Figure<'_>, text_size: Rems, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    let ink = value_ink(figure, &p);
    h_flex()
        .gap(dp(8.))
        .child(
            div()
                .text_size(text_size)
                .font_weight(ui::TITLE_WEIGHT)
                .text_color(ink)
                .whitespace_nowrap()
                .child(figure.value.clone()),
        )
        .children(figure.note.cloned().map(|note| {
            match figure.tone {
                Some(tone) => ui::tag(tone, None, note, cx).into_any_element(),
                None => div()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child(note)
                    .into_any_element(),
            }
        }))
}

/// Ink, or the tone's text colour when the figure is tinted; an unknown
/// tone reads muted.
fn value_ink(figure: &Figure<'_>, p: &crate::palette::Palette) -> Hsla {
    match figure.tone.filter(|_| figure.tinted) {
        None => p.ink,
        Some(Tone::Good) => p.good_ink,
        Some(Tone::Warn) => p.warn_ink,
        Some(Tone::Crit) => p.crit_ink,
        Some(_) => p.muted,
    }
}

/// A gauge's colour: the accent, a warning or critical tone, or grey for
/// a last-known reading.
fn gauge_fill(tone: Option<Tone>, p: &crate::palette::Palette) -> Hsla {
    match tone {
        Some(Tone::Crit) => p.crit,
        Some(Tone::Warn) => p.warn,
        Some(Tone::Unknown) => p.unk,
        _ => p.accent,
    }
}

/// A gauge's fill, or a bar list's.
pub fn gauge(fraction: f32, tone: Option<Tone>, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    let color = gauge_fill(tone, &p);
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

/// One part per item in its tone, on a gauge's track.
fn segments(parts: &[Tone], cx: &App) -> impl IntoElement {
    let p = palette(cx);
    h_flex()
        .flex_none()
        .w_full()
        .max_w(dp(156.))
        .h(dp(6.))
        .gap(px(2.))
        .rounded(px(3.))
        .overflow_hidden()
        .children(parts.iter().map(|tone| {
            div().flex_1().h_full().bg(match tone {
                Tone::Good => p.good,
                Tone::Warn => p.warn,
                Tone::Crit => p.crit,
                _ => p.unk,
            })
        }))
}

/// A figure's muted line, wrapping.
fn detail(line: &SharedString, cx: &App) -> impl IntoElement {
    div()
        .pt(dp(2.))
        .text_size(dp(12.))
        .text_color(palette(cx).muted)
        .whitespace_normal()
        .child(line.clone())
}

/// Grey history, and the last stretch in the spark's colour. Status
/// belongs to the figure's tag, never to its timeseries.
fn sparkline(spark: Spark<'_>, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    let (history, recent) = (p.faint, spark.recent);
    // The paint runs after this frame's borrow ends, so it keeps a copy.
    let (ys, tail) = (spark.ys.to_vec(), spark.tail);
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            let count = ys.len();
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
                    top + height * (1. - ys[index]),
                )
            };
            let stroke = |from: usize, to: usize, color: Hsla, window: &mut gpui_kit::Window| {
                let mut path = PathBuilder::stroke(px(1.5));
                let mut drawing = false;
                for (index, y) in ys.iter().enumerate().take(to + 1).skip(from) {
                    if !y.is_finite() {
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
            stroke(0, tail, history, window);
            stroke(tail, count - 1, recent, window);
            if let Some(last) = ys.iter().rposition(|y| y.is_finite()) {
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

#[cfg(test)]
mod tests {
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Context, Render, TestAppContext, Window, size};

    use super::*;

    fn figure(value: &SharedString) -> Figure<'_> {
        Figure {
            name: None,
            value,
            note: None,
            tone: Some(Tone::Crit),
            gauge: None,
            segments: None,
            spark: None,
            detail: None,
            tinted: false,
        }
    }

    #[gpui_kit::test]
    fn a_tinted_figure_takes_its_tone_and_an_untinted_one_ink(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            let p = palette(cx);
            let value = SharedString::from("3 failing");
            let plain = figure(&value);
            assert_eq!(value_ink(&plain, &p), p.ink);
            let tinted = Figure {
                tinted: true,
                ..plain
            };
            assert_eq!(value_ink(&tinted, &p), p.crit_ink);
            let unknown = Figure {
                tone: Some(Tone::Unknown),
                ..tinted
            };
            assert_eq!(value_ink(&unknown, &p), p.muted);
            let toneless = Figure {
                tone: None,
                ..tinted
            };
            assert_eq!(value_ink(&toneless, &p), p.ink);
            // A last-known gauge is grey, not the accent of a good one.
            assert_eq!(gauge_fill(Some(Tone::Unknown), &p), p.unk);
            assert_eq!(gauge_fill(Some(Tone::Good), &p), p.accent);
        });
    }

    /// Four cards in a column: a bare figure, one with a detail line, one
    /// with a meter of parts, and a named figure with both.
    struct Cards;

    impl Render for Cards {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let value = SharedString::from("4 / 6 Ready");
            let line = SharedString::from("3 control planes · 3 workers");
            let name = SharedString::from("nodes");
            let parts = [Tone::Good, Tone::Good, Tone::Warn, Tone::Crit];
            let bare = figure(&value);
            let cards = [
                ("bare", bare),
                (
                    "detail",
                    Figure {
                        detail: Some(&line),
                        ..bare
                    },
                ),
                (
                    "segments",
                    Figure {
                        segments: Some(&parts),
                        ..bare
                    },
                ),
            ];
            v_flex()
                .w(px(320.))
                .children(cards.map(|(id, figure)| {
                    StatCard::new(CardHeader::new(id, "Nodes"))
                        .render(figures(&[figure], cx), cx)
                        .h_auto()
                }))
                .child(
                    StatCard::new(CardHeader::new("named", "Nodes"))
                        .render(
                            figures(
                                &[
                                    Figure {
                                        name: Some(&name),
                                        detail: Some(&line),
                                        segments: Some(&parts),
                                        ..bare
                                    },
                                    Figure {
                                        name: Some(&name),
                                        ..bare
                                    },
                                ],
                                cx,
                            ),
                            cx,
                        )
                        .h_auto(),
                )
        }
    }

    #[gpui_kit::test]
    fn a_detail_adds_its_line_and_segments_their_meter(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            cx.set_reduce_motion(true);
        });
        let handle = cx.open_window(size(px(400.), px(800.)), |window, cx| {
            let view = cx.new(|_| Cards);
            Root::new(view, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let height = |id: &str| window.find(id.to_owned()).bounds().size.height;
            let bare = height("bare");
            let line = height("detail") - bare;
            assert!(
                // The 4 dp gap, 2 dp of padding and one 12 dp line; two
                // lines would pass 36.
                line > dp_px(18., window) && line < dp_px(36., window),
                "a detail adds one line: {line:?}"
            );
            let meter = height("segments") - bare;
            assert!(
                (meter - dp_px(10., window)).abs() < px(0.5),
                "segments add a 6 dp meter after a 4 dp gap: {meter:?}"
            );
            assert!(
                height("named") > bare,
                "a named figure shows its detail and meter too"
            );
        })
        .unwrap();
    }
}
