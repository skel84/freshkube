//! A terminal: an `alacritty_terminal` grid drawn with GPUI. Its owner feeds
//! it bytes and receives what it sends back as `TerminalEvent`s; it knows
//! nothing about where the bytes come from.
//!
//! - **State.** `Term` and the `vte` parser, with `SCROLLBACK` lines of
//!   history. `feed` parses bytes into the grid at once and schedules one
//!   rebuild of the visible rows' style runs (`snapshot.rs`), so `render`
//!   and the paint (`paint.rs`) only draw.
//! - **Size.** The grid follows the element's bounds, its cell taken from the
//!   theme's monospace font, so the text size scales it. A new size resizes
//!   `Term` and is emitted, at most every `RESIZE_INTERVAL`.
//! - **Keys** (`input.rs`) are encoded from the terminal's modes ahead of the
//!   app's bindings; only Command shortcuts reach the app. Typed text comes
//!   through the platform input handler, so Option, dead keys and input
//!   methods compose characters.
//! - **Mouse** (`mouse.rs`): selection by dragging, words and lines by double
//!   and triple click, and the wheel through the scrollback.
//! - **The emulator's requests** (`listener.rs`): replies, clipboard,
//!   colour queries and the title.

mod input;
mod listener;
mod mouse;
mod paint;
mod snapshot;
#[cfg(any(test, feature = "stress"))]
pub(crate) mod streams;
#[cfg(test)]
mod tests;

use std::rc::Rc;
use std::time::{Duration, Instant};

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::{Config, Osc52, Term};
use alacritty_terminal::vte::ansi::Processor;
use gpui_kit::component::{ActiveTheme, Theme};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, Hsla, MouseButton, Pixels, Point,
    SharedString, Size, Subscription, Task, TestSupportExt, Window, div, rgb,
};

use crate::palette::terminal_colors;
use crate::perf;
use crate::ui::dp;
use listener::Listener;
use snapshot::Snapshot;

/// The key context of a focused terminal.
const CONTEXT: &str = "Terminal";
/// Lines of history kept above the screen.
const SCROLLBACK: usize = 10_000;
/// How often a changing size may resize the grid and be emitted.
pub(crate) const RESIZE_INTERVAL: Duration = Duration::from_millis(100);
/// Line height as a multiple of the monospace font size.
const LINE_HEIGHT: f32 = 1.4;
/// Space between the element's edge and the grid, in `dp`.
const PADDING: f32 = 8.;

/// A grid size in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TerminalSize {
    pub(crate) columns: u16,
    pub(crate) rows: u16,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self {
            columns: 80,
            rows: 24,
        }
    }
}

impl Dimensions for TerminalSize {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        usize::from(self.rows)
    }

    fn columns(&self) -> usize {
        usize::from(self.columns)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TerminalEvent {
    /// Bytes for the program: keys, pastes and the emulator's replies.
    Output(Vec<u8>),
    /// The grid's new size, at most every `RESIZE_INTERVAL`.
    Resize(TerminalSize),
    /// The title the program set, or `None` once it resets it.
    Title(Option<SharedString>),
    /// Command-Escape: hand the keyboard back to the owner.
    Leave,
}

/// Where the grid was last laid out: its top-left corner and one cell.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Geometry {
    origin: Point<Pixels>,
    cell: Size<Pixels>,
}

/// A size waiting for `RESIZE_INTERVAL` to pass since the last resize.
#[derive(Default)]
struct Resizing {
    last: Option<Instant>,
    pending: Option<TerminalSize>,
    timer: Option<Task<()>>,
}

