//! What differs by platform, in one place
//! ([docs/DESIGN.md](../../docs/DESIGN.md#the-platform-module-and-the-app-menu)):
//! where the window controls sit and the room the header leaves them, the
//! title bar, the app menu and the keys only one platform has, the
//! terminal's own shortcuts and where the preferences live. Nothing else tests the platform, so a
//! page asks this module, and tests ask it about every [`Platform`], not
//! only the one they run on.

use gpui_kit::Styled as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::TitleBar;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::{
    App, DefiniteLength, KeyBinding, MenuItem, Modifiers, Pixels, Point, SharedString,
    SystemMenuType, TitlebarOptions, Window, point, px,
};

use crate::page::{APP_HEADER_HEIGHT, PANE_PADDING};
use crate::ui::{BASE_TEXT, dp};

gpui_kit::actions!(
    app,
    [
        /// Shows what the app is, its version and where it comes from.
        About,
        /// Opens the header's Settings.
        OpenSettings,
        /// Opens Settings on its Coroot section, where the connection and
        /// its project are.
        OpenCorootSettings,
        /// Opens Settings on its Clusters section, where each entry's
        /// Coroot link is.
        OpenClustersSettings,
        /// Opens Settings on its Delivery sources section, where the setup
        /// file's delivery keys are shown.
        OpenSourcesSettings,
        /// Opens Settings on its Argo CD destinations section.
        OpenDestinationsSettings,
        /// Hides the app (macOS).
        Hide,
        /// Hides every other app (macOS).
        HideOthers,
        /// Shows every app again (macOS).
        ShowAll,
        /// Minimizes the window.
        Minimize,
        /// Zooms the window, or puts it back (macOS).
        Zoom,
        /// Opens the menu button's menu, where there is no menu bar.
        OpenAppMenu,
    ]
);

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

    /// The default button's side in a trailing-aligned action pair.
    pub const fn default_button(self) -> Side {
        match self {
            Platform::MacOs | Platform::Linux => Side::Trailing,
            Platform::Windows => Side::Leading,
        }
    }

    /// The name of the system store that keeps credentials.
    pub const fn credential_store(self) -> &'static str {
        match self {
            Platform::MacOs => "the Keychain",
            Platform::Windows => "Credential Manager",
            Platform::Linux => "the Secret Service",
        }
    }

    /// The room the header leaves at its edges: on macOS, the traffic
    /// lights' at its leading edge. Kit's `TitleBar` draws Windows' caption
    /// buttons, and Linux's when the compositor asks the app to, after the
    /// header's content, so the header's content needs no padding for them;
    /// they still take width from the bar, which `controls` counts so that
    /// the centred search lands on the window's middle. Windows has all
    /// three buttons here; [`header_insets_in`] counts the ones a window
    /// draws.
    pub fn header_insets(self) -> HeaderInsets {
        let leading = match self {
            Platform::MacOs => px(TRAFFIC_LIGHT_INSET).into(),
            Platform::Windows | Platform::Linux => dp(PANE_PADDING).into(),
        };
        HeaderInsets {
            leading,
            trailing: dp(PANE_PADDING).into(),
            controls: match self {
                Platform::Windows => caption_buttons_width(3),
                Platform::MacOs | Platform::Linux => px(0.),
            },
        }
    }

    /// Where the app's menus go: macOS's menu bar, or elsewhere a button
    /// in the header that opens them.
    pub const fn app_menu(self) -> AppMenu {
        match self {
            Platform::MacOs => AppMenu::Bar,
            Platform::Windows | Platform::Linux => AppMenu::Button,
        }
    }

    /// The entries macOS's menus have and the app supplies nothing for:
    /// Services and the hiding entries in the app's menu, Minimize and Zoom
    /// in Window. Elsewhere the desktop's own controls do the same, so there
    /// are none.
    pub fn system_items(self, menu: SystemMenu) -> Vec<MenuItem> {
        if self != Platform::MacOs {
            return Vec::new();
        }
        match menu {
            SystemMenu::App => vec![
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide Freshkube", Hide),
                MenuItem::action("Hide others", HideOthers),
                MenuItem::action("Show all", ShowAll),
            ],
            SystemMenu::Window => vec![
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Zoom", Zoom),
            ],
        }
    }

    /// The keys only this platform has: macOS's Settings, Hide, Hide
    /// others and Minimize, and elsewhere F10 for the menu button.
    /// Settings has no key elsewhere, since Control-, is the dock's.
    pub fn key_bindings(self) -> Vec<KeyBinding> {
        match self {
            Platform::MacOs => vec![
                KeyBinding::new("cmd-,", OpenSettings, None),
                KeyBinding::new("cmd-h", Hide, None),
                KeyBinding::new("alt-cmd-h", HideOthers, None),
                KeyBinding::new("cmd-m", Minimize, None),
            ],
            Platform::Windows | Platform::Linux => {
                vec![KeyBinding::new("f10", OpenAppMenu, None)]
            }
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

    /// The Copy link shortcut as a tooltip names it: Shift-Command-C on
    /// macOS, Ctrl-Shift-C elsewhere (the keymap binds `secondary-shift-c`).
    pub const fn copy_link_tooltip(self) -> &'static str {
        match self {
            Platform::MacOs => "Copy link (Shift-Command-C)",
            Platform::Windows | Platform::Linux => "Copy link (Ctrl-Shift-C)",
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

impl Platform {
    /// What goes before a command so that it runs with the environment
    /// variable `name` set to `value`, as a person types it in the platform's
    /// shell: `NAME='value' ` in a POSIX shell and `$env:NAME='value'; ` in
    /// PowerShell. The value is single-quoted, whatever it holds.
    pub fn set_variable(self, name: &str, value: &str) -> String {
        match self {
            Platform::MacOs | Platform::Linux => {
                format!("{name}='{}' ", value.replace('\'', "'\\''"))
            }
            Platform::Windows => format!("$env:{name}='{}'; ", value.replace('\'', "''")),
        }
    }
}

/// Where the app's menus go ([`Platform::app_menu`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppMenu {
    /// The global menu bar: macOS's.
    Bar,
    /// A button first in the header that opens the menus.
    Button,
}

/// A menu that has entries of the system's ([`Platform::system_items`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemMenu {
    /// The app's own menu, named after it.
    App,
    /// Window.
    Window,
}

/// The room the header leaves at its edges for the window's controls, with
/// its own padding: lengths for `pl` and `pr`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeaderInsets {
    pub leading: DefiniteLength,
    pub trailing: DefiniteLength,
    /// The caption buttons Kit's `TitleBar` draws after the header's
    /// content: room the bar loses at its trailing edge, which the content
    /// does not pad for.
    pub controls: Pixels,
}

