//! The stepper cell and the rail, for a table whose rows are stages in a
//! line, such as the change page's Overview matrix (docs/DESIGN.md, "The
//! Overview").
//!
//! A stepper cell has two lines. On the first the track runs from edge to
//! edge with the status glyph on it, inset [`CELL_PAD`] like the header's
//! label; on the second sits one word. The track into a cell takes that
//! cell's style, so a row reads as one line that changes as it goes.
//!
//! The rail is the same line turned down: the pinned first column's node on
//! each row's first line, joined to the rows above and below.

use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Div, ElementId, Hsla, PathBuilder, Role, SharedString, Stateful,
    TestSupportExt, canvas, div, point, px,
};

use super::{CELL_PAD, TableColumn, cell};
use crate::palette::palette;
use crate::tooltip::FollowTooltip as _;
use crate::ui::{self, Tone, dp};

/// The height of each of a stepper cell's two lines, in dp.
pub const LINE: f32 = 16.;
/// The height of a row of stepper cells, in dp: two lines with 4 dp above
/// and below.
pub const STEP_ROW: f32 = 40.;
/// The width of the rail, in dp: the node's column at a pinned cell's left.
pub const RAIL: f32 = 14.;
/// The gap the track leaves either side of its glyph, in dp.
const BREATH: f32 = 3.;
/// The glyph's width, in dp.
const GLYPH: f32 = 10.;

/// How a track is stroked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track {
    /// Confirmed: solid, in the status colour.
    Solid,
    /// Claimed or informative: dashed, in the accent.
    Dashed,
    /// Waiting or can't be confirmed: dotted.
    Dotted,
    /// Not reached: a hairline.
    Hairline,
    /// Nothing: the last cell's far end.
    None,
}

impl Track {
    /// The track into a cell of `tone`: solid for OK, dashed for Info,
    /// dotted for Waiting and for what can't be confirmed, and a hairline
    /// for anything else (not reached).
    pub fn into_tone(tone: Tone) -> Self {
        match tone {
            Tone::Good | Tone::Warn | Tone::Crit | Tone::Died => Self::Solid,
            Tone::Info => Self::Dashed,
            Tone::Waiting | Tone::Unknown | Tone::Integration => Self::Dotted,
            Tone::Accent | Tone::Outline => Self::Hairline,
        }
    }

    /// The dash pattern in dp, on then off; `None` for a solid line.
    fn dashes(self) -> Option<[f32; 2]> {
        match self {
            Self::Dotted => Some([1.5, 3.5]),
            Self::Dashed => Some([5., 3.]),
            _ => None,
        }
    }

    fn width(self) -> f32 {
        match self {
            Self::Hairline => 1.,
            _ => 1.5,
        }
    }
}

/// The colour a track of `tone` is drawn in.
fn track_color(tone: Tone, track: Track, cx: &App) -> Hsla {
    let p = palette(cx);
    match (track, tone) {
        (Track::Hairline, _) => p.line,
        (_, Tone::Good) => p.good,
        (_, Tone::Warn) => p.warn,
        (_, Tone::Crit | Tone::Died) => p.crit,
        (Track::Dashed, _) | (_, Tone::Info) => p.accent,
        _ => p.line_strong,
    }
}

/// A stroke across its own bounds, vertically centred, from `from` dp in
/// from the left to `to` dp short of the right.
fn stroke(
    id: impl Into<ElementId>,
    track: Track,
    color: Hsla,
    from: f32,
    to: f32,
    vertical: bool,
) -> Observed<Stateful<Div>> {
    div()
        .id(id)
        .test_support()
        .h_full()
        .min_w_0()
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    if track == Track::None {
                        return;
                    }
                    let unit = ui::dp_px(1., window);
                    let mut path = PathBuilder::stroke(unit * track.width());
                    if let Some([on, off]) = track.dashes() {
                        path = path.dash_array(&[unit * on, unit * off]);
                    }
                    if vertical {
                        let x = bounds.center().x;
                        path.move_to(point(x, bounds.top() + unit * from));
                        path.line_to(point(x, bounds.bottom() - unit * to));
                    } else {
                        let y = bounds.center().y;
                        path.move_to(point(bounds.left() + unit * from, y));
                        path.line_to(point(bounds.right() - unit * to, y));
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                },
            )
            .size_full(),
        )
}

/// One stepper cell: what it is and how its track runs.
pub struct Step {
    /// The cell's id; its parts are `<id>-track-in`, `-glyph`, `-track-out`
    /// and `-word`.
    pub id: SharedString,
    pub tone: Tone,
    pub word: SharedString,
    /// How the track runs into this cell.
    pub into: Track,
    /// How it runs on into the next cell, `Track::None` at the row's end.
    pub onward: Track,
    /// The tone the next cell's track takes its colour from.
    pub onward_tone: Tone,
    /// Shown on hover and the cell's accessible label.
    pub tooltip: SharedString,
}

