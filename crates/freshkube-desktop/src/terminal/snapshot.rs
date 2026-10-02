//! The visible rows as style runs, built when the grid changes so that a
//! frame only paints. A run is a stretch of cells sharing one style; text
//! outside ASCII gets a run per cell, so wide characters, emoji and combining
//! marks start exactly on their column whatever the font's advance.

use std::ops::Range;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use gpui_kit::{Hsla, Rgba, SharedString};

use super::listener::Listener;
use crate::palette::TerminalColors;

/// A stretch of one row's cells in one style.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Run {
    pub(super) column: usize,
    pub(super) cells: usize,
    pub(super) text: SharedString,
    pub(super) style: Style,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Style {
    pub(super) foreground: Hsla,
    /// `None` where the cell shows the terminal's own background.
    pub(super) background: Option<Hsla>,
    pub(super) bold: bool,
    pub(super) italic: bool,
    pub(super) underline: Option<Underline>,
    pub(super) strikethrough: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Underline {
    pub(super) color: Hsla,
    pub(super) wavy: bool,
}

/// The cell under the cursor, so a block cursor can draw it inverted.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Cursor {
    pub(super) line: usize,
    pub(super) column: usize,
    pub(super) cells: usize,
    pub(super) shape: CursorShape,
    pub(super) text: SharedString,
    pub(super) color: Hsla,
    /// The text's colour under a block cursor: the cell's background.
    pub(super) text_color: Hsla,
}

/// Where the view is in the scrollback, while it is scrolled back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ScrollPosition {
    pub(super) offset: usize,
    pub(super) history: usize,
    pub(super) screen: usize,
}

#[derive(Debug, Default)]
pub(super) struct Snapshot {
    pub(super) rows: Vec<Vec<Run>>,
    /// Selected columns of each row on screen that has any.
    pub(super) selected: Vec<(usize, Range<usize>)>,
    pub(super) cursor: Option<Cursor>,
    pub(super) scrolled: Option<ScrollPosition>,
}

impl Snapshot {
    pub(super) fn build(term: &Term<Listener>, palette: &TerminalColors) -> Self {
        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        let lines = term.screen_lines();
        let columns = term.columns();
        let colors = Colorist {
            overrides: content.colors,
            palette,
        };
        let mut rows: Vec<Vec<Run>> = vec![Vec::new(); lines];
        let mut building: Option<(usize, Building)> = None;
        let mut cursor = None;
        let cursor_point = content.cursor.point;
        for indexed in content.display_iter {
            let line = (indexed.point.line.0 + offset) as usize;
            let column = indexed.point.column.0;
            let cell: &Cell = indexed.cell;
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let style = colors.style(cell);
            let cells = if cell.flags.contains(Flags::WIDE_CHAR) {
                2
            } else {
                1
            };
            let zerowidth = cell.zerowidth();
            let simple = cell.c.is_ascii() && cells == 1 && zerowidth.is_none();
            if indexed.point == cursor_point && content.cursor.shape != CursorShape::Hidden {
                let mut text = String::from(cell.c);
                text.extend(zerowidth.into_iter().flatten());
                cursor = Some(Cursor {
                    line,
                    column,
                    cells,
                    shape: content.cursor.shape,
                    text: text.into(),
                    color: colors.overrides[NamedColor::Cursor].map_or(style.foreground, hsla),
                    text_color: style
                        .background
                        .unwrap_or_else(|| hsla(palette_rgb(palette.background))),
                });
            }
            match &mut building {
                Some((at, run))
                    if *at == line
                        && run.simple
                        && simple
                        && run.style == style
                        && run.column + run.cells == column =>
                {
                    run.text.push(cell.c);
                    run.cells += 1;
                }
                _ => {
                    if let Some((at, run)) = building.take() {
                        run.finish(&mut rows[at]);
                    }
                    let mut text = String::from(cell.c);
                    text.extend(zerowidth.into_iter().flatten());
                    building = Some((
                        line,
                        Building {
                            column,
                            cells,
                            text,
                            style,
                            simple,
                        },
                    ));
                }
            }
        }
        if let Some((at, run)) = building {
            run.finish(&mut rows[at]);
        }
        let selected = content.selection.map_or_else(Vec::new, |range| {
            (0..lines)
                .filter_map(|line| {
                    let at = line as i32 - offset;
                    if at < range.start.line.0 || at > range.end.line.0 {
                        return None;
                    }
                    let from = if at == range.start.line.0 {
                        range.start.column.0
                    } else {
                        0
                    };
                    let to = if at == range.end.line.0 {
                        range.end.column.0 + 1
                    } else {
                        columns
                    };
                    (from < to).then_some((line, from..to.min(columns)))
                })
                .collect()
        });
        let history = term.grid().history_size();
        Self {
            rows,
            selected,
            cursor,
            scrolled: (offset > 0).then_some(ScrollPosition {
                offset: offset as usize,
                history,
                screen: lines,
            }),
        }
    }
}

struct Building {
    column: usize,
    cells: usize,
    text: String,
    style: Style,
    simple: bool,
}

