//! A cell's text cut at a word (#522): rows are one line high, so text too
//! long for its column can't wrap. GPUI's own truncation cuts mid-word
//! ("Waiting for promo…"); [`word_cut`] ends at the last whole word that
//! fits with its ellipsis ("Waiting for…"), and cuts a word only when the
//! first is too long alone. The caller puts the whole text in the cell's
//! tooltip.
//!
//! The text style is the one in force where the cell is laid out, taken in
//! `request_layout`, as GPUI's own text takes it: the measuring and the
//! painting run later, outside the row's and the cell's styles.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::{
    App, AvailableSpace, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, LayoutId,
    Pixels, ShapedLine, SharedString, Style, TextAlign, TextStyle, TruncateFrom, Window, px, size,
};

const ELLIPSIS: &str = "…";

/// `text` in the cell's font and colour, cut at a word when it doesn't fit.
pub fn word_cut(id: impl Into<ElementId>, text: impl Into<SharedString>) -> WordCut {
    WordCut {
        id: id.into(),
        text: text.into(),
    }
}

pub struct WordCut {
    id: ElementId,
    text: SharedString,
}

/// Where a cut that ends at a character moves back to: the end of the
/// last whole word in `cut`, the untrimmed prefix of `whole` that fits. A cut that already ends
/// a word stays, as does one that ends before punctuation that ends a
/// word ("Not readable" of "Not readable: …"); one inside the first word
/// keeps the characters it has.
pub(crate) fn at_word<'a>(cut: &'a str, whole: &str) -> &'a str {
    let ends_word = whole[cut.len()..]
        .trim_start_matches(is_separator)
        .chars()
        .next()
        .is_none_or(char::is_whitespace)
        || whole[cut.len()..].starts_with(char::is_whitespace);
    let cut = if ends_word {
        cut
    } else {
        match cut.rfind(char::is_whitespace) {
            Some(ix) if !cut[..ix].trim().is_empty() => &cut[..ix],
            _ => cut,
        }
    };
    // No separator is left hanging before the ellipsis; a closing bracket
    // or a percent sign belongs to its word and stays.
    cut.trim_end_matches(|c: char| c.is_whitespace() || is_separator(c))
}

/// Punctuation that ends a clause and may hang before the ellipsis.
fn is_separator(c: char) -> bool {
    matches!(
        c,
        ',' | ';' | ':' | '.' | '·' | '-' | '–' | '—' | '/' | '(' | '['
    )
}

/// The text shaped for one width.
#[derive(Default)]
pub struct Cut {
    style: TextStyle,
    line_height: Pixels,
    shaped: Option<(Option<Pixels>, ShapedLine)>,
    /// The font, size and colour the line was shaped in.
    #[cfg(any(test, feature = "testing"))]
    ran: Option<(gpui_kit::Font, Pixels, gpui_kit::Hsla)>,
}

impl WordCut {
    fn shape(&self, cut: &mut Cut, width: Option<Pixels>, window: &mut Window, cx: &mut App) {
        if cut
            .shaped
            .as_ref()
            .is_some_and(|(shaped, _)| *shaped == width)
        {
            return;
        }
        let style = cut.style.clone();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let run = |len| style.to_run(len);
        let mut text = self.text.clone();
        if let Some(width) = width {
            // Where the text must end, before GPUI's own trim: `truncate_line`
            // strips trailing punctuation, a `)` or `%` that ends a word too.
            let mut wrapper = cx.text_system().line_wrapper(style.font(), font_size);
            if let Some(ix) =
                wrapper.should_truncate_line(&self.text, width, ELLIPSIS, TruncateFrom::End)
            {
                text = format!("{}{ELLIPSIS}", at_word(&self.text[..ix], &self.text)).into();
            }
        }
        let shaped =
            window
                .text_system()
                .shape_line(text.clone(), font_size, &[run(text.len())], None);
        #[cfg(any(test, feature = "testing"))]
        {
            cut.ran = Some((style.font(), font_size, style.color));
        }
        cut.shaped = Some((width, shaped));
    }
}

