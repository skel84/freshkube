//! The status bar's page segment: where the visible page's data comes
//! from, how much of it, its state and when it last changed. The page
//! derives its [`Segment`] when its data changes; the shell draws the
//! visible page's segment after its own status ([docs/DESIGN.md](../../docs/DESIGN.md)).
//!
//! A bar too narrow for the whole line drops parts by weight instead of
//! cutting the end: minor parts first, then ordinary ones from the end.
//! The first part and toned (warn, crit) parts always stay.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::h_flex;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, AvailableSpace, Bounds, Element, ElementId, Font, GlobalElementId, Hsla,
    InspectorElementId, LayoutId, Pixels, Role, ShapedLine, SharedString, Style, TestSupportExt,
    TextAlign, TextRun, TruncateFrom, Window, px, size,
};

use crate::palette::palette;
use crate::ui::Tone;

/// How much a part matters when the bar can't show the whole line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Weight {
    /// Dropped first: what the shell shows already, or a count of none.
    Minor,
    Normal,
    /// Never dropped: the first part and toned parts.
    Keep,
}

/// One part of a segment, such as a count or a state. A toned part, such
/// as `reconnecting`, is drawn in its tone's ink.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    text: SharedString,
    tone: Option<Tone>,
    minor: bool,
}

impl Part {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            tone: None,
            minor: false,
        }
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = Some(tone);
        self
    }

    /// Marks a part the bar drops first when it is narrow, such as the
    /// time of the last read, which the shell's status already shows, or a
    /// count of none. A warning or failure is never minor.
    pub fn minor(mut self) -> Self {
        self.minor = true;
        self
    }

    fn weight(&self) -> Weight {
        match self.tone {
            Some(Tone::Warn | Tone::Crit | Tone::Died) => Weight::Keep,
            _ if self.minor => Weight::Minor,
            _ => Weight::Normal,
        }
    }
}

impl<T: Into<SharedString>> From<T> for Part {
    fn from(text: T) -> Self {
        Self::new(text)
    }
}

/// A line of parts joined by ` · `, and its toned ranges.
#[derive(Clone, Debug, Default, PartialEq)]
struct Line {
    text: SharedString,
    toned: Rc<[(Range<usize>, Tone)]>,
}

impl Line {
    fn new<'a>(parts: impl IntoIterator<Item = &'a Part>) -> Self {
        let (mut text, mut toned) = (String::new(), Vec::new());
        for part in parts {
            if !text.is_empty() {
                text.push_str(" · ");
            }
            let start = text.len();
            text.push_str(&part.text);
            if let Some(tone) = part.tone {
                toned.push((start..text.len(), tone));
            }
        }
        Self {
            text: text.into(),
            toned: toned.into(),
        }
    }
}

/// The lines a bar may show, longest first: the whole line, then one part
/// fewer at a time, minor parts from the end and then ordinary ones from
/// the end. The first part stays, after the context when there is one, and
/// toned ones stay in every line. When a toned part is left, the last line
/// drops the first part too: a warning matters more than a count.
fn ladder(parts: &[Part], context: bool) -> Rc<[Line]> {
    let lead = usize::from(context);
    let droppable = |weight| {
        (lead + 1..parts.len())
            .rev()
            .filter(move |&ix| parts[ix].weight() == weight)
    };
    let shown_line = |shown: &[bool]| {
        Line::new(
            parts
                .iter()
                .zip(shown)
                .filter(|(_, shown)| **shown)
                .map(|(part, _)| part),
        )
    };
    let mut shown = vec![true; parts.len()];
    let mut lines = vec![Line::new(parts)];
    for ix in droppable(Weight::Minor).chain(droppable(Weight::Normal)) {
        shown[ix] = false;
        lines.push(shown_line(&shown));
    }
    let warns = droppable(Weight::Keep).next().is_some();
    if warns && parts[lead].weight() != Weight::Keep {
        shown[lead] = false;
        lines.push(shown_line(&shown));
    }
    lines.into()
}

/// What a page puts in the status bar. A page keeps one, derived when its
/// data changes, and the shell reads it while the page shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    /// The context the page reads.
    context: Option<SharedString>,
    /// The parts after the context, and the context first, each as the
    /// lines a narrower bar shows.
    rest: Rc<[Line]>,
    full: Rc<[Line]>,
    note: Option<SharedString>,
}