impl Building {
    /// Keeps the run without the trailing blanks it would draw nothing for:
    /// spaces on the terminal's own background.
    fn finish(mut self, row: &mut Vec<Run>) {
        let plain = self.style.background.is_none()
            && self.style.underline.is_none()
            && !self.style.strikethrough;
        if plain && self.simple {
            let kept = self.text.trim_end_matches(' ').len();
            self.cells -= self.text.len() - kept;
            self.text.truncate(kept);
        }
        if !(plain && self.text.chars().all(|c| c == ' ')) {
            row.push(Run {
                column: self.column,
                cells: self.cells,
                text: self.text.into(),
                style: self.style,
            });
        }
    }
}

/// Resolves a cell's colours: the program's own palette changes first, then
/// the theme's.
struct Colorist<'a> {
    overrides: &'a Colors,
    palette: &'a TerminalColors,
}

impl Colorist<'_> {
    fn style(&self, cell: &Cell) -> Style {
        let flags = cell.flags;
        let background_rgb = self.rgb(Color::Named(NamedColor::Background));
        let mut foreground = self.rgb(cell.fg);
        let mut background = match cell.bg {
            Color::Named(NamedColor::Background) => None,
            color => Some(self.rgb(color)),
        };
        if flags.contains(Flags::INVERSE) {
            let swapped = background.unwrap_or(background_rgb);
            background = Some(foreground);
            foreground = swapped;
        }
        if flags.contains(Flags::DIM) {
            foreground = blend(foreground, background.unwrap_or(background_rgb), 0.4);
        }
        if flags.contains(Flags::HIDDEN) {
            foreground = background.unwrap_or(background_rgb);
        }
        let foreground = hsla(foreground);
        let underline = flags.intersects(Flags::ALL_UNDERLINES).then(|| Underline {
            color: cell
                .underline_color()
                .map_or(foreground, |color| hsla(self.rgb(color))),
            wavy: flags.contains(Flags::UNDERCURL),
        });
        Style {
            foreground,
            background: background.map(hsla),
            bold: flags.contains(Flags::BOLD),
            italic: flags.contains(Flags::ITALIC),
            underline,
            strikethrough: flags.contains(Flags::STRIKEOUT),
        }
    }

    fn rgb(&self, color: Color) -> Rgb {
        match color {
            Color::Spec(rgb) => rgb,
            Color::Indexed(index) => self.index(usize::from(index)),
            Color::Named(name) => self.index(name as usize),
        }
    }

    fn index(&self, index: usize) -> Rgb {
        self.overrides[index].unwrap_or_else(|| default_rgb(index, self.palette))
    }
}

/// The theme's colour at an index of the emulator's colour table: the 16
/// ANSI colours, the 256-colour cube and grey ramp, then the named colours
/// from 256 on (foreground, background, cursor, dim and bright variants).
pub(super) fn default_rgb(index: usize, palette: &TerminalColors) -> Rgb {
    const BACKGROUND: usize = NamedColor::Background as usize;
    const DIM_BLACK: usize = NamedColor::DimBlack as usize;
    const DIM_WHITE: usize = NamedColor::DimWhite as usize;
    const DIM_FOREGROUND: usize = NamedColor::DimForeground as usize;
    let background = palette_rgb(palette.background);
    match index {
        0..16 => palette_rgb(palette.ansi[index]),
        16..232 => {
            let cube = index - 16;
            let step = |value: usize| if value == 0 { 0 } else { 55 + value as u8 * 40 };
            Rgb {
                r: step(cube / 36),
                g: step(cube / 6 % 6),
                b: step(cube % 6),
            }
        }
        232..=255 => {
            let grey = 8 + (index - 232) as u8 * 10;
            Rgb {
                r: grey,
                g: grey,
                b: grey,
            }
        }
        BACKGROUND => background,
        DIM_BLACK..=DIM_WHITE => blend(
            palette_rgb(palette.ansi[index - DIM_BLACK]),
            background,
            0.4,
        ),
        DIM_FOREGROUND => blend(palette_rgb(palette.foreground), background, 0.4),
        // The foreground, bright foreground and cursor.
        _ => palette_rgb(palette.foreground),
    }
}

pub(super) fn palette_rgb(value: u32) -> Rgb {
    Rgb {
        r: (value >> 16) as u8,
        g: (value >> 8) as u8,
        b: value as u8,
    }
}

/// `from` moved `amount` of the way toward `to`.
fn blend(from: Rgb, to: Rgb, amount: f32) -> Rgb {
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * amount).round() as u8;
    Rgb {
        r: mix(from.r, to.r),
        g: mix(from.g, to.g),
        b: mix(from.b, to.b),
    }
}

pub(super) fn hsla(rgb: Rgb) -> Hsla {
    Rgba {
        r: f32::from(rgb.r) / 255.,
        g: f32::from(rgb.g) / 255.,
        b: f32::from(rgb.b) / 255.,
        a: 1.,
    }
    .into()
}
