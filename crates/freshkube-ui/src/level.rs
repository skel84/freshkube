//! The tag that shows a cluster's protection level
//! ([docs/WRITE_MODEL.md](../../../docs/WRITE_MODEL.md#the-visible-level),
//! [docs/DESIGN.md](../../../docs/DESIGN.md#the-header)). This crate knows no
//! cluster, so [`Mark`] is the look of a level and the caller says which
//! level it is.
use gpui_kit::component::h_flex;
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, FontWeight, Hsla, SharedString, svg, transparent_black};

use crate::palette::{Palette, palette};
use crate::ui::dp;

/// One protection level's look: a glyph, a word and a tone, never colour
/// alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    ReadOnly,
    Protected,
    Guarded,
    Open,
}

impl Mark {
    pub const ALL: [Mark; 4] = [Mark::ReadOnly, Mark::Protected, Mark::Guarded, Mark::Open];

    pub fn label(self) -> &'static str {
        match self {
            Mark::ReadOnly => "Read-only",
            Mark::Protected => "Protected",
            Mark::Guarded => "Guarded",
            Mark::Open => "Open",
        }
    }

    /// A single-colour drawing on a 16 grid, compiled in like the status
    /// glyphs: a lock, a shield, a shield with a tick, an open lock.
    fn drawing(self) -> &'static [u8] {
        match self {
            Mark::ReadOnly => include_bytes!("../assets/glyphs/level-read-only.svg"),
            Mark::Protected => include_bytes!("../assets/glyphs/level-protected.svg"),
            Mark::Guarded => include_bytes!("../assets/glyphs/level-guarded.svg"),
            Mark::Open => include_bytes!("../assets/glyphs/level-open.svg"),
        }
    }

    /// The fill, the word's ink and the glyph's. The tint and the glyph carry
    /// the tone: Read-only is Info's, Protected Crit's, Open Warn's. The word
    /// is `ink`, since a tone's own text on its tint falls under 4.5:1 on the
    /// switcher's `surface_2` in dark. Guarded is the default, so it is the
    /// quietest: no fill, the outline tag's `muted` ink and a hairline
    /// border (see [`tag`]).
    pub fn inks(self, p: &Palette) -> Inks {
        match self {
            Mark::ReadOnly => Inks::new(p.accent_soft, p.ink, p.accent),
            Mark::Protected => Inks::new(p.crit_soft, p.ink, p.crit_ink),
            Mark::Guarded => Inks::new(transparent_black(), p.muted, p.muted),
            Mark::Open => Inks::new(p.warn_soft, p.ink, p.warn_ink),
        }
    }
}

/// A tag's colours.
#[derive(Clone, Copy)]
pub struct Inks {
    pub fill: Hsla,
    pub word: Hsla,
    pub glyph: Hsla,
}

impl Inks {
    fn new(fill: Hsla, word: Hsla, glyph: Hsla) -> Self {
        Self { fill, word, glyph }
    }
}

/// The 20 dp pill: the glyph and the word, or the glyph alone, which keeps
/// the word for the tooltip and the accessibility label. The caller gives it
/// an id and those.
pub fn tag(mark: Mark, glyph_only: bool, cx: &App) -> Div {
    let p = palette(cx);
    let inks = mark.inks(&p);
    h_flex()
        .flex_none()
        .justify_center()
        .gap(dp(5.))
        .h(dp(20.))
        .when(glyph_only, |this| this.w(dp(22.)))
        .when(!glyph_only, |this| this.px(dp(8.)))
        .rounded_full()
        .bg(inks.fill)
        .text_color(inks.word)
        .text_size(dp(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap()
        .when(mark == Mark::Guarded, |this| {
            this.border_1().border_color(p.line_strong)
        })
        .child(
            svg()
                .data(mark.drawing())
                .flex_none()
                .size(dp(12.))
                .text_color(inks.glyph),
        )
        .when(!glyph_only, |this| {
            this.child(SharedString::from(mark.label()))
        })
}
