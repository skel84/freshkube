//! Semantic colors the gpui-kit theme doesn't carry (status inks, soft
//! fills). Values match `assets/theme.json` and the design mockup.
use gpui_kit::component::ActiveTheme;
use gpui_kit::{App, Hsla, rgb, rgba};

#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub(crate) surface: Hsla,
    pub(crate) surface_2: Hsla,
    pub(crate) hover: Hsla,
    pub(crate) ink: Hsla,
    pub(crate) ink_2: Hsla,
    pub(crate) muted: Hsla,
    pub(crate) faint: Hsla,
    pub(crate) line: Hsla,
    pub(crate) line_strong: Hsla,
    pub(crate) track: Hsla,
    pub(crate) accent: Hsla,
    pub(crate) accent_soft: Hsla,
    pub(crate) accent_line: Hsla,
    pub(crate) good: Hsla,
    pub(crate) good_ink: Hsla,
    pub(crate) good_soft: Hsla,
    pub(crate) warn: Hsla,
    pub(crate) warn_ink: Hsla,
    pub(crate) warn_soft: Hsla,
    pub(crate) warn_line: Hsla,
    pub(crate) crit: Hsla,
    pub(crate) crit_ink: Hsla,
    pub(crate) crit_soft: Hsla,
    pub(crate) unk: Hsla,
    pub(crate) unk_ink: Hsla,
    pub(crate) unk_soft: Hsla,
    pub(crate) mark: Hsla,
}

fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

fn hexa(value: u32) -> Hsla {
    rgba(value).into()
}

fn light() -> Palette {
    Palette {
        surface: hex(0xFFFFFF),
        surface_2: hex(0xF0F2F5),
        hover: hexa(0x10141B0E),
        ink: hex(0x10141B),
        ink_2: hex(0x394150),
        muted: hex(0x5C6574),
        faint: hex(0x8A93A2),
        line: hex(0xDDE1E8),
        line_strong: hex(0xC8CED8),
        track: hex(0xE4E7EC),
        accent: hex(0x2B59E0),
        accent_soft: hexa(0x2B59E017),
        accent_line: hexa(0x2B59E06B),
        good: hex(0x0CA30C),
        good_ink: hex(0x0A760A),
        good_soft: hexa(0x0CA30C1C),
        warn: hex(0xFAB219),
        warn_ink: hex(0x8A5800),
        warn_soft: hexa(0xFAB2192E),
        warn_line: hexa(0xD6920073),
        crit: hex(0xD03B3B),
        crit_ink: hex(0xB02727),
        crit_soft: hexa(0xD03B3B1A),
        unk: hex(0x8A93A2),
        unk_ink: hex(0x566070),
        unk_soft: hexa(0x8A93A226),
        mark: hexa(0xFAB21966),
    }
}

fn dark() -> Palette {
    Palette {
        surface: hex(0x151A21),
        surface_2: hex(0x1B2129),
        hover: hexa(0xE3E7EE0F),
        ink: hex(0xE3E7EE),
        ink_2: hex(0xC1C8D3),
        muted: hex(0x8C96A6),
        faint: hex(0x5F6979),
        line: hex(0x232A35),
        line_strong: hex(0x2F3845),
        track: hex(0x232A34),
        accent: hex(0x7C9BFF),
        accent_soft: hexa(0x7C9BFF21),
        accent_line: hexa(0x7C9BFF80),
        good: hex(0x0CA30C),
        good_ink: hex(0x3FCC3F),
        good_soft: hexa(0x0CA30C2E),
        warn: hex(0xFAB219),
        warn_ink: hex(0xFAB219),
        warn_soft: hexa(0xFAB21921),
        warn_line: hexa(0xFAB21966),
        crit: hex(0xD03B3B),
        crit_ink: hex(0xFF7070),
        crit_soft: hexa(0xD03B3B33),
        unk: hex(0x6B7586),
        unk_ink: hex(0x8C96A6),
        unk_soft: hexa(0x8C96A624),
        mark: hexa(0xFAB21952),
    }
}

pub(crate) fn palette(cx: &App) -> Palette {
    if cx.theme().mode.is_dark() {
        dark()
    } else {
        light()
    }
}