pub(crate) struct TerminalView {
    term: Term<Listener>,
    parser: Processor,
    listener: Listener,
    focus: FocusHandle,
    snapshot: Rc<Snapshot>,
    /// A snapshot rebuild is already scheduled for this effect cycle.
    refresh_pending: bool,
    size: TerminalSize,
    resizing: Resizing,
    geometry: Geometry,
    title: Option<SharedString>,
    /// The left button went down on the grid and hasn't come up yet.
    selecting: bool,
    /// Wheel movement not yet worth a whole line.
    wheel: f32,
    /// Text an input method is composing, drawn at the cursor.
    marked: Option<String>,
    /// The program ended: the screen stays, without a cursor.
    ended: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TerminalView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        input::register(cx.entity().downgrade(), cx);
        let focus = cx.focus_handle();
        let listener = Listener::default();
        let size = TerminalSize::default();
        let term = Term::new(config(), &size, listener.clone());
        let subscriptions = vec![
            // Light and dark have their own colours.
            cx.observe_global::<Theme>(|this: &mut Self, cx| this.refresh(cx)),
            // The cursor is hollow without focus.
            cx.on_focus(&focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&focus, window, |_, _, cx| cx.notify()),
        ];
        let mut view = Self {
            term,
            parser: Processor::new(),
            listener,
            focus,
            snapshot: Rc::default(),
            refresh_pending: false,
            size,
            resizing: Resizing::default(),
            geometry: Geometry::default(),
            title: None,
            selecting: false,
            wheel: 0.,
            marked: None,
            ended: false,
            _subscriptions: subscriptions,
        };
        view.refresh(cx);
        view
    }

    /// Parses bytes from the program into the grid. The visible rows are
    /// rebuilt once, after every `feed` of this effect cycle.
    pub(crate) fn feed(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let parsing = perf::span("terminal.feed");
        perf::value("terminal.bytes", bytes.len() as f64);
        self.parser.advance(&mut self.term, bytes);
        drop(parsing);
        self.handle_requests(cx);
        if !self.refresh_pending {
            self.refresh_pending = true;
            let this = cx.entity().downgrade();
            cx.defer(move |cx| {
                let _ = this.update(cx, |this, cx| {
                    this.refresh_pending = false;
                    this.refresh(cx);
                });
            });
        }
    }

    /// Starts over with an empty screen and history, as a new terminal
    /// would, but at the size the element already gave it.
    pub(crate) fn reset(&mut self, cx: &mut Context<Self>) {
        self.listener = Listener::default();
        self.term = Term::new(config(), &self.size, self.listener.clone());
        self.parser = Processor::new();
        self.selecting = false;
        self.wheel = 0.;
        self.marked = None;
        self.ended = false;
        if self.title.take().is_some() {
            cx.emit(TerminalEvent::Title(None));
        }
        self.refresh(cx);
    }

    /// The program ended, so the screen no longer takes input: it stops
    /// drawing the cursor until `reset`.
    pub(crate) fn end(&mut self, cx: &mut Context<Self>) {
        if !std::mem::replace(&mut self.ended, true) {
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(crate) fn ended(&self) -> bool {
        self.ended
    }

    /// The grid's size, which the program should be told about.
    pub(crate) fn size(&self) -> TerminalSize {
        self.size
    }

    /// The title the program set. Its owner hears of it as an event.
    #[cfg(test)]
    pub(crate) fn title(&self) -> Option<&SharedString> {
        self.title.as_ref()
    }

    /// Rebuilds what the next frame paints and redraws.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let _span = perf::span("terminal.snapshot");
        crate::desktop::probe::hit("terminal.snapshot");
        self.snapshot = Rc::new(Snapshot::build(&self.term, terminal_colors(cx)));
        cx.notify();
    }

    /// Sends bytes to the program on the user's behalf: the view returns to
    /// the bottom and the selection ends, as typing does in a terminal.
    fn send(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if bytes.is_empty() {
            return;
        }
        let scrolled = self.term.grid().display_offset() != 0;
        if scrolled {
            self.term.scroll_display(Scroll::Bottom);
        }
        if scrolled || self.term.selection.take().is_some() {
            self.refresh(cx);
        }
        cx.emit(TerminalEvent::Output(bytes));
    }

    /// Takes the grid's place and cell from the last layout, and resizes the
    /// grid when it now fits another number of cells.
    fn lay_out(&mut self, geometry: Geometry, size: TerminalSize, cx: &mut Context<Self>) {
        self.geometry = geometry;
        if size == self.size {
            self.resizing.pending = None;
            self.resizing.timer = None;
            return;
        }
        if self.resizing.pending == Some(size) {
            return;
        }
        let now = cx.background_executor().now();
        let wait = self.resizing.last.map_or(Duration::ZERO, |last| {
            RESIZE_INTERVAL.saturating_sub(now.saturating_duration_since(last))
        });
        if wait.is_zero() {
            self.resize(size, cx);
            return;
        }
        self.resizing.pending = Some(size);
        if self.resizing.timer.is_none() {
            self.resizing.timer = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                let _ = this.update(cx, |this, cx| {
                    this.resizing.timer = None;
                    if let Some(size) = this.resizing.pending.take() {
                        this.resize(size, cx);
                    }
                });
            }));
        }
    }

    fn resize(&mut self, size: TerminalSize, cx: &mut Context<Self>) {
        self.term.resize(size);
        self.size = size;
        self.resizing.last = Some(cx.background_executor().now());
        cx.emit(TerminalEvent::Resize(size));
        self.refresh(cx);
    }

    /// The screen's text, one string per row with trailing blanks trimmed.
    #[cfg(test)]
    pub(crate) fn screen_text(&self) -> Vec<String> {
        use alacritty_terminal::index::{Column, Line};
        let grid = self.term.grid();
        let offset = grid.display_offset() as i32;
        (0..grid.screen_lines() as i32)
            .map(|line| {
                let row = &grid[Line(line - offset)];
                let text: String = (0..grid.columns())
                    .map(|column| &row[Column(column)])
                    .filter(|cell| {
                        !cell
                            .flags
                            .contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER)
                    })
                    .map(|cell| cell.c)
                    .collect();
                text.trim_end().to_owned()
            })
            .collect()
    }
}

fn config() -> Config {
    Config {
        scrolling_history: SCROLLBACK,
        // The user chose to let programs read the clipboard as well as write
        // it (docs/POD_EXEC.md).
        osc52: Osc52::CopyPaste,
        ..Config::default()
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("terminal");
        let _span = perf::span("terminal.render");
        let colors = terminal_colors(cx);
        let theme = cx.theme();
        let fonts = paint::Fonts::new(theme.mono_font_family.clone(), theme.mono_font_size);
        let selection = theme.selection;
        let thumb: Hsla = rgb(colors.foreground).into();
        let label = self
            .title
            .clone()
            .unwrap_or_else(|| SharedString::new_static("Terminal"));
        div()
            .id("terminal")
            .test_support()
            .aria_label(label)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(rgb(colors.background))
            .p(dp(PADDING))
            .cursor_text()
            .on_action(cx.listener(|this, _: &input::CopySelection, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &input::PasteClipboard, _, cx| this.paste(cx)))
            .on_action(
                cx.listener(|_, _: &input::LeaveTerminal, _, cx| cx.emit(TerminalEvent::Leave)),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event, window, cx| this.mouse_down(event, window, cx)),
            )
            .on_scroll_wheel(cx.listener(|this, event, _, cx| this.scroll_wheel(event, cx)))
            .child(paint::grid(paint::Grid {
                view: cx.entity(),
                snapshot: self.snapshot.clone(),
                focus: self.focus.clone(),
                marked: self.marked.clone(),
                ended: self.ended,
                fonts,
                selection,
                thumb: thumb.opacity(0.35),
            }))
    }
}