impl Default for Segment {
    fn default() -> Self {
        Self::new(None::<SharedString>, Vec::<Part>::new())
    }
}

impl Segment {
    /// A segment of `parts` after the page's `context`, which the shell
    /// leaves out where its own status already names it. When the bar is
    /// narrow, the context and toned parts stay, and the first part stays
    /// until only it and toned parts are left.
    pub fn new(
        context: Option<impl Into<SharedString>>,
        parts: impl IntoIterator<Item = impl Into<Part>>,
    ) -> Self {
        let context = context.map(Into::into);
        let mut parts: Vec<Part> = parts.into_iter().map(Into::into).collect();
        let rest = ladder(&parts, false);
        let full = match &context {
            Some(context) => {
                parts.insert(0, Part::new(context.clone()));
                ladder(&parts, true)
            }
            None => rest.clone(),
        };
        Self {
            context,
            rest,
            full,
            note: None,
        }
    }

    /// A sentence the tooltip adds under the line, such as how a count
    /// was sampled.
    pub fn note(mut self, note: impl Into<SharedString>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// The whole line the bar shows when its status names the context
    /// `named`.
    pub fn text(&self, named: Option<&str>) -> &SharedString {
        &self.ladder(named)[0].text
    }

    fn ladder(&self, named: Option<&str>) -> &Rc<[Line]> {
        match (&self.context, named) {
            (Some(context), Some(named)) if context == named => &self.rest,
            _ => &self.full,
        }
    }
}

/// Draws `segment`, or nothing when it is empty: the longest of its lines
/// that fits, the shortest cut at its end when none does, with the whole
/// line in its tooltip and accessibility label. `named` is the context
/// the shell's status already names.
pub fn segment(
    id: impl Into<ElementId>,
    segment: &Segment,
    named: Option<&str>,
    cx: &App,
) -> Option<AnyElement> {
    let ladder = segment.ladder(named);
    let line = ladder[0].text.clone();
    if line.is_empty() {
        return None;
    }
    let p = palette(cx);
    let inks = Inks {
        crit: p.crit_ink,
        warn: p.warn_ink,
        good: p.good_ink,
        other: p.muted,
    };
    let tip: SharedString = match &segment.note {
        Some(note) => format!("{line}\n{note}").into(),
        None => line.clone(),
    };
    let id = id.into();
    Some(
        h_flex()
            .id(id.clone())
            .test_support()
            .role(Role::Status)
            .aria_label(line)
            .min_w_0()
            .child(Fitted {
                id,
                ladder: ladder.clone(),
                inks,
            })
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .into_any_element(),
    )
}

/// The inks of toned parts, from the palette.
#[derive(Clone, Copy, PartialEq)]
struct Inks {
    crit: Hsla,
    warn: Hsla,
    good: Hsla,
    other: Hsla,
}

impl Inks {
    fn of(&self, tone: Tone) -> Hsla {
        match tone {
            Tone::Crit | Tone::Died => self.crit,
            Tone::Warn => self.warn,
            Tone::Good => self.good,
            _ => self.other,
        }
    }
}

/// A segment's lines, of which it draws the longest that fits.
struct Fitted {
    id: ElementId,
    ladder: Rc<[Line]>,
    inks: Inks,
}

/// What a fit was shaped for. A different ladder, font or ink shapes again.
#[derive(Clone, PartialEq)]
struct Key {
    ladder: Rc<[Line]>,
    font: Font,
    font_size: Pixels,
    ink: Hsla,
    inks: Inks,
}

/// Which line a width shows.
#[derive(Clone, Copy)]
enum Shown {
    Rung(usize),
    Cut,
}

/// A segment's lines as shaped so far, kept across frames. The bar draws
/// again on every hover; the same width then only compares widths, and a
/// line is shaped once, when a width first needs it.
#[derive(Default)]
struct Fit {
    key: Option<Key>,
    line_height: Pixels,
    shaped: Vec<ShapedLine>,
    /// The last line cut to a width, when no line fits.
    cut: Option<(Pixels, ShapedLine)>,
}

impl Fit {
    fn reset(&mut self, key: Key, line_height: Pixels) {
        if self.key.as_ref() != Some(&key) {
            *self = Self {
                key: Some(key),
                ..Self::default()
            };
        }
        self.line_height = line_height;
    }

