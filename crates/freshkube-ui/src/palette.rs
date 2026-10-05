//! Semantic colors the gpui-kit theme doesn't carry (status inks, soft
//! fills). Values match `assets/theme.json`; the dark set is the Fog
//! look in docs/DESIGN.md.
use gpui_kit::component::ActiveTheme;
use gpui_kit::{App, Hsla, rgb, rgba};

#[derive(Clone, Copy)]
pub struct Palette {
    pub surface: Hsla,
    pub surface_2: Hsla,
    pub hover: Hsla,
    pub ink: Hsla,
    pub ink_2: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub line: Hsla,
    pub line_strong: Hsla,
    pub track: Hsla,
    pub accent: Hsla,
    pub accent_soft: Hsla,
    pub accent_line: Hsla,
    pub good: Hsla,
    pub good_ink: Hsla,
    pub good_soft: Hsla,
    pub warn: Hsla,
    pub warn_ink: Hsla,
    pub warn_soft: Hsla,
    pub warn_line: Hsla,
    pub crit: Hsla,
    pub crit_ink: Hsla,
    pub crit_soft: Hsla,
    pub crit_line: Hsla,
    pub unk: Hsla,
    pub unk_ink: Hsla,
    pub unk_soft: Hsla,
    pub mark: Hsla,
    pub memory: Hsla,
    pub integration: Hsla,
    pub on_fill: Hsla,
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
        crit_line: hexa(0xD03B3B66),
        unk: hex(0x8A93A2),
        unk_ink: hex(0x566070),
        unk_soft: hexa(0x8A93A226),
        mark: hexa(0xFAB21966),
        memory: hex(0x7560B9),
        integration: hex(0x7560B9),
        on_fill: hex(0xFFFFFF),
    }
}

/// The Fog look (docs/DESIGN.md).
fn dark() -> Palette {
    Palette {
        surface: hex(0x2C3037),
        surface_2: hex(0x343943),
        hover: hex(0x3C424D),
        ink: hex(0xEEF1F5),
        ink_2: hex(0xCDD3DC),
        muted: hex(0xA0A7B2),
        faint: hex(0x737A85),
        line: hex(0x393E47),
        line_strong: hex(0x4A505B),
        track: hex(0x3C424D),
        accent: hex(0x8AB4F8),
        accent_soft: hexa(0x8AB4F829),
        accent_line: hexa(0x8AB4F880),
        good: hex(0x82D4AB),
        good_ink: hex(0x82D4AB),
        good_soft: hexa(0x82D4AB24),
        warn: hex(0xF2C46D),
        warn_ink: hex(0xF2C46D),
        warn_soft: hexa(0xF2C46D24),
        warn_line: hexa(0xF2C46D66),
        crit: hex(0xF28B82),
        crit_ink: hex(0xF5A097),
        crit_soft: hexa(0xF28B8229),
        crit_line: hexa(0xF28B8266),
        unk: hex(0x737A85),
        unk_ink: hex(0xA0A7B2),
        unk_soft: hexa(0xA0A7B224),
        mark: hexa(0xF2C46D52),
        memory: hex(0xB7AAF7),
        integration: hex(0xB7AAF7),
        on_fill: hex(0x14223B),
    }
}

pub fn palette(cx: &App) -> Palette {
    if cx.theme().mode.is_dark() {
        dark()
    } else {
        light()
    }
}

/// The terminal's colours: the 16 ANSI colours, then its own foreground
/// and background, as `0xRRGGBB`. Tuned for contrast on the terminal's
/// background, so yellow and white stay readable on a white terminal.
pub struct TerminalColors {
    pub ansi: [u32; 16],
    pub foreground: u32,
    pub background: u32,
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
        0x3C424D, 0xF28B82, 0x82D4AB, 0xF2C46D, 0x8AB4F8, 0xB08CF0, 0x3FC4D0, 0xCDD3DC, 0x737A85,
        0xF5A097, 0x6FE3A8, 0xFFC25C, 0x7AB4FF, 0xC9AEF7, 0x72D9E2, 0xEEF1F5,
    ],
    foreground: 0xEEF1F5,
    background: 0x2C3037,
};

pub fn terminal_colors(cx: &App) -> &'static TerminalColors {
    if cx.theme().mode.is_dark() {
        &TERMINAL_DARK
    } else {
        &TERMINAL_LIGHT
    }
}

/// Semantic diff inks, with white labels above 5:1.
pub fn flame_color(delta: i8) -> Hsla {
    hex(match delta {
        d if d < -10 => 0x3F65A0,
        d if d < 0 => 0x3F5A80,
        0 => 0x454B56,
        d if d < 15 => 0x7E4A49,
        _ => 0xA64B46,
    })
}
/// Ordered latency buckets and their separate semantic error row.
pub fn heat_color(level: usize, error: bool) -> Hsla {
    const BLUE: [u32; 6] = [0x323845, 0x33466A, 0x3D5C92, 0x5379BB, 0x7AA0E6, 0xB3CEFA];
    const ERROR: [u32; 6] = [0x3A3036, 0x604044, 0x885553, 0xB36962, 0xD97B72, 0xF28B82];
    hex(if error { ERROR } else { BLUE }[level.min(5)])
}