/// What the header's row is made of, in dp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeaderRow {
    /// Between the leading inset and the trailing controls less the
    /// trailing inset: the room the three parts share.
    pub bar: f32,
    /// What the leading side's content needs.
    pub leading: f32,
    /// What the trailing side's content needs.
    pub trailing: f32,
    /// The inner-edge paddings from `platform::centring_padding`.
    pub leading_pad: f32,
    pub trailing_pad: f32,
    /// Between the three parts.
    pub gap: f32,
    /// The least the search may shrink to while it stays centred.
    pub search_least: f32,
    /// The least it may shrink to once it has given up the middle: its
    /// icon.
    pub search_floor: f32,
}

impl HeaderRow {
    /// The width each side's content box must at least keep so that the
    /// search is centred on the window, or `None` when the sides need so
    /// much that even the search at its least width would not be: then it
    /// moves right of the middle rather than shrink further. Both sides keep
    /// this much, so the search takes what remains: the bar less twice the
    /// share, the paddings and the gaps. Its centre is the window's middle
    /// whatever the paddings, since they differ by what the window's edges
    /// take (`platform::centring_padding`).
    pub fn centred_share(self) -> Option<f32> {
        let share = self.leading.max(self.trailing);
        let used = 2. * share + self.leading_pad + self.trailing_pad + 2. * self.gap;
        (used + self.search_least <= self.bar).then_some(share)
    }

