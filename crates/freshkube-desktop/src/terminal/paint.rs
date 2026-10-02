//! Draws a snapshot. Before painting, the element measures one cell in the
//! theme's monospace font and tells the view how many fit its bounds; then it
//! paints backgrounds, the selection, text, the cursor, the scroll position
//! and any text an input method is composing, and nothing else.

use std::rc::Rc;

use alacritty_terminal::vte::ansi::CursorShape;
use gpui_kit::{
    App, Bounds, ContentMask, DispatchPhase, ElementInputHandler, Entity, FocusHandle, Font,
    FontStyle, FontWeight, Hsla, IntoElement, MouseButton, MouseMoveEvent, MouseUpEvent, Pixels,
    SharedString, StrikethroughStyle, Styled, TextAlign, TextRun, UnderlineStyle, Window, canvas,
    fill, font, outline, point, px, size,
};

use super::snapshot::{Run, Snapshot};
use super::{Geometry, LINE_HEIGHT, TerminalSize, TerminalView};
use crate::perf;
use crate::ui::dp_px;

/// The faces a terminal draws with, all from the theme's monospace family.
#[derive(Clone)]
pub(super) struct Fonts {
    regular: Font,
    bold: Font,
    italic: Font,
    bold_italic: Font,
    size: Pixels,
}

impl Fonts {
    pub(super) fn new(family: SharedString, size: Pixels) -> Self {
        // Without ligatures characters show as typed, and shaping costs half:
        // JetBrains Mono's contextual alternates were most of the paint.
        let regular = Font {
            features: gpui_kit::FontFeatures::disable_ligatures(),
            ..font(family)
        };
        let bold = Font {
            weight: FontWeight::SEMIBOLD,
            ..regular.clone()
        };
        let italic = Font {
            style: FontStyle::Italic,
            ..regular.clone()
        };
        let bold_italic = Font {
            style: FontStyle::Italic,
            ..bold.clone()
        };
        Self {
            regular,
            bold,
            italic,
            bold_italic,
            size,
        }
    }

    fn face(&self, bold: bool, italic: bool) -> &Font {
        match (bold, italic) {
            (false, false) => &self.regular,
            (true, false) => &self.bold,
            (false, true) => &self.italic,
            (true, true) => &self.bold_italic,
        }
    }

    /// One cell: the advance of a monospace glyph by a rounded line height.
    fn cell(&self, window: &Window) -> gpui_kit::Size<Pixels> {
        let text = window.text_system();
        let width = text
            .advance(text.resolve_font(&self.regular), self.size, 'm')
            .map_or(self.size * 0.6, |advance| advance.width);
        size(width, (self.size * LINE_HEIGHT).round())
    }
}

/// What the paint needs besides the snapshot.
pub(super) struct Grid {
    pub(super) view: Entity<TerminalView>,
    pub(super) snapshot: Rc<Snapshot>,
    pub(super) focus: FocusHandle,
    pub(super) marked: Option<String>,
    pub(super) fonts: Fonts,
    pub(super) selection: Hsla,
    pub(super) thumb: Hsla,
}

pub(super) fn grid(grid: Grid) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| prepaint(grid, bounds, window, cx),
        |bounds, grid, window, cx| paint(&grid, bounds, window, cx),
    )
    .size_full()
}

/// Fits the grid to the bounds. The view may resize and rebuild the
/// snapshot, so the paint takes the snapshot from it afterwards.
fn prepaint(mut grid: Grid, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) -> Grid {
    let cell = grid.fonts.cell(window);
    let fit = |space: Pixels, unit: Pixels| {
        (f32::from(space) / f32::from(unit).max(1.))
            .floor()
            .clamp(1., f32::from(u16::MAX)) as u16
    };
    let size = TerminalSize {
        columns: fit(bounds.size.width, cell.width).max(2),
        rows: fit(bounds.size.height, cell.height),
    };
    let geometry = Geometry {
        origin: bounds.origin,
        cell,
    };
    grid.snapshot = grid.view.update(cx, |view, cx| {
        view.lay_out(geometry, size, cx);
        view.snapshot.clone()
    });
    grid
}

fn paint(grid: &Grid, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let _span = perf::span("terminal.paint");
    let cell = grid.fonts.cell(window);
    let origin = bounds.origin;
    let at = |line: usize, column: usize| {
        point(
            origin.x + cell.width * column as f32,
            origin.y + cell.height * line as f32,
        )
    };
    let width = |cells: usize| cell.width * cells as f32;
    let snapshot = &grid.snapshot;
    let mask = ContentMask {
        bounds: Bounds::new(
            origin,
            size(bounds.size.width + dp_px(8., window), bounds.size.height),
        ),
    };
    window.with_content_mask(Some(mask), |window| {
        for (line, runs) in snapshot.rows.iter().enumerate() {
            for run in runs {
                if let Some(background) = run.style.background {
                    let area =
                        Bounds::new(at(line, run.column), size(width(run.cells), cell.height));
                    window.paint_quad(fill(area, background));
                }
            }
        }
        for (line, columns) in &snapshot.selected {
            let area = Bounds::new(
                at(*line, columns.start),
                size(width(columns.len()), cell.height),
            );
            window.paint_quad(fill(area, grid.selection));
        }
        for (line, runs) in snapshot.rows.iter().enumerate() {
            for run in runs {
                paint_run(
                    run,
                    None,
                    at(line, run.column),
                    cell.height,
                    &grid.fonts,
                    window,
                    cx,
                );
            }
        }
        paint_cursor(grid, &at, cell, window, cx);
        paint_scroll_position(grid, bounds, window);
    });
    listen(grid, bounds, window, cx);
}

