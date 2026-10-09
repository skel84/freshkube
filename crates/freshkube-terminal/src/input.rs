//! The keyboard. A focused terminal takes keys ahead of the app's bindings,
//! so Escape, Tab, Control-Tab and the arrows reach the program. Command
//! shortcuts pass through on macOS; Ctrl-Shift shortcuts do on Linux and
//! Windows. Keys are encoded from the terminal's modes
//! (application cursor keys, bracketed paste). Typed text arrives through the
//! platform input handler, so Option, dead keys and input methods compose
//! characters before they are sent.
//!
//! GPUI reports keypad keys as their main-keyboard twins, so application
//! keypad mode can't change what they send; they type digits and Enter, as
//! most terminals do by default.

use std::ops::Range;

use alacritty_terminal::term::TermMode;
use gpui_kit::{
    App, Bounds, ClipboardItem, Context, EntityInputHandler, Global, KeyBinding, Keystroke,
    KeystrokeEvent, Pixels, Subscription, UTF16Selection, WeakEntity, Window, actions, point,
};

use super::{CONTEXT, TerminalView};

actions!(terminal, [CopySelection, PasteClipboard, LeaveTerminal]);

/// Every terminal view, so a keystroke can find the focused one; and the
/// interceptor that hands it the keys. Set once per App, which also marks
/// the bindings as registered.
struct Terminals {
    views: Vec<WeakEntity<TerminalView>>,
    _interceptor: Subscription,
}

impl Global for Terminals {}

pub(super) fn register(view: WeakEntity<TerminalView>, cx: &mut App) {
    if !cx.has_global::<Terminals>() {
        let keys = freshkube_ui::platform::terminal_keys();
        cx.bind_keys([
            KeyBinding::new(keys.copy, CopySelection, Some(CONTEXT)),
            KeyBinding::new(keys.paste, PasteClipboard, Some(CONTEXT)),
            KeyBinding::new(keys.leave, LeaveTerminal, Some(CONTEXT)),
        ]);
        let interceptor = cx.intercept_keystrokes(intercept);
        cx.set_global(Terminals {
            views: Vec::new(),
            _interceptor: interceptor,
        });
    }
    let terminals = cx.global_mut::<Terminals>();
    terminals.views.retain(|view| view.upgrade().is_some());
    terminals.views.push(view);
}

fn is_shortcut(keystroke: &Keystroke) -> bool {
    freshkube_ui::platform::terminal_keys().is_shortcut(&keystroke.modifiers)
}

/// Runs before any binding: a key for a focused terminal goes to it, unless
/// it is a platform shortcut or text the input handler will deliver. Every
/// other Control key stops there, even one that sends nothing.
fn intercept(event: &KeystrokeEvent, window: &mut Window, cx: &mut App) {
    let keystroke = &event.keystroke;
    if is_shortcut(keystroke)
        || types_text(keystroke)
        || !event
            .context_stack
            .iter()
            .any(|context| context.contains(CONTEXT))
    {
        return;
    }
    let focused = cx.global::<Terminals>().views.iter().find_map(|view| {
        let view = view.upgrade()?;
        view.read(cx).focus.is_focused(window).then_some(view)
    });
    let Some(view) = focused else {
        return;
    };
    let handled = view.update(cx, |view, cx| {
        match encode_key(keystroke, *view.term.mode()) {
            Some(bytes) => {
                view.send(bytes, cx);
                true
            }
            None => false,
        }
    });
    // A Control key the shell has no bytes for is still the shell's: the
    // app's Control bindings, such as the dock's Control-. and Control-,,
    // wait until the keyboard leaves the terminal.
    if handled || keystroke.modifiers.control {
        cx.stop_propagation();
    }
}

/// Whether the key types text that the platform input handler delivers:
/// printable characters, with Shift or Option but not Control.
fn types_text(keystroke: &Keystroke) -> bool {
    !keystroke.modifiers.control
        && !keystroke.modifiers.function
        && keystroke
            .key_char
            .as_deref()
            .is_some_and(|text| !text.is_empty() && !text.chars().any(char::is_control))
}

