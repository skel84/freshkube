//! Container squares: one small square per container, in the order the pod
//! runs them, init containers first and dimmed (docs/DESIGN.md, G7). Colour
//! carries the state, an outline a container that restarted or has no state
//! yet; the caller's tooltip names each container, so colour is never alone.

use gpui_kit::{prelude::*, *};

use crate::palette::Palette;
use crate::ui::{Tone, dp, glyph_color};

/// A square's side.
pub const SIZE: f32 = 8.;
/// The space between squares.
pub const GAP: f32 = 3.;
/// How many squares a cell draws before it counts the rest as `+N`.
pub const SHOWN: usize = 8;
/// The `+N` after the last square: its width, at the cell's text size.
const MORE_WIDTH: f32 = 22.;
/// A data mark's radius (docs/DESIGN.md, Tokens).
const RADIUS: f32 = 3.;

/// One container's square.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Square {
    /// Good, Warn, Crit (Died draws as Crit) or Unknown, which draws grey.
    pub tone: Tone,
    /// Drawn as a ring: restarted, or no state reported yet.
    pub outlined: bool,
    /// An init container.
    pub dim: bool,
}

impl Square {
    pub fn new(tone: Tone) -> Self {
        Self {
            tone,
            outlined: false,
            dim: false,
        }
    }

    pub fn outlined(mut self, outlined: bool) -> Self {
        self.outlined = outlined;
        self
    }

    pub fn dim(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }

    /// The row glyph's colour for the same tone, so a square agrees
    /// with it.
    fn color(&self, p: &Palette) -> Hsla {
        glyph_color(self.tone, p).unwrap_or(p.unk_ink)
    }
}

/// A pod's squares with their `+N`, derived once when the pod's row
/// arrives rather than on every frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Squares {
    squares: Vec<Square>,
    more: Option<SharedString>,
}

impl Squares {
    pub fn new(squares: Vec<Square>) -> Self {
        let hidden = squares.len().saturating_sub(SHOWN);
        Self {
            more: (hidden > 0).then(|| format!("+{hidden}").into()),
            squares,
        }
    }

    /// The `+N` drawn after the last square, when some don't show.
    pub fn more(&self) -> Option<&SharedString> {
        self.more.as_ref()
    }
}

impl std::ops::Deref for Squares {
    type Target = [Square];

    fn deref(&self) -> &[Square] {
        &self.squares
    }
}

/// The width in dp that `count` squares take, with their `+N`.
pub const fn width(count: usize) -> f32 {
    let shown = if count < SHOWN { count } else { SHOWN };
    let squares = if shown == 0 {
        0.
    } else {
        shown as f32 * SIZE + (shown - 1) as f32 * GAP
    };
    if count > SHOWN {
        squares + GAP + MORE_WIDTH
    } else {
        squares
    }
}

/// The squares in a row, at most [`SHOWN`], then `+N` in muted text.
pub fn squares(squares: &Squares, p: &Palette) -> Div {
    h_flex()
        .flex_none()
        .gap(dp(GAP))
        .children(squares.iter().take(SHOWN).map(|square| {
            let color = square.color(p);
            let drawn = div()
                .flex_none()
                .size(dp(SIZE))
                .rounded(px(RADIUS))
                .when(square.dim, |this| this.opacity(0.5));
            if square.outlined {
                drawn.border_2().border_color(color)
            } else {
                drawn.bg(color)
            }
        }))
        .when_some(squares.more(), |this, more| {
            this.child(
                div()
                    .id("squares-more")
                    .test_support()
                    .flex_none()
                    .text_size(dp(11.))
                    .text_color(p.muted)
                    .child(more.clone()),
            )
        })
}

fn h_flex() -> Div {
    div().flex().flex_row().items_center()
}

#[cfg(test)]
mod tests {
    use super::{GAP, MORE_WIDTH, SHOWN, SIZE, Square, Squares, width};
    use crate::ui::Tone;

    #[test]
    fn the_width_fits_the_squares_and_counts_the_rest() {
        assert_eq!(width(0), 0.);
        assert_eq!(width(1), SIZE);
        assert_eq!(width(3), 3. * SIZE + 2. * GAP);
        assert_eq!(width(SHOWN), width(SHOWN + 5) - GAP - MORE_WIDTH);
    }

    #[test]
    fn the_rest_is_counted_once() {
        let of = |count| Squares::new(vec![Square::new(Tone::Good); count]);
        assert_eq!(of(SHOWN).more(), None);
        assert_eq!(of(SHOWN + 4).more().map(|more| more.as_ref()), Some("+4"));
        assert_eq!(of(SHOWN + 4).len(), SHOWN + 4);
    }
}