    fn key(&self) -> &Key {
        self.key.as_ref().expect("a fit is reset before it is used")
    }

    fn runs(&self, line: &Line) -> Vec<TextRun> {
        let key = self.key();
        let run = |len, color| TextRun {
            len,
            font: key.font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let (mut runs, mut at) = (Vec::new(), 0);
        for (range, tone) in line.toned.iter() {
            if range.start > at {
                runs.push(run(range.start - at, key.ink));
            }
            runs.push(run(range.len(), key.inks.of(*tone)));
            at = range.end;
        }
        if line.text.len() > at {
            runs.push(run(line.text.len() - at, key.ink));
        }
        runs
    }

    /// The line `width` shows: the longest that fits, or the last one cut
    /// at its end. Without a width, the whole line.
    fn choose(&mut self, width: Option<Pixels>, window: &mut Window, cx: &mut App) -> Shown {
        let ladder = self.key().ladder.clone();
        let font_size = self.key().font_size;
        for (ix, line) in ladder.iter().enumerate() {
            if self.shaped.len() == ix {
                let runs = self.runs(line);
                let shaped =
                    window
                        .text_system()
                        .shape_line(line.text.clone(), font_size, &runs, None);
                self.shaped.push(shaped);
            }
            match width {
                Some(width) if self.shaped[ix].width() > width => {}
                _ => return Shown::Rung(ix),
            }
        }
        let width = width.expect("a line without a width is shown whole");
        if !self.cut.as_ref().is_some_and(|(cut, _)| *cut == width) {
            let last = &ladder[ladder.len() - 1];
            let runs = self.runs(last);
            let mut wrapper = cx
                .text_system()
                .line_wrapper(self.key().font.clone(), font_size);
            let (text, runs) =
                wrapper.truncate_line(last.text.clone(), width, "…", &runs, TruncateFrom::End);
            let shaped = window
                .text_system()
                .shape_line(text, font_size, &runs, None);
            self.cut = Some((width, shaped));
        }
        Shown::Cut
    }

    fn line(&self, shown: Shown) -> &ShapedLine {
        match shown {
            Shown::Rung(ix) => &self.shaped[ix],
            Shown::Cut => &self.cut.as_ref().expect("chosen").1,
        }
    }
}

impl IntoElement for Fitted {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Fitted {
    type RequestLayoutState = Rc<RefCell<Fit>>;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = window.text_style();
        let rem = window.rem_size();
        let font_size = style.font_size.to_pixels(rem);
        let line_height = window.pixel_snap(style.line_height.to_pixels(font_size.into(), rem));
        let key = Key {
            ladder: self.ladder.clone(),
            font: style.font(),
            font_size,
            ink: style.color,
            inks: self.inks,
        };
        let fit = window.with_element_state(
            id.expect("a fitted segment has an id"),
            |fit: Option<Rc<RefCell<Fit>>>, _| {
                let fit = fit.unwrap_or_default();
                fit.borrow_mut().reset(key, line_height);
                (fit.clone(), fit)
            },
        );
        // The bar may squeeze the segment to nothing; the measure then
        // picks a shorter line.
        let mut layout = Style::default();
        layout.min_size.width = px(0.).into();
        let measured = fit.clone();
        let layout = window.request_measured_layout(layout, move |known, available, window, cx| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(width) => Some(width),
                _ => None,
            });
            let mut fit = measured.borrow_mut();
            let shown = fit.choose(width, window, cx);
            size(fit.line(shown).width().ceil(), fit.line_height)
        });
        (layout, fit)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        fit: &mut Self::RequestLayoutState,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut fit = fit.borrow_mut();
        let shown = fit.choose(Some(bounds.size.width), window, cx);
        let (line, line_height) = (fit.line(shown), fit.line_height);
        #[cfg(any(test, feature = "testing"))]
        SHOWN.with(|map| map.borrow_mut().insert(self.id.clone(), line.text.clone()));
        let _ = line.paint(
            bounds.origin,
            line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }
}

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static SHOWN: RefCell<std::collections::HashMap<ElementId, SharedString>> =
        RefCell::default();
}

