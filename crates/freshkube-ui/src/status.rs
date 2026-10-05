//! The status bar's page segment: where the visible page's data comes
//! from, how much of it, its state and when it last changed. The page
//! derives its [`Segment`] when its data changes; the shell draws the
//! visible page's segment after its own status ([docs/DESIGN.md](../../docs/DESIGN.md)).

use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::h_flex;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ElementId, HighlightStyle, Role, SharedString, StyledText, TestSupportExt, div,
};

use crate::palette::palette;
use crate::ui::Tone;

/// One part of a segment, such as a count or a state. A toned part, such
/// as `reconnecting`, is drawn in its tone's ink.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    text: SharedString,
    tone: Option<Tone>,
}

impl Part {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            tone: None,
        }
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = Some(tone);
        self
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

/// What a page puts in the status bar. A page keeps one, derived when its
/// data changes, and the shell reads it while the page shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Segment {
    /// The context the page reads.
    context: Option<SharedString>,
    /// The parts after the context, and the context first.
    rest: Line,
    full: Line,
    note: Option<SharedString>,
}

impl Segment {
    /// A segment of `parts` after the page's `context`, which the shell
    /// leaves out where its own status already names it.
    pub fn new(
        context: Option<impl Into<SharedString>>,
        parts: impl IntoIterator<Item = impl Into<Part>>,
    ) -> Self {
        let context = context.map(Into::into);
        let parts: Vec<Part> = parts.into_iter().map(Into::into).collect();
        let rest = Line::new(&parts);
        let full = match &context {
            Some(context) => Line::new(std::iter::once(&Part::new(context.clone())).chain(&parts)),
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

    /// The line the bar shows when its status names the context `named`.
    pub fn text(&self, named: Option<&str>) -> &SharedString {
        &self.line(named).text
    }

    fn line(&self, named: Option<&str>) -> &Line {
        match (&self.context, named) {
            (Some(context), Some(named)) if context == named => &self.rest,
            _ => &self.full,
        }
    }
}

/// Draws `segment`, or nothing when it is empty: one line that truncates
/// at its end, with the whole line in its tooltip and accessibility label.
/// `named` is the context the shell's status already names.
pub fn segment(
    id: impl Into<ElementId>,
    segment: &Segment,
    named: Option<&str>,
    cx: &App,
) -> Option<AnyElement> {
    let line = segment.line(named);
    if line.text.is_empty() {
        return None;
    }
    let p = palette(cx);
    let highlights = line.toned.iter().map(|(range, tone)| {
        let color = match tone {
            Tone::Crit | Tone::Died => p.crit_ink,
            Tone::Warn => p.warn_ink,
            Tone::Good => p.good_ink,
            _ => p.muted,
        };
        (
            range.clone(),
            HighlightStyle {
                color: Some(color),
                ..Default::default()
            },
        )
    });
    let styled = StyledText::new(line.text.clone()).with_highlights(highlights);
    let tip: SharedString = match &segment.note {
        Some(note) => format!("{}\n{note}", line.text).into(),
        None => line.text.clone(),
    };
    Some(
        h_flex()
            .id(id)
            .test_support()
            .role(Role::Status)
            .aria_label(line.text.clone())
            .min_w_0()
            .child(div().min_w_0().truncate().child(styled))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let (full, rest) = (segment.line(None), segment.line(Some("prod-fra")));
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
}