/// A stepper cell for `column`, two lines in the row's 40 dp: the track
/// through the glyph on the first, the word on the second.
pub fn stepper_cell(column: &impl TableColumn, step: Step, cx: &App) -> Observed<Stateful<Div>> {
    let p = palette(cx);
    let id = step.id;
    let sub = |part: &str| SharedString::from(format!("{id}-{part}"));
    let line_in = stroke(
        sub("track-in"),
        step.into,
        track_color(step.tone, step.into, cx),
        0.,
        BREATH,
        false,
    )
    .w(dp(CELL_PAD))
    .flex_none();
    let line_out = stroke(
        sub("track-out"),
        step.onward,
        track_color(step.onward_tone, step.onward, cx),
        BREATH,
        0.,
        false,
    )
    .flex_1();
    let first = h_flex()
        .h(dp(LINE))
        .w_full()
        .items_center()
        .child(line_in)
        .child(
            div()
                .id(sub("glyph"))
                .test_support()
                .flex_none()
                .w(dp(GLYPH))
                .flex()
                .items_center()
                .justify_center()
                .children(ui::status_glyph(step.tone, cx)),
        )
        .child(line_out);
    let word = div()
        .id(sub("word"))
        .test_support()
        .h(dp(LINE))
        .px(dp(CELL_PAD))
        .flex()
        .items_center()
        .text_size(dp(11.))
        .text_color(p.ink_2)
        .whitespace_nowrap()
        .truncate()
        .child(step.word.clone());
    cell(column)
        .id(id.clone())
        .test_support()
        .role(Role::Cell)
        .aria_label(step.tooltip.clone())
        .px_0()
        .h_full()
        .flex()
        .flex_col()
        .justify_center()
        .follow_tooltip(step.tooltip)
        .child(first)
        .child(word)
}

/// Where a row sits in the rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailAt {
    First,
    Middle,
    Last,
    Only,
}

impl RailAt {
    /// The position of row `ix` of `count`.
    pub fn of(ix: usize, count: usize) -> Self {
        match (ix == 0, ix + 1 >= count) {
            (true, true) => Self::Only,
            (true, false) => Self::First,
            (false, true) => Self::Last,
            (false, false) => Self::Middle,
        }
    }

    fn above(self) -> bool {
        matches!(self, Self::Middle | Self::Last)
    }

    fn below(self) -> bool {
        matches!(self, Self::First | Self::Middle)
    }
}

/// The rail's drawing for a pinned cell, [`RAIL`] wide: its node on the
/// first line, and a segment up to the row above and down to the row below.
/// `into` is the segment from above (dotted into a waiting Stage), `onward`
/// the one below.
pub fn rail(
    id: impl Into<SharedString>,
    at: RailAt,
    tone: Tone,
    into: Track,
    onward: Track,
    cx: &App,
) -> Div {
    let id = id.into();
    let sub = |part: &str| SharedString::from(format!("{id}-{part}"));
    let color = |track| track_color(tone, track, cx);
    let node_y = 3. + LINE / 2.;
    let up = stroke(
        sub("up"),
        if at.above() { into } else { Track::None },
        color(into),
        0.,
        0.,
        true,
    )
    .w_full()
    .h(dp(node_y));
    let down = stroke(
        sub("down"),
        if at.below() { onward } else { Track::None },
        color(onward),
        0.,
        0.,
        true,
    )
    .w_full()
    .flex_1();
    let node = div()
        .id(sub("node"))
        .test_support()
        .flex_none()
        .size(dp(7.))
        .my(dp(1.))
        .rounded_full()
        .border_2()
        .border_color(color(Track::Solid))
        .bg(cx.theme().background);
    v_flex()
        .flex_none()
        .w(dp(RAIL))
        .h_full()
        .items_center()
        // The row's own border is 1 px either side; the segments reach past
        // it so the rail stays one line from row to row.
        .mt(px(-1.))
        .mb(px(-1.))
        .child(up)
        .child(node)
        .child(down)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_track_into_a_cell_takes_the_cells_style() {
        assert_eq!(Track::into_tone(Tone::Good), Track::Solid);
        assert_eq!(Track::into_tone(Tone::Info), Track::Dashed);
        assert_eq!(Track::into_tone(Tone::Waiting), Track::Dotted);
        assert_eq!(Track::into_tone(Tone::Unknown), Track::Dotted);
        assert_eq!(Track::into_tone(Tone::Outline), Track::Hairline);
    }

    #[test]
    fn a_dotted_track_dashes_finer_than_a_dashed_one() {
        let [dotted, _] = Track::Dotted.dashes().unwrap();
        let [dashed, _] = Track::Dashed.dashes().unwrap();
        assert!(dotted < dashed);
        assert!(Track::Solid.dashes().is_none());
        assert!(Track::Hairline.dashes().is_none());
    }

    #[test]
    fn the_rail_joins_every_row_but_the_ends() {
        assert_eq!(RailAt::of(0, 1), RailAt::Only);
        assert_eq!(RailAt::of(0, 3), RailAt::First);
        assert_eq!(RailAt::of(1, 3), RailAt::Middle);
        assert_eq!(RailAt::of(2, 3), RailAt::Last);
        assert!(!RailAt::First.above() && RailAt::First.below());
        assert!(RailAt::Last.above() && !RailAt::Last.below());
        assert!(!RailAt::Only.above() && !RailAt::Only.below());
    }

    #[test]
    fn two_lines_and_their_padding_make_the_row() {
        assert_eq!(STEP_ROW, 2. * LINE + 2. * 4.);
    }
}
