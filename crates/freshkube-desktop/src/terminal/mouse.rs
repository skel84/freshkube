//! The mouse: a drag selects cells, a double click a word, a triple click a
//! line; the wheel scrolls through the history, or sends arrow keys to a
//! program on the alternate screen. Programs can't ask for mouse reports yet.

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::{TermMode, viewport_to_point};
use gpui_kit::{Context, MouseDownEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent, Window};

use super::{Geometry, TerminalView};

/// Lines scrolled by one notch of a mouse wheel.
const WHEEL_LINES: f32 = 3.;

impl TerminalView {
    pub(super) fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        let kind = match event.click_count {
            0 | 1 => SelectionType::Simple,
            2 => SelectionType::Semantic,
            _ => SelectionType::Lines,
        };
        let (point, side) = self.grid_point(event.position);
        let mut selection = Selection::new(kind, point, side);
        // A double or triple click selects its word or line at once.
        if kind != SelectionType::Simple {
            selection.update(point, side);
        }
        self.term.selection = Some(selection);
        self.selecting = true;
        self.refresh(cx);
    }

    /// Extends the selection while the button is down, scrolling when the
    /// pointer passes the top or bottom of the grid.
    pub(super) fn mouse_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        let top = self.geometry.origin.y;
        let bottom = top + self.geometry.cell.height * self.term.screen_lines() as f32;
        if position.y < top {
            self.term.scroll_display(Scroll::Delta(1));
        } else if position.y >= bottom {
            self.term.scroll_display(Scroll::Delta(-1));
        }
        let (point, side) = self.grid_point(position);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
        }
        self.refresh(cx);
    }

    /// Ends a drag; a click that selected nothing leaves no selection.
    pub(super) fn mouse_up(&mut self, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.selecting) {
            return;
        }
        if self
            .term
            .selection
            .as_ref()
            .is_some_and(Selection::is_empty)
        {
            self.term.selection = None;
            self.refresh(cx);
        }
    }

    pub(super) fn scroll_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let line_height = self.geometry.cell.height;
        if line_height <= Pixels::ZERO {
            return;
        }
        // Positive is toward the history. Trackpads give pixels, which add up
        // until they make a line; a wheel gives notches.
        self.wheel += match event.delta {
            ScrollDelta::Pixels(delta) => f32::from(delta.y) / f32::from(line_height),
            ScrollDelta::Lines(delta) => delta.y * WHEEL_LINES,
        };
        let lines = self.wheel.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.wheel -= lines as f32;
        let mode = *self.term.mode();
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            let arrow: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            let bytes = arrow.repeat(lines.unsigned_abs() as usize);
            cx.emit(super::TerminalEvent::Output(bytes));
            return;
        }
        let before = self.term.grid().display_offset();
        self.term.scroll_display(Scroll::Delta(lines));
        if self.term.grid().display_offset() != before {
            self.refresh(cx);
        }
    }

    /// The cell under a window position, clamped to the grid, and which half
    /// of it the position is on.
    fn grid_point(&self, position: Point<Pixels>) -> (GridPoint, Side) {
        let Geometry { origin, cell } = self.geometry;
        let columns = self.term.columns();
        let lines = self.term.screen_lines();
        let x = f32::from(position.x - origin.x).max(0.);
        let y = f32::from(position.y - origin.y).max(0.);
        let width = f32::from(cell.width).max(1.);
        let height = f32::from(cell.height).max(1.);
        let column = ((x / width) as usize).min(columns.saturating_sub(1));
        let line = ((y / height) as usize).min(lines.saturating_sub(1));
        let side = if x - column as f32 * width < width / 2. {
            Side::Left
        } else {
            Side::Right
        };
        let point = viewport_to_point(
            self.term.grid().display_offset(),
            GridPoint::new(line, Column(column)),
        );
        (point, side)
    }
}
