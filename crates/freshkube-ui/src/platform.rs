//! What differs by platform, in one place
//! ([docs/DESIGN.md](../../docs/DESIGN.md#the-platform-module-and-the-app-menu)):
//! where the window controls sit and the room the header leaves them, the
//! title bar, the keys only one platform has, the terminal's own shortcuts
//! and where the preferences live. Nothing else tests the platform, so a
//! page asks this module, and tests ask it about every [`Platform`], not
//! only the one they run on.

use gpui_kit::component::TitleBar;
use gpui_kit::{
    DefiniteLength, Modifiers, Pixels, Point, SharedString, TitlebarOptions, Window, point, px,
};

use crate::page::{APP_HEADER_HEIGHT, PANE_PADDING};
use crate::ui::{BASE_TEXT, dp};

/// The platforms the app adapts to. Every Unix but macOS is Linux.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Windows,
    Linux,
}

impl Platform {
    /// Every platform, for tests that check each one's answers.
    pub const ALL: [Platform; 3] = [Platform::MacOs, Platform::Windows, Platform::Linux];

    /// The platform the app was built for.
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }

    /// The room the header leaves at its edges: on macOS, the traffic
    /// lights' at its leading edge. Kit's `TitleBar` draws Windows' caption
    /// buttons, and Linux's when the compositor asks the app to, after the
    /// header's content, so they need no room here.
    pub fn header_insets(self) -> HeaderInsets {
        let leading = match self {
            Platform::MacOs => px(TRAFFIC_LIGHT_INSET).into(),
            Platform::Windows | Platform::Linux => dp(PANE_PADDING).into(),
        };
        HeaderInsets {
            leading,
            trailing: dp(PANE_PADDING).into(),
        }
    }

    /// The primary modifier as a shortcut's label starts: `⌘K`, `Ctrl+K`.
    pub const fn primary_modifier(self) -> &'static str {
        match self {
            Platform::MacOs => "⌘",
            Platform::Windows | Platform::Linux => "Ctrl+",
        }
    }

    /// The keystroke that closes the selected tab: Command-W on macOS;
    /// elsewhere Ctrl-W is a terminal's, so the dock takes Ctrl-Shift-W.
    pub const fn close_tab_key(self) -> &'static str {
        match self {
            Platform::MacOs => "cmd-w",
            Platform::Windows | Platform::Linux => "ctrl-shift-w",
        }
    }

    /// The terminal's own shortcuts: Command on macOS; elsewhere plain
    /// Control keys are the shell's, so they take Control with Shift.
    pub const fn terminal_keys(self) -> TerminalKeys {
        match self {
            Platform::MacOs => TerminalKeys {
                copy: "secondary-c",
                paste: "secondary-v",
                leave: "secondary-escape",
                leave_label: "⌘Esc",
                control_shift: false,
            },
            Platform::Windows | Platform::Linux => TerminalKeys {
                copy: "ctrl-shift-c",
                paste: "ctrl-shift-v",
                leave: "ctrl-shift-q",
                leave_label: "Ctrl+Shift+Q",
                control_shift: true,
            },
        }
    }

    /// The folder, under the platform's configuration folder, that holds
    /// the preferences: `~/Library/Application Support/Freshkube` on macOS,
    /// `%APPDATA%\Freshkube` on Windows and `~/.config/freshkube` on Linux.
    pub const fn preferences_folder(self) -> &'static str {
        match self {
            Platform::MacOs | Platform::Windows => "Freshkube",
            Platform::Linux => "freshkube",
        }
    }
}

/// The room the header leaves at its edges for the window's controls, with
/// its own padding: lengths for `pl` and `pr`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeaderInsets {
    pub leading: DefiniteLength,
    pub trailing: DefiniteLength,
}

/// The terminal's copy, paste and leave keystrokes, the label of leave, and
/// which modifiers let a key past a focused terminal to the app's keymap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalKeys {
    pub copy: &'static str,
    pub paste: &'static str,
    pub leave: &'static str,
    pub leave_label: &'static str,
    control_shift: bool,
}

impl TerminalKeys {
    /// Whether a key with these modifiers is the app's, not the shell's:
    /// anything with the platform key, and elsewhere than macOS Control with
    /// Shift.
    pub fn is_shortcut(&self, modifiers: &Modifiers) -> bool {
        modifiers.platform || (self.control_shift && modifiers.control && modifiers.shift)
    }
}

/// The header's insets on this platform ([`Platform::header_insets`]).
pub fn header_insets() -> HeaderInsets {
    Platform::current().header_insets()
}

/// The primary modifier's label on this platform
/// ([`Platform::primary_modifier`]).
pub const fn primary_modifier() -> &'static str {
    Platform::current().primary_modifier()
}

/// The terminal's shortcuts on this platform ([`Platform::terminal_keys`]).
pub const fn terminal_keys() -> TerminalKeys {
    Platform::current().terminal_keys()
}

/// The window's title bar: Kit's, with the traffic lights centred on the
/// header at the text size `rem`. Other platforms ignore their position.
pub fn titlebar_options(title: impl Into<SharedString>, rem: f32) -> TitlebarOptions {
    TitlebarOptions {
        title: Some(title.into()),
        traffic_light_position: Some(traffic_light_position(rem)),
        ..TitleBar::title_bar_options()
    }
}

