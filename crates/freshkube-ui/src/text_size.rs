//! The text size the user chooses, in Settings or with Command-= and
//! Command-- (Command-0 restores the default), remembered between launches.
//!
//! Kit sizes its controls from the theme's font size, which `Root` makes the
//! window's rem size. The app's own layout is in `ui::dp` lengths, which are
//! rems too, so text, rows, padding and widths all scale together.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::Theme;
use gpui_kit::{App, Global, KeyBinding};

use crate::ui::BASE_TEXT;

/// The sizes offered, in pixels of the theme's base text.
pub const STEPS: [f32; 6] = [12., 13., 14., 16., 18., 20.];
/// Monospace text keeps its proportion to the base text, as the theme sets
/// them at the default size.
const MONO_RATIO: f32 = 12.5 / BASE_TEXT;

gpui_kit::actions!(text_size, [LargerText, SmallerText, DefaultText]);

struct TextSize {
    size: f32,
    /// Where the choice is saved; `None` keeps it for this session only.
    preferences: Option<PathBuf>,
}

impl Global for TextSize {}

/// Applies the size saved in `preferences`, or the default, and binds the
/// shortcuts. Call it once, after `theme::install`.
pub fn install(preferences: Option<PathBuf>, cx: &mut App) {
    let size = preferences.as_deref().and_then(saved).unwrap_or(BASE_TEXT);
    // Debug builds can start at a size, for visual checks, without
    // saving it.
    #[cfg(debug_assertions)]
    let size = std::env::var("FRESHKUBE_TEXT_SIZE")
        .ok()
        .and_then(|size| size.parse::<f32>().ok())
        .filter(|size| STEPS.contains(size))
        .unwrap_or(size);
    cx.set_global(TextSize { size, preferences });
    cx.bind_keys([
        // Command-Shift-= arrives as `+`. macOS's View menu shows the first
        // binding, so `⌘+` comes before `⌘=`.
        KeyBinding::new("secondary-+", LargerText, None),
        KeyBinding::new("secondary-=", LargerText, None),
        KeyBinding::new("secondary--", SmallerText, None),
        KeyBinding::new("secondary-0", DefaultText, None),
    ]);
    cx.on_action(|_: &LargerText, cx| step(1, cx));
    cx.on_action(|_: &SmallerText, cx| step(-1, cx));
    cx.on_action(|_: &DefaultText, cx| set(BASE_TEXT, cx));
    apply(cx);
}

/// The chosen size, in pixels of base text.
pub fn current(cx: &App) -> f32 {
    cx.try_global::<TextSize>()
        .map_or(BASE_TEXT, |text| text.size)
}

/// Chooses one of `STEPS`, applies it to every window and saves it.
pub fn set(size: f32, cx: &mut App) {
    if !STEPS.contains(&size) || size == current(cx) {
        return;
    }
    if !cx.has_global::<TextSize>() {
        return;
    }
    let text = cx.global_mut::<TextSize>();
    text.size = size;
    let preferences = text.preferences.clone();
    apply(cx);
    if let Some(path) = preferences {
        cx.background_executor()
            // A failed save only means the next launch starts at the default.
            .spawn(async move {
                let _ = save(&path, size);
            })
            .detach();
    }
}

fn step(by: isize, cx: &mut App) {
    let at = STEPS
        .iter()
        .position(|size| *size == current(cx))
        .unwrap_or(1);
    let next = at.saturating_add_signed(by).min(STEPS.len() - 1);
    set(STEPS[next], cx);
}

/// Writes the size into both registered themes and reloads the current one.
/// Kit reloads a theme's font sizes with it, so an appearance change keeps
/// the size.
fn apply(cx: &mut App) {
    let size = current(cx);
    let theme = Theme::global_mut(cx);
    for config in [&mut theme.light_theme, &mut theme.dark_theme] {
        let mut sized = (**config).clone();
        sized.font_size = Some(size);
        sized.mono_font_size = Some(size * MONO_RATIO);
        *config = Rc::new(sized);
    }
    let mode = theme.mode;
    Theme::change(mode, None, cx);
}

/// The size saved in a preferences file, if it holds one of `STEPS`.
fn saved(path: &Path) -> Option<f32> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let size = value.get("text_size")?.as_f64()? as f32;
    STEPS.contains(&size).then_some(size)
}

/// Sets `text_size` in the preferences file, keeping anything else in it.
fn save(path: &Path, size: f32) -> std::io::Result<()> {
    let mut value = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    value["text_size"] = serde_json::json!(size);
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    std::fs::write(path, format!("{value:#}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("freshkube-text-size-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn a_saved_size_reads_back_and_keeps_other_preferences() {
        let directory = directory("round-trip");
        let path = directory.join("nested").join("preferences.json");
        assert_eq!(saved(&path), None);
        save(&path, 18.).unwrap();
        assert_eq!(saved(&path), Some(18.));
        std::fs::write(&path, r#"{"other": true, "text_size": 16}"#).unwrap();
        save(&path, 20.).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["other"], true);
        assert_eq!(saved(&path), Some(20.));
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn an_unreadable_or_unknown_size_is_ignored() {
        let directory = directory("unknown");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("preferences.json");
        for text in [
            "not json",
            "[]",
            r#"{"text_size": 15}"#,
            r#"{"text_size": "big"}"#,
        ] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(saved(&path), None, "{text}");
        }
        // A file that isn't an object is replaced, not merged into.
        std::fs::write(&path, "[]").unwrap();
        save(&path, 12.).unwrap();
        assert_eq!(saved(&path), Some(12.));
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