    /// The search's width then: the room left, up to its own width.
    pub fn centred_search(self, share: f32, widest: f32) -> f32 {
        let room = self.bar - 2. * share - self.leading_pad - self.trailing_pad - 2. * self.gap;
        room.min(widest)
    }

    /// Whether the sides keep the padding that centres the search. Where the
    /// switcher is not a fixed width there is plenty of room and they do; where
    /// it is, they do only while the search can still be centred.
    pub fn keeps_centring_padding(self, fixed_switcher: bool) -> bool {
        !fixed_switcher || self.centred_share().is_some()
    }

    /// The search's width, centred or not. Once it has given up the middle,
    /// the sides keep their content's width without padding (the controls
    /// after the bar are outside it, so padding for them would only add to the
    /// sides' minimum), and the search takes what is left, down to its
    /// floor, so the row fits the bar for as long as the floor allows.
    pub fn search_width(self, widest: f32) -> f32 {
        match self.centred_share() {
            Some(share) => self.centred_search(share, widest),
            None => {
                let room = self.bar - self.leading - self.trailing - 2. * self.gap;
                room.clamp(self.search_floor, widest)
            }
        }
    }
}

/// One of Kit's caption buttons is as wide as its title bar is high, in
/// pixels whatever the text size.
const CAPTION_BUTTON_WIDTH: f32 = 34.;

/// The width of `count` caption buttons.
pub fn caption_buttons_width(count: usize) -> Pixels {
    px(CAPTION_BUTTON_WIDTH * count as f32)
}

/// What the header's two sides pad on their inner edges so the search
/// between them is centred on the window, not on the bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CentringPadding {
    /// The leading side's padding on its trailing edge.
    pub leading_side: Pixels,
    /// The trailing side's padding on its leading edge.
    pub trailing_side: Pixels,
}

/// The bar runs from the leading inset to the trailing controls, and the
/// content stops a trailing inset short of those. The sides share what the
/// search leaves, so they must differ in width by the room the insets and
/// controls take unequally: the leading side pads by the trailing inset and
/// the controls, the trailing side by the leading inset.
pub fn centring_padding(insets: HeaderInsets, rem_size: Pixels) -> CentringPadding {
    let length = |length: DefiniteLength| length.to_pixels(px(0.).into(), rem_size);
    CentringPadding {
        leading_side: length(insets.trailing) + insets.controls,
        trailing_side: length(insets.leading),
    }
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

/// A control's side within an ordered pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Leading,
    Trailing,
}

/// The default button's side on this platform ([`Platform::default_button`]).
pub const fn default_button() -> Side {
    Platform::current().default_button()
}

/// The system credential store on this platform ([`Platform::credential_store`]).
pub const fn credential_store() -> &'static str {
    Platform::current().credential_store()
}

/// The header's insets on this platform ([`Platform::header_insets`]).
pub fn header_insets() -> HeaderInsets {
    Platform::current().header_insets()
}

/// The header's insets in this window: where the platform draws caption
/// buttons, the ones the window shows. Linux draws them only when the
/// window is client-side decorated, and a window manager may not honour
/// minimize or maximize; Close is always drawn.
pub fn header_insets_in(window: &Window) -> HeaderInsets {
    let platform = Platform::current();
    let mut insets = platform.header_insets();
    let drawn = match platform {
        Platform::MacOs => false,
        Platform::Windows => true,
        Platform::Linux => matches!(
            window.window_decorations(),
            gpui_kit::Decorations::Client { .. }
        ),
    };
    if drawn {
        let supported = window.window_controls();
        let count = 1 + usize::from(supported.minimize) + usize::from(supported.maximize);
        insets.controls = caption_buttons_width(count);
    }
    insets
}