/// Puts the window's controls where the header expects them at the text
/// size `rem`: on macOS, the traffic lights, which keep their size in
/// points, centred on the header again. Elsewhere Kit's buttons fill the
/// header's height, and the desktop's decorations are its own.
pub fn place_window_controls(window: &Window, rem: f32) {
    #[cfg(target_os = "macos")]
    window.set_traffic_light_position(traffic_light_position(rem));
    #[cfg(not(target_os = "macos"))]
    let _ = (window, rem);
}

/// Where the header's content starts on macOS, in points: clear of the
/// traffic lights, which don't scale with the text size.
const TRAFFIC_LIGHT_INSET: f32 = 80.;
/// The close button's left edge from the window's, in points. AppKit's
/// button frames are 14 × 16 with the 12 pt circle 1 in and 2 down, so the
/// circle starts at 15, as far as it sits from the header's top at 13 px.
const TRAFFIC_LIGHT_X: f32 = 14.;
/// The traffic light circle's diameter, and how far below its button
/// frame's top it starts, in points.
const TRAFFIC_LIGHT: f32 = 12.;
const TRAFFIC_LIGHT_TOP: f32 = 2.;
// The zoom button's circle, two 20 pt steps from the close button's, ends
// at 67 pt: the header's content starts at least 12 pt after it.
const _: () =
    assert!(TRAFFIC_LIGHT_INSET - (TRAFFIC_LIGHT_X + 1. + 2. * 20. + TRAFFIC_LIGHT) >= 12.);

/// Where macOS puts the close button's frame so the traffic lights are
/// centred on the header, whose dp lengths are `rem` pixels per 13: the
/// lights stay one size, so a fixed position centres them at only one
/// text size, and the shell places them again when the text size changes.
fn traffic_light_position(rem: f32) -> Point<Pixels> {
    let header = APP_HEADER_HEIGHT * rem / BASE_TEXT;
    // Centre the circle on the header less its 1 px hairline.
    let top = (header - 1. - TRAFFIC_LIGHT) / 2. - TRAFFIC_LIGHT_TOP;
    point(px(TRAFFIC_LIGHT_X), px(top))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The traffic lights' circles sit centred on the header less its
    /// hairline at every text size, and their left edge as far from the
    /// window's as from the header's top at the default size.
    #[test]
    fn traffic_lights_centre_on_the_header_at_every_text_size() {
        for rem in crate::text_size::STEPS {
            let header = APP_HEADER_HEIGHT * rem / BASE_TEXT - 1.;
            let at = traffic_light_position(rem);
            let top = f32::from(at.y) + TRAFFIC_LIGHT_TOP;
            let bottom = header - top - TRAFFIC_LIGHT;
            assert!(
                (top - bottom).abs() < 0.01,
                "{rem}: {top} above, {bottom} below"
            );
        }
        let at = traffic_light_position(BASE_TEXT);
        let (left, top) = (f32::from(at.x) + 1., f32::from(at.y) + TRAFFIC_LIGHT_TOP);
        assert!(
            (left - top).abs() <= 0.5,
            "{left} from the left, {top} from the top"
        );
    }

    /// macOS leaves the traffic lights their 80 points; elsewhere the
    /// header pads its leading edge as its trailing one.
    #[test]
    fn the_header_clears_the_traffic_lights_on_macos_alone() {
        let padding: DefiniteLength = dp(PANE_PADDING).into();
        for platform in Platform::ALL {
            let insets = platform.header_insets();
            assert_eq!(insets.trailing, padding, "{platform:?}");
            let leading = match platform {
                Platform::MacOs => px(80.).into(),
                Platform::Windows | Platform::Linux => padding,
            };
            assert_eq!(insets.leading, leading, "{platform:?}");
        }
    }

    /// Every key the module names parses, and each platform's keys and
    /// labels are its own: Command on macOS, Control with Shift elsewhere.
    #[test]
    fn each_platform_has_its_own_keys() {
        for platform in Platform::ALL {
            let keys = platform.terminal_keys();
            for key in [platform.close_tab_key(), keys.copy, keys.paste, keys.leave] {
                gpui_kit::Keystroke::parse(key)
                    .unwrap_or_else(|e| panic!("{platform:?} {key}: {e}"));
            }
        }
        assert_eq!(Platform::MacOs.primary_modifier(), "⌘");
        assert_eq!(Platform::Windows.primary_modifier(), "Ctrl+");
        assert_eq!(Platform::MacOs.terminal_keys().leave_label, "⌘Esc");
        assert_eq!(Platform::Linux.terminal_keys().leave_label, "Ctrl+Shift+Q");
        assert_eq!(Platform::Windows.close_tab_key(), "ctrl-shift-w");
    }

    /// Off macOS, Control with Shift is the app's, and plain Control stays
    /// the shell's; on macOS both are the shell's, and Command is the app's.
    #[test]
    fn the_terminal_lets_the_platforms_shortcuts_through() {
        let control = Modifiers::control();
        let control_shift = Modifiers::control_shift();
        let command = Modifiers::command();
        let mac = Platform::MacOs.terminal_keys();
        let linux = Platform::Linux.terminal_keys();
        assert!(mac.is_shortcut(&command));
        assert!(!mac.is_shortcut(&control_shift));
        assert!(!mac.is_shortcut(&control));
        assert!(linux.is_shortcut(&control_shift));
        assert!(!linux.is_shortcut(&control));
    }
}
