//! Which result tabs the strip shows. The strip never scrolls by pixels: it
//! shows the tabs from `first` on, clipped to the last one that fits whole, so
//! no tab is ever cut at an edge. Widths come from layout (each tab notes its
//! bounds as it is drawn), never from the length of a label. A step or a
//! change of selection reveals the selected tab; a sideways wheel moves by one
//! whole tab per notch.
use gpui_kit::{Bounds, Pixels, ScrollDelta, px};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Half a pixel: layout rounds, and a tab ending a hair past the strip fits.
const EPSILON: f32 = 0.5;

#[derive(Clone, Default)]
pub(super) struct TabFit(Rc<Fit>);

#[derive(Default)]
struct Fit {
    /// The first tab drawn; the ones before it are left out.
    first: Cell<usize>,
    /// The room the strip has, at the last frame; `None` before one.
    avail: Cell<Option<Pixels>>,
    /// The width of the whole tabs that fit from `first`.
    clip: Cell<Option<Pixels>>,
    selected: Cell<usize>,
    count: Cell<usize>,
    /// The selected tab is to be brought into view.
    reveal: Cell<bool>,
    /// What a wheel has moved since it last stepped.
    wheel: Cell<f32>,
    /// The tabs drawn this frame and where.
    spans: RefCell<Vec<(usize, Bounds<Pixels>)>>,
}

impl TabFit {
    /// At the start of a frame's render: the tabs there are and the selected
    /// one. A new selection is revealed.
    pub(super) fn begin(&self, selected: usize, count: usize) {
        let fit = &self.0;
        if fit.selected.replace(selected) != selected || fit.count.replace(count) != count {
            fit.reveal.set(true);
        }
        fit.first.set(fit.first.get().min(count.saturating_sub(1)));
        fit.spans.borrow_mut().clear();
    }

    pub(super) fn first(&self) -> usize {
        self.0.first.get()
    }

    pub(super) fn avail(&self) -> Option<Pixels> {
        self.0.avail.get()
    }

    pub(super) fn clip(&self) -> Option<Pixels> {
        self.0.clip.get()
    }

    /// A tab's bounds, noted as it is prepainted.
    pub(super) fn note(&self, ix: usize, bounds: Bounds<Pixels>) {
        self.0.spans.borrow_mut().push((ix, bounds));
    }

    /// The strip's room, noted as it is prepainted. A new width reveals the
    /// selected tab again; a wider one starts from the first tab, so tabs
    /// left out before `first` come back instead of leaving a gap.
    pub(super) fn set_avail(&self, width: Pixels) {
        if let Some(old) = self.0.avail.replace(Some(width))
            && old != width
        {
            if width > old {
                self.0.first.set(0);
            }
            self.0.reveal.set(true);
        }
    }

    /// After a frame: reveals the selected tab if due and fits the clip to the
    /// whole tabs from `first`. Returns whether the next frame must draw again.
    pub(super) fn settle(&self) -> bool {
        let fit = &self.0;
        let Some(avail) = fit.avail.get() else {
            return false;
        };
        let spans = fit.spans.borrow();
        let find = |ix: usize| spans.iter().find(|(i, _)| *i == ix).map(|(_, b)| *b);
        let mut first = fit.first.get();
        let Some(mut origin) = find(first) else {
            // `first` moved since the frame drew (a wider strip): draw it.
            return true;
        };
        let room = avail + px(EPSILON);
        let mut moved = false;
        if fit.reveal.get() {
            let selected = fit.selected.get();
            if selected < first {
                // Not measured while left out: draw from it and measure again.
                fit.first.set(selected);
                return true;
            }
            while first < selected {
                let Some(tab) = find(selected) else { break };
                if tab.right() - origin.left() <= room {
                    break;
                }
                first += 1;
                match find(first) {
                    Some(next) => origin = next,
                    None => break,
                }
                moved = true;
            }
            fit.reveal.set(false);
        }
        // The whole tabs that fit from `first`, and the width they take.
        let mut right = origin.right();
        for (_, tab) in spans.iter().filter(|(ix, _)| *ix > first) {
            if tab.right() - origin.left() <= room {
                right = tab.right();
            } else {
                break;
            }
        }
        let clip = (right - origin.left()).min(avail);
        let old = fit.clip.replace(Some(clip));
        fit.first.set(first);
        moved || old.is_none_or(|old| (old - clip).abs() > px(EPSILON))
    }

    /// Whether the last tab still lies past the strip's end when it starts at
    /// `first`, by the bounds of the last frame. Each wheel event of one frame
    /// asks this of the `first` the one before it set, so a flick stops where
    /// the last tab ends the strip. Unknown (not drawn yet) reads as no.
    fn tabs_beyond_the_strip(&self) -> bool {
        let fit = &self.0;
        let (Some(avail), Some(last)) = (fit.avail.get(), fit.count.get().checked_sub(1)) else {
            return false;
        };
        let spans = fit.spans.borrow();
        let find = |ix: usize| spans.iter().find(|(i, _)| *i == ix).map(|(_, b)| *b);
        match (find(fit.first.get()), find(last)) {
            (Some(first), Some(last)) => last.right() - first.left() > avail + px(EPSILON),
            _ => false,
        }
    }

    /// A wheel of `delta` (negative reveals later tabs). A notch of a mouse
    /// (`ScrollDelta::Lines`) moves one whole tab per event; pixels, as a
    /// trackpad sends them, add up and move one when they have gone `notch`
    /// far. Returns whether the strip moved.
    pub(super) fn wheel(&self, delta: ScrollDelta, line: Pixels, notch: Pixels) -> bool {
        let fit = &self.0;
        let total = match delta {
            ScrollDelta::Lines(lines) => lines.x * f32::from(line),
            ScrollDelta::Pixels(pixels) => fit.wheel.get() + f32::from(pixels.x),
        };
        let step = matches!(delta, ScrollDelta::Lines(_)) && total != 0.;
        if !step && total.abs() < f32::from(notch) {
            fit.wheel.set(total);
            return false;
        }
        fit.wheel.set(0.);
        let first = fit.first.get();
        if total < 0. {
            if !self.tabs_beyond_the_strip() {
                return false;
            }
            fit.first.set(first + 1);
        } else {
            if first == 0 {
                return false;
            }
            fit.first.set(first - 1);
        }
        true
    }
}
