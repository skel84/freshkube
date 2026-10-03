//! Semantic colors the gpui-kit theme doesn't carry (status inks, soft
//! fills). Values match `assets/theme.json`; the dark set is the Console
//! look in docs/DESIGN.md.
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

/// The Console look (docs/DESIGN.md).
fn dark() -> Palette {
    Palette {
        surface: hex(0x17181B),
        surface_2: hex(0x1E2024),
        hover: hex(0x25272C),
        ink: hex(0xF2F3F5),
        ink_2: hex(0xC9CCD1),
        muted: hex(0x8E939B),
        faint: hex(0x5E636B),
        line: hex(0x24262A),
        line_strong: hex(0x33363C),
        track: hex(0x25272C),
        accent: hex(0x4797FF),
        accent_soft: hexa(0x4797FF29),
        accent_line: hexa(0x4797FF80),
        good: hex(0x3DD68C),
        good_ink: hex(0x3DD68C),
        good_soft: hexa(0x3DD68C24),
        warn: hex(0xF5A623),
        warn_ink: hex(0xF5A623),
        warn_soft: hexa(0xF5A62324),
        warn_line: hexa(0xF5A62366),
        crit: hex(0xF0484E),
        crit_ink: hex(0xFF6B70),
        crit_soft: hexa(0xF0484E29),
        unk: hex(0x5E636B),
        unk_ink: hex(0x8E939B),
        unk_soft: hexa(0x8E939B24),
        mark: hexa(0xF5A62352),
    }
}

pub(crate) fn palette(cx: &App) -> Palette {
    if cx.theme().mode.is_dark() {
        dark()
    } else {
        light()
    }
}

/// The terminal's colours: the 16 ANSI colours, then its own foreground
/// and background, as `0xRRGGBB`. Tuned for contrast on the terminal's
/// background, so yellow and white stay readable on a white terminal.
pub(crate) struct TerminalColors {
    pub(crate) ansi: [u32; 16],
    pub(crate) foreground: u32,
    pub(crate) background: u32,
}

const TERMINAL_LIGHT: TerminalColors = TerminalColors {
    ansi: [
        0x10141B, 0xB02727, 0x0A760A, 0x8A5800, 0x2B59E0, 0x8E3AB8, 0x0B7A85, 0x8A93A2, 0x5C6574,
        0xD03B3B, 0x0CA30C, 0xB07A00, 0x4A72F0, 0xA855D0, 0x1496A3, 0x394150,
    ],
    foreground: 0x10141B,
    background: 0xFFFFFF,
};

const TERMINAL_DARK: TerminalColors = TerminalColors {
    ansi: [
        0x25272C, 0xF0484E, 0x3DD68C, 0xF5A623, 0x4797FF, 0xB08CF0, 0x3FC4D0, 0xC9CCD1, 0x5E636B,
        0xFF6B70, 0x6FE3A8, 0xFFC25C, 0x7AB4FF, 0xC9AEF7, 0x72D9E2, 0xF2F3F5,
    ],
    foreground: 0xF2F3F5,
    background: 0x17181B,
};

pub(crate) fn terminal_colors(cx: &App) -> &'static TerminalColors {
    if cx.theme().mode.is_dark() {
        &TERMINAL_DARK
    } else {
        &TERMINAL_LIGHT
    }
}