/// The line the segment `id` last drew, for UI tests: the tooltip and
/// accessibility label always hold the whole line.
#[cfg(any(test, feature = "testing"))]
pub fn shown(id: impl Into<ElementId>) -> Option<SharedString> {
    SHOWN.with(|map| map.borrow().get(&id.into()).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(segment: &Segment, named: Option<&str>) -> Vec<String> {
        segment
            .ladder(named)
            .iter()
            .map(|line| line.text.to_string())
            .collect()
    }

    #[test]
    fn a_segment_joins_its_parts_and_leaves_out_a_context_already_named() {
        let segment = Segment::new(
            Some("prod-fra"),
            [
                Part::new("140 pods"),
                Part::new("reconnecting").tone(Tone::Warn),
            ],
        );
        assert_eq!(segment.text(None), "prod-fra · 140 pods · reconnecting");
        assert_eq!(segment.text(Some("prod-fra")), "140 pods · reconnecting");
        assert_eq!(
            segment.text(Some("staging")),
            "prod-fra · 140 pods · reconnecting"
        );
        let (full, rest) = (
            &segment.ladder(None)[0],
            &segment.ladder(Some("prod-fra"))[0],
        );
        assert_eq!(&full.text[full.toned[0].0.clone()], "reconnecting");
        assert_eq!(&rest.text[rest.toned[0].0.clone()], "reconnecting");
    }

    #[test]
    fn a_segment_without_a_context_or_parts_is_empty() {
        let none: Option<&str> = None;
        assert_eq!(
            Segment::new(none, ["12 services on 3 nodes"]).text(None),
            "12 services on 3 nodes"
        );
        assert!(Segment::default().text(None).is_empty());
        assert_eq!(
            Segment::new(Some("prod-fra"), Vec::<Part>::new()).text(Some("prod-fra")),
            ""
        );
    }

    #[test]
    fn a_narrower_line_drops_minor_parts_then_ordinary_ones_from_the_end() {
        let segment = Segment::new(
            Some("prod-fra"),
            [
                Part::new("11 deployments"),
                Part::new("0 statefulsets").minor(),
                Part::new("4 daemonsets"),
                Part::new("2 failing").tone(Tone::Crit),
                Part::new("all ready").tone(Tone::Good),
                Part::new("updated 10:42").minor(),
            ],
        );
        assert_eq!(
            lines(&segment, Some("prod-fra")),
            [
                "11 deployments · 0 statefulsets · 4 daemonsets · 2 failing · all ready · updated 10:42",
                "11 deployments · 0 statefulsets · 4 daemonsets · 2 failing · all ready",
                "11 deployments · 4 daemonsets · 2 failing · all ready",
                "11 deployments · 4 daemonsets · 2 failing",
                "11 deployments · 2 failing",
                // A warning outlasts the first part.
                "2 failing",
            ]
        );
        // The context stays in every line.
        assert_eq!(
            lines(&segment, None).last().map(String::as_str),
            Some("prod-fra · 2 failing")
        );
        // A dropped part takes its tone with it; a kept one keeps its own.
        let last = &segment.ladder(None)[5];
        assert_eq!(last.toned.len(), 1);
        assert_eq!(&last.text[last.toned[0].0.clone()], "2 failing");
    }

    #[test]
    fn a_toned_warning_is_never_minor() {
        let part = Part::new("0 ready").minor().tone(Tone::Warn);
        assert_eq!(part.weight(), Weight::Keep);
        let segment = Segment::new(None::<&str>, [Part::new("etcd"), part]);
        assert_eq!(lines(&segment, None), ["etcd · 0 ready", "0 ready"]);
    }

    #[test]
    fn without_a_warning_the_first_part_stays() {
        let segment = Segment::new(
            None::<&str>,
            [Part::new("Talos v1.13"), Part::new("kubelet v1.34")],
        );
        assert_eq!(
            lines(&segment, None),
            ["Talos v1.13 · kubelet v1.34", "Talos v1.13"]
        );
    }
}