/// The bytes a key sends, or `None` for a key the terminal doesn't use.
/// Keys that type text arrive as text instead (`types_text`).
pub(super) fn encode_key(keystroke: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let modifiers = &keystroke.modifiers;
    if is_shortcut(keystroke) {
        return None;
    }
    // xterm's modifier parameter: 1 + Shift 1 + Alt 2 + Control 4.
    let parameter = 1
        + u8::from(modifiers.shift)
        + 2 * u8::from(modifiers.alt)
        + 4 * u8::from(modifiers.control);
    let cursor = |letter: char| {
        Some(if parameter > 1 {
            format!("\x1b[1;{parameter}{letter}").into_bytes()
        } else if mode.contains(TermMode::APP_CURSOR) {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        })
    };
    let tilde = |number: u8| {
        Some(if parameter > 1 {
            format!("\x1b[{number};{parameter}~").into_bytes()
        } else {
            format!("\x1b[{number}~").into_bytes()
        })
    };
    let function = |letter: char| {
        Some(if parameter > 1 {
            format!("\x1b[1;{parameter}{letter}").into_bytes()
        } else {
            format!("\x1bO{letter}").into_bytes()
        })
    };
    match keystroke.key.as_str() {
        "up" => return cursor('A'),
        "down" => return cursor('B'),
        "right" => return cursor('C'),
        "left" => return cursor('D'),
        "home" => return cursor('H'),
        "end" => return cursor('F'),
        "insert" => return tilde(2),
        "delete" => return tilde(3),
        "pageup" => return tilde(5),
        "pagedown" => return tilde(6),
        "f1" => return function('P'),
        "f2" => return function('Q'),
        "f3" => return function('R'),
        "f4" => return function('S'),
        "f5" => return tilde(15),
        "f6" => return tilde(17),
        "f7" => return tilde(18),
        "f8" => return tilde(19),
        "f9" => return tilde(20),
        "f10" => return tilde(21),
        "f11" => return tilde(23),
        "f12" => return tilde(24),
        "enter" => return Some(b"\r".to_vec()),
        "escape" => return Some(b"\x1b".to_vec()),
        "tab" if modifiers.shift => return Some(b"\x1b[Z".to_vec()),
        "tab" => return Some(b"\t".to_vec()),
        "backspace" if modifiers.control => return Some(vec![0x08]),
        "backspace" => return Some(vec![0x7f]),
        "space" if modifiers.control => return Some(vec![0]),
        "space" => return Some(b" ".to_vec()),
        _ => {}
    }
    if !modifiers.control {
        // Plain characters arrive as text; with an input handler in the
        // way (tests, or none yet painted) send what the key types.
        return keystroke
            .key_char
            .as_ref()
            .filter(|text| !text.is_empty())
            .map(|text| text.as_bytes().to_vec());
    }
    let mut chars = keystroke.key.chars();
    let (Some(key), None) = (chars.next(), chars.next()) else {
        return None;
    };
    let byte = match key.to_ascii_lowercase() {
        letter @ 'a'..='z' => letter as u8 - b'a' + 1,
        '@' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    };
    // Control with Option: Meta, Escape before the control character.
    Some(if modifiers.alt {
        vec![0x1b, byte]
    } else {
        vec![byte]
    })
}