impl IntoElement for WordCut {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for WordCut {
    type RequestLayoutState = Rc<RefCell<Cut>>;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = window.text_style();
        let rem = window.rem_size();
        let font_size = style.font_size.to_pixels(rem);
        let cut = Rc::new(RefCell::new(Cut {
            line_height: window.pixel_snap(style.line_height.to_pixels(font_size.into(), rem)),
            style,
            shaped: None,
            #[cfg(any(test, feature = "testing"))]
            ran: None,
        }));
        let mut layout = Style::default();
        layout.min_size.width = px(0.).into();
        layout.flex_shrink = 1.;
        let measured = cut.clone();
        let this = WordCut {
            id: self.id.clone(),
            text: self.text.clone(),
        };
        let layout = window.request_measured_layout(layout, move |known, available, window, cx| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(width) => Some(width),
                _ => None,
            });
            let mut cut = measured.borrow_mut();
            this.shape(&mut cut, width, window, cx);
            let shaped = &cut.shaped.as_ref().expect("shaped").1;
            size(shaped.width().ceil(), cut.line_height)
        });
        (layout, cut)
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
        cut: &mut Self::RequestLayoutState,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut cut = cut.borrow_mut();
        // Shaped again only when the line doesn't fit the bounds, or when
        // it was cut for less room than they give. A flex parent sizes the
        // cell to the measured line, a little under the width it was cut
        // for: that line still fits and keeps its words.
        let width = bounds.size.width;
        let fits = cut.shaped.as_ref().is_some_and(|(cut_for, line)| {
            *cut_for == Some(width)
                || (line.width() <= width
                    && (line.text == self.text || cut_for.is_some_and(|cut_for| width <= cut_for)))
        });
        if !fits {
            self.shape(&mut cut, Some(width), window, cx);
        }
        let line = &cut.shaped.as_ref().expect("shaped").1;
        #[cfg(any(test, feature = "testing"))]
        if let Some((font, size, color)) = cut.ran.clone() {
            let text = line.text.clone();
            SHOWN.with(|map| {
                map.borrow_mut().insert(
                    self.id.clone(),
                    Shown {
                        text,
                        font: font.family.clone(),
                        size,
                        color,
                        run_font: font,
                    },
                )
            });
        }
        let _ = line.paint(
            bounds.origin,
            cut.line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }
}

/// What a cell last drew and the style it shaped it in, for UI tests.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub text: SharedString,
    pub font: SharedString,
    pub size: Pixels,
    pub color: gpui_kit::Hsla,
    /// The whole font, for measuring what else would have fitted.
    pub(crate) run_font: gpui_kit::Font,
}

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static SHOWN: RefCell<std::collections::HashMap<ElementId, Shown>> =
        RefCell::default();
}

/// The text the cell `id` last drew, for UI tests.
#[cfg(any(test, feature = "testing"))]
pub fn shown(id: impl Into<ElementId>) -> Option<SharedString> {
    shown_style(id).map(|shown| shown.text)
}

/// The text the cell `id` last drew, with its font, size and colour.
#[cfg(any(test, feature = "testing"))]
pub fn shown_style(id: impl Into<ElementId>) -> Option<Shown> {
    SHOWN.with(|map| map.borrow().get(&id.into()).cloned())
}

#[cfg(test)]
mod tests {
    use super::{ELLIPSIS, at_word, shown_style, word_cut};
    use gpui_kit::component::Root;
    use gpui_kit::prelude::*;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{Context, Pixels, TestAppContext, Window, div, px, size};

    struct Cell(Pixels, &'static str);

    impl Render for Cell {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .flex()
                .w(self.0)
                .overflow_hidden()
                .child(word_cut("cell", self.1))
        }
    }