/// The primary modifier's label on this platform
/// ([`Platform::primary_modifier`]).
pub const fn primary_modifier() -> &'static str {
    Platform::current().primary_modifier()
}

/// The Copy link tooltip on this platform ([`Platform::copy_link_tooltip`]).
pub const fn copy_link_tooltip() -> &'static str {
    Platform::current().copy_link_tooltip()
}

/// The terminal's shortcuts on this platform ([`Platform::terminal_keys`]).
pub const fn terminal_keys() -> TerminalKeys {
    Platform::current().terminal_keys()
}

/// Where the app's menus go on this platform ([`Platform::app_menu`]).
pub const fn app_menu() -> AppMenu {
    Platform::current().app_menu()
}

/// Binds this platform's own keys ([`Platform::key_bindings`]) and answers
/// the window and app actions anywhere, so they work in every window and
/// with nothing focused. The window ones act on the active window once the
/// action that asked has finished with it.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys(Platform::current().key_bindings());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| on_active_window(cx, Window::minimize_window));
    cx.on_action(|_: &Zoom, cx| on_active_window(cx, Window::zoom_window));
}

fn on_active_window(cx: &mut App, act: fn(&Window)) {
    cx.defer(move |cx| {
        if let Some(window) = cx.active_window() {
            _ = window.update(cx, |_, window, _| act(window));
        }
    });
}

