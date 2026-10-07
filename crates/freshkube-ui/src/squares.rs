//! Container squares: one small square per container, in the order the pod
//! runs them, init containers first and dimmed (docs/DESIGN.md, G7). Colour
//! carries the state, an outline a container that restarted or has no state
//! yet; the caller's tooltip names each container, so colour is never alone.

use gpui_kit::{prelude::*, *};

use crate::palette::Palette;
use crate::ui::{Tone, dp};

/// A square's side.
pub const SIZE: f32 = 8.;
/// The space between squares.
pub const GAP: f32 = 3.;
/// How many squares a cell draws before it counts the rest as `+N`.
pub const SHOWN: usize = 8;
/// The `+N` after the last square: its width, at the cell's text size.
const MORE_WIDTH: f32 = 22.;

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

    fn color(&self, p: &Palette) -> Hsla {
        match self.tone {
            Tone::Good => p.good,
            Tone::Warn => p.warn,
            Tone::Crit | Tone::Died => p.crit,
            _ => p.unk,
        }
    }
}

/// The width in dp that `count` squares take, with their `+N`.
pub fn width(count: usize) -> f32 {
    let shown = count.min(SHOWN) as f32;
    let squares = shown * SIZE + (shown - 1.).max(0.) * GAP;
    if count > SHOWN {
        squares + GAP + MORE_WIDTH
    } else {
        squares
    }
}

/// The squares in a row, at most [`SHOWN`], then `+N` in muted text.
pub fn squares(squares: &[Square], p: &Palette) -> Div {
    let hidden = squares.len().saturating_sub(SHOWN);
    h_flex()
        .flex_none()
        .gap(dp(GAP))
        .children(squares.iter().take(SHOWN).map(|square| {
            let color = square.color(p);
            let drawn = div()
                .flex_none()
                .size(dp(SIZE))
                .rounded(px(2.))
                .when(square.dim, |this| this.opacity(0.5));
            if square.outlined {
                drawn.border_2().border_color(color)
            } else {
                drawn.bg(color)
            }
        }))
        .when(hidden > 0, |this| {
            this.child(
                div()
                    .flex_none()
                    .text_size(dp(11.))
                    .text_color(p.muted)
                    .child(format!("+{hidden}")),
            )
        })
}

fn h_flex() -> Div {
    div().flex().flex_row().items_center()
}

#[cfg(test)]
mod tests {
    use super::{GAP, MORE_WIDTH, SHOWN, SIZE, width};

    #[test]
    fn the_width_fits_the_squares_and_counts_the_rest() {
        assert_eq!(width(0), 0.);
        assert_eq!(width(1), SIZE);
        assert_eq!(width(3), 3. * SIZE + 2. * GAP);
        assert_eq!(width(SHOWN), width(SHOWN + 5) - GAP - MORE_WIDTH);
    }
}