/// What a paste sends: bracketed when the program asked for it, with any
/// Escape removed so the text can't end the bracket early; otherwise with
/// line ends as Return.
pub(super) fn encode_paste(text: &str, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let mut bytes = b"\x1b[200~".to_vec();
        bytes.extend(text.replace('\x1b', "").into_bytes());
        bytes.extend(b"\x1b[201~");
        bytes
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

impl TerminalView {
    pub(super) fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self
            .term
            .selection_to_string()
            .filter(|text| !text.is_empty())
        {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub(super) fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let bytes = encode_paste(&text, *self.term.mode());
        self.send(bytes, cx);
    }

    /// The cell under the cursor, where an input method shows its window.
    fn cursor_bounds(&self) -> Option<Bounds<Pixels>> {
        let cursor = self.snapshot.cursor.as_ref()?;
        let cell = self.geometry.cell;
        let origin = self.geometry.origin
            + point(
                cell.width * cursor.column as f32,
                cell.height * cursor.line as f32,
            );
        Some(Bounds::new(origin, cell))
    }

    fn marked_len(&self) -> usize {
        self.marked
            .as_deref()
            .map_or(0, |text| text.encode_utf16().count())
    }
}

/// Text an input method is composing stays in the view until it commits;
/// committed text goes to the program.
impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let marked: Vec<u16> = self.marked.as_deref()?.encode_utf16().collect();
        let range = range.start.min(marked.len())..range.end.min(marked.len());
        *adjusted_range = Some(range.clone());
        String::from_utf16(&marked[range]).ok()
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self.marked_len();
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|_| 0..self.marked_len())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.marked.take().is_some() {
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.marked.take().is_some() {
            cx.notify();
        }
        self.send(text.as_bytes().to_vec(), cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!text.is_empty()).then(|| text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.cursor_bounds()
    }

    fn character_index_for_point(
        &mut self,
        _: gpui_kit::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::term::TermMode;
    use gpui_kit::Keystroke;

    use super::{encode_key, encode_paste};

    fn key(text: &str, mode: TermMode) -> Option<Vec<u8>> {
        encode_key(&Keystroke::parse(text).unwrap(), mode)
    }

    fn plain(text: &str) -> Option<Vec<u8>> {
        key(text, TermMode::empty())
    }

    #[test]
    fn cursor_keys_follow_application_cursor_mode() {
        assert_eq!(plain("up"), Some(b"\x1b[A".to_vec()));
        assert_eq!(plain("left"), Some(b"\x1b[D".to_vec()));
        assert_eq!(plain("home"), Some(b"\x1b[H".to_vec()));
        assert_eq!(key("up", TermMode::APP_CURSOR), Some(b"\x1bOA".to_vec()));
        assert_eq!(key("end", TermMode::APP_CURSOR), Some(b"\x1bOF".to_vec()));
        // A modifier always uses the CSI form with xterm's parameter.
        assert_eq!(
            key("shift-up", TermMode::APP_CURSOR),
            Some(b"\x1b[1;2A".to_vec())
        );
        assert_eq!(plain("alt-left"), Some(b"\x1b[1;3D".to_vec()));
        assert_eq!(plain("ctrl-right"), Some(b"\x1b[1;5C".to_vec()));
    }

    #[test]
    fn editing_and_function_keys() {
        assert_eq!(plain("enter"), Some(b"\r".to_vec()));
        assert_eq!(plain("escape"), Some(b"\x1b".to_vec()));
        assert_eq!(plain("backspace"), Some(vec![0x7f]));
        assert_eq!(plain("ctrl-backspace"), Some(vec![0x08]));
        assert_eq!(plain("tab"), Some(b"\t".to_vec()));
        assert_eq!(plain("shift-tab"), Some(b"\x1b[Z".to_vec()));
        assert_eq!(plain("ctrl-tab"), Some(b"\t".to_vec()));
        assert_eq!(plain("delete"), Some(b"\x1b[3~".to_vec()));
        assert_eq!(plain("pageup"), Some(b"\x1b[5~".to_vec()));
        assert_eq!(plain("shift-pagedown"), Some(b"\x1b[6;2~".to_vec()));
        assert_eq!(plain("f1"), Some(b"\x1bOP".to_vec()));
        assert_eq!(plain("ctrl-f1"), Some(b"\x1b[1;5P".to_vec()));
        assert_eq!(plain("f5"), Some(b"\x1b[15~".to_vec()));
        assert_eq!(plain("f12"), Some(b"\x1b[24~".to_vec()));
    }

    #[test]
    fn control_characters() {
        assert_eq!(plain("ctrl-a"), Some(vec![1]));
        assert_eq!(plain("ctrl-c"), Some(vec![3]));
        assert_eq!(plain("ctrl-z"), Some(vec![26]));
        assert_eq!(plain("ctrl-space"), Some(vec![0]));
        assert_eq!(plain("ctrl-["), Some(vec![0x1b]));
        assert_eq!(plain("ctrl-]"), Some(vec![0x1d]));
        assert_eq!(plain("ctrl-\\"), Some(vec![0x1c]));
        assert_eq!(plain("ctrl-alt-x"), Some(vec![0x1b, 24]));
        assert_eq!(plain("ctrl-="), None);
    }

    #[test]
    fn command_keys_are_never_encoded() {
        assert_eq!(plain("cmd-c"), None);
        assert_eq!(plain("cmd-escape"), None);
        assert_eq!(plain("cmd-up"), None);
    }

    #[test]
    fn pastes_are_bracketed_only_when_asked() {
        assert_eq!(
            encode_paste("a\nb\r\nc", TermMode::empty()),
            b"a\rb\rc".to_vec()
        );
        assert_eq!(
            encode_paste("rm -rf x\x1b[201~\n", TermMode::BRACKETED_PASTE),
            b"\x1b[200~rm -rf x[201~\n\x1b[201~".to_vec()
        );
    }
}