/// The menu button, where there is no menu bar ([`AppMenu::Button`]): a
/// ghost icon button, its tooltip "Menu" with its key.
pub fn menu_button(id: impl Into<gpui_kit::ElementId>) -> Button {
    Button::new(id)
        .ghost()
        .size(dp(24.))
        .icon(IconName::Menu)
        .accessibility_label("Menu")
        .tooltip_with_action("Menu", &OpenAppMenu, None)
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

    #[test]
    fn a_variable_is_set_in_each_shells_own_way_and_quoted() {
        for posix in [Platform::MacOs, Platform::Linux] {
            assert_eq!(posix.set_variable("A", "/a b/it's"), "A='/a b/it'\\''s' ");
        }
        assert_eq!(
            Platform::Windows.set_variable("A", "C:\\a b\\it's"),
            "$env:A='C:\\a b\\it''s'; "
        );
    }

    #[test]
    fn the_copy_link_tooltip_names_each_platforms_keys() {
        assert!(Platform::MacOs.copy_link_tooltip().contains("Command"));
        for other in [Platform::Linux, Platform::Windows] {
            assert!(other.copy_link_tooltip().contains("Ctrl-Shift-C"));
        }
        assert_eq!(copy_link_tooltip(), Platform::current().copy_link_tooltip());
    }

    #[test]
    fn default_button_follows_each_platforms_order() {
        assert_eq!(Platform::MacOs.default_button(), Side::Trailing);
        assert_eq!(Platform::Linux.default_button(), Side::Trailing);
        assert_eq!(Platform::Windows.default_button(), Side::Leading);
        assert_eq!(default_button(), Platform::current().default_button());
    }

    #[test]
    fn credentials_name_each_platforms_store() {
        assert_eq!(Platform::MacOs.credential_store(), "the Keychain");
        assert_eq!(Platform::Windows.credential_store(), "Credential Manager");
        assert_eq!(Platform::Linux.credential_store(), "the Secret Service");
        assert_eq!(credential_store(), Platform::current().credential_store());
    }

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

    /// The sides' padding follows the insets: equal where they are, and
    /// unequal with the traffic lights, with Windows' caption buttons and
    /// with both, so the centred search lands on the window's middle on every
    /// platform, not only the one a test runs on.
    #[test]
    fn the_sides_pad_by_the_other_edges_room() {
        let rem = px(16.);
        let pad = |leading: f32, trailing: f32, controls: f32| {
            centring_padding(
                HeaderInsets {
                    leading: px(leading).into(),
                    trailing: px(trailing).into(),
                    controls: px(controls),
                },
                rem,
            )
        };
        // Equal insets, nothing after the bar: both sides pad 12.
        assert_eq!(pad(12., 12., 0.).leading_side, px(12.));
        assert_eq!(pad(12., 12., 0.).trailing_side, px(12.));
        // macOS: 80 pt leading, so the trailing side pads 80 and the
        // leading side 12.
        assert_eq!(pad(80., 12., 0.).leading_side, px(12.));
        assert_eq!(pad(80., 12., 0.).trailing_side, px(80.));
        // Windows: three caption buttons after the bar.
        assert_eq!(pad(12., 12., 102.).leading_side, px(114.));
        assert_eq!(pad(12., 12., 102.).trailing_side, px(12.));
        // Both.
        assert_eq!(pad(80., 12., 102.).leading_side, px(114.));
        assert_eq!(pad(80., 12., 102.).trailing_side, px(80.));
        // The sides' widths differ by what the edges take, and the search
        // is the window's middle: a 1000 px window, a 320 search, no gaps.
        for (leading, trailing, controls) in [(12., 12., 0.), (80., 12., 0.), (12., 12., 102.)] {
            let pad = pad(leading, trailing, controls);
            let bar = 1000. - leading - controls - trailing;
            let share =
                (bar - 320. - f32::from(pad.leading_side) - f32::from(pad.trailing_side)) / 2.;
            let left = leading + share + f32::from(pad.leading_side);
            assert!(
                (left + 160. - 500.).abs() < 0.01,
                "{leading} {trailing} {controls}: the search is centred at {}",
                left + 160.
            );
        }
    }

    /// Windows has the three caption buttons, as wide as Kit's title bar is
    /// high; macOS and Linux have none before a window says otherwise.
    #[test]
    fn only_windows_counts_caption_buttons_before_a_window_says() {
        assert_eq!(Platform::Windows.header_insets().controls, px(102.));
        assert_eq!(Platform::MacOs.header_insets().controls, px(0.));
        assert_eq!(Platform::Linux.header_insets().controls, px(0.));
    }

    fn row(leading: f32, trailing: f32, leading_pad: f32, trailing_pad: f32) -> HeaderRow {
        HeaderRow {
            bar: 736.,
            leading,
            trailing,
            leading_pad,
            trailing_pad,
            gap: 6.,
            search_least: 200.,
            search_floor: 28.,
        }
    }

    /// A leading side wider than half of what the search leaves makes the
    /// search shrink around the middle, not slide: both sides keep the
    /// wider one's share.
    #[test]
    fn a_wide_leading_side_shrinks_the_search_around_the_middle() {
        let row = row(212., 88., 12., 12.);
        let share = row.centred_share().expect("there is room to centre");
        assert_eq!(share, 212.);
        let width = row.centred_search(share, 320.);
        assert_eq!(width, 736. - 424. - 24. - 12.);
        assert!((200. ..320.).contains(&width));
    }

    /// With room to spare the search keeps its own width.
    #[test]
    fn a_narrow_leading_side_leaves_the_search_its_width() {
        let row = row(100., 88., 12., 12.);
        let share = row.centred_share().unwrap();
        assert_eq!(row.centred_search(share, 320.), 320.);
    }

    /// Past its least width the search moves rather than shrink: there is
    /// no share to keep, and the sides keep their content's width.
    #[test]
    fn past_its_least_width_the_search_gives_up_the_middle() {
        assert_eq!(row(300., 88., 12., 12.).centred_share(), None);
        // The edges count: the same sides with unequal paddings (macOS's
        // traffic lights, Windows' caption buttons) need more room.
        assert!(row(212., 88., 12., 12.).centred_share().is_some());
        assert_eq!(row(212., 88., 114., 80.).centred_share(), None);
    }

    /// Once the search has given up the middle the row fits the bar, with
    /// Windows' insets too: its padding is dropped, not counted, and the
    /// search takes what the sides leave, down to its floor.
    #[test]
    fn a_row_that_gave_up_the_middle_fits_the_bar_with_windows_insets() {
        // 760 px at text size 16 is 585 dp; less 12 dp each side and three
        // caption buttons (102 px, 82.8 dp).
        let bar = 585. - 24. - 102. * 13. / 16.;
        let row = HeaderRow {
            bar,
            leading: 212.,
            trailing: 88.,
            leading_pad: 12. + 82.8,
            trailing_pad: 12.,
            gap: 6.,
            search_least: 200.,
            search_floor: 28.,
        };
        assert_eq!(row.centred_share(), None);
        let search = row.search_width(320.);
        assert!(search >= 28.);
        assert!(
            row.leading + search + row.trailing + 2. * row.gap <= bar + 0.01,
            "{search} wide in a bar of {bar}"
        );
        // A bar too short even for the floor is the icon's problem, not the
        // padding's: the row overruns by exactly what the floor needs.
        let short = HeaderRow { bar: 300., ..row };
        assert_eq!(short.search_width(320.), 28.);
    }

    /// Where the switcher is not a fixed width the sides keep the padding
    /// that centres the search, whatever the insets: a leading side of
    /// unknown width is not a reason to drop it (it was, on macOS and
    /// Windows, at 1280).
    #[test]
    fn a_free_switcher_keeps_the_centring_padding() {
        let row = HeaderRow {
            bar: 1100.,
            leading: f32::INFINITY,
            trailing: 88.,
            leading_pad: 12.,
            trailing_pad: 80.,
            gap: 6.,
            search_least: 200.,
            search_floor: 28.,
        };
        assert_eq!(row.centred_share(), None);
        assert!(row.keeps_centring_padding(false));
        // A fixed switcher keeps it only while the search can be centred.
        assert!(!row.keeps_centring_padding(true));
        let fixed = HeaderRow {
            leading: 212.,
            ..row
        };
        assert!(fixed.keeps_centring_padding(true));
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

    /// macOS has its menu bar and the keys its menus show; elsewhere the
    /// header's button holds the menus and F10 opens it, and the system's
    /// entries are the desktop's.
    #[test]
    fn macos_has_the_bar_and_the_others_the_button() {
        let keys = |platform: Platform| -> Vec<String> {
            platform
                .key_bindings()
                .iter()
                .flat_map(|binding| binding.keystrokes().iter().map(|key| key.unparse()))
                .collect()
        };
        // GPUI names Command after the host (`cmd` on macOS, `super`
        // elsewhere), so the expected keys are named on the same host.
        let named = |keys: &[&str]| -> Vec<String> {
            keys.iter()
                .map(|key| gpui_kit::Keystroke::parse(key).unwrap().unparse())
                .collect()
        };
        assert_eq!(Platform::MacOs.app_menu(), AppMenu::Bar);
        assert_eq!(
            keys(Platform::MacOs),
            named(&["cmd-,", "cmd-h", "alt-cmd-h", "cmd-m"])
        );
        assert_eq!(Platform::MacOs.system_items(SystemMenu::App).len(), 5);
        assert_eq!(Platform::MacOs.system_items(SystemMenu::Window).len(), 2);
        for platform in [Platform::Windows, Platform::Linux] {
            assert_eq!(platform.app_menu(), AppMenu::Button);
            assert_eq!(keys(platform), named(&["f10"]));
            assert!(platform.system_items(SystemMenu::App).is_empty());
            assert!(platform.system_items(SystemMenu::Window).is_empty());
        }
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
