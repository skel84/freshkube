//! The tag that shows a cluster's protection level
//! ([docs/WRITE_MODEL.md](../../../docs/WRITE_MODEL.md#the-visible-level),
//! [docs/DESIGN.md](../../../docs/DESIGN.md#the-header)). This crate knows no
//! cluster, so [`Mark`] is the look of a level and the caller says which
//! level it is.
use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, FontWeight, Hsla, SharedString, px};

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

    /// The pinned icon set's glyph: a lock, a shield, a shield with a tick,
    /// an open lock.
    fn icon(self) -> IconName {
        match self {
            Mark::ReadOnly => IconName::Lock,
            Mark::Protected => IconName::Shield,
            Mark::Guarded => IconName::ShieldCheck,
            Mark::Open => IconName::LockOpen,
        }
    }

    /// The fill, the word's ink and the glyph's. The tint and the glyph carry
    /// the tone: Read-only is Info's, Protected Crit's, Open Warn's. The word
    /// is `ink`, since a tone's own text on its tint falls under 4.5:1 on the
    /// switcher's `surface_2` in dark. Guarded is the default, so it is the
    /// quietest: `surface_2` with `muted` ink, and a 1 px `line_strong`
    /// border (see [`tag`]) because the switcher it sits in is `surface_2`.
    pub fn inks(self, p: &Palette) -> Inks {
        match self {
            Mark::ReadOnly => Inks::new(p.accent_soft, p.ink, p.accent),
            Mark::Protected => Inks::new(p.crit_soft, p.ink, p.crit_ink),
            Mark::Guarded => Inks::new(p.surface_2, p.muted, p.muted),
            Mark::Open => Inks::new(p.warn_soft, p.ink, p.warn_ink),
        }
    }

    /// The 2 px cap a surface of this level wears (#878): Read-only,
    /// Protected and Open, in their glyph's colour. Guarded, the default,
    /// has none, so a cap always means a level that differs from it.
    pub fn cap(self, p: &Palette) -> Option<Hsla> {
        (self != Mark::Guarded).then(|| self.inks(p).glyph)
    }

    /// The 1 px hairline a tag has: Guarded only, whose `surface_2` is the
    /// switcher's own.
    pub fn border(self, p: &Palette) -> Option<Hsla> {
        (self == Mark::Guarded).then_some(p.line_strong)
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

/// A level's glyph alone, in `ink`, at a tab's size.
pub fn glyph(mark: Mark, ink: Hsla) -> Icon {
    Icon::new(mark.icon()).size(dp(12.)).text_color(ink)
}

/// How a tag is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// The 20 dp pill with the glyph and the word: the status bar, and the
    /// header from 1000 dp.
    Word,
    /// A 20 dp square with the glyph alone, filled and inked as the tag is,
    /// never bare: the header under 1000 dp. The word is in its tooltip and
    /// its accessibility label.
    Square,
    /// The pill at a table row's size (18 dp, 11 px).
    Cell,
}

/// A level's tag. The caller gives it an id, a tooltip and a label.
pub fn tag(mark: Mark, form: Form, cx: &App) -> Div {
    let p = palette(cx);
    let inks = mark.inks(&p);
    let (height, text) = match form {
        Form::Cell => (18., 11.),
        _ => (20., 11.5),
    };
    h_flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(dp(5.))
        .h(dp(height))
        .map(|this| match form {
            Form::Square => this.w(dp(20.)).rounded(px(6.)),
            _ => this.px(dp(8.)).rounded_full(),
        })
        .bg(inks.fill)
        .text_color(inks.word)
        .text_size(dp(text))
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap()
        .when_some(mark.border(&p), |this, line| {
            this.border_1().border_color(line)
        })
        .child(Icon::new(mark.icon()).size(dp(12.)).text_color(inks.glyph))
        .when(form != Form::Square, |this| {
            this.child(SharedString::from(mark.label()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::{dark, light};

    /// Every level's glyph is in the app's assets: a name missing from them
    /// draws nothing, which left Open blank.
    #[test]
    fn every_levels_glyph_loads() {
        use crate::theme::AppAssets;
        use gpui_kit::AssetSource;
        for mark in Mark::ALL {
            let path = mark.icon().path();
            let loaded = AppAssets.load(&path).unwrap();
            assert!(
                loaded.is_some_and(|svg| !svg.is_empty()),
                "{mark:?}: {path}"
            );
        }
    }

    #[test]
    fn only_guarded_has_a_border_and_it_is_line_strong() {
        for p in [light(), dark()] {
            for mark in Mark::ALL {
                let border = mark.border(&p);
                if mark == Mark::Guarded {
                    assert_eq!(border, Some(p.line_strong));
                    // Its fill is the switcher's own, which is why.
                    assert_eq!(mark.inks(&p).fill, p.surface_2);
                } else {
                    assert_eq!(border, None, "{mark:?}");
                }
            }
        }
    }
}
