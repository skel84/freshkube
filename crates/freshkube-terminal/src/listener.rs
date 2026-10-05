//! What the emulator asks of its host while it parses: replies for the
//! program, the clipboard, colours and the title. `Term` calls the listener
//! in the middle of `feed`, with the view borrowed, so the listener only
//! queues; `handle_requests` answers once the bytes are parsed.

use std::cell::RefCell;
use std::rc::Rc;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::Dimensions;
use gpui_kit::{ClipboardItem, Context};

use super::{TerminalEvent, TerminalView};
use freshkube_ui::palette::terminal_colors;

#[derive(Clone, Default)]
pub(super) struct Listener(Rc<RefCell<Vec<Event>>>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        self.0.borrow_mut().push(event);
    }
}

impl TerminalView {
    /// Answers what the emulator asked for while parsing.
    pub(super) fn handle_requests(&mut self, cx: &mut Context<Self>) {
        let requests = std::mem::take(&mut *self.listener.0.borrow_mut());
        for request in requests {
            match request {
                // Replies to queries such as the cursor position or device
                // attributes.
                Event::PtyWrite(text) => cx.emit(TerminalEvent::Output(text.into_bytes())),
                // OSC 52: tmux and vim yank to the Mac's clipboard...
                Event::ClipboardStore(_, text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text))
                }
                // ...and may read it back.
                Event::ClipboardLoad(_, format) => {
                    let text = cx
                        .read_from_clipboard()
                        .and_then(|item| item.text())
                        .unwrap_or_default();
                    cx.emit(TerminalEvent::Output(format(&text).into_bytes()));
                }
                Event::ColorRequest(index, format) => {
                    let color = self.term.colors()[index].unwrap_or_else(|| {
                        super::snapshot::default_rgb(index, terminal_colors(cx))
                    });
                    cx.emit(TerminalEvent::Output(format(color).into_bytes()));
                }
                Event::TextAreaSizeRequest(format) => {
                    let cell = self.geometry.cell;
                    let size = WindowSize {
                        num_lines: self.term.screen_lines() as u16,
                        num_cols: self.term.columns() as u16,
                        cell_width: f32::from(cell.width).round() as u16,
                        cell_height: f32::from(cell.height).round() as u16,
                    };
                    cx.emit(TerminalEvent::Output(format(size).into_bytes()));
                }
                Event::Title(title) => {
                    self.title = Some(title.into());
                    cx.emit(TerminalEvent::Title(self.title.clone()));
                    cx.notify();
                }
                Event::ResetTitle => {
                    self.title = None;
                    cx.emit(TerminalEvent::Title(None));
                    cx.notify();
                }
                // The bell is ignored; the rest concern a local pty, a
                // mouse cursor or blinking, which this view doesn't have.
                Event::Bell
                | Event::Wakeup
                | Event::MouseCursorDirty
                | Event::CursorBlinkingChange
                | Event::Exit
                | Event::ChildExit(_) => {}
            }
        }
    }
}