fn paint_run(
    run: &Run,
    color: Option<Hsla>,
    origin: gpui_kit::Point<Pixels>,
    line_height: Pixels,
    fonts: &Fonts,
    window: &mut Window,
    cx: &mut App,
) {
    if run.text.trim_start().is_empty() && run.style.underline.is_none() && !run.style.strikethrough
    {
        return;
    }
    let style = &run.style;
    let color = color.unwrap_or(style.foreground);
    let text_run = TextRun {
        len: run.text.len(),
        font: fonts.face(style.bold, style.italic).clone(),
        color,
        background_color: None,
        underline: style.underline.map(|underline| UnderlineStyle {
            thickness: px(1.),
            color: Some(underline.color),
            wavy: underline.wavy,
        }),
        strikethrough: style.strikethrough.then_some(StrikethroughStyle {
            thickness: px(1.),
            color: Some(color),
        }),
    };
    let shaped = window
        .text_system()
        .shape_line(run.text.clone(), fonts.size, &[text_run], None);
    let _ = shaped.paint(origin, line_height, TextAlign::Left, None, window, cx);
}

/// The program's cursor shape while focused; a hollow block without focus.
/// A block cursor redraws its cell's text in the background colour.
fn paint_cursor(
    grid: &Grid,
    at: &impl Fn(usize, usize) -> gpui_kit::Point<Pixels>,
    cell: gpui_kit::Size<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(cursor) = &grid.snapshot.cursor else {
        return;
    };
    let origin = at(cursor.line, cursor.column);
    let area = Bounds::new(origin, size(cell.width * cursor.cells as f32, cell.height));
    let focused = grid.focus.is_focused(window);
    let stroke = px(1.).max(window.rem_size() / 8.);
    match cursor.shape {
        CursorShape::Block if focused => {
            window.paint_quad(fill(area, cursor.color));
            if grid.marked.is_none() {
                let run = cursor_run(grid, cursor);
                paint_run(
                    &run,
                    Some(cursor.text_color),
                    origin,
                    cell.height,
                    &grid.fonts,
                    window,
                    cx,
                );
            }
        }
        CursorShape::Beam if focused => {
            window.paint_quad(fill(
                Bounds::new(origin, size(stroke, cell.height)),
                cursor.color,
            ));
        }
        CursorShape::Underline if focused => {
            let bar = Bounds::new(
                point(origin.x, origin.y + cell.height - stroke),
                size(area.size.width, stroke),
            );
            window.paint_quad(fill(bar, cursor.color));
        }
        _ => window.paint_quad(outline(area, cursor.color, gpui_kit::BorderStyle::Solid)),
    }
    if let Some(marked) = &grid.marked {
        let run = Run {
            column: cursor.column,
            cells: marked.chars().count(),
            text: marked.clone().into(),
            style: super::snapshot::Style {
                underline: Some(super::snapshot::Underline {
                    color: cursor.color,
                    wavy: false,
                }),
                background: None,
                ..cursor_run(grid, cursor).style
            },
        };
        paint_run(&run, None, origin, cell.height, &grid.fonts, window, cx);
    }
}

/// The run under the cursor, styled as it is in the grid.
fn cursor_run(grid: &Grid, cursor: &super::snapshot::Cursor) -> Run {
    let style = grid.snapshot.rows[cursor.line]
        .iter()
        .find(|run| (run.column..run.column + run.cells).contains(&cursor.column))
        .map(|run| run.style)
        .unwrap_or(super::snapshot::Style {
            foreground: cursor.color,
            background: None,
            bold: false,
            italic: false,
            underline: None,
            strikethrough: false,
        });
    Run {
        column: cursor.column,
        cells: cursor.cells,
        text: cursor.text.clone(),
        style,
    }
}

/// A thin thumb in the right padding, only while scrolled back.
fn paint_scroll_position(grid: &Grid, bounds: Bounds<Pixels>, window: &mut Window) {
    let Some(position) = grid.snapshot.scrolled else {
        return;
    };
    let total = (position.history + position.screen) as f32;
    let track = bounds.size.height;
    let height = (track * (position.screen as f32 / total)).max(dp_px(16., window));
    let top = (track - height) * (1. - position.offset as f32 / position.history.max(1) as f32);
    let thumb = Bounds::new(
        point(bounds.right() + dp_px(2., window), bounds.top() + top),
        size(dp_px(4., window), height),
    );
    window.paint_quad(fill(thumb, grid.thumb).corner_radii(px(2.)));
}

/// Hands typed text to the view, and follows a selection drag beyond the
/// element until the button comes up.
fn listen(grid: &Grid, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    window.handle_input(
        &grid.focus,
        ElementInputHandler::new(bounds, grid.view.clone()),
        cx,
    );
    let view = grid.view.downgrade();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble && event.dragging() {
            let _ = view.update(cx, |view, cx| view.mouse_drag(event.position, cx));
        }
    });
    let view = grid.view.downgrade();
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
            let _ = view.update(cx, |view, cx| view.mouse_up(cx));
        }
    });
}