    /// Every whole word that fits shows: at each width, swept a pixel at a
    /// time, a cut cell ends at a word, and the next word with its
    /// ellipsis wouldn't have fitted, a `)` or `%` that ends it included.
    #[gpui_kit::test]
    fn a_cut_keeps_every_word_that_fits(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
        });
        for text in [
            "0 critical, 3 high (2 fixable) · SBOM · immutable",
            "Failed: error rate 2.4 %, above 1 %",
        ] {
            let mut cell = None;
            let handle = cx.open_window(size(px(800.), px(100.)), |window, cx| {
                let view = cx.new(|_| Cell(px(10.), text));
                cell = Some(view.clone());
                Root::new(view, window, cx)
            });
            let view = cell.unwrap();
            // Where a word ends: before a space, without the separators
            // left hanging before it.
            let ends: Vec<&str> = text
                .match_indices(' ')
                .map(|(ix, _)| text[..ix].trim_end_matches([',', '·', ':', ' ']))
                .filter(|end| !end.is_empty())
                .collect();
            let mut cuts = 0;
            for width in 10..600 {
                cx.update_window(handle.into(), |_, window, cx| {
                    view.update(cx, |cell, cx| {
                        cell.0 = px(width as f32);
                        cx.notify();
                    });
                    window.render_frame(cx);
                    let shown = shown_style("cell").unwrap();
                    let Some(kept) = shown.text.strip_suffix(ELLIPSIS) else {
                        assert_eq!(shown.text.as_ref(), text);
                        return;
                    };
                    cuts += 1;
                    let Some(next) = ends.iter().find(|end| end.len() > kept.len()) else {
                        return;
                    };
                    let longer = format!("{next}{ELLIPSIS}");
                    let run = gpui_kit::TextRun {
                        len: longer.len(),
                        font: shown.run_font.clone(),
                        color: shown.color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let fits = window
                        .text_system()
                        .shape_line(longer.clone().into(), shown.size, &[run], None)
                        .width()
                        < px(width as f32);
                    assert!(!fits, "at {width}: {:?} drawn, {longer:?} fits", shown.text);
                })
                .unwrap();
            }
            assert!(cuts > 0);
        }
    }

    #[test]
    fn a_cut_inside_a_word_moves_back_to_the_last_whole_one() {
        let whole = "Waiting for promotion: auto-promotion off";
        assert_eq!(at_word("Waiting for promo", whole), "Waiting for");
        assert_eq!(
            at_word("Waiting for promotion: au", whole),
            "Waiting for promotion"
        );
    }

    #[test]
    fn a_cut_at_the_end_of_a_word_stays_without_its_separator() {
        let whole = "push to main · 7 of 7 tasks";
        assert_eq!(at_word("push to main ·", whole), "push to main");
        assert_eq!(at_word("push to", whole), "push to");
    }

    #[test]
    fn punctuation_that_ends_a_word_keeps_the_word() {
        let whole = "Not readable: pods is forbidden";
        assert_eq!(at_word("Not readable", whole), "Not readable");
        assert_eq!(at_word("Not readable:", whole), "Not readable");
        let whole = "3 of 3 analyses passed.";
        assert_eq!(
            at_word("3 of 3 analyses passed", whole),
            "3 of 3 analyses passed"
        );
    }

    #[test]
    fn a_closing_bracket_or_percent_stays_with_its_word() {
        let whole = "CPU 92% (throttled) over the hour";
        assert_eq!(at_word("CPU 92% (throttled)", whole), "CPU 92% (throttled)");
        assert_eq!(at_word("CPU 92%", whole), "CPU 92%");
    }

    #[test]
    fn a_cut_just_after_a_closing_bracket_or_percent_keeps_the_word() {
        // The untrimmed prefixes that fit, as the measuring gives them.
        let whole = "0 critical, 3 high (2 fixable) · SBOM · immutable";
        assert_eq!(
            at_word("0 critical, 3 high (2 fixable)", whole),
            "0 critical, 3 high (2 fixable)"
        );
        assert_eq!(
            at_word("0 critical, 3 high (2 fixable) ", whole),
            "0 critical, 3 high (2 fixable)"
        );
        assert_eq!(
            at_word("0 critical, 3 high (2 fix", whole),
            "0 critical, 3 high (2"
        );
        let whole = "Failed: error rate 2.4 %, above 1 %";
        assert_eq!(
            at_word("Failed: error rate 2.4 %", whole),
            "Failed: error rate 2.4 %"
        );
        assert_eq!(
            at_word("Failed: error rate 2.4 %,", whole),
            "Failed: error rate 2.4 %"
        );
    }

    #[test]
    fn a_first_word_too_long_alone_keeps_the_characters_that_fit() {
        assert_eq!(at_word("sha256:9c4e", "sha256:9c4eb21a run"), "sha256:9c4e");
    }
}
